<script setup>
import { onMounted, onBeforeUnmount, ref, watch, nextTick, computed } from 'vue'
import { Terminal } from '@xterm/xterm'
import { FitAddon } from '@xterm/addon-fit'
import { SearchAddon } from '@xterm/addon-search'
import { WebLinksAddon } from '@xterm/addon-web-links'
import { WebglAddon } from '@xterm/addon-webgl'
import { ImageAddon } from '@xterm/addon-image'
import { openUrl } from '@tauri-apps/plugin-opener'
import { getCurrentWebview } from '@tauri-apps/api/webview'
import { readText, writeText } from '@tauri-apps/plugin-clipboard-manager'
import { api, Channel, asBytes, uid } from '../lib/api'
import {
  store,
  panes,
  splitFocused,
  closePane,
  detachPane,
  openMenu,
  openSftp,
  loadVault,
  toggleBroadcast,
  broadcastPeers,
  confirm,
  toast,
  err,
  openCast,
  targetTitle,
} from '../lib/store'
import { termTheme } from '../lib/appearance'
import { actionFor, withHint } from '../lib/keymap'
import { promptInjection, MARKER } from '../lib/prompt'
import { resolveSnippet } from '../lib/snippetvars'
import { createHighlighter } from '../lib/highlight'
import { joinMultipartImages } from '../lib/iipmultipart'
import { LineTracker, suggestFor, firstWord, cursorContext } from '../lib/suggest'
import { Recorder, fmtClock } from '../lib/cast'
import { MarkStream, tildePath, fmtDuration, boxOptions } from '../lib/blocks'
import Icon from './Icon.vue'
import HostStats from './HostStats.vue'
import CommandBar from './CommandBar.vue'
import DiffPanel from './DiffPanel.vue'

const props = defineProps({ leaf: Object, tab: Object, visible: Boolean, split: Boolean })
const emit = defineEmits(['grab'])
const el = ref(null)
const status = ref('connecting')
const message = ref('')
const searching = ref(false)
const query = ref('')
const searchInput = ref(null)
const target = props.leaf.target
/** local | ssh | telnet | serial */
const proto = target.kind === 'local' ? 'local' : target.host.protocol || 'ssh'
const title = computed(() => targetTitle(target))
const focused = computed(() => props.tab.focus === props.leaf.id)
const showStats = computed(() => proto === 'ssh' && !!props.leaf.stats)
/** This pane sends and receives broadcast input. */
const broadcasting = computed(() => !!props.tab.broadcast && !props.leaf.noBroadcast)

let term,
  fit,
  search,
  sid = null,
  ro,
  raf = 0,
  lastSize = ''
const S = () => store.vault.settings || {}
const theme = termTheme

function options() {
  const s = S()
  return {
    fontFamily: s.font_family || 'Menlo, monospace',
    fontSize: s.font_size || 13,
    lineHeight: s.line_height || 1.2,
    cursorStyle: s.cursor_style || 'block',
    cursorBlink: s.cursor_blink ?? true,
    cursorInactiveStyle: s.cursor_inactive || 'outline',
    scrollback: s.scrollback || 10000,
    fontWeight: s.font_weight || 'normal',
    fontWeightBold: s.font_weight_bold || 'bold',
    letterSpacing: +s.letter_spacing || 0,
    minimumContrastRatio: +s.min_contrast || 1,
    drawBoldTextInBrightColors: s.bold_bright ?? true,
    macOptionIsMeta: s.option_meta ?? true,
    theme: theme(),
  }
}

function doFit() {
  // while panes are dragged around the layout changes every few pixels: resize once, on drop
  if (store.layoutDrag || !props.visible || !el.value?.offsetWidth) return
  try {
    fit.fit()
  } catch {
    return
  }
  const key = term.cols + 'x' + term.rows
  if (key !== lastSize && sid && status.value === 'open') {
    lastSize = key
    api.term_resize({ id: sid, cols: term.cols, rows: term.rows }).catch(() => {})
  }
}
const scheduleFit = () => {
  cancelAnimationFrame(raf)
  raf = requestAnimationFrame(doFit)
}

// ---------- autosuggestions ----------
const root = ref(null)
const tracker = new LineTracker()
// project terminals keep their own history (other projects' commands are still a fallback)
const histTarget = target.kind === 'local' ? (target.cwd ? `local:${target.cwd}` : 'local') : target.host.host
const ghost = ref(null) // { text, x, y, w, cw, ch }
let ghostRaf = 0
const refreshGhost = () => {
  cancelAnimationFrame(ghostRaf)
  ghostRaf = requestAnimationFrame(computeGhost)
}
function computeGhost() {
  ghost.value = null
  if (!term || S().autosuggest === false || status.value !== 'open' || inj || barOn.value) return
  if (!tracker.valid || !tracker.buf) return
  const b = term.buffer.active
  if (b.type !== 'normal' || b.viewportY !== b.baseY) return // full-screen app or scrolled back
  const ctx = cursorContext(term)
  // only when the typed text is visibly echoed (not a password prompt) and nothing is drawn after it
  if (!ctx || ctx.free < 1 || !ctx.before.endsWith(tracker.buf)) return
  const text = suggestFor(tracker.buf, {
    history: store.vault.history,
    snippets: store.vault.snippets,
    target: histTarget,
  })
  if (!text) return
  const screen = term.element?.querySelector('.xterm-screen')
  if (!screen || !root.value) return
  const r = screen.getBoundingClientRect()
  const o = root.value.getBoundingClientRect()
  const cw = r.width / term.cols
  const ch = r.height / term.rows
  const x = r.left - o.left + b.cursorX * cw
  ghost.value = { text, x, y: r.top - o.top + b.cursorY * ch, w: Math.min(r.right - o.left - x, ctx.free * cw), cw, ch }
}
function remember(line) {
  const cmd = line.trim()
  if (!cmd || line.startsWith(' ')) return // leading space = "don't remember", like HISTCONTROL=ignorespace
  const h = (store.vault.history ||= [])
  const i = h.findIndex((x) => x.cmd === cmd && x.target === histTarget)
  if (i >= 0) h.splice(i, 1)
  h.push({ cmd, target: histTarget, at: Math.floor(Date.now() / 1000) })
  if (h.length > 2000) h.splice(0, h.length - 2000)
  if (S().save_history !== false) api.history_add({ cmd, target: histTarget }).catch(() => {})
}
/** Keys/text on their way to the shell: follow the line, remember finished commands. */
function track(d) {
  const ctx = d === '\r' && cursorContext(term) // read the screen before the shell reacts
  const line = tracker.input(d)
  if (line != null && ctx?.before.endsWith(line)) remember(line)
  ghost.value = null
}
function acceptGhost(word) {
  const t = word ? firstWord(ghost.value.text) : ghost.value.text
  tracker.input(t)
  typed(t)
  ghost.value = null
}

// ---------- K-Shell prompt ----------
// Once the shell is idle at its first prompt, type the prompt snippet and hide everything
// until the snippet's marker comes back, then erase the old prompt line.
const MARK = new TextEncoder().encode(MARKER)
let inj = null // { id, phase: 'wait' | 'hide', seen, chunks, quiet, max }

/**
 * What to type into a fresh shell, or null: the K-Shell prompt, the working-directory
 * reporter (OSC 7) and, after a reconnect or a reopened tab, a cd back to the last folder.
 */
function injectionPlan() {
  const s = S()
  const local = target.kind === 'local'
  if (proto === 'telnet' || proto === 'serial') return null // a device's console, not a shell we know
  if (target.container) return null // a container's sh may not be bash/zsh, and a cd would miss
  // Only into a plain bash/zsh login shell: a startup command (tmux attach, sudo -i…) would
  // receive the line instead.
  const safe = local
    ? store.platform.os !== 'windows' &&
      !target.startup_command?.trim() &&
      (!s.local_shell?.trim() || /(^|\/)(bash|zsh)$/.test(s.local_shell.trim()))
    : !target.host.startup_command?.trim()
  if (!safe) return null
  const scope = s.prompt_scope || 'all'
  const prompt =
    s.prompt_enabled && s.prompt_segments?.length && scope !== (local ? 'ssh' : 'local') ? s.prompt_segments : null
  const cwd = s.track_cwd !== false
  const cd = !local && cwd ? props.leaf.cwd || '' : '' // local shells are started in the folder directly
  const boxScope = boxOptions(s).scope
  const blocks = !!s.input_bar && boxScope !== (local ? 'ssh' : 'local')
  return prompt || cwd || cd || blocks ? { prompt, cwd, cd, blocks } : null
}
function cancelPrompt() {
  if (!inj) return
  clearTimeout(inj.quiet)
  clearTimeout(inj.max)
  const chunks = inj.phase === 'hide' ? inj.chunks : []
  inj = null
  for (const c of chunks) put(c)
}
function armPrompt(id, plan) {
  inj = {
    id,
    plan,
    phase: 'wait',
    seen: false,
    chunks: [],
    quiet: 0,
    max: setTimeout(() => firePrompt(id, true), 5000),
  }
}
function firePrompt(id, force = false) {
  if (!inj || inj.id !== id || sid !== id || inj.phase !== 'wait') return
  if (status.value !== 'open' || (!inj.seen && !force)) return
  clearTimeout(inj.quiet)
  clearTimeout(inj.max)
  inj.phase = 'hide'
  inj.max = setTimeout(cancelPrompt, 5000) // no marker (slow link, not bash/zsh-like): show what happened
  write(promptInjection(inj.plan))
}
function indexOfBytes(hay, needle) {
  outer: for (let i = 0; i <= hay.length - needle.length; i++) {
    for (let j = 0; j < needle.length; j++) if (hay[i + j] !== needle[j]) continue outer
    return i
  }
  return -1
}
/** Show output from the session (and record it when a recording is running). */
function put(b) {
  if (S().input_bar) marks.feed(b)
  else show(b)
}
function show(b) {
  term.write(b)
  rec?.out(b)
}

// ---------- command input at the bottom ----------
// With the setting on, a hook marks the shell's prompts and commands (lib/blocks.js): the
// prompt is kept off screen, commands are typed in CommandBar, and each one is shown as a
// block (folder and branch, the command, its output, then its time and exit code).
const cmdBar = ref(null)
const bar = ref(newBar())
function newBar() {
  return { marked: false, phase: 'idle', info: { cwd: '', home: '', git: '', env: '' }, running: null, last: null }
}
/** The box is shown: the shell has the hook and the session is open. */
const barOn = computed(() => !!S().input_bar && bar.value.marked && status.value === 'open')
/** Command box options from Settings, with their defaults. */
const box = computed(() => boxOptions(S()))
const barIdle = () => barOn.value && bar.value.phase === 'idle'
// A program that takes over the terminal (htop, vim, claude, opencode…) gets the whole pane:
// the box and its "running" line are hidden until it ends. Such programs switch to the
// alternate screen, or turn on mouse reports, bracketed paste, application cursor keys, or
// hide the cursor to draw their own.
const takeover = ref(false)
let cursorHidden = false
const barShown = computed(() => barOn.value && !(bar.value.phase === 'running' && takeover.value))
function checkTakeover() {
  if (takeover.value || !barOn.value || bar.value.phase !== 'running' || !term || !box.value.hide_fullscreen) return
  const m = term.modes
  if (
    term.buffer.active.type === 'alternate' ||
    m.mouseTrackingMode !== 'none' ||
    m.bracketedPasteMode ||
    m.applicationCursorKeysMode ||
    cursorHidden
  )
    takeover.value = true
}
let block = null // { cmd, start, code, ms, deco } of the command running
let revealTimer = 0
let revealFrom = 0
const decos = []
const marks = new MarkStream({ show, event: onMark })

function onMark(name, v) {
  const b = bar.value
  if (name === 'info') b.info = v
  else if (name === 'C') {
    clearTimeout(revealTimer)
    showCursor(true)
    takeover.value = false
    b.phase = 'running'
    if (block) block.start = performance.now()
    else b.running = { cmd: '', since: Date.now() }
  } else if (name === 'D') {
    if (b.phase !== 'running') return
    const ms = block ? performance.now() - block.start : null
    b.last = { code: v, ms }
    if (block) {
      block.code = v
      block.ms = ms
      paintBlock(block)
    }
    block = null
  } else if (name === 'A') {
    refreshChanges()
    clearTimeout(revealTimer)
    const first = !b.marked
    b.marked = true
    b.phase = 'idle'
    b.running = null
    block = null
    ghost.value = null
    if (first) tracker.reset()
    showCursor(false)
    takeover.value = false
    if (focused.value && props.visible) nextTick(() => barIdle() && cmdBar.value?.focus())
  }
}
/**
 * The terminal's cursor: hidden while the shell waits (you type in the box), shown again when
 * a command runs. Written to the screen only; the shell never sees it.
 */
function showCursor(on) {
  // at the prompt, also undo the line editor's modes: their "off" was kept off screen with it
  term?.write(on ? '\x1b[?25h' : '\x1b[?25l\x1b[?2004l\x1b[?1l')
}

const clean = (s) => s.replace(/[\x00-\x09\x0b-\x1f\x7f]/g, '')
/** Write the head of a block (where, then the command) and mark it for its time and exit code. */
function writeHead(cmd) {
  const b = term.buffer.active
  const i = bar.value.info
  const o = box.value
  const where = o.head_where
    ? [i.env && `(${i.env})`, tildePath(i.cwd, i.home), i.git && `git:(${i.git})`].filter(Boolean).join(' ')
    : ''
  let pre = b.cursorX > 0 ? '\r\n' : ''
  if (b.baseY + b.cursorY > 0 && (o.head_rule || o.head_where)) pre += '\r\n' // a blank line between blocks
  const blk = { cmd, start: performance.now(), code: null, ms: null, deco: null }
  term.write(pre, () => {
    const m = term.registerMarker(0)
    const d = m && term.registerDecoration({ marker: m, width: term.cols, layer: 'top' })
    if (!d) return m?.dispose()
    blk.deco = d
    d.onRender((el) => {
      el.classList.add('kblock')
      el.style.width = '100%'
      paintBlock(blk, el)
    })
    decos.push(d)
    if (decos.length > 500) decos.shift().dispose()
  })
  const head =
    (where ? `\x1b[2m${clean(where)}\x1b[22m\r\n` : '') + `\x1b[1m${clean(cmd).replace(/\n/g, '\r\n')}\x1b[22m\r\n`
  term.write(head)
  rec?.out(new TextEncoder().encode(pre + head))
  return blk
}
function paintBlock(blk, el = blk.deco?.element) {
  if (!el) return
  const o = box.value
  el.dataset.meta =
    blk.ms == null || !o.head_time
      ? ''
      : blk.code
        ? `exit ${blk.code} · ${fmtDuration(blk.ms)}`
        : `(${fmtDuration(blk.ms)})`
  el.classList.toggle('bad', !!blk.code && o.head_time)
  el.classList.toggle('norule', !o.head_rule)
}

/** Run a command typed in the box. */
function submit(text) {
  const cmd = text.replace(/\r\n?/g, '\n').replace(/\n+$/, '')
  if (!cmd.trim() || !barIdle()) return
  remember(cmd)
  block = writeHead(cmd)
  // ^U first: anything typed into the shell's own line while the last command ran is dropped
  const line = cmd.includes('\n') && marks.paste ? `\x1b[200~${cmd}\x1b[201~` : cmd.replace(/\n/g, '\r')
  revealFrom = marks.held.length
  write('\x15' + line + '\r')
  if (props.tab.broadcast) for (const p of broadcastPeers(props.tab, props.leaf.id)) p.write(cmd + '\r')
  bar.value.phase = 'running'
  bar.value.running = { cmd, since: Date.now() }
  // No mark back (the command isn't complete, so the shell asks for more, or it asks
  // something before running it): show what the shell printed and let keys go to it.
  clearTimeout(revealTimer)
  revealTimer = setTimeout(() => {
    if (!marks.hidden || bar.value.phase !== 'running') return
    showCursor(true)
    marks.reveal(revealFrom)
  }, 1200)
  term.focus()
}
/** Text for the box from a paste, a snippet or another pane, while it is shown. */
function toBar(t, run = false) {
  if (run) return submit(t)
  cmdBar.value?.insert(t)
}
/** Names in a folder of the machine the shell runs on, for Tab in the box. */
const listDir = (dir) => api.path_list({ host: target.kind === 'local' ? null : target.host, dir })
/** Git branches of the repository the shell is in, for the branch chip. */
const listBranches = (dir) => api.git_branches({ host: target.kind === 'local' ? null : target.host, dir })

// ---------- uncommitted git changes: the chip of the box, and the panel it opens ----------
const changes = ref(null) // { files, added, removed }
const diffOpen = ref(false)
const diffVersion = ref(0)
const diffWidth = ref(50) // % of the pane the changes panel takes
let changesBusy = false
let changesAgain = false
let changesStale = false
let changesKey = ''
/**
 * Count the changes again; the shell has just come back to its prompt. One git run at a time
 * (prompts in a row share the next one), and none while the pane is out of sight: it counts
 * when it is shown again.
 */
async function refreshChanges() {
  const i = bar.value.info
  if (!i.git) {
    changes.value = null
    diffOpen.value = false
    changesKey = ''
    return
  }
  if (!props.visible) return void (changesStale = true)
  if (changesBusy) return void (changesAgain = true)
  changesBusy = true
  changesStale = false
  try {
    const list = await api.git_changes({ host: target.kind === 'local' ? null : target.host, dir: i.cwd })
    const key = i.cwd + '\n' + list.map((c) => `${c.path}\t${c.added}\t${c.removed}`).join('\n')
    if (key !== changesKey) {
      changesKey = key
      changes.value = {
        files: list.length,
        added: list.reduce((n, c) => n + c.added, 0),
        removed: list.reduce((n, c) => n + c.removed, 0),
      }
    }
    // the panel reads the diff again (it skips the work itself when the text is the same)
    if (diffOpen.value) diffVersion.value++
  } catch {
    changes.value = null
    changesKey = ''
  } finally {
    changesBusy = false
  }
  if (changesAgain) {
    changesAgain = false
    refreshChanges()
  }
}
watch(
  () => props.visible,
  (v) => v && changesStale && refreshChanges(),
)
const loadDiff = () => api.git_diff({ host: target.kind === 'local' ? null : target.host, dir: bar.value.info.cwd })
function focusPane() {
  if (barIdle()) cmdBar.value?.focus()
  else term?.focus()
}
watch(
  () => !!S().input_bar,
  (on) => {
    if (on) return
    // turned off: show the prompt that was kept off screen, and the shell is used as usual
    if (marks.hidden) marks.reveal()
    showCursor(true)
    bar.value = newBar()
    block = null
  },
)
function output(b) {
  if (!inj) return put(b)
  if (inj.phase === 'wait') {
    put(b)
    inj.seen = true
    clearTimeout(inj.quiet)
    const id = inj.id
    inj.quiet = setTimeout(() => firePrompt(id), 450)
    return
  }
  inj.chunks.push(b)
  const all = new Uint8Array(inj.chunks.reduce((n, c) => n + c.length, 0))
  let o = 0
  for (const c of inj.chunks) all.set(c, (o += c.length) - c.length)
  const i = indexOfBytes(all, MARK)
  if (i < 0) return
  clearTimeout(inj.max)
  inj = null
  put('\r\x1b[2K')
  put(all.subarray(i + MARK.length))
}

// ---------- recording (.cast) ----------
// What the pane shows is recorded, never what is typed: passwords typed at a prompt stay out.
let rec = null
const recording = ref(null) // { since } while recording
const recClock = ref('0:00')
let recTimer = 0
async function startRecording() {
  if (rec || !term) return
  const r = new Recorder()
  rec = r
  try {
    await r.start({ label: title.value, cols: term.cols, rows: term.rows })
  } catch (e) {
    rec = null
    return err(e)
  }
  recording.value = { since: Date.now() }
  recClock.value = '0:00'
  recTimer = setInterval(() => (recClock.value = fmtClock(r.elapsed())), 1000)
}
async function stopRecording({ quiet = false } = {}) {
  const r = rec
  if (!r) return
  rec = null
  recording.value = null
  clearInterval(recTimer)
  try {
    const path = await r.stop()
    if (quiet) return
    if (r.failed) err(`Part of the recording couldn't be written: ${r.failed}`)
    toast(`Recording saved: ${path}`, 'info', 10000, [
      { label: 'Play', run: () => openCast(path) },
      { label: 'Show in folder', run: () => api.rec_reveal({ path }).catch(err) },
    ])
  } catch (e) {
    if (!quiet) err(e)
  }
}
const toggleRecording = () => (rec ? stopRecording() : startRecording())

async function connect() {
  const id = uid()
  sid = id
  status.value = 'connecting'
  message.value = ''
  cancelPrompt()
  tracker.reset()
  marks.reset()
  bar.value = newBar()
  changes.value = null
  diffOpen.value = false
  block = null
  clearTimeout(revealTimer)
  const plan = injectionPlan()
  if (plan) armPrompt(id, plan)
  const ch = new Channel()
  ch.onmessage = (m) => {
    if (sid !== id) return
    const b = asBytes(m)
    if (b) return output(b)
    const ev = typeof m === 'string' ? JSON.parse(m) : m
    if (ev.type === 'exit') {
      cancelPrompt()
      status.value = 'closed'
      // An SSH shell that ends without an exit status lost its connection: try again by itself.
      // (Telnet and serial have no exit status, so a dropped line can't be told from a logout.)
      if (proto === 'ssh' && ev.code == null && S().auto_reconnect !== false) {
        term.write('\r\n\x1b[33m[connection lost]\x1b[0m\r\n')
        return scheduleReconnect()
      }
      term.write(
        `\r\n\x1b[2m[session closed${ev.code != null ? ' · exit ' + ev.code : ''}] press Enter to reconnect\x1b[0m\r\n`,
      )
    }
  }
  try {
    doFit()
    if (target.kind === 'local') {
      await api.term_open_local({
        id,
        cols: term.cols,
        rows: term.rows,
        shell: S().local_shell?.trim() || null,
        // last folder the shell reported (reconnect, reopened tab), else where it was opened
        cwd: props.leaf.cwd || target.cwd || S().local_cwd?.trim() || null,
        startupCommand: target.startup_command || null,
        container: target.container || null,
        onData: ch,
      })
    } else if (proto === 'telnet') {
      const port = target.host.port || 23
      term.write(`\x1b[2mConnecting to ${target.host.host}${port !== 23 ? ':' + port : ''} (Telnet)…\x1b[0m\r\n`)
      await api.term_open_telnet({ id, host: target.host, cols: term.cols, rows: term.rows, onData: ch })
    } else if (proto === 'serial') {
      const h = target.host
      const parity = { odd: 'O', even: 'E' }[h.parity] || 'N'
      term.write(
        `\x1b[2mOpening ${h.host} at ${h.baud_rate || 9600} baud, ${h.data_bits || 8}${parity}${h.stop_bits || 1}…\x1b[0m\r\n`,
      )
      await api.term_open_serial({ id, host: h, onData: ch })
    } else {
      term.write(
        `\x1b[2mConnecting to ${target.host.username ? target.host.username + '@' : ''}${target.host.host}…\x1b[0m\r\n`,
      )
      const container = target.container || null
      await api.term_open_ssh({ id, host: target.host, cols: term.cols, rows: term.rows, container, onData: ch })
      const saved = store.vault.hosts.find((h) => h.id === target.host.id)
      if (saved && !saved.os)
        api
          .host_detect_os({ host: saved })
          .then(loadVault)
          .catch(() => {})
    }
    status.value = 'open'
    retries = 0
    lastSize = term.cols + 'x' + term.rows
    scheduleFit()
  } catch (e) {
    cancelPrompt()
    if (target.kind === 'local' && props.leaf.cwd) {
      delete props.leaf.cwd // the last folder is gone: open where the terminal normally starts
      return connect()
    }
    status.value = 'error'
    message.value = String(e)
    if (retries > 0) {
      // reconnecting after a drop: the network may still be down, keep trying
      term.write(`\x1b[2m✖ ${String(e)}\x1b[0m\r\n`)
      return scheduleReconnect()
    }
    term.write(`\r\n\x1b[31m✖ ${String(e)}\x1b[0m\r\n\x1b[2mPress Enter to retry\x1b[0m\r\n`)
  }
}

// ---------- auto-reconnect (SSH) ----------
const RETRY_DELAYS = [1, 2, 4, 8, 15, 30, 30, 30, 30, 30] // seconds; about 3 minutes in all
let retries = 0,
  retryTimer = 0
const reconnecting = ref(false)
function scheduleReconnect() {
  clearTimeout(retryTimer)
  if (retries >= RETRY_DELAYS.length) {
    reconnecting.value = false
    retries = 0
    term.write('\x1b[2mGave up reconnecting. Press Enter to try again.\x1b[0m\r\n')
    return
  }
  const wait = RETRY_DELAYS[retries++]
  reconnecting.value = true
  term.write(
    `\x1b[2mReconnecting in ${wait}s (attempt ${retries}/${RETRY_DELAYS.length}) · Enter: now · Ctrl+C: stop\x1b[0m\r\n`,
  )
  retryTimer = setTimeout(reconnectNow, wait * 1000)
}
function reconnectNow() {
  clearTimeout(retryTimer)
  reconnecting.value = false
  if (unmounted) return
  connect()
}
function stopReconnect() {
  clearTimeout(retryTimer)
  reconnecting.value = false
  retries = 0
  term.write('\x1b[2mStopped reconnecting. Press Enter to reconnect.\x1b[0m\r\n')
}
// Back online (Wi-Fi switched, laptop woke up): don't wait for the timer.
function onOnline() {
  if (reconnecting.value) reconnectNow()
}

const write = (data) => {
  if (sid && status.value === 'open') api.term_write({ id: sid, data }).catch(() => {})
}

// ---------- broadcast input ----------
// xterm's onData also carries the terminal's own replies to the program running in it
// (cursor position, device attributes, window/cell size and graphics queries, colour
// queries…), focus events and mouse reports. Those belong to this pane only.
const AUTOMATIC =
  /^\x1b(\[[\d;?>]*[RcnSt]|\[\?[\d;]*\$y|\[[IO]$|\[<[\d;]*[Mm]|\[M|\][\s\S]*(\x07|\x1b\\)$|P[\s\S]*\x1b\\$)/
/** Send typed text to this pane and, with broadcast on, to the other panes of the tab. */
function typed(d) {
  write(d)
  if (props.tab.broadcast && !AUTOMATIC.test(d)) for (const p of broadcastPeers(props.tab, props.leaf.id)) p.input(d)
}
/** A whole command (snippet): every broadcasting pane runs it. */
function runCommand(cmd) {
  tracker.reset()
  write(cmd)
  if (props.tab.broadcast) for (const p of broadcastPeers(props.tab, props.leaf.id)) p.write(cmd)
}
// ---------- keyword highlighting ----------
let hl = null
const configureHighlight = () => hl?.configure(S().highlight === false ? [] : S().highlight_rules, theme())
watch(
  () => [
    S().highlight,
    S().highlight_rules,
    S().term_theme,
    S().term_theme_light,
    S().term_theme_sync,
    store.systemDark,
  ],
  configureHighlight,
  {
    deep: true,
  },
)

// ---------- renderer ----------
// WebGL draws fast but every context holds GPU memory and a glyph atlas of a few MB, and
// the webview only allows a handful of contexts at once. Panes of tabs you aren't looking
// at give theirs back after a while (xterm falls back to its DOM renderer, which costs
// nothing while hidden) and take a new one when they are shown again.
const GL_IDLE_MS = 30_000
let gl = null,
  glTimer = 0
function attachGl() {
  if (gl || !term) return
  try {
    const a = new WebglAddon()
    a.onContextLoss(() => {
      a.dispose()
      if (gl === a) gl = null
    })
    term.loadAddon(a)
    gl = a
  } catch {
    gl = null // no WebGL: the DOM renderer stays
  }
}
function releaseGl() {
  if (!gl) return
  const a = gl
  gl = null
  a.dispose()
}

// ---------- inline images (Sixel, iTerm2 inline images) ----------
// img2sixel, imgcat, chafa, timg… draw pictures into the terminal. Decoded images are kept
// per pane up to the memory limit (oldest dropped first, shown as a placeholder).
let img = null,
  imgJoin = null
function configureImages() {
  if (!term) return
  const s = S()
  if (s.images === false) {
    imgJoin?.dispose()
    img?.dispose()
    img = imgJoin = null
    return
  }
  const mb = +s.image_memory_mb || 64
  if (img) {
    if (img.storageLimit !== mb) img.storageLimit = mb
    return
  }
  try {
    img = new ImageAddon({ storageLimit: mb })
    term.loadAddon(img)
    imgJoin = joinMultipartImages(term, img) // imgcat's default, multi-part form
  } catch {
    img = imgJoin = null
  }
}

// ---------- files dropped from the OS: type their paths ----------
const dropping = ref(false)
let unlistenDrop = null,
  unmounted = false
/** Quote a path for the shell: POSIX single quotes, or double quotes for a Windows local shell. */
function shellQuote(p) {
  if (target.kind === 'local' && store.platform.os === 'windows') return /[\s&()^%!;,'"]/.test(p) ? `"${p}"` : p
  return /^[\w@%+=:,./-]+$/.test(p) ? p : `'${p.replace(/'/g, `'\\''`)}'`
}
/** Is the (physical-pixel) drop position over this pane? */
function overMe(pos) {
  if (store.active !== props.tab.id || !props.visible || !root.value) return false
  const r = window.devicePixelRatio || 1
  const x = pos.x / r,
    y = pos.y / r
  const b = root.value.getBoundingClientRect()
  return x >= b.left && x < b.right && y >= b.top && y < b.bottom
}
function onDragDrop(ev) {
  const p = ev.payload
  if (p.type === 'enter' || p.type === 'over') dropping.value = status.value === 'open' && overMe(p.position)
  else if (p.type === 'drop') {
    const hit = dropping.value || (status.value === 'open' && overMe(p.position))
    dropping.value = false
    if (!hit || !p.paths?.length) return
    props.tab.focus = props.leaf.id
    const paths = p.paths.map(shellQuote).join(' ') + ' '
    if (barIdle()) return toBar(paths)
    term.paste(paths)
    term.focus()
  } else dropping.value = false
}

async function paste() {
  let t
  try {
    t = await readText()
  } catch {
    return
  }
  if (!t) return
  if (barIdle()) return toBar(t.replace(/\r\n?/g, '\n')) // nothing runs until Enter
  // Several lines run as several commands the moment they land in a shell: ask first.
  if (S().paste_warn !== false && /[\r\n]/.test(t)) {
    const lines = t.replace(/\r\n?/g, '\n').replace(/\n$/, '').split('\n')
    const preview = lines.slice(0, 6).join('\n') + (lines.length > 6 ? `\n… ${lines.length - 6} more` : '')
    const ok = await confirm(
      `Paste ${lines.length} line${lines.length === 1 ? '' : 's'}?`,
      lines.length === 1
        ? 'It ends with a line break, so it runs as soon as it is pasted.'
        : 'Each line may run as a command as soon as it is pasted.',
      { okText: 'Paste', danger: false, detail: preview },
    )
    term.focus()
    if (!ok) return
  }
  term.paste(t)
}

// ---------- bell ----------
const ringing = ref(false)
let audio = null
function bell() {
  const mode = S().bell || 'visual'
  if (mode === 'visual' || mode === 'both') {
    ringing.value = false
    requestAnimationFrame(() => (ringing.value = true))
    setTimeout(() => (ringing.value = false), 180)
  }
  if (mode === 'sound' || mode === 'both') {
    try {
      audio ||= new AudioContext()
      const o = audio.createOscillator(),
        g = audio.createGain()
      o.frequency.value = 880
      g.gain.setValueAtTime(0.08, audio.currentTime)
      g.gain.exponentialRampToValueAtTime(0.0001, audio.currentTime + 0.15)
      o.connect(g).connect(audio.destination)
      o.start()
      o.stop(audio.currentTime + 0.15)
    } catch {}
  }
}
function copySel() {
  const s = term.getSelection()
  if (s) writeText(s).catch(() => {})
}

function doSearch(dir = 1) {
  if (!query.value) return
  const o = {
    caseSensitive: false,
    decorations: { activeMatchColorOverviewRuler: '#e07a45', matchOverviewRuler: '#888' },
  }
  dir > 0 ? search.findNext(query.value, o) : search.findPrevious(query.value, o)
}
async function openSearch() {
  searching.value = true
  await nextTick()
  searchInput.value?.focus()
}

// ---------- server stats panel (SSH) ----------
function toggleStats() {
  if (proto !== 'ssh') return
  props.leaf.stats = !props.leaf.stats
  if (!props.leaf.stats) term?.focus()
}

function splitHere(dir) {
  props.tab.focus = props.leaf.id
  splitFocused(dir)
}
function closeSearch() {
  searching.value = false
  search.clearDecorations()
  term.focus()
}
function menu(e) {
  props.tab.focus = props.leaf.id
  openMenu(e, [
    { label: 'Copy', icon: 'copy', action: copySel },
    { label: 'Paste', icon: 'download', action: paste },
    { label: 'Clear', icon: 'x', action: () => term.clear() },
    { label: 'Find…', icon: 'search', action: openSearch },
    {
      label: recording.value ? 'Stop recording' : 'Start recording',
      icon: recording.value ? 'stop' : 'record',
      action: toggleRecording,
    },
    '-',
    { label: 'Split right', icon: 'splitH', action: () => splitFocused('row') },
    { label: 'Split down', icon: 'splitV', action: () => splitFocused('col') },
    ...(proto === 'ssh'
      ? [
          { label: 'Open SFTP here', icon: 'folder', action: () => openSftp(target.host, { right: props.leaf.cwd }) },
          {
            label: props.leaf.stats ? 'Hide server stats' : 'Show server stats',
            icon: 'activity',
            action: toggleStats,
          },
        ]
      : proto === 'local'
        ? [
            {
              label: 'Open in file manager',
              icon: 'folder',
              action: () => openSftp(null, { left: props.leaf.cwd || target.cwd }),
            },
          ]
        : []),
    ...(store.vault.snippets.length
      ? [
          '-',
          ...store.vault.snippets.slice(0, 10).map((s) => ({
            label: s.name,
            icon: 'code',
            action: () => {
              resolveSnippet(s).then((cmd) => {
                if (cmd != null) runCommand(cmd + '\r')
                term.focus()
              })
            },
          })),
        ]
      : []),
    '-',
    ...(props.split
      ? [
          {
            label: props.tab.broadcast ? 'Stop broadcasting input' : 'Broadcast input to all panes',
            icon: 'broadcast',
            action: () => toggleBroadcast(props.tab),
          },
        ]
      : []),
    ...(props.tab.broadcast
      ? [
          {
            label: props.leaf.noBroadcast ? 'Include this pane in broadcast' : 'Leave this pane out of broadcast',
            icon: 'broadcast',
            action: () => (props.leaf.noBroadcast = !props.leaf.noBroadcast),
          },
        ]
      : []),
    ...(props.split
      ? [{ label: 'Move to new tab', icon: 'popout', action: () => detachPane(props.tab, props.leaf.id) }]
      : []),
    { label: 'Close pane', icon: 'x', danger: true, action: () => closePane(props.tab, props.leaf.id) },
  ])
}

onMounted(async () => {
  term = new Terminal({
    ...options(),
    allowProposedApi: true,
    rightClickSelectsWord: false,
  })
  fit = new FitAddon()
  search = new SearchAddon()
  term.loadAddon(fit)
  term.loadAddon(search)
  term.loadAddon(
    new WebLinksAddon((ev, uri) => {
      if (ev.metaKey || ev.ctrlKey) openUrl(uri)
    }),
  )
  term.open(el.value)
  configureImages()
  if (props.visible) attachGl()
  hl = createHighlighter(term)
  configureHighlight()
  term.onWriteParsed(() => hl.schedule())

  term.onData((d) => {
    if (status.value === 'open') {
      // the shell waits for a command: what is typed here goes to the box
      if (barIdle() && !AUTOMATIC.test(d)) return cmdBar.value?.key(d)
      track(d)
      return typed(d)
    }
    if (reconnecting.value && d === '\x03') return stopReconnect()
    if (d === '\r' && status.value !== 'connecting') {
      if (reconnecting.value) return reconnectNow()
      term.clear()
      connect()
    }
  })
  // The user started typing before the prompt was installed: leave the shell alone.
  // (onKey, not onData — onData also carries the terminal's automatic replies to shell queries.)
  term.onKey(() => inj?.phase === 'wait' && cancelPrompt())
  term.onWriteParsed(refreshGhost)
  term.onWriteParsed(checkTakeover)
  // follow whether the program hid the cursor (xterm doesn't say); handled as usual after
  for (const final of ['h', 'l'])
    term.parser.registerCsiHandler({ prefix: '?', final }, (params) => {
      if (params.includes(25)) cursorHidden = final === 'l'
      return false
    })
  term.onCursorMove(refreshGhost)
  term.onScroll(refreshGhost)
  term.onResize(refreshGhost)
  term.onResize(({ cols, rows }) => rec?.resize(cols, rows))
  term.onBell(bell)
  // OSC 7 (file://host/path): the shell's working directory, from our hook or the shell itself
  term.parser.registerOscHandler(7, (data) => {
    const m = /^file:\/\/[^/]*(\/.*)$/.exec(data)
    if (m) {
      let dir = m[1]
      try {
        dir = decodeURIComponent(dir)
      } catch {
        /* sent unencoded */
      }
      if (/^\/[A-Za-z]:\//.test(dir)) dir = dir.slice(1) // file:///C:/… on Windows
      if (props.leaf.cwd !== dir) props.leaf.cwd = dir
    }
    return true
  })
  term.onSelectionChange(() => {
    if (S().copy_on_select && term.hasSelection()) copySel()
  })
  term.attachCustomKeyEventHandler((e) => {
    if (e.type !== 'keydown') return true
    const mac = store.platform.os === 'macos'
    const a = actionFor(e)
    if ((a?.id === 'accept_suggestion' || a?.id === 'accept_suggestion_word') && ghost.value) {
      acceptGhost(a.id === 'accept_suggestion_word')
      e.preventDefault()
      return false
    }
    if (a?.id === 'copy' && term.hasSelection()) {
      copySel()
      return false
    }
    if (a?.id === 'paste') {
      paste()
      e.preventDefault()
      return false
    }
    if (a?.id === 'find') {
      openSearch()
      e.preventDefault()
      return false
    }
    if (a?.id === 'clear') {
      term.clear()
      e.preventDefault()
      return false
    }
    if (mac && e.metaKey) return false // let app shortcuts through
    return true
  })
  el.value.addEventListener('contextmenu', (e) => {
    if (S().right_click_paste && !e.shiftKey && !term.hasSelection()) {
      e.preventDefault()
      paste()
    } else menu(e)
  })
  el.value.addEventListener('mousedown', () => {
    props.tab.focus = props.leaf.id
  })

  panes.set(props.leaf.id, {
    write: (t) => {
      // a snippet ending in a line break runs; one without waits in the box for Enter
      if (barIdle()) return toBar(t.replace(/[\r\n]+$/, ''), /[\r\n]$/.test(t))
      tracker.reset() // snippets run as whole commands
      write(t)
    },
    /** Keystrokes broadcast from another pane of the tab. */
    input: (d) => {
      if (status.value !== 'open') return // never reconnect a closed pane from another pane's Enter
      if (barIdle() && !AUTOMATIC.test(d)) return cmdBar.value?.key(d)
      track(d)
      write(d)
    },
    focus: focusPane,
    toggleRecording,
    toggleStats,
    term,
  })
  ro = new ResizeObserver(scheduleFit)
  ro.observe(el.value)
  window.addEventListener('online', onOnline)
  getCurrentWebview()
    .onDragDropEvent(onDragDrop)
    .then((u) => (unmounted ? u() : (unlistenDrop = u)))
    .catch(() => {})
  await nextTick()
  doFit()
  connect()
  if (focused.value) focusPane()
})

watch(
  () => props.visible,
  (v) => {
    clearTimeout(glTimer)
    if (v)
      nextTick(() => {
        attachGl()
        doFit()
        if (focused.value) focusPane()
      })
    else glTimer = setTimeout(releaseGl, GL_IDLE_MS)
  },
)
watch(
  () => store.layoutDrag,
  (d) => !d && scheduleFit(),
)
watch(focused, (f) => {
  if (f && props.visible) focusPane()
})
watch(
  () => [store.vault.settings, store.systemDark],
  () => {
    if (!term) return
    Object.assign(term.options, options())
    configureImages()
    scheduleFit()
  },
  { deep: true },
)

onBeforeUnmount(() => {
  stopRecording({ quiet: true }) // a closed pane keeps what was recorded
  cancelPrompt()
  clearTimeout(revealTimer)
  cancelAnimationFrame(ghostRaf)
  ro?.disconnect()
  cancelAnimationFrame(raf)
  unmounted = true
  hl?.dispose()
  clearTimeout(glTimer)
  audio?.close().catch(() => {})
  clearTimeout(retryTimer)
  window.removeEventListener('online', onOnline)
  unlistenDrop?.()
  panes.delete(props.leaf.id)
  if (sid) api.term_close({ id: sid }).catch(() => {})
  sid = null
  term?.dispose()
})
</script>

<template>
  <div
    ref="root"
    class="tp"
    :class="{
      focused: focused && split,
      broadcasting,
      dropping,
      ringing,
      withstats: showStats,
      withdiff: diffOpen && barOn,
    }"
    :style="{
      background: theme().background,
      '--term-bg': theme().background,
      '--term-fg': theme().foreground,
      '--diff-w': diffWidth + '%',
    }"
  >
    <div
      v-if="split"
      class="ph"
      :style="{ color: theme().foreground }"
      title="Drag to move this pane"
      @mousedown="emit('grab', $event, title)"
    >
      <span class="st" :class="reconnecting ? 'connecting' : status"></span>
      <span class="t">{{ title }}</span>
      <button v-if="recording" class="recbadge" @click.stop="stopRecording()" title="Stop recording">
        <i></i>REC {{ recClock }}
      </button>
      <button
        v-if="tab.broadcast"
        class="iconbtn bc"
        :class="{ off: leaf.noBroadcast }"
        @click="leaf.noBroadcast = !leaf.noBroadcast"
        :title="
          leaf.noBroadcast
            ? 'Not receiving broadcast input: click to include this pane'
            : 'Receiving broadcast input: click to leave this pane out'
        "
      >
        <Icon name="broadcast" :size="13" />
      </button>
      <button
        v-if="proto === 'ssh'"
        class="iconbtn"
        :class="{ on: leaf.stats }"
        @click="toggleStats"
        :title="withHint(leaf.stats ? 'Hide server stats' : 'Show server stats', 'server_stats')"
      >
        <Icon name="activity" :size="13" />
      </button>
      <button class="iconbtn" @click="splitFocused('row')" title="Split right">
        <Icon name="splitH" :size="13" />
      </button>
      <button class="iconbtn" @click="splitFocused('col')" title="Split down"><Icon name="splitV" :size="13" /></button>
      <button class="iconbtn" @click="detachPane(tab, leaf.id)" title="Move to new tab">
        <Icon name="popout" :size="13" />
      </button>
      <button class="iconbtn" @click="closePane(tab, leaf.id)" title="Close pane"><Icon name="x" :size="13" /></button>
    </div>
    <div v-else class="tools">
      <button class="iconbtn" @click="openSearch" :title="withHint('Find', 'find')">
        <Icon name="search" :size="14" />
      </button>
      <button
        v-if="proto === 'ssh'"
        class="iconbtn"
        @click="toggleStats"
        :title="withHint(leaf.stats ? 'Hide server stats' : 'Show server stats', 'server_stats')"
      >
        <Icon name="activity" :size="14" />
      </button>
      <button class="iconbtn" @click="(e) => menu(e)" title="Snippets & actions">
        <Icon name="code" :size="14" />
      </button>
      <button class="iconbtn" @click="splitHere('row')" :title="withHint('Split right', 'split_right')">
        <Icon name="splitH" :size="14" />
      </button>
      <button class="iconbtn" @click="splitHere('col')" :title="withHint('Split down', 'split_down')">
        <Icon name="splitV" :size="14" />
      </button>
    </div>
    <div v-if="searching" class="find">
      <input
        ref="searchInput"
        v-model="query"
        placeholder="Find"
        @keydown.enter.exact="doSearch(1)"
        @keydown.shift.enter="doSearch(-1)"
        @keydown.esc="closeSearch"
        @input="doSearch(1)"
      />
      <button class="iconbtn" @click="doSearch(-1)"><Icon name="up" :size="13" /></button>
      <button class="iconbtn" @click="doSearch(1)"><Icon name="down" :size="13" /></button>
      <button class="iconbtn" @click="closeSearch">
        <Icon name="x" :size="13" />
      </button>
    </div>
    <div class="body">
      <div class="termcol">
        <div ref="el" class="xt"></div>
        <CommandBar
          v-if="barShown"
          ref="cmdBar"
          :phase="bar.phase"
          :info="bar.info"
          :running="bar.running"
          :last="bar.last"
          :target="histTarget"
          :list="listDir"
          :branches="listBranches"
          :changes="changes"
          :changes-open="diffOpen"
          :options="box"
          :colors="theme()"
          :font="{ family: S().font_family || 'Menlo, monospace', size: S().font_size || 13 }"
          @mousedown="tab.focus = leaf.id"
          @submit="submit"
          @send="typed"
          @clear="term.clear()"
          @find="openSearch"
          @focus-term="term.focus()"
          @changes="diffOpen = !diffOpen"
        />
      </div>
      <DiffPanel
        v-if="diffOpen && barOn"
        :info="bar.info"
        :load="loadDiff"
        :version="diffVersion"
        :colors="theme()"
        :font="{ family: S().font_family || 'Menlo, monospace', size: S().font_size || 13 }"
        @close="diffOpen = false"
        @width="diffWidth = $event"
      />
      <HostStats
        v-if="showStats"
        :host="target.host"
        :active="visible && status === 'open'"
        :style="{ color: theme().foreground }"
        @close="toggleStats"
      />
    </div>
    <div
      v-if="ghost"
      class="ghost"
      :style="{
        left: ghost.x + 'px',
        top: ghost.y + 'px',
        width: ghost.w + 'px',
        height: ghost.ch + 'px',
        lineHeight: ghost.ch + 'px',
        color: theme().foreground,
        fontFamily: S().font_family,
        fontSize: (S().font_size || 13) + 'px',
      }"
    >
      <span v-for="(c, i) in ghost.text" :key="i" :style="{ width: ghost.cw + 'px' }">{{ c }}</span>
    </div>
    <button
      v-if="recording && !split"
      class="recbadge float"
      @click="stopRecording()"
      :title="withHint('Stop recording', 'record')"
    >
      <i></i>REC {{ recClock }}
    </button>
    <div v-if="status === 'connecting' && target.kind === 'ssh'" class="loading"><span class="spin"></span></div>
  </div>
</template>

<style scoped>
.tp {
  position: absolute;
  inset: 0;
  display: flex;
  flex-direction: column;
  border: 1px solid transparent;
}
.tp.focused {
  border-color: color-mix(in srgb, var(--accent) 45%, transparent);
}
.tp.broadcasting {
  border-color: color-mix(in srgb, var(--warn) 55%, transparent);
}
.tp.broadcasting.focused {
  border-color: var(--warn);
}
.tp.ringing::after {
  content: '';
  position: absolute;
  inset: 0;
  pointer-events: none;
  background: color-mix(in srgb, var(--text) 10%, transparent);
  z-index: 5;
}
.tp.dropping,
.tp.dropping.broadcasting {
  border-color: var(--accent);
  box-shadow: inset 0 0 0 1px var(--accent);
}
.ph .bc {
  color: var(--warn);
  opacity: 1;
}
.ph .bc.off {
  color: inherit;
  opacity: 0.45;
}
.body {
  flex: 1;
  min-height: 0;
  display: flex;
}
.termcol {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  container-type: inline-size;
}
.xt {
  flex: 1;
  min-width: 0;
  min-height: 0;
  padding: 6px 4px 4px 10px;
}
/* head of a command block (input at the bottom): a rule above it, time and exit code at the right */
.xt :deep(.kblock) {
  pointer-events: none;
  border-top: 1px solid rgba(127, 127, 127, 0.22);
  margin-top: -1px;
}
/* the time sits on the terminal's background, so a long folder line runs under it instead of through it */
.xt :deep(.kblock)::after {
  content: attr(data-meta);
  position: absolute;
  right: 12px; /* clear of the scrollbar */
  top: 0;
  height: 100%;
  display: flex;
  align-items: center;
  padding: 0 4px 0 22px;
  font-size: 11px;
  color: color-mix(in srgb, var(--term-fg) 55%, var(--term-bg));
  background: linear-gradient(to right, transparent, var(--term-bg) 18px);
}
.xt :deep(.kblock.norule),
.xt :deep(.kblock.norule.bad) {
  border-top-color: transparent;
}
.xt :deep(.kblock[data-meta=''])::after {
  display: none;
}
.xt :deep(.kblock.bad) {
  border-top-color: color-mix(in srgb, var(--danger) 55%, transparent);
  box-shadow: inset 2px 0 0 var(--danger);
}
.xt :deep(.kblock.bad)::after {
  color: var(--danger);
}
.ph {
  height: 26px;
  flex: none;
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 0 4px 0 10px;
  font-size: 11.5px;
  opacity: 0.8;
  border-bottom: 1px solid rgba(127, 127, 127, 0.15);
}
.ph {
  cursor: grab;
}
.ph .t {
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.ph .iconbtn {
  width: 22px;
  height: 22px;
  color: inherit;
}
.st {
  width: 7px;
  height: 7px;
  border-radius: 50%;
  background: var(--warn);
}
.st.open {
  background: var(--ok);
}
.st.closed,
.st.error {
  background: var(--danger);
}
.recbadge {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 1px 7px;
  border: 0;
  border-radius: 9px;
  font: inherit;
  font-size: 10.5px;
  font-weight: 600;
  font-variant-numeric: tabular-nums;
  color: #fff;
  background: var(--danger);
  cursor: pointer;
}
.recbadge i {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: #fff;
  animation: blink 1.2s step-end infinite;
}
.recbadge.float {
  position: absolute;
  right: 14px;
  bottom: 10px;
  z-index: 3;
  opacity: 0.9;
}
@keyframes blink {
  50% {
    opacity: 0.2;
  }
}
.tools {
  position: absolute;
  top: 6px;
  right: 14px;
  z-index: 3;
  display: flex;
  gap: 2px;
  padding: 2px;
  border-radius: 8px;
  background: var(--bg-2);
  border: 1px solid var(--line);
  opacity: 0;
  transition: opacity 0.15s;
}
.tp.withstats .tools,
.tp.withstats .find,
.tp.withstats .loading,
.tp.withstats .recbadge.float {
  right: 278px;
}
.tp.withdiff .tools,
.tp.withdiff .find,
.tp.withdiff .loading,
.tp.withdiff .recbadge.float {
  right: calc(var(--diff-w, 50%) + 14px);
}
.ph .iconbtn.on {
  color: var(--accent);
  opacity: 1;
}
.tp:hover .tools {
  opacity: 0.95;
}
.find {
  position: absolute;
  top: 6px;
  right: 14px;
  z-index: 4;
  display: flex;
  align-items: center;
  gap: 2px;
  padding: 3px 4px 3px 10px;
  border-radius: 8px;
  background: var(--bg-2);
  border: 1px solid var(--line);
  box-shadow: var(--shadow);
}
.find input {
  width: 180px;
  border: 0;
  outline: 0;
  background: none;
  color: var(--text);
}
.loading {
  position: absolute;
  top: 10px;
  right: 16px;
}
.spin {
  display: block;
  width: 16px;
  height: 16px;
  border: 2px solid var(--line);
  border-top-color: var(--accent);
  border-radius: 50%;
  animation: spin 0.8s linear infinite;
}
@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}
.ghost {
  position: absolute;
  z-index: 2;
  overflow: hidden;
  white-space: pre;
  pointer-events: none;
  opacity: 0.38;
}
.ghost span {
  display: inline-block;
  text-align: center;
}
:deep(.xterm) {
  height: 100%;
}
:deep(.xterm-viewport) {
  background: transparent !important;
}
</style>

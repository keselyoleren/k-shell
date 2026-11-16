<script setup>
// The box at the bottom of a terminal where commands are typed (Settings → Terminal →
// "Type commands at the bottom"). TerminalPane shows it while the shell waits for a command;
// while one runs, keys go to the terminal and this shows what is running.
import { computed, nextTick, onBeforeUnmount, ref, watch } from 'vue'
import { store } from '../lib/store'
import { suggestFor, firstWord } from '../lib/suggest'
import { actionFor, hint } from '../lib/keymap'
import {
  tildePath,
  fmtDuration,
  pathWord,
  resolveDir,
  matchNames,
  commonStart,
  escapePath,
  BOX_DEFAULTS,
  BOX_CHIPS,
} from '../lib/blocks'
import Icon from './Icon.vue'
import ChipPicker from './ChipPicker.vue'

const props = defineProps({
  /** idle: the shell waits for a command · running: one is running */
  phase: String,
  /** { cwd, home, git, env } from the shell */
  info: Object,
  /** { cmd, since } of the running command */
  running: Object,
  /** { code, ms } of the last command */
  last: Object,
  /** history target of the pane (see TerminalPane) */
  target: String,
  colors: Object,
  font: Object,
  /** (folder) => Promise<names>, folders ending in "/" */
  list: Function,
  /** (folder) => Promise<branch names> of the git repository there */
  branches: Function,
  /** { files, added, removed } not committed yet, or null */
  changes: Object,
  /** the changes panel is open */
  changesOpen: Boolean,
  /** Settings → Command input (lib/blocks.js BOX_DEFAULTS) */
  options: { type: Object, default: () => ({}) },
})
const emit = defineEmits(['submit', 'send', 'clear', 'find', 'focus-term', 'changes'])
const S = () => store.vault.settings || {}
const mac = computed(() => store.platform.os === 'macos')

const text = ref('')
const input = ref(null)
const atEnd = ref(true)
const rows = computed(() => Math.min(8, text.value.split('\n').length))
const path = computed(() => {
  const p = tildePath(props.info?.cwd, props.info?.home)
  return p.length > 48 ? '…' + p.slice(-47) : p // keep the end: the folder you are in
})

const ghost = computed(() => {
  if (S().autosuggest === false || !atEnd.value || !text.value || text.value.includes('\n')) return ''
  // a folder for cd comes from the folder itself, not from commands run elsewhere
  if (props.options.path_complete !== false && pathWord(text.value)?.dirsOnly) return pathGhost.value
  return suggestFor(text.value, { history: store.vault.history, snippets: store.vault.snippets, target: props.target })
})

// ---------- path completion: Tab completes the file or folder being typed ----------
const menu = ref(null) // { items, sel, w, before }
const pathGhost = ref('')
const listings = new Map() // folder -> { at, names: Promise }
function namesIn(dir) {
  const c = listings.get(dir)
  if (c && Date.now() - c.at < 5000) return c.names
  const names = (props.list?.(dir) || Promise.resolve([])).catch(() => [])
  listings.set(dir, { at: Date.now(), names })
  return names
}
/** Matches for the path word before the caret: { w, items, before } or null. */
async function pathMatches() {
  const t = input.value
  const before = text.value.slice(0, t ? t.selectionEnd : text.value.length)
  const w = pathWord(before)
  if (!w) return null
  const names = await namesIn(resolveDir(w.dir, props.info?.cwd, props.info?.home))
  if (text.value.slice(0, before.length) !== before) return null // typed on meanwhile
  return { w, items: matchNames(names, w), before }
}
/** Put `name` in place of the path word; a whole file name gets a space after it. */
function usePath(m, name, whole) {
  const add = escapePath(m.w.dir + name) + (whole && !name.endsWith('/') ? ' ' : '')
  const rest = text.value.slice(m.before.length)
  setText(text.value.slice(0, m.w.start) + add + rest, m.w.start + add.length)
}
async function complete() {
  menu.value = null
  if (props.options.path_complete === false) return ghost.value && accept(false)
  const m = await pathMatches()
  if (!m || !m.items.length) {
    if (ghost.value) accept(false)
    return
  }
  if (m.items.length === 1) return usePath(m, m.items[0], true)
  const common = commonStart(m.items)
  if (common.length > m.w.prefix.length) usePath(m, common, false)
  await nextTick()
  const again = await pathMatches()
  if (again?.items.length > 1) menu.value = { ...again, sel: -1 }
}
function pick(i) {
  const m = menu.value
  menu.value = null
  if (m) usePath(m, m.items[i], true)
  focus()
}
const menuEl = ref(null)
function moveSel(d) {
  const m = menu.value
  const n = m.items.length
  m.sel = m.sel < 0 ? (d > 0 ? 0 : n - 1) : (m.sel + d + n) % n
  nextTick(() => menuEl.value?.querySelector('.on')?.scrollIntoView({ block: 'nearest' }))
}
watch(text, async (v) => {
  pathGhost.value = ''
  const w = pathWord(v)
  if (!w?.dirsOnly || !w.prefix || S().autosuggest === false || props.options.path_complete === false) return
  const names = await namesIn(resolveDir(w.dir, props.info?.cwd, props.info?.home))
  if (text.value !== v) return
  const first = names.find((n) => n.endsWith('/') && n.startsWith(w.prefix))
  pathGhost.value = first ? escapePath(first.slice(w.prefix.length)) : ''
})
// the shell's folder changed: listings may be out of date
watch(
  () => [props.info?.cwd, props.phase],
  () => listings.clear(),
)
/** Keys while the list of matches is open; true when handled. */
function menuKey(e) {
  const m = menu.value
  if (e.key === 'Tab' || e.key === 'ArrowDown' || e.key === 'ArrowUp') {
    e.preventDefault()
    moveSel(e.key === 'ArrowUp' || (e.key === 'Tab' && e.shiftKey) ? -1 : 1)
    return true
  }
  if (e.key === 'Enter' && m.sel >= 0) {
    e.preventDefault()
    pick(m.sel)
    return true
  }
  if (e.key === 'Escape') {
    e.preventDefault()
    menu.value = null
    return true
  }
  if (!['Shift', 'Control', 'Alt', 'Meta'].includes(e.key)) menu.value = null
  return false
}

function caret() {
  const t = input.value
  atEnd.value = !t || (t.selectionStart === t.selectionEnd && t.selectionEnd === text.value.length)
}
function setText(v, at = v.length) {
  text.value = v
  nextTick(() => {
    const t = input.value
    if (t) t.selectionStart = t.selectionEnd = at
    caret()
  })
}
function insert(s) {
  const t = input.value
  const a = t ? t.selectionStart : text.value.length
  const b = t ? t.selectionEnd : text.value.length
  hist = null
  setText(text.value.slice(0, a) + s + text.value.slice(b), a + s.length)
  focus()
}
function focus() {
  const go = () => input.value?.focus({ preventScroll: true })
  if (input.value) go()
  else nextTick(go) // the input is drawn again when a command has finished
}

// ---------- history: ↑/↓ go through earlier commands starting with what was typed ----------
let hist = null // { list, i, draft }
function step(dir) {
  if (!hist) {
    const draft = text.value
    const seen = new Set()
    const all = store.vault.history || []
    const mine = []
    const other = []
    for (let i = all.length - 1; i >= 0; i--) {
      const h = all[i]
      if (seen.has(h.cmd) || !h.cmd.startsWith(draft) || h.cmd === draft) continue
      seen.add(h.cmd)
      ;(h.target === props.target ? mine : other).push(h.cmd)
    }
    hist = { list: [...mine, ...other], i: -1, draft }
  }
  const i = hist.i + dir
  if (i >= hist.list.length) return
  if (i < 0) {
    setText(hist.draft)
    hist = null
    return
  }
  hist.i = i
  setText(hist.list[i])
}

function accept(word) {
  setText(text.value + (word ? firstWord(ghost.value) : ghost.value))
}
function submit() {
  const v = text.value
  hist = null
  menu.value = null
  setText('')
  emit('submit', v)
}

function onKey(e) {
  if (e.isComposing) return
  if (menu.value && menuKey(e)) return
  const t = input.value
  const a = actionFor(e)
  if ((a?.id === 'accept_suggestion' || a?.id === 'accept_suggestion_word') && ghost.value) {
    e.preventDefault()
    return accept(a.id === 'accept_suggestion_word')
  }
  if (a?.id === 'clear' || a?.id === 'find') {
    e.preventDefault()
    return emit(a.id)
  }
  const ctrl = e.ctrlKey && !e.metaKey && !e.altKey
  const firstLine = !text.value.slice(0, t.selectionStart).includes('\n')
  const lastLine = !text.value.slice(t.selectionEnd).includes('\n')
  switch (true) {
    case e.key === 'Enter' && !e.shiftKey && !e.altKey && !e.ctrlKey && !e.metaKey:
      e.preventDefault()
      return submit()
    case e.key === 'ArrowUp' && !e.shiftKey && firstLine:
      e.preventDefault()
      return step(1)
    case e.key === 'ArrowDown' && !e.shiftKey && lastLine && !!hist:
      e.preventDefault()
      return step(-1)
    case e.key === 'Tab' && !e.shiftKey:
      e.preventDefault()
      return complete()
    case e.key === 'ArrowRight' && atEnd.value && !!ghost.value && !e.shiftKey && !e.metaKey && !ctrl:
      e.preventDefault()
      return accept(e.altKey)
    case e.key === 'Escape' && !!hist:
      e.preventDefault()
      setText(hist.draft)
      hist = null
      return
    case ctrl && e.key === 'c':
      // with a selection on Windows/Linux, Ctrl+C copies like everywhere else
      if (!mac.value && t.selectionStart !== t.selectionEnd) return
      e.preventDefault()
      hist = null
      return setText('')
    case ctrl && e.key === 'd' && !text.value:
      e.preventDefault()
      return emit('send', '\x04')
    case ctrl && e.key === 'l':
      e.preventDefault()
      return emit('clear')
    case ctrl && e.key === 'u':
      e.preventDefault()
      return setText(text.value.slice(t.selectionStart), 0)
    case ctrl && e.key === 'w' && !mac.value: {
      e.preventDefault()
      const before = text.value.slice(0, t.selectionStart).replace(/\S+\s*$/, '')
      return setText(before + text.value.slice(t.selectionStart), before.length)
    }
  }
}
/** A key sent to the terminal while this box is shown (the terminal had focus). */
function key(d) {
  if (d === '\r') return submit()
  if (d === '\x7f' || d === '\b') {
    hist = null
    return setText(text.value.slice(0, -1))
  }
  if (d === '\x03') return setText('')
  if (/^[^\x00-\x1f\x7f]+$/.test(d)) insert(d)
  else focus()
}

// ---------- running ----------
const now = ref(Date.now())
let tick = 0
watch(
  () => props.phase,
  (p) => {
    clearInterval(tick)
    if (p === 'running') tick = setInterval(() => (now.value = Date.now()), 1000)
  },
  { immediate: true },
)
onBeforeUnmount(() => clearInterval(tick))
const elapsed = computed(() =>
  props.running ? fmtDuration(Math.max(0, now.value - props.running.since)).replace(/\.\d+s$/, 's') : '',
)
// ---------- chips above the input, in the order set in Settings ----------
const chips = computed(() => props.options.chips || BOX_DEFAULTS.chips)
const ANSI = ['black', 'red', 'green', 'yellow', 'blue', 'magenta', 'cyan', 'white']
/** The chip's colour from the terminal theme, like the prompt segments (changes and exit have their own). */
const tint = (id) => (BOX_CHIPS[id] ? { color: props.colors[ANSI[BOX_CHIPS[id].color]] } : null)
// the time chip: one timer for the minute, only while it is shown
const clock = ref('')
let clockTimer = 0
function tickClock() {
  clock.value = new Date().toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })
  clockTimer = setTimeout(tickClock, 60_000 - (Date.now() % 60_000))
}
watch(
  () => chips.value.includes('time'),
  (on) => {
    clearTimeout(clockTimer)
    if (on) tickClock()
  },
  { immediate: true },
)
onBeforeUnmount(() => clearTimeout(clockTimer))

const stopKey = computed(() => (mac.value ? '⌃C' : 'Ctrl+C'))

// ---------- chips: click the folder to go to another one, the branch to switch ----------
const picker = ref(null) // { kind, items, left }
const root = ref(null)
function openPicker(kind, e) {
  if (props.phase !== 'idle') return
  if (picker.value?.kind === kind) return (picker.value = null)
  const width = root.value?.clientWidth || 0
  const left = Math.max(8, Math.min(e.currentTarget.offsetLeft, width - 428))
  picker.value = { kind, items: null, left }
  const cwd = props.info?.cwd || ''
  const load =
    kind === 'dir'
      ? namesIn(cwd).then((names) => {
          const byName = (a, b) => a.localeCompare(b, undefined, { sensitivity: 'base' })
          const dirs = names.filter((n) => n.endsWith('/')).map((n) => n.slice(0, -1))
          const files = names.filter((n) => !n.endsWith('/'))
          return [
            ...(cwd !== '/' ? [{ label: '.. (Parent folder)', icon: 'up', value: '..', dir: true, muted: true }] : []),
            ...dirs.sort(byName).map((n) => ({ label: n, icon: 'folder', value: n, dir: true })),
            ...files.sort(byName).map((n) => ({ label: n, icon: 'file', value: n })),
          ]
        })
      : (props.branches?.(cwd) || Promise.resolve([])).then((list) =>
          list.map((b) => ({ label: b, icon: 'branch', value: b, current: b === props.info?.git })),
        )
  load
    .catch(() => [])
    .then((items) => {
      if (picker.value?.kind === kind) picker.value = { ...picker.value, items }
    })
}
function pickChip(it) {
  const kind = picker.value?.kind
  picker.value = null
  focus()
  if (kind === 'dir') {
    if (it.dir) emit('submit', `cd ${escapePath(it.value)}`)
    else insert(escapePath(it.value) + ' ') // a file: put its name in the command
  } else if (!it.current) emit('submit', `git checkout ${escapePath(it.value)}`)
}
watch(
  () => props.phase,
  () => (picker.value = null),
)

defineExpose({ focus, insert, key, setText })
</script>

<template>
  <div
    ref="root"
    class="cb"
    :style="{
      color: colors.foreground,
      '--fg': colors.foreground,
      fontFamily: font.family,
      fontSize: font.size + 'px',
    }"
  >
    <div v-if="menu && phase === 'idle'" ref="menuEl" class="menu" @mousedown.prevent>
      <div
        v-for="(n, i) in menu.items.slice(0, 300)"
        :key="n"
        class="item"
        :class="{ on: i === menu.sel, dir: n.endsWith('/') }"
        @click="pick(i)"
      >
        <Icon :name="n.endsWith('/') ? 'folder' : 'file'" :size="12" />
        <span
          ><b>{{ n.slice(0, menu.w.prefix.length) }}</b
          >{{ n.slice(menu.w.prefix.length) }}</span
        >
      </div>
      <div class="foot">
        {{ menu.items.length }} matches in {{ menu.w.dir || tildePath(info.cwd, info.home) || '.' }} · <kbd>⇥</kbd> next
        · <kbd>↵</kbd> choose · <kbd>esc</kbd> close
      </div>
    </div>
    <ChipPicker
      v-if="picker"
      :items="picker.items"
      :left="picker.left"
      :placeholder="picker.kind === 'dir' ? 'Search folders and files…' : 'Search branches…'"
      :empty="picker.kind === 'dir' ? 'This folder is empty' : 'No branches found'"
      @pick="pickChip"
      @close="picker = null"
    />
    <div class="ctx">
      <template v-for="id in chips" :key="id">
        <span v-if="id === 'env' && info.env" class="chip env" :style="tint(id)" title="Python environment"
          ><Icon name="terminal" :size="11" /><span class="lbl">{{ info.env }}</span></span
        >
        <span v-else-if="id === 'user' && info.user" class="chip" :style="tint(id)" title="User"
          ><Icon name="user" :size="11" /><span class="lbl">{{ info.user }}</span></span
        >
        <span v-else-if="id === 'host' && info.host" class="chip" :style="tint(id)" title="Host"
          ><Icon name="server" :size="11" /><span class="lbl">{{ info.host }}</span></span
        >
        <button
          v-else-if="id === 'cwd' && path"
          class="chip path"
          :class="{ open: picker?.kind === 'dir' }"
          :style="tint(id)"
          :disabled="phase !== 'idle'"
          :title="`${info.cwd}\nClick to go to another folder`"
          @mousedown.prevent
          @click="openPicker('dir', $event)"
        >
          <Icon name="folder" :size="11" /><span class="lbl">{{ path }}</span>
        </button>
        <button
          v-else-if="id === 'git' && info.git"
          class="chip git"
          :class="{ open: picker?.kind === 'branch' }"
          :style="tint(id)"
          :disabled="phase !== 'idle'"
          title="Git branch: click to switch to another one"
          @mousedown.prevent
          @click="openPicker('branch', $event)"
        >
          <Icon name="branch" :size="11" /><span class="lbl">{{ info.git }}</span>
        </button>
        <button
          v-else-if="id === 'changes' && info.git && changes && changes.files"
          class="chip changes"
          :class="{ open: changesOpen }"
          :title="changesOpen ? 'Hide changes' : 'View changes'"
          @mousedown.prevent
          @click="emit('changes')"
        >
          <Icon name="file" :size="11" />{{ changes.files }} •
          <span class="add">+{{ changes.added }}</span>
          <span class="del">-{{ changes.removed }}</span>
        </button>
        <span
          v-else-if="id === 'exit' && last && last.code"
          class="chip bad"
          :title="`The last command failed with exit code ${last.code}`"
          >exit {{ last.code }}</span
        >
        <span
          v-else-if="id === 'duration' && last && last.ms != null"
          class="chip"
          :style="tint(id)"
          title="How long the last command took"
          ><Icon name="clock" :size="11" />{{ fmtDuration(last.ms) }}</span
        >
        <span v-else-if="id === 'time'" class="chip" :style="tint(id)" title="Time"
          ><Icon name="clock" :size="11" />{{ clock }}</span
        >
      </template>
      <span class="grow"></span>
      <span v-if="phase === 'idle' && options.hints !== false" class="keys"
        ><template v-if="options.path_complete !== false"><kbd>⇥</kbd> complete path · </template><kbd>⇧↵</kbd> new line
        · <kbd>↑</kbd> history<template v-if="hint('accept_suggestion')">
          · <kbd>{{ hint('accept_suggestion') }}</kbd> accept</template
        ></span
      >
    </div>
    <div v-if="phase === 'idle'" class="edit" @mousedown.self.prevent="focus()">
      <div class="mirror" aria-hidden="true">
        <span class="typed">{{ text }}</span
        ><span class="sug">{{ ghost }}</span>
      </div>
      <textarea
        ref="input"
        v-model="text"
        :rows="rows"
        spellcheck="false"
        autocapitalize="off"
        autocomplete="off"
        autocorrect="off"
        placeholder="Type a command"
        @keydown="onKey"
        @input="((hist = null), caret())"
        @keyup="caret"
        @mouseup="caret"
        @select="caret"
      ></textarea>
    </div>
    <div v-else class="run" @mousedown.prevent="emit('focus-term')">
      <span class="spin"></span>
      <span class="cmd">{{ running?.cmd || 'Running' }}</span>
      <span class="dim">{{ elapsed }}</span>
      <span class="grow"></span>
      <span class="dim">keys go to the program</span>
      <button class="stop" @mousedown.stop.prevent="emit('send', '\x03')" :title="`Interrupt (${stopKey})`">
        <Icon name="stop" :size="10" /> {{ stopKey }}
      </button>
    </div>
  </div>
</template>

<style scoped>
.cb {
  position: relative;
  flex: none;
  padding: 7px 10px 8px;
  border-top: 1px solid color-mix(in srgb, var(--fg) 14%, transparent);
  line-height: 1.45;
}
.ctx {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
  margin-bottom: 5px;
  font-size: 11.5px;
}
/* Chips share the row: the folder gives way first, then the branch; the others keep their size */
.chip {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  flex: none;
  min-width: 0;
  max-width: 100%;
  overflow: hidden;
  padding: 1px 7px;
  border-radius: 5px;
  border: 1px solid color-mix(in srgb, var(--fg) 16%, transparent);
  background: color-mix(in srgb, var(--fg) 6%, transparent);
  white-space: nowrap;
  font-weight: 600;
}
button.chip {
  font-family: inherit;
  font-size: inherit;
  color: inherit;
  cursor: pointer;
}
button.chip:not(:disabled):hover,
button.chip.open {
  border-color: color-mix(in srgb, var(--accent) 60%, transparent);
  background: color-mix(in srgb, var(--accent) 14%, transparent);
}
button.chip:disabled {
  cursor: default;
}
.chip.changes {
  gap: 4px;
}
.chip.changes .add {
  color: var(--ok);
}
.chip.changes .del {
  color: var(--danger);
}
.chip :deep(svg) {
  flex: none;
}
.chip .lbl {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
}
.chip.path {
  flex: 0 4 auto;
  min-width: 70px;
}
.chip.git {
  flex: 0 1 auto;
  min-width: 70px;
}
.chip.bad {
  color: var(--danger);
  border-color: color-mix(in srgb, var(--danger) 40%, transparent);
}
.grow {
  flex: 1;
}
.keys {
  opacity: 0.45;
  white-space: nowrap;
  font-family: var(--font, system-ui);
  font-size: 11px;
}
.keys kbd {
  font: inherit;
}
@container (max-width: 640px) {
  .keys {
    display: none;
  }
}
@container (max-width: 420px) {
  .chip.env {
    display: none;
  }
}
.edit {
  position: relative;
  cursor: text;
}
/* matches for Tab, above the input */
.menu {
  position: absolute;
  left: 8px;
  bottom: calc(100% - 4px);
  z-index: 6;
  width: min(460px, calc(100% - 16px));
  max-height: 260px;
  overflow-y: auto;
  padding: 4px;
  border-radius: 8px;
  border: 1px solid color-mix(in srgb, var(--fg) 18%, transparent);
  background: var(--bg-2);
  color: var(--text);
  box-shadow: 0 8px 24px rgba(0, 0, 0, 0.35);
  font-size: 0.95em;
}
.menu .item {
  display: flex;
  align-items: center;
  gap: 7px;
  padding: 3px 8px;
  border-radius: 5px;
  white-space: nowrap;
  cursor: pointer;
}
.menu .item span {
  overflow: hidden;
  text-overflow: ellipsis;
}
.menu .item b {
  color: var(--accent);
}
.menu .item.dir :deep(svg) {
  color: var(--accent);
}
.menu .item:hover {
  background: color-mix(in srgb, var(--text) 8%, transparent);
}
.menu .item.on {
  background: color-mix(in srgb, var(--accent) 22%, transparent);
}
.menu .foot {
  position: sticky;
  bottom: -4px;
  margin: 4px -4px -4px;
  padding: 4px 10px;
  border-top: 1px solid var(--line);
  background: var(--bg-2);
  font-family: var(--font, system-ui);
  font-size: 11px;
  opacity: 0.8;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.menu .foot kbd {
  font: inherit;
}
.mirror,
textarea {
  margin: 0;
  padding: 0;
  font: inherit;
  line-height: inherit;
  letter-spacing: inherit;
  white-space: pre-wrap;
  word-break: break-all;
}
.mirror {
  position: absolute;
  inset: 0;
  pointer-events: none;
  overflow: hidden;
}
.mirror .typed {
  color: transparent;
}
.mirror .sug {
  opacity: 0.4;
}
textarea {
  display: block;
  width: 100%;
  border: 0;
  outline: 0;
  resize: none;
  background: transparent;
  color: inherit;
  caret-color: var(--accent);
  overflow-y: auto;
}
textarea::placeholder {
  color: inherit;
  opacity: 0.35;
}
.run {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
  cursor: text;
  white-space: nowrap;
}
.run .cmd {
  overflow: hidden;
  text-overflow: ellipsis;
  font-weight: 600;
}
.run .dim {
  opacity: 0.5;
  font-size: 0.9em;
}
/* fast commands come and go: show the running state only once it lasts */
.run > * {
  animation: appear 0.15s 0.3s both;
}
@keyframes appear {
  from {
    opacity: 0;
  }
}
.spin {
  width: 9px;
  height: 9px;
  flex: none;
  border-radius: 50%;
  border: 2px solid color-mix(in srgb, var(--accent) 35%, transparent);
  border-top-color: var(--accent);
  animation:
    appear 0.15s 0.3s both,
    turn 0.8s linear infinite;
}
@keyframes turn {
  to {
    transform: rotate(360deg);
  }
}
.stop {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 1px 7px;
  border-radius: 5px;
  border: 1px solid color-mix(in srgb, var(--fg) 18%, transparent);
  background: transparent;
  color: inherit;
  font: inherit;
  font-size: 11px;
  cursor: pointer;
}
.stop:hover {
  border-color: var(--danger);
  color: var(--danger);
}
</style>

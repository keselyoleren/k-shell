// Command input at the bottom of a terminal (Settings → Terminal): the shell's own prompt is
// hidden and commands are typed in a box under the output, each run shown as a block.
//
// A hook added to bash (4.4+) and zsh marks the parts of the output with OSC 133, the
// sequences other terminals use for "shell integration":
//   D;<exit>  the last command finished          (before the prompt)
//   A         the prompt starts                   (the shell is waiting for a command)
//   C         a command starts running            (zsh preexec, bash PS0)
// plus OSC 7771 with what the box shows above the input: folder, home, git branch, env,
// user and host.
// Everything from A to C (the prompt and the shell echoing the command) is kept off screen.

/** Shell code (bash 4.4+ / zsh) that prints the marks. Other shells (and bash 3.2) skip it. */
export const BLOCKS_SCRIPT = [
  '_ksb_s(){ _ksb_e=$?; return $_ksb_e; }',
  '_ksb_p(){ local b h=${HOSTNAME:-$HOST}; b=$(git symbolic-ref --short -q HEAD 2>/dev/null || git rev-parse --short HEAD 2>/dev/null);' +
    ` printf '\\033]133;D;%s\\007\\033]7771;%s\\037%s\\037%s\\037%s\\037%s\\037%s\\007\\033]133;A\\007' "$_ksb_e" "$PWD" "$HOME" "$b" "\${CONDA_DEFAULT_ENV:-\${VIRTUAL_ENV##*/}}" "\${USER:-\${LOGNAME:-$(id -un 2>/dev/null)}}" "\${h%%.*}";` +
    ' return $_ksb_e; }',
  `_ksb_c(){ printf '\\033]133;C\\007'; }`,
  'if [ -n "$ZSH_VERSION" ]; then precmd_functions=(_ksb_s ${precmd_functions:#_ksb_*} _ksb_p); preexec_functions=(${preexec_functions:#_ksb_c} _ksb_c)',
  `elif (( BASH_VERSINFO[0] > 4 || (BASH_VERSINFO[0] == 4 && BASH_VERSINFO[1] >= 4) )); then PROMPT_COMMAND=$'_ksb_s\\n'"\${PROMPT_COMMAND}"$'\\n_ksb_p'; PS0="\${PS0}"$'\\e]133;C\\a'; fi`,
].join('; ')

const ESC = 0x1b
const BEL = 0x07
const MAX_CARRY = 8192
const MAX_HELD = 256 * 1024
const PASTE_ON = [ESC, 0x5b, 0x3f, 0x32, 0x30, 0x30, 0x34, 0x68] // ESC[?2004h
const PASTE_OFF = [ESC, 0x5b, 0x3f, 0x32, 0x30, 0x30, 0x34, 0x6c] // ESC[?2004l

function lastIndexOf(hay, needle) {
  outer: for (let i = hay.length - needle.length; i >= 0; i--) {
    for (let j = 0; j < needle.length; j++) if (hay[i + j] !== needle[j]) continue outer
    return i
  }
  return -1
}

/**
 * Splits terminal output on the marks. `feed(bytes)` calls `show(bytes)` for what belongs on
 * screen and `event(name, value)` for each mark, in stream order.
 *   event('A') · event('C') · event('D', exitCode) · event('info', { cwd, home, git, env })
 */
export class MarkStream {
  constructor({ show, event }) {
    this.show = show
    this.event = event
    this.reset()
  }
  reset() {
    this.carry = null
    this.hidden = false
    this.held = [] // output kept off screen since the last A
    this.heldSize = 0
    this.paste = false // the shell's line editor has bracketed paste on
  }
  feed(bytes) {
    if (this.carry) {
      const all = new Uint8Array(this.carry.length + bytes.length)
      all.set(this.carry)
      all.set(bytes, this.carry.length)
      bytes = all
      this.carry = null
    }
    let from = 0
    let i = 0
    while (i < bytes.length) {
      i = bytes.indexOf(ESC, i) // native search: output without escapes costs next to nothing
      if (i < 0) break
      const m = this.mark(bytes, i)
      if (m === null) {
        // a mark may continue in the next chunk
        if (bytes.length - i <= MAX_CARRY) {
          this.emit(bytes.subarray(from, i))
          this.carry = bytes.slice(i)
          return
        }
        i++
        continue
      }
      if (!m) {
        i++
        continue
      }
      this.emit(bytes.subarray(from, i))
      i = from = m.end
      this.handle(m.id, m.arg)
    }
    this.emit(bytes.subarray(from))
  }
  /** The mark starting at `i`: { id, arg, end }, false if it isn't one, null if cut off. */
  mark(b, i) {
    if (i + 1 >= b.length) return null
    if (b[i + 1] !== 0x5d) return false // ESC ]
    let j = i + 2
    let id = ''
    while (j < b.length && b[j] >= 0x30 && b[j] <= 0x39 && id.length < 5) id += String.fromCharCode(b[j++])
    if (j >= b.length) return null
    if (id !== '133' && id !== '7771') return false
    if (b[j] !== 0x3b) return false // ;
    for (let k = j + 1; k < b.length; k++) {
      if (b[k] === BEL) return { id, arg: new TextDecoder().decode(b.subarray(j + 1, k)), end: k + 1 }
      if (b[k] === ESC) {
        if (k + 1 >= b.length) return null
        if (b[k + 1] === 0x5c) return { id, arg: new TextDecoder().decode(b.subarray(j + 1, k)), end: k + 2 }
        return false
      }
    }
    return null
  }
  emit(b) {
    if (!b.length) return
    if (!this.hidden) return this.show(b)
    const on = lastIndexOf(b, PASTE_ON)
    const off = lastIndexOf(b, PASTE_OFF)
    if (on >= 0 || off >= 0) this.paste = on > off
    if (this.heldSize + b.length <= MAX_HELD) {
      this.held.push(b.slice())
      this.heldSize += b.length
    }
  }
  handle(id, arg) {
    if (id === '7771') {
      const [cwd = '', home = '', git = '', env = '', user = '', host = ''] = arg.split('\x1f')
      return this.event('info', { cwd, home, git, env, user, host })
    }
    const [kind, code] = arg.split(';')
    if (kind === 'A') {
      this.hidden = true
      this.held = []
      this.heldSize = 0
    } else if (kind === 'C') {
      this.hidden = false
      this.held = []
      this.heldSize = 0
    }
    if (kind === 'D') this.event('D', code === undefined || code === '' ? null : +code)
    else if (kind === 'A' || kind === 'C') this.event(kind)
  }
  /** Stop hiding and put back what was kept off screen since chunk `from` of `held`. */
  reveal(from = 0) {
    const held = this.held.slice(from)
    this.hidden = false
    this.held = []
    this.heldSize = 0
    for (const b of held) this.show(b)
  }
}

// ---------- path completion in the box ----------
/** Commands whose argument is a folder: only folders are offered. */
const DIR_COMMANDS = new Set(['cd', 'pushd', 'rmdir', 'z'])
/** Characters a shell would read as something else in a file name. */
export const escapePath = (s) => s.replace(/([\s'"\\$`!&;|<>()*?#{}[\]])/g, '\\$1')

/**
 * The path being typed at the end of `before` (the text up to the caret), or null when that
 * word is a command name. { start, dir, prefix, dirsOnly }: `start` is where the word begins
 * in `before`, `dir` the folder part as typed ("", "src/", "~/", "/etc/"), `prefix` the rest.
 */
export function pathWord(before) {
  const line = before.slice(before.lastIndexOf('\n') + 1)
  const word = /(?:\\.|[^\s\\])*$/.exec(line)[0]
  const head = line.slice(0, line.length - word.length)
  const cmd = head
    .split(/&&|\|\||[;|(]/)
    .at(-1)
    .trim()
    .split(/\s+/)
  const isCommand = !cmd[0] || (cmd[0] === 'sudo' && cmd.length === 1)
  if (isCommand && !/[/~.]/.test(word[0] || '')) return null
  const raw = word.replace(/\\(.)/g, '$1')
  const slash = raw.lastIndexOf('/')
  return {
    start: before.length - word.length,
    dir: raw.slice(0, slash + 1),
    prefix: raw.slice(slash + 1),
    dirsOnly: DIR_COMMANDS.has(cmd[0] === 'sudo' ? cmd[1] : cmd[0]),
  }
}

/** Absolute folder for the `dir` part of a path word, from the shell's folder and home. */
export function resolveDir(dir, cwd, home) {
  if (dir === '~/' || dir.startsWith('~/')) return home + dir.slice(1)
  if (dir.startsWith('/')) return dir
  return (cwd || '/').replace(/\/$/, '') + '/' + dir
}

/** Names in `names` (folders end in "/") that complete `w`, case-insensitive if none match exactly. */
export function matchNames(names, w) {
  let c = w.dirsOnly ? names.filter((n) => n.endsWith('/')) : names
  if (!w.prefix.startsWith('.')) c = c.filter((n) => !n.startsWith('.'))
  const exact = c.filter((n) => n.startsWith(w.prefix))
  if (exact.length) return exact
  const p = w.prefix.toLowerCase()
  return c.filter((n) => n.toLowerCase().startsWith(p))
}

/** Longest start shared by all `names`. */
export function commonStart(names) {
  if (!names.length) return ''
  let s = names[0]
  for (const n of names) while (!n.startsWith(s)) s = s.slice(0, -1)
  return s
}

/** "~/src/app" for a folder under home. */
export function tildePath(cwd, home) {
  if (!cwd) return ''
  if (home && home !== '/' && (cwd === home || cwd.startsWith(home + '/'))) return '~' + cwd.slice(home.length)
  return cwd
}

/** "0.032s", "4.1s", "2m 05s" */
export function fmtDuration(ms) {
  if (ms == null) return ''
  if (ms < 10_000) return (ms / 1000).toFixed(ms < 1000 ? 3 : 1) + 's'
  const s = Math.round(ms / 1000)
  if (s < 3600) return `${Math.floor(s / 60)}m ${String(s % 60).padStart(2, '0')}s`
  return `${Math.floor(s / 3600)}h ${String(Math.floor((s % 3600) / 60)).padStart(2, '0')}m`
}

/** Options of the command box (Settings → Terminal → Command input); all on by default. */
export const BOX_DEFAULTS = {
  scope: 'all',
  chips: ['env', 'cwd', 'git', 'changes', 'exit'],
  head_where: true,
  head_time: true,
  head_rule: true,
  path_complete: true,
  hints: true,
  hide_fullscreen: true,
}
/**
 * Chips the box can show above the input, in the shape of the prompt segments (lib/prompt.js)
 * so the same editor builds both. color = ANSI colour index.
 */
export const BOX_CHIPS = {
  env: {
    label: 'Python environment',
    icon: 'terminal',
    color: 2,
    sample: 'base',
    hint: 'conda or virtualenv, when one is active',
  },
  user: { label: 'User', icon: 'user', color: 2, sample: 'user' },
  host: { label: 'Host', icon: 'server', color: 6, sample: 'server' },
  cwd: { label: 'Folder', icon: 'folder', color: 4, sample: '~/project/api', hint: 'click it to go to another folder' },
  git: { label: 'Git branch', icon: 'branch', color: 5, sample: 'main', hint: 'click it to switch branch' },
  changes: {
    label: 'Uncommitted changes',
    icon: 'file',
    color: 7,
    sample: '3 • +42 -7',
    hint: 'click it to see the changes',
  },
  exit: {
    label: 'Exit code',
    icon: 'info',
    color: 1,
    sample: 'exit 1',
    hint: 'shown only when the last command failed',
  },
  duration: { label: 'Last command time', icon: 'clock', color: 3, sample: '2.4s' },
  time: { label: 'Time', icon: 'clock', color: 6, sample: '15:48' },
}

/** The command box options in `settings`, with defaults for the ones not set. */
export const boxOptions = (settings) => ({ ...BOX_DEFAULTS, ...(settings?.command_box || {}) })

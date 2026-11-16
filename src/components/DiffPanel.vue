<script setup>
// Uncommitted changes of the git repository a terminal is in, next to the terminal: each
// changed file with its added and removed lines, and a list of the files to jump to.
// Opened from the changes chip of the command box (CommandBar).
import { computed, onBeforeUnmount, onMounted, ref, shallowRef, watch } from 'vue'
import { store, toast } from '../lib/store'
import { parseDiff, splitPath } from '../lib/gitdiff'
import { tildePath } from '../lib/blocks'
import Icon from './Icon.vue'

const props = defineProps({
  /** { cwd, home, git } of the shell */
  info: Object,
  /** () => Promise<unified diff text> */
  load: Function,
  /** changes again (a command finished): read the diff again */
  version: Number,
  colors: Object,
  font: Object,
})
const emit = defineEmits(['close', 'width'])

/** Lines drawn per file; a bigger diff says how many more there are. */
const MAX_LINES = 3000

// Diffs can have tens of thousands of lines: they are kept out of Vue's deep reactivity
// (shallowRef) and only replaced when the diff text itself changed.
const files = shallowRef(null)
const error = ref('')
const loading = ref(false)
const collapsed = ref(new Set())
const hl = shallowRef({}) // path -> { old: lines, new: lines }
const scroller = ref(null)
let lastText = null

const totals = computed(() => {
  const f = files.value || []
  return { n: f.length, added: f.reduce((s, x) => s + x.added, 0), removed: f.reduce((s, x) => s + x.removed, 0) }
})

let again = false
async function reload() {
  if (loading.value) return void (again = true) // one more read once this one is done
  loading.value = true
  try {
    const text = await props.load()
    error.value = ''
    if (text !== lastText) {
      lastText = text
      const parsed = parseDiff(text)
      for (const f of parsed) index(f)
      files.value = parsed
      highlight(parsed)
    }
  } catch (e) {
    error.value = String(e)
  } finally {
    loading.value = false
  }
  if (again) {
    again = false
    reload()
  }
}
onMounted(reload)
watch(() => props.version, reload)
watch(
  () => props.info?.cwd,
  () => {
    collapsed.value = new Set()
    reload()
  },
)

/** Where each line is in the old and the new text of the file, for its colours. */
function index(f) {
  let oi = 0
  let ni = 0
  let shown = 0
  for (const h of f.hunks) {
    for (const l of h.lines) {
      if (l.t !== '+') l.oi = oi++
      if (l.t !== '-') l.ni = ni++
    }
    h.from = shown
    shown += h.lines.length
  }
  f.size = shown
}

// Colours are worked out one file at a time, letting the app breathe in between, and kept
// for files whose lines didn't change since the last read.
const hlCache = new Map() // path -> { key, value }
let run = 0
async function highlight(list) {
  const id = ++run
  const { highlightLines } = await import('../lib/codeeditor')
  const out = {}
  let changed = false
  for (const f of list) {
    if (f.binary || f.size > MAX_LINES) continue
    const side = (skip) =>
      f.hunks
        .flatMap((h) => h.lines)
        .filter((l) => l.t !== skip)
        .map((l) => l.text)
        .join('\n')
    const nw = side('-')
    const old = f.removed ? side('+') : ''
    const key = nw + '\0' + old
    const hit = hlCache.get(f.path)
    if (hit?.key === key) {
      out[f.path] = hit.value
      continue
    }
    try {
      const value = { new: await highlightLines(f.path, nw), old: old ? await highlightLines(f.path, old) : null }
      hlCache.set(f.path, { key, value })
      out[f.path] = value
      changed = true
    } catch {
      /* no colours for this one */
    }
    if (id !== run) return
    await new Promise((r) => setTimeout(r)) // keep typing and scrolling smooth
    if (id !== run) return
  }
  for (const p of hlCache.keys()) if (!out[p]) hlCache.delete(p)
  if (changed || Object.keys(out).length !== Object.keys(hl.value).length) hl.value = out
}
function pieces(f, l) {
  const h = hl.value[f.path]
  const lines = l.t === '-' ? h?.old : h?.new
  return lines?.[l.t === '-' ? l.oi : l.ni] || null
}

// ---------- width: drag the left edge ----------
const WIDTH_KEY = 'diffPanel.width'
const width = ref(readWidth()) // share of the terminal area, %
watch(width, (w) => emit('width', w), { immediate: true })
function readWidth() {
  try {
    const v = parseFloat(localStorage.getItem(WIDTH_KEY))
    if (v >= 20 && v <= 85) return v
  } catch {
    /* storage unavailable */
  }
  return 50
}
const root = ref(null)
const dragging = ref(false)
let stopDrag = null
function startDrag(e) {
  const area = root.value?.parentElement?.getBoundingClientRect()
  if (!area || e.button !== 0) return
  e.preventDefault()
  dragging.value = true
  store.layoutDrag = true // terminals resize once, when the drag ends
  document.body.style.cursor = 'col-resize'
  let frame = 0
  const move = (ev) => {
    cancelAnimationFrame(frame)
    frame = requestAnimationFrame(() => {
      const pct = ((area.right - ev.clientX) / area.width) * 100
      width.value = Math.min(85, Math.max(20, pct))
    })
  }
  const up = () => {
    cancelAnimationFrame(frame)
    window.removeEventListener('mousemove', move)
    window.removeEventListener('mouseup', up)
    dragging.value = false
    store.layoutDrag = false
    document.body.style.cursor = ''
    stopDrag = null
    try {
      localStorage.setItem(WIDTH_KEY, String(Math.round(width.value * 10) / 10))
    } catch {
      /* not kept */
    }
  }
  window.addEventListener('mousemove', move)
  window.addEventListener('mouseup', up)
  stopDrag = up
}
function resetWidth() {
  width.value = 50
  try {
    localStorage.removeItem(WIDTH_KEY)
  } catch {
    /* nothing kept */
  }
}
onBeforeUnmount(() => {
  run++
  stopDrag?.()
})

function toggle(path) {
  const s = new Set(collapsed.value)
  s.has(path) ? s.delete(path) : s.add(path)
  collapsed.value = s
}
function jump(path) {
  if (collapsed.value.has(path)) toggle(path)
  const el = scroller.value?.querySelector(`[data-path="${CSS.escape(path)}"]`)
  el?.scrollIntoView({ block: 'start', behavior: 'smooth' })
}
function copyPath(path) {
  navigator.clipboard.writeText(path).then(() => toast('Path copied'))
}
const title = computed(() => tildePath(props.info?.cwd, props.info?.home))
</script>

<template>
  <div
    ref="root"
    class="dp"
    :class="{ dragging }"
    :style="{
      flexBasis: width + '%',
      '--fg': colors.foreground,
      '--bgc': colors.background,
      '--c-k': colors.magenta,
      '--c-s': colors.green,
      '--c-o': colors.cyan,
      '--c-n': colors.yellow,
      '--c-c': colors.brightBlack,
      '--c-f': colors.blue,
      '--c-t': colors.cyan,
      '--c-p': colors.red,
      '--c-d': colors.yellow,
      color: colors.foreground,
      background: colors.background,
    }"
  >
    <div
      class="grip"
      title="Drag to resize · double-click for half and half"
      @mousedown="startDrag"
      @dblclick="resetWidth"
    ></div>
    <div class="top">
      <span class="where" :title="info.cwd"
        >{{ title }}<template v-if="info.git">: {{ info.git }}</template></span
      >
      <span v-if="files" class="stat"
        ><Icon name="file" :size="11" /> {{ totals.n }} · <b class="add">+{{ totals.added }}</b>
        <b class="del">-{{ totals.removed }}</b></span
      >
      <span class="grow"></span>
      <button class="iconbtn" :class="{ spinning: loading }" title="Read the changes again" @click="reload">
        <Icon name="refresh" :size="13" />
      </button>
      <button class="iconbtn" title="Close" @click="emit('close')"><Icon name="x" :size="13" /></button>
    </div>
    <div class="sub"><Icon name="forward" :size="12" /> Uncommitted changes</div>

    <div class="dp-body">
      <div ref="scroller" class="files" :style="{ fontFamily: font.family, fontSize: font.size - 1 + 'px' }">
        <div v-if="error" class="note err">{{ error }}</div>
        <div v-else-if="!files" class="note">Reading the changes…</div>
        <div v-else-if="!files.length" class="note">
          No uncommitted changes. The working tree matches the last commit.
        </div>
        <section v-for="f in files" :key="f.path" class="file" :data-path="f.path">
          <header @click="toggle(f.path)">
            <Icon :name="collapsed.has(f.path) ? 'right' : 'down'" :size="12" />
            <span class="path" :title="f.path">{{ f.path }}</span>
            <button class="iconbtn" title="Copy path" @click.stop="copyPath(f.path)">
              <Icon name="copy" :size="12" />
            </button>
            <span v-if="f.isNew" class="tag">new</span>
            <span v-else-if="f.isDeleted" class="tag del">deleted</span>
            <span class="counts"
              ><b class="add">+{{ f.added }}</b> · <b class="del">-{{ f.removed }}</b></span
            >
          </header>
          <template v-if="!collapsed.has(f.path)">
            <div v-if="f.binary" class="note">Binary file: no line changes to show.</div>
            <div v-else-if="!f.hunks.length" class="note">Only the file mode changed.</div>
            <table v-else class="code">
              <tbody v-for="(h, hi) in f.hunks" :key="hi">
                <template v-if="h.from < MAX_LINES">
                  <tr v-if="h.skipped" class="gap">
                    <td class="num"><Icon name="more" :size="12" /></td>
                    <td>{{ h.skipped }} unmodified line{{ h.skipped === 1 ? '' : 's' }}</td>
                  </tr>
                  <tr
                    v-for="(l, li) in h.lines.slice(0, MAX_LINES - h.from)"
                    :key="li"
                    :class="l.t === '+' ? 'add' : l.t === '-' ? 'del' : ''"
                  >
                    <td class="num">{{ l.t === '-' ? l.o : l.n }}</td>
                    <td class="src">
                      <template v-if="pieces(f, l)"
                        ><span v-for="(p, pi) in pieces(f, l)" :key="pi" :class="p[1]">{{ p[0] }}</span></template
                      ><template v-else>{{ l.text }}</template>
                    </td>
                  </tr>
                </template>
              </tbody>
            </table>
            <div v-if="f.size > MAX_LINES" class="note">
              {{ f.size - MAX_LINES }} more lines not shown. Open the file to see all of it.
            </div>
          </template>
        </section>
      </div>

      <nav v-if="files && files.length" class="list">
        <button v-for="f in files" :key="f.path" :title="f.path" @click="jump(f.path)">
          <span class="name">{{ splitPath(f.path)[1] }}</span>
          <span class="dir">{{ splitPath(f.path)[0].replace(/\/$/, '') }}</span>
          <span class="grow"></span>
          <b v-if="f.added" class="add">+{{ f.added }}</b>
          <b v-if="f.removed" class="del">-{{ f.removed }}</b>
        </button>
      </nav>
    </div>
  </div>
</template>

<style scoped>
.dp {
  position: relative;
  flex: 0 0 50%;
  min-width: 300px;
  max-width: 85%;
  display: flex;
  flex-direction: column;
  min-height: 0;
  border-left: 1px solid color-mix(in srgb, var(--fg) 14%, transparent);
  font-family: var(--font, system-ui);
  font-size: 12.5px;
  container-type: inline-size;
}
/* the left edge: drag to make the panel wider or narrower */
.grip {
  position: absolute;
  left: -4px;
  top: 0;
  bottom: 0;
  width: 8px;
  z-index: 4;
  cursor: col-resize;
}
.grip::after {
  content: '';
  position: absolute;
  left: 3px;
  top: 0;
  bottom: 0;
  width: 2px;
  background: transparent;
  transition: background 0.15s;
}
.grip:hover::after,
.dragging .grip::after {
  background: var(--accent);
}
.dragging {
  user-select: none;
}
.dragging .files {
  pointer-events: none;
}
.top,
.sub {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 0 8px 0 12px;
  white-space: nowrap;
}
.top {
  height: 32px;
  flex: none;
}
.where {
  overflow: hidden;
  text-overflow: ellipsis;
  opacity: 0.85;
}
.stat {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  opacity: 0.9;
}
.grow {
  flex: 1;
}
.iconbtn {
  width: 24px;
  height: 24px;
  color: inherit;
}
.spinning :deep(svg) {
  animation: turn 0.8s linear infinite;
}
@keyframes turn {
  to {
    transform: rotate(360deg);
  }
}
.sub {
  height: 28px;
  flex: none;
  font-weight: 600;
  border-bottom: 1px solid color-mix(in srgb, var(--fg) 12%, transparent);
}
.add {
  color: var(--ok);
}
.del {
  color: var(--danger);
}
.dp-body {
  flex: 1;
  min-height: 0;
  display: flex;
}
.files {
  flex: 1;
  min-width: 0;
  overflow: auto;
  padding: 10px;
}
.note {
  padding: 10px 12px;
  opacity: 0.6;
  font-family: var(--font, system-ui);
  font-size: 12px;
}
.note.err {
  color: var(--danger);
  opacity: 1;
  white-space: pre-wrap;
}
.file {
  /* files scrolled out of sight are not laid out or painted */
  content-visibility: auto;
  contain-intrinsic-size: auto 400px;
  margin-bottom: 12px;
  border: 1px solid color-mix(in srgb, var(--fg) 12%, transparent);
  border-radius: 8px;
  overflow: hidden;
}
.file header {
  position: sticky;
  top: -10px;
  z-index: 1;
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 10px;
  background: color-mix(in srgb, var(--fg) 7%, var(--bgc));
  border-bottom: 1px solid color-mix(in srgb, var(--fg) 10%, transparent);
  cursor: pointer;
  font-family: var(--font, system-ui);
  font-size: 12.5px;
}
.file header .path {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.file header .counts {
  margin-left: auto;
  padding: 1px 7px;
  border-radius: 5px;
  border: 1px solid color-mix(in srgb, var(--fg) 14%, transparent);
  white-space: nowrap;
  font-size: 11.5px;
}
.tag {
  padding: 0 6px;
  border-radius: 4px;
  font-size: 10.5px;
  background: color-mix(in srgb, var(--ok) 20%, transparent);
  color: var(--ok);
}
.tag.del {
  background: color-mix(in srgb, var(--danger) 20%, transparent);
  color: var(--danger);
}
.code {
  width: 100%;
  border-collapse: collapse;
  line-height: 1.55;
}
.code td {
  padding: 0 10px 0 0;
  white-space: pre;
  vertical-align: top;
}
.code .num {
  width: 1%;
  min-width: 44px;
  padding: 0 12px 0 10px;
  text-align: right;
  opacity: 0.45;
  user-select: none;
}
.code tr.add {
  background: color-mix(in srgb, var(--ok) 16%, transparent);
}
.code tr.add .num {
  box-shadow: inset 3px 0 0 var(--ok);
  opacity: 0.8;
}
.code tr.del {
  background: color-mix(in srgb, var(--danger) 16%, transparent);
}
.code tr.del .num {
  box-shadow: inset 3px 0 0 var(--danger);
  opacity: 0.8;
}
.code tr.gap td {
  padding-top: 2px;
  padding-bottom: 2px;
  font-family: var(--font, system-ui);
  font-size: 11px;
  opacity: 0.5;
  background: color-mix(in srgb, var(--fg) 4%, transparent);
}
.tk-k {
  color: var(--c-k);
}
.tk-i {
  font-style: italic;
}
.tk-s {
  color: var(--c-s);
}
.tk-o {
  color: var(--c-o);
}
.tk-n {
  color: var(--c-n);
}
.tk-c {
  color: var(--c-c);
  font-style: italic;
}
.tk-f {
  color: var(--c-f);
}
.tk-t {
  color: var(--c-t);
}
.tk-p {
  color: var(--c-p);
}
.tk-d {
  color: var(--c-d);
}
.list {
  flex: 0 0 230px;
  overflow-y: auto;
  padding: 8px 6px;
  border-left: 1px solid color-mix(in srgb, var(--fg) 10%, transparent);
}
.list button {
  display: flex;
  align-items: baseline;
  gap: 6px;
  width: 100%;
  padding: 4px 8px;
  border: 0;
  border-radius: 5px;
  background: none;
  color: inherit;
  font: inherit;
  font-size: 12px;
  text-align: left;
  white-space: nowrap;
  cursor: pointer;
}
.list button:hover {
  background: color-mix(in srgb, var(--fg) 8%, transparent);
}
.list .name {
  flex: none;
}
.list .dir {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  font-size: 11px;
  opacity: 0.5;
}
.list b {
  font-size: 11px;
  font-weight: 500;
}
@container (max-width: 620px) {
  .list {
    display: none;
  }
}
</style>

<script setup>
// Plays a terminal recording (asciicast v1/v2/v3 `.cast` file) in a read-only terminal.
import { ref, computed, watch, onMounted, onBeforeUnmount, nextTick } from 'vue'
import { Terminal } from '@xterm/xterm'
import { api } from '../lib/api'
import { store, err } from '../lib/store'
import { termTheme } from '../lib/appearance'
import { parseCast, limitIdle, fmtClock } from '../lib/cast'
import Icon from './Icon.vue'

const props = defineProps({ tab: Object, visible: Boolean })

const SPEEDS = [0.5, 1, 1.5, 2, 4]
const IDLE = 2 // seconds a pause is shortened to with "Skip pauses" on

const el = ref(null)
const error = ref('')
const loading = ref(true)
const cast = ref(null)
const playing = ref(false)
const pos = ref(0)
const speed = ref(1)
const skipIdle = ref(true)

let term = null,
  raw = null, // parsed file, times as recorded
  events = [], // times after the idle limit
  idx = 0, // next event to show
  raf = 0,
  wallStart = 0,
  posStart = 0

const duration = computed(() => (events.length ? events.at(-1)[0] : 0))
const S = () => store.vault.settings || {}

function apply() {
  const limit = skipIdle.value ? Math.min(IDLE, raw.idleLimit || IDLE) : raw.idleLimit
  events = limitIdle(raw.events, limit)
}

/** Show every event up to and including `upto`. */
function advance(upto) {
  let out = ''
  while (idx < events.length && events[idx][0] <= upto) {
    const [, kind, data] = events[idx++]
    if (kind === 'o') out += data
    else {
      const m = /^(\d+)x(\d+)$/.exec(data)
      if (m) {
        if (out) term.write(out)
        out = ''
        term.resize(+m[1], +m[2])
      }
    }
  }
  if (out) term.write(out)
}

function frame() {
  const t = posStart + ((performance.now() - wallStart) / 1000) * speed.value
  if (t >= duration.value) {
    advance(duration.value)
    pos.value = duration.value
    playing.value = false
    return
  }
  advance(t)
  pos.value = t
  raf = requestAnimationFrame(frame)
}
function play() {
  if (!term || !events.length) return
  if (pos.value >= duration.value) seek(0)
  playing.value = true
  wallStart = performance.now()
  posStart = pos.value
  cancelAnimationFrame(raf)
  raf = requestAnimationFrame(frame)
}
function pause() {
  playing.value = false
  cancelAnimationFrame(raf)
}
const toggle = () => (playing.value ? pause() : play())

/** Jump to `t` seconds: redraw from the start up to there. */
function seek(t) {
  t = Math.max(0, Math.min(duration.value, +t || 0))
  const was = playing.value
  pause()
  term.reset()
  term.resize(cast.value.cols, cast.value.rows)
  idx = 0
  advance(t)
  pos.value = t
  if (was && t < duration.value) play()
}
function setSpeed(v) {
  speed.value = v
  if (playing.value) play() // restart the clock at the new speed
}
function toggleIdle() {
  // stay at the same event while the timeline changes (limitIdle keeps events one to one)
  const i = idx
  skipIdle.value = !skipIdle.value
  apply()
  seek(i ? events[i - 1][0] : 0)
}

function key(e) {
  if (e.key === ' ' || e.key === 'k') toggle()
  else if (e.key === 'ArrowRight') seek(pos.value + 5)
  else if (e.key === 'ArrowLeft') seek(pos.value - 5)
  else if (e.key === 'Home') seek(0)
  else if (e.key === 'End') seek(duration.value)
  else if (e.key === '>' || e.key === '.')
    setSpeed(SPEEDS[Math.min(SPEEDS.length - 1, SPEEDS.indexOf(speed.value) + 1)])
  else if (e.key === '<' || e.key === ',') setSpeed(SPEEDS[Math.max(0, SPEEDS.indexOf(speed.value) - 1)])
  else return
  e.preventDefault()
}

onMounted(async () => {
  try {
    raw = parseCast(await api.cast_read({ path: props.tab.path }))
  } catch (e) {
    error.value = String(e?.message || e)
    loading.value = false
    return
  }
  cast.value = raw
  apply()
  const s = S()
  term = new Terminal({
    cols: raw.cols,
    rows: raw.rows,
    fontFamily: s.font_family || 'Menlo, monospace',
    fontSize: s.font_size || 13,
    lineHeight: s.line_height || 1.2,
    letterSpacing: +s.letter_spacing || 0,
    fontWeight: s.font_weight || 'normal',
    fontWeightBold: s.font_weight_bold || 'bold',
    drawBoldTextInBrightColors: s.bold_bright ?? true,
    theme: termTheme(),
    cursorBlink: false,
    disableStdin: true,
    scrollback: 5000,
    allowProposedApi: true,
  })
  loading.value = false
  await nextTick()
  term.open(el.value)
  if (props.visible) play()
})

watch(
  () => props.visible,
  (v) => !v && pause(),
)
watch(
  () => [store.vault.settings, store.systemDark],
  () => term && (term.options.theme = termTheme()),
  { deep: true },
)
onBeforeUnmount(() => {
  cancelAnimationFrame(raf)
  term?.dispose()
})

const reveal = () => api.rec_reveal({ path: props.tab.path }).catch(err)
</script>

<template>
  <div class="player" tabindex="0" @keydown="key" :style="{ background: termTheme().background }">
    <div v-if="error" class="msg">
      <b>Couldn't play this recording</b>
      <span>{{ error }}</span>
      <span class="dim">{{ tab.path }}</span>
    </div>
    <div v-else-if="loading" class="msg">Loading…</div>
    <template v-else>
      <div class="stage" @click="toggle">
        <div ref="el" class="screen"></div>
      </div>
      <div class="bar">
        <button class="iconbtn" @click="toggle" :title="playing ? 'Pause (Space)' : 'Play (Space)'">
          <Icon :name="playing ? 'pause' : 'play'" :size="15" />
        </button>
        <span class="time">{{ fmtClock(pos) }} / {{ fmtClock(duration) }}</span>
        <input
          class="seek"
          type="range"
          min="0"
          :max="duration"
          step="0.05"
          :value="pos"
          @input="seek($event.target.value)"
          title="Seek (← / →: 5 seconds)"
        />
        <div class="speeds" title="Playback speed (< / >)">
          <button v-for="v in SPEEDS" :key="v" :class="{ on: speed === v }" @click="setSpeed(v)">{{ v }}×</button>
        </div>
        <button
          class="chip"
          :class="{ on: skipIdle }"
          @click="toggleIdle"
          :title="`Shorten pauses longer than ${IDLE} seconds`"
        >
          Skip pauses
        </button>
        <span class="size" :title="cast.title">{{ cast.cols }}×{{ cast.rows }}</span>
        <button class="iconbtn" @click="reveal" title="Show in folder"><Icon name="folder" :size="14" /></button>
      </div>
    </template>
  </div>
</template>

<style scoped>
.player {
  position: absolute;
  inset: 0;
  display: flex;
  flex-direction: column;
  outline: none;
}
.stage {
  flex: 1;
  min-height: 0;
  overflow: auto;
  display: flex;
  padding: 14px;
  cursor: pointer;
}
.screen {
  margin: auto;
}
.bar {
  flex: none;
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 6px 12px;
  background: var(--bg-2);
  border-top: 1px solid var(--line);
  font-size: 12px;
}
.time,
.size {
  font-variant-numeric: tabular-nums;
  color: var(--text-2, inherit);
  white-space: nowrap;
}
.size {
  opacity: 0.6;
}
.seek {
  flex: 1;
  min-width: 80px;
  accent-color: var(--accent);
}
.speeds {
  display: flex;
  border: 1px solid var(--line);
  border-radius: 7px;
  overflow: hidden;
}
.speeds button,
.chip {
  border: 0;
  background: none;
  color: inherit;
  font: inherit;
  font-size: 11px;
  padding: 3px 7px;
  cursor: pointer;
}
.speeds button.on {
  background: var(--accent);
  color: #fff;
}
.chip {
  border: 1px solid var(--line);
  border-radius: 7px;
}
.chip.on {
  border-color: var(--accent);
  color: var(--accent);
}
.msg {
  margin: auto;
  display: flex;
  flex-direction: column;
  gap: 6px;
  align-items: center;
  text-align: center;
  color: var(--text);
  padding: 20px;
  word-break: break-all;
}
.dim {
  opacity: 0.6;
  font-size: 12px;
}
</style>

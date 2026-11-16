<script setup>
// Chips to pick from on top, the chosen ones below in order: click or drag to add, drag to
// reorder, × or ⌫ to remove. Used by the prompt editor and the command box editor.
import { computed, ref } from 'vue'
import Icon from './Icon.vue'

const props = defineProps({
  /** chosen ids, in order */
  modelValue: Array,
  /** id -> { label, icon, sample, color (ANSI index), multi?, hint? } */
  catalogue: Object,
  /** terminal theme, for the chips' colours */
  colors: Object,
  title: String,
  /** ids "Restore default" goes back to */
  defaults: Array,
  /** shown in the empty drop area */
  placeholder: { type: String, default: 'Click or drag chips here' },
})
const emit = defineEmits(['update:modelValue'])

const ANSI = ['black', 'red', 'green', 'yellow', 'blue', 'magenta', 'cyan', 'white']
const colorOf = (id) =>
  id === 'newline' ? props.colors.foreground : props.colors[ANSI[props.catalogue[id]?.color ?? 7]]
const text = (id) => (id === 'newline' ? 'Line break' : props.catalogue[id]?.sample)

const list = computed(() => props.modelValue || [])
const set = (v) => emit('update:modelValue', v)
// chips not used yet (line breaks can be used any number of times)
const available = computed(() =>
  Object.keys(props.catalogue).filter((id) => props.catalogue[id].multi || !list.value.includes(id)),
)
const isDefault = computed(() => list.value.join() === (props.defaults || []).join())

function add(id, at = list.value.length) {
  const v = [...list.value]
  v.splice(at, 0, id)
  set(v)
}
function remove(i) {
  const v = [...list.value]
  v.splice(i, 1)
  set(v)
}
function move(i, d) {
  const j = i + d
  if (j < 0 || j >= list.value.length) return
  const v = [...list.value]
  const [x] = v.splice(i, 1)
  v.splice(j, 0, x)
  set(v)
}

// ---------- drag & drop ----------
// Mouse events, not HTML5 drag & drop: Tauri keeps native drag & drop for file drops, so the
// webview never receives HTML5 drag events.
const drag = ref(null) // { from: 'palette' | 'zone', id, index, x, y, where: 'zone' | 'palette' | null }
const over = ref(-1) // insertion index in the zone
const zoneEl = ref(null)
const paletteEl = ref(null)
let justDragged = false

/** Insertion index for a pointer position: before the first chip on the pointer's row whose middle is right of it. */
function indexAt(x, y) {
  const chips = [...(zoneEl.value?.querySelectorAll('.seg.placed') || [])]
  for (let i = 0; i < chips.length; i++) {
    const r = chips[i].getBoundingClientRect()
    if (y < r.top - 4) return i // pointer is on an earlier row
    if (y <= r.bottom + 4 && x < r.left + r.width / 2) return i
  }
  return chips.length
}
const inside = (el, x, y) => {
  const r = el?.getBoundingClientRect()
  return r && x >= r.left && x <= r.right && y >= r.top && y <= r.bottom
}
function grab(e, from, id, index = -1) {
  if (e.button !== 0 || e.target.closest('.x')) return
  const sx = e.clientX,
    sy = e.clientY
  let started = false
  const move = (ev) => {
    if (!started) {
      if (Math.hypot(ev.clientX - sx, ev.clientY - sy) < 4) return
      started = true
      drag.value = { from, id, index, x: 0, y: 0, where: null }
    }
    const d = drag.value
    d.x = ev.clientX
    d.y = ev.clientY
    d.where = inside(zoneEl.value, d.x, d.y) ? 'zone' : inside(paletteEl.value, d.x, d.y) ? 'palette' : null
    over.value = d.where === 'zone' ? indexAt(d.x, d.y) : -1
  }
  const up = () => {
    window.removeEventListener('mousemove', move)
    window.removeEventListener('mouseup', up)
    if (!started) return
    const d = drag.value
    if (d.where === 'zone') {
      let at = over.value
      const v = [...list.value]
      if (d.from === 'zone') {
        v.splice(d.index, 1)
        if (d.index < at) at--
      }
      v.splice(at, 0, d.id)
      set(v)
    } else if (d.where === 'palette' && d.from === 'zone') remove(d.index)
    drag.value = null
    over.value = -1
    justDragged = true // the click after a drag must not also add the chip
    setTimeout(() => (justDragged = false))
  }
  window.addEventListener('mousemove', move)
  window.addEventListener('mouseup', up)
  e.preventDefault()
}
function clickAdd(id) {
  if (!justDragged) add(id)
}
function chipKey(e, i) {
  if (e.key === 'Backspace' || e.key === 'Delete') remove(i)
  else if (e.altKey && e.key === 'ArrowLeft') move(i, -1)
  else if (e.altKey && e.key === 'ArrowRight') move(i, 1)
  else return
  e.preventDefault()
}

defineExpose({ dragging: () => !!drag.value })
</script>

<template>
  <div class="builder" :style="{ background: colors.background, color: colors.foreground }">
    <div class="bhead">
      <b>{{ title }}</b>
      <button class="link" :disabled="isDefault" @click="set([...defaults])">Restore default</button>
    </div>

    <div ref="paletteEl" class="palette" :class="{ target: drag?.from === 'zone' && drag.where === 'palette' }">
      <button
        v-for="id in available"
        :key="id"
        class="seg"
        :title="`${catalogue[id].label}${catalogue[id].hint ? ' — ' + catalogue[id].hint : ''} · click or drag to add`"
        :style="{ color: colorOf(id) }"
        @mousedown="grab($event, 'palette', id)"
        @click="clickAdd(id)"
      >
        <Icon :name="catalogue[id].icon" :size="12" />
        {{ text(id) }}
      </button>
      <span v-if="!available.length" class="placeholder">All chips are in use</span>
    </div>

    <div class="zone" :class="{ active: drag?.where === 'zone' }" ref="zoneEl">
      <template v-for="(id, i) in list" :key="i + id">
        <span
          v-if="catalogue[id]"
          class="seg placed"
          :class="{
            nl: id === 'newline',
            dragging: drag?.from === 'zone' && drag.index === i,
            'ins-before': over === i,
            'ins-after': i === list.length - 1 && over === list.length,
          }"
          tabindex="0"
          :title="`${catalogue[id].label} · drag to reorder, ⌫ to remove`"
          :style="{ color: colorOf(id) }"
          @mousedown="grab($event, 'zone', id, i)"
          @keydown="chipKey($event, i)"
        >
          <Icon :name="catalogue[id].icon" :size="12" />
          {{ text(id) }}
          <button class="x" title="Remove" @click="remove(i)"><Icon name="x" :size="11" /></button>
        </span>
      </template>

      <span v-if="!list.length" class="placeholder">{{ placeholder }}</span>
    </div>
  </div>

  <Teleport to="body">
    <div
      v-if="drag"
      class="seg floating"
      :style="{ left: drag.x + 'px', top: drag.y + 'px', color: colorOf(drag.id), background: colors.background }"
    >
      <Icon :name="catalogue[drag.id].icon" :size="12" />
      {{ text(drag.id) }}
    </div>
  </Teleport>
</template>

<style scoped>
.builder {
  border-radius: 12px;
  border: 1px solid var(--accent);
  padding: 14px 16px 16px;
}
.bhead {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 10px;
}
.link {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  border: 0;
  background: none;
  color: inherit;
  cursor: pointer;
  font-size: 12px;
  opacity: 0.85;
}
.link:hover:not(:disabled) {
  opacity: 1;
  text-decoration: underline;
}
.link:disabled {
  opacity: 0.35;
  cursor: default;
}
.palette,
.zone {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 6px;
}
.palette {
  min-height: 30px;
  margin-bottom: 12px;
}
.zone {
  min-height: 52px;
  padding: 10px;
  border-radius: 9px;
  border: 1.5px dashed rgba(127, 127, 127, 0.45);
  transition: border-color 0.12s;
}
.zone.active {
  border-color: var(--accent);
}
.seg {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  height: 26px;
  padding: 0 8px;
  border-radius: 6px;
  border: 1px solid rgba(127, 127, 127, 0.28);
  background: rgba(127, 127, 127, 0.12);
  font:
    600 12px ui-monospace,
    'JetBrains Mono',
    Menlo,
    Consolas,
    monospace;
  cursor: grab;
  white-space: nowrap;
}
.seg:hover {
  border-color: rgba(127, 127, 127, 0.6);
}
.seg.placed {
  padding-right: 3px;
  outline: none;
}
.seg.placed:focus-visible {
  box-shadow: 0 0 0 2px var(--accent);
}
.seg.nl {
  flex-basis: auto;
  opacity: 0.75;
}
.seg.floating {
  position: fixed;
  z-index: 200;
  transform: translate(-50%, -50%);
  pointer-events: none;
  box-shadow: var(--shadow);
}
.palette.target {
  outline: 1.5px dashed #f87171;
  outline-offset: 4px;
  border-radius: 6px;
}
.seg.dragging {
  opacity: 0.35;
}
.x {
  display: grid;
  place-items: center;
  width: 18px;
  height: 18px;
  border: 0;
  border-radius: 4px;
  background: none;
  color: #f87171;
  cursor: pointer;
}
.x:hover {
  background: rgba(248, 113, 113, 0.18);
}
/* insertion marker drawn as a shadow so the layout never shifts under the pointer */
.seg.ins-before {
  box-shadow: -5px 0 0 -2px var(--accent);
}
.seg.ins-after {
  box-shadow: 5px 0 0 -2px var(--accent);
}
.placeholder {
  font-size: 12px;
  opacity: 0.55;
}
</style>

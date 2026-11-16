<script setup>
// A searchable list that opens above a chip of the command box (folders, git branches).
import { computed, nextTick, onMounted, ref, watch } from 'vue'
import Icon from './Icon.vue'

const props = defineProps({
  /** [{ label, icon, value, current?, muted? }], or null while loading */
  items: Array,
  placeholder: String,
  /** shown when there is nothing to list */
  empty: String,
  /** left edge in px, inside the command box */
  left: Number,
})
const emit = defineEmits(['pick', 'close'])
const q = ref('')
const sel = ref(0)
const search = ref(null)
const listEl = ref(null)

const shown = computed(() => {
  const s = q.value.trim().toLowerCase()
  const all = props.items || []
  return s ? all.filter((i) => i.label.toLowerCase().includes(s)) : all
})
watch(shown, () => (sel.value = 0))
onMounted(() => nextTick(() => search.value?.focus()))

function move(d) {
  const n = shown.value.length
  if (!n) return
  sel.value = (sel.value + d + n) % n
  nextTick(() => listEl.value?.querySelector('.on')?.scrollIntoView({ block: 'nearest' }))
}
function onKey(e) {
  if (e.key === 'ArrowDown' || (e.key === 'Tab' && !e.shiftKey)) move(1)
  else if (e.key === 'ArrowUp' || (e.key === 'Tab' && e.shiftKey)) move(-1)
  else if (e.key === 'Enter') shown.value[sel.value] && emit('pick', shown.value[sel.value])
  else if (e.key === 'Escape') emit('close')
  else return
  e.preventDefault()
  e.stopPropagation()
}
</script>

<template>
  <div class="picker" :style="{ left: left + 'px' }" @mousedown.stop>
    <input
      ref="search"
      v-model="q"
      :placeholder="placeholder"
      spellcheck="false"
      autocomplete="off"
      @keydown="onKey"
      @blur="emit('close')"
    />
    <div ref="listEl" class="list" @mousedown.prevent>
      <div v-if="!items" class="note">Loading…</div>
      <div v-else-if="!shown.length" class="note">{{ q ? 'No matches' : empty }}</div>
      <div
        v-for="(it, i) in shown.slice(0, 500)"
        :key="it.value + it.label"
        class="item"
        :class="{ on: i === sel, muted: it.muted }"
        @mousemove="sel = i"
        @click="emit('pick', it)"
      >
        <Icon :name="it.icon" :size="13" />
        <span class="label">{{ it.label }}</span>
        <Icon v-if="it.current" name="check" :size="13" class="cur" />
      </div>
    </div>
  </div>
</template>

<style scoped>
.picker {
  position: absolute;
  bottom: calc(100% - 4px);
  z-index: 7;
  width: min(420px, calc(100% - 16px));
  display: flex;
  flex-direction: column;
  border-radius: 9px;
  border: 1px solid var(--line);
  background: var(--bg-2);
  color: var(--text);
  box-shadow: 0 10px 28px rgba(0, 0, 0, 0.4);
  overflow: hidden;
  font-family: var(--font, system-ui);
  font-size: 13px;
}
input {
  border: 0;
  border-bottom: 1px solid var(--line);
  outline: 0;
  padding: 9px 12px;
  background: color-mix(in srgb, var(--bg) 60%, transparent);
  color: inherit;
  font: inherit;
}
.list {
  max-height: 300px;
  overflow-y: auto;
  padding: 4px 0;
}
.item {
  display: flex;
  align-items: center;
  gap: 9px;
  padding: 6px 14px;
  cursor: pointer;
  white-space: nowrap;
}
.item :deep(svg) {
  flex: none;
  opacity: 0.7;
}
.item .label {
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
}
.item.muted {
  color: var(--text-3);
}
.item.on {
  background: var(--accent);
  color: #fff;
}
.item.on :deep(svg) {
  opacity: 1;
}
.item .cur {
  color: var(--ok);
}
.item.on .cur {
  color: inherit;
}
.note {
  padding: 8px 14px;
  color: var(--text-3);
}
</style>

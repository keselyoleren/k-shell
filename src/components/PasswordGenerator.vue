<script>
import { reactive } from 'vue'
import { DEFAULT_OPTS } from '../lib/passgen'
// Shared by every generator so the last choice is kept while the app runs.
const opts = reactive({ ...DEFAULT_OPTS })
</script>

<script setup>
import { ref, watch } from 'vue'
import { generatePassword, strength } from '../lib/passgen'
import Icon from './Icon.vue'

const emit = defineEmits(['use', 'close'])
const value = ref('')
const regen = () => (value.value = generatePassword(opts))
watch(opts, regen, { immediate: true, deep: true })

const KINDS = [
  ['lower', 'a-z'],
  ['upper', 'A-Z'],
  ['digits', '0-9'],
  ['symbols', '!@#'],
]
// Keep at least one character set on.
function toggle(k) {
  if (opts[k] && KINDS.filter(([x]) => opts[x]).length === 1) return
  opts[k] = !opts[k]
}
</script>

<template>
  <div class="gen card">
    <div class="out">
      <span class="mono val">{{ value }}</span>
      <button type="button" class="iconbtn" title="Generate again" @click="regen"><Icon name="refresh" /></button>
    </div>
    <div class="meter" :data-level="strength(value).level">
      <i></i><i></i><i></i><i></i>
      <span class="dim">{{ strength(value).label }} · {{ strength(value).bits }} bits</span>
    </div>
    <div class="row len">
      <label class="dim">Length</label>
      <input type="range" min="8" max="64" v-model.number="opts.length" />
      <span class="mono num">{{ opts.length }}</span>
    </div>
    <div class="row kinds">
      <button
        v-for="[k, l] in KINDS"
        :key="k"
        type="button"
        class="chip mono"
        :class="{ on: opts[k] }"
        @click="toggle(k)"
      >
        {{ l }}
      </button>
      <label class="amb dim"><input type="checkbox" v-model="opts.avoidAmbiguous" /> No look-alikes (0 O 1 l I)</label>
    </div>
    <div class="row foot">
      <button type="button" class="btn" @click="emit('close')">Cancel</button>
      <button type="button" class="btn primary" @click="emit('use', value)">Use password</button>
    </div>
  </div>
</template>

<style scoped>
.gen {
  padding: 12px;
  display: flex;
  flex-direction: column;
  gap: 10px;
  margin-top: 6px;
}
.out {
  display: flex;
  align-items: center;
  gap: 6px;
  background: var(--bg);
  border-radius: 7px;
  padding: 6px 6px 6px 10px;
}
.val {
  flex: 1;
  font-size: 13px;
  word-break: break-all;
  -webkit-user-select: text;
  user-select: text;
}
.meter {
  display: flex;
  align-items: center;
  gap: 4px;
  font-size: 11.5px;
}
.meter i {
  width: 28px;
  height: 4px;
  border-radius: 2px;
  background: var(--bg-3);
}
.meter span {
  margin-left: 6px;
}
.meter[data-level='1'] i:nth-child(-n + 1) {
  background: var(--danger);
}
.meter[data-level='2'] i:nth-child(-n + 2) {
  background: var(--warn);
}
.meter[data-level='3'] i:nth-child(-n + 3),
.meter[data-level='4'] i {
  background: var(--ok);
}
.len {
  gap: 10px;
}
.len input {
  flex: 1;
}
.num {
  width: 22px;
  text-align: right;
}
.kinds {
  gap: 6px;
  flex-wrap: wrap;
}
.kinds .chip {
  cursor: pointer;
  border: 0;
  height: 24px;
}
.amb {
  display: flex;
  align-items: center;
  gap: 5px;
  font-size: 12px;
  margin-left: 4px;
}
.foot {
  justify-content: flex-end;
  gap: 6px;
}
</style>

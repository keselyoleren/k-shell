<script setup>
// Pick and order the chips above the command box (Settings → Terminal → Command input),
// with the same builder as the prompt editor.
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { store } from '../lib/store'
import { termTheme } from '../lib/appearance'
import { BOX_CHIPS, BOX_DEFAULTS, boxOptions } from '../lib/blocks'
import Icon from './Icon.vue'
import ChipBuilder from './ChipBuilder.vue'

const emit = defineEmits(['close'])
const initial = [...boxOptions(store.vault.settings).chips]
const draft = ref([...initial])
const changed = computed(() => draft.value.join() !== initial.join())
const th = computed(() => termTheme())

const ANSI = ['black', 'red', 'green', 'yellow', 'blue', 'magenta', 'cyan', 'white']
const colorOf = (id) => th.value[ANSI[BOX_CHIPS[id]?.color ?? 7]]

function save() {
  store.vault.settings.command_box = { ...boxOptions(store.vault.settings), chips: [...draft.value] }
  emit('close')
}
const builder = ref(null)
const esc = (e) => e.key === 'Escape' && !builder.value?.dragging() && emit('close')
onMounted(() => window.addEventListener('keydown', esc))
onBeforeUnmount(() => window.removeEventListener('keydown', esc))
</script>

<template>
  <div class="overlay" @mousedown.self="emit('close')">
    <div class="modal pe">
      <h2>Edit command box</h2>

      <ChipBuilder
        ref="builder"
        v-model="draft"
        :catalogue="BOX_CHIPS"
        :colors="th"
        title="Above the input"
        :defaults="BOX_DEFAULTS.chips"
      />

      <div class="previewbox">
        <div class="ptitle"><b>Preview</b></div>
        <div class="term mono" :style="{ background: th.background, color: th.foreground, '--fg': th.foreground }">
          <div class="chips">
            <span
              v-for="id in draft"
              :key="id"
              class="chip"
              :class="{ bad: id === 'exit' }"
              :style="id === 'exit' ? { color: th.red } : id === 'changes' ? null : { color: colorOf(id) }"
            >
              <Icon v-if="id !== 'exit' && id !== 'changes'" :name="BOX_CHIPS[id].icon" :size="11" />
              <template v-if="id === 'changes'"
                ><Icon name="file" :size="11" /> 3 • <i :style="{ color: th.green }">+42</i>
                <i :style="{ color: th.red }">-7</i></template
              >
              <template v-else>{{ BOX_CHIPS[id].sample }}</template>
            </span>
            <span v-if="!draft.length" class="dim">No chips: only the input is shown</span>
          </div>
          <div class="input">Type a command<i class="cur" :style="{ background: th.cursor }"></i></div>
        </div>
        <p class="dim note">
          Chips with nothing to show are hidden: the environment when none is active, the branch and changes outside a
          git repository, the exit code when the last command succeeded.
        </p>
      </div>

      <div class="modal-foot">
        <button class="btn" style="flex: 1; justify-content: center" @click="emit('close')">Cancel</button>
        <button class="btn primary" style="flex: 1; justify-content: center" :disabled="!changed" @click="save">
          Save changes
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.pe {
  width: min(680px, calc(100vw - 40px));
}
.previewbox {
  margin-top: 14px;
  padding: 14px 16px;
  border-radius: 12px;
  background: var(--bg-3);
}
.ptitle {
  margin-bottom: 10px;
}
.term {
  padding: 10px 12px;
  border-radius: 8px;
  font-size: 13px;
  line-height: 1.5;
}
.chips {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 6px;
  margin-bottom: 6px;
  font-size: 11.5px;
}
.chip {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 1px 7px;
  border-radius: 5px;
  border: 1px solid color-mix(in srgb, var(--fg) 16%, transparent);
  background: color-mix(in srgb, var(--fg) 6%, transparent);
  font-weight: 600;
  white-space: nowrap;
}
.chip i {
  font-style: normal;
}
.chip.bad {
  border-color: color-mix(in srgb, currentColor 40%, transparent);
}
.input {
  opacity: 0.45;
}
.cur {
  display: inline-block;
  width: 2px;
  height: 1.1em;
  margin-left: 2px;
  vertical-align: -0.2em;
}
.note {
  margin: 8px 0 0;
  font-size: 12px;
}
</style>

<script setup>
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { writeText } from '@tauri-apps/plugin-clipboard-manager'
import { store, toast } from '../lib/store'
import { termTheme } from '../lib/appearance'
import { SEGMENTS, DEFAULT_PROMPT, promptRcSnippet } from '../lib/prompt'
import Icon from './Icon.vue'
import ChipBuilder from './ChipBuilder.vue'

const emit = defineEmits(['close'])
const initial = [...(store.vault.settings.prompt_segments || DEFAULT_PROMPT)]
const draft = ref([...initial])
const changed = computed(() => draft.value.join() !== initial.join())

const ANSI = ['black', 'red', 'green', 'yellow', 'blue', 'magenta', 'cyan', 'white']
const th = computed(() => termTheme())
const colorOf = (id) => th.value[ANSI[SEGMENTS[id]?.color ?? 7]]

// preview lines: [[id, …], …] split on line breaks
const lines = computed(() => {
  const out = [[]]
  for (const id of draft.value) id === 'newline' ? out.push([]) : out.at(-1).push(id)
  return out
})

function save() {
  store.vault.settings.prompt_segments = [...draft.value]
  emit('close')
}
async function copyRc() {
  try {
    await writeText(promptRcSnippet(draft.value))
    toast('Copied — paste it at the end of ~/.bashrc or ~/.zshrc')
  } catch (e) {
    toast(String(e), 'error')
  }
}
const builder = ref(null)
const esc = (e) => e.key === 'Escape' && !builder.value?.dragging() && emit('close')
onMounted(() => window.addEventListener('keydown', esc))
onBeforeUnmount(() => window.removeEventListener('keydown', esc))
</script>

<template>
  <div class="overlay" @mousedown.self="emit('close')">
    <div class="modal pe">
      <h2>Edit prompt</h2>

      <ChipBuilder
        ref="builder"
        v-model="draft"
        :catalogue="SEGMENTS"
        :colors="th"
        title="K-Shell prompt"
        :defaults="DEFAULT_PROMPT"
        placeholder="Click or drag segments here"
      />

      <div class="previewbox">
        <div class="ptitle">
          <b>Preview</b>
          <button class="link" @click="copyRc"><Icon name="copy" :size="12" /> Copy for .bashrc / .zshrc</button>
        </div>
        <div class="term mono" :style="{ background: th.background, color: th.foreground }">
          <div v-for="(line, li) in lines" :key="li">
            <template v-for="(id, k) in line" :key="k">
              <span :style="{ color: colorOf(id) }">{{ SEGMENTS[id].sample }}</span
              >{{ ' ' }}
            </template>
            <i v-if="li === lines.length - 1" class="cur" :style="{ background: th.cursor }"></i>
          </div>
        </div>
        <p class="dim note">
          Segments that don't apply (no git repo, no virtualenv, last command succeeded…) are hidden automatically.
        </p>
      </div>

      <div class="modal-foot">
        <button class="btn" style="flex: 1; justify-content: center" @click="emit('close')">Cancel</button>
        <button
          class="btn primary"
          style="flex: 1; justify-content: center"
          :disabled="!changed || !draft.length"
          @click="save"
        >
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
.ptitle {
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
.link:hover {
  opacity: 1;
  text-decoration: underline;
}
.previewbox {
  margin-top: 14px;
  padding: 14px 16px;
  border-radius: 12px;
  background: var(--bg-3);
}
.term {
  padding: 10px 12px;
  border-radius: 8px;
  font-size: 13px;
  line-height: 1.5;
  white-space: pre-wrap;
  word-break: break-all;
}
.cur {
  display: inline-block;
  width: 0.6em;
  height: 1.1em;
  vertical-align: -0.2em;
}
.note {
  margin: 8px 0 0;
  font-size: 12px;
}
</style>

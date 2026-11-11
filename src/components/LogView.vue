<script setup>
// Recent log lines of a container or pod: time column, error/warning colouring, filter, line count,
// "previous run" for a restarted pod, and a button that follows the logs live in a terminal tab.
import { computed, nextTick, ref, watch } from 'vue'
import Icon from './Icon.vue'

const props = defineProps({
  /** (tail, previous) => Promise<string> */
  load: Function,
  /** reloads when it changes (another container) */
  source: String,
  /** offer the logs of the run before the last restart */
  canPrevious: Boolean,
  /** opens a tab that follows the logs; null hides the button */
  follow: Function,
})

const tail = ref(200)
const previous = ref(false)
const wrap = ref(true)
const q = ref('')
const text = ref('')
const error = ref('')
const loading = ref(false)
const body = ref(null)

const ANSI = /\x1b\[[0-9;?]*[ -/]*[@-~]/g
const STAMP = /^(\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(?:\.\d+)?(?:Z|[+-]\d\d:\d\d))\s?/
const ERR = /\b(error|err|fatal|panic|exception|critical|failed|failure)\b/i
const WARN = /\b(warn|warning|deprecated)\b/i

const lines = computed(() =>
  text.value
    .split('\n')
    .filter((l, i, a) => l || i < a.length - 1)
    .map((raw) => {
      const m = raw.match(STAMP)
      const msg = (m ? raw.slice(m[0].length) : raw).replace(ANSI, '')
      const d = m ? new Date(m[1]) : null
      return {
        time: d ? d.toLocaleTimeString([], { hour12: false }) : '',
        full: d ? d.toLocaleString() : '',
        msg,
        level: ERR.test(msg) ? 'err' : WARN.test(msg) ? 'warn' : '',
      }
    }),
)
const shown = computed(() => {
  const s = q.value.trim().toLowerCase()
  return s ? lines.value.filter((l) => l.msg.toLowerCase().includes(s)) : lines.value
})
const errors = computed(() => lines.value.filter((l) => l.level === 'err').length)

let seq = 0
async function reload() {
  const n = ++seq
  loading.value = true
  error.value = ''
  try {
    const t = await props.load(tail.value, previous.value)
    if (n !== seq) return
    text.value = t
    await nextTick()
    if (body.value) body.value.scrollTop = body.value.scrollHeight
  } catch (e) {
    if (n === seq) {
      error.value = String(e)
      text.value = ''
    }
  } finally {
    if (n === seq) loading.value = false
  }
}
watch(
  () => props.source,
  () => {
    previous.value = false
    reload()
  },
  { immediate: true },
)
watch([tail, previous], reload)
defineExpose({ reload })
</script>

<template>
  <div class="logs">
    <div class="lbar">
      <div class="lsearch">
        <Icon name="search" :size="13" />
        <input v-model="q" placeholder="Filter lines…" spellcheck="false" />
      </div>
      <span class="count dim">
        {{ q.trim() ? `${shown.length} of ${lines.length}` : `${lines.length} lines` }}
        <span v-if="errors" class="errn">· {{ errors }} error{{ errors === 1 ? '' : 's' }}</span>
      </span>
      <label v-if="canPrevious" class="opt" title="Logs of the run before the last restart">
        <input type="checkbox" v-model="previous" /> Previous run
      </label>
      <select class="select sm" v-model.number="tail" title="How many recent lines to load">
        <option :value="100">Last 100</option>
        <option :value="200">Last 200</option>
        <option :value="1000">Last 1000</option>
        <option :value="5000">Last 5000</option>
      </select>
      <button class="iconbtn" :class="{ active: wrap }" @click="wrap = !wrap" title="Wrap long lines">
        <Icon name="newline" :size="13" />
      </button>
      <button class="iconbtn" :disabled="loading" @click="reload" title="Reload">
        <Icon name="refresh" :size="13" :class="{ spin: loading }" />
      </button>
      <button v-if="follow" class="btn sm" @click="follow()" title="Follow new lines live in a terminal tab">
        <Icon name="popout" :size="12" /> Follow
      </button>
    </div>
    <pre v-if="error" class="lerr">{{ error }}</pre>
    <div v-else ref="body" class="lbody mono" :class="{ wrap }">
      <div v-if="!lines.length && !loading" class="dim empty">No log output.</div>
      <div v-for="(l, i) in shown" :key="i" class="ln" :class="l.level">
        <span class="t" :title="l.full">{{ l.time }}</span
        ><span class="m">{{ l.msg }}</span>
      </div>
    </div>
  </div>
</template>

<style scoped>
.logs {
  display: flex;
  flex-direction: column;
  min-height: 0;
  flex: 1;
}
.lbar {
  display: flex;
  align-items: center;
  gap: 8px;
  padding-bottom: 8px;
}
.lsearch {
  display: flex;
  align-items: center;
  gap: 6px;
  flex: 1;
  min-width: 120px;
  max-width: 280px;
  height: 28px;
  padding: 0 8px;
  border: 1px solid var(--line);
  border-radius: 7px;
  background: var(--bg);
  color: var(--text-3);
}
.lsearch input {
  flex: 1;
  min-width: 0;
  border: 0;
  outline: 0;
  background: none;
  color: var(--text);
  font: inherit;
  font-size: 12px;
}
.count {
  font-size: 12px;
  margin-right: auto;
  white-space: nowrap;
}
.errn {
  color: var(--danger);
}
.opt {
  display: flex;
  align-items: center;
  gap: 4px;
  font-size: 12px;
  color: var(--text-2);
  white-space: nowrap;
}
.select.sm {
  height: 28px;
  font-size: 12px;
  padding: 0 6px;
  width: auto;
}
.lbody {
  flex: 1;
  min-height: 0;
  overflow: auto;
  background: var(--bg);
  border: 1px solid var(--line);
  border-radius: 8px;
  padding: 6px 0;
  font-size: 12px;
  line-height: 1.5;
}
.ln {
  display: flex;
  gap: 10px;
  padding: 0 10px;
  white-space: pre;
}
.lbody.wrap .ln .m {
  white-space: pre-wrap;
  word-break: break-word;
}
.ln:hover {
  background: var(--bg-3);
}
.ln .t {
  flex: none;
  color: var(--text-3);
  user-select: none;
}
.ln.err .m {
  color: var(--danger);
}
.ln.warn .m {
  color: var(--warn);
}
.empty {
  padding: 20px;
  text-align: center;
}
.lerr {
  white-space: pre-wrap;
  color: var(--danger);
  background: var(--bg-3);
  border-radius: 8px;
  padding: 10px 12px;
  font-size: 12px;
  margin: 0;
}
.spin {
  animation: spin 0.8s linear infinite;
}
@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}
</style>

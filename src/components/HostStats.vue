<script setup>
// Server stats beside an SSH terminal: host info, CPU, memory, network, disks and the busiest
// processes, with short history graphs. Polls `host_stats` over the terminal's own SSH
// connection, only while `active`.
import { ref, reactive, computed, watch, onBeforeUnmount } from 'vue'
import { api } from '../lib/api'
import { store } from '../lib/store'
import { cpuUsage, rate, pct, fmtUptime, fmtBytes, fmtRate, fmtPct, level, push } from '../lib/stats'
import Icon from './Icon.vue'
import Sparkline from './Sparkline.vue'

const props = defineProps({ host: Object, active: Boolean })
const emit = defineEmits(['close'])

const POINTS = 60
const stats = ref(null)
const cpu = ref(null) // { busy, user, system, iowait, steal } or { busy } (estimate)
const net = ref(null) // { rx, tx } bytes per second
const hist = reactive({ cpu: [], mem: [], rx: [], tx: [] })
const error = ref('')
const updated = ref(null)
let prev = null, // { ticks, net, at }
  timer = 0,
  busy = false,
  stopped = true

const interval = () => Math.max(1, +store.vault.settings?.stats_interval || 3)

async function poll() {
  clearTimeout(timer)
  if (stopped || busy) return
  busy = true
  let next = interval() * 1000
  try {
    const s = await api.host_stats({ host: props.host })
    if (stopped) return
    const at = performance.now()
    error.value = ''
    if (s.cpu_ticks) {
      const c = cpuUsage(prev?.ticks, s.cpu_ticks)
      if (c) cpu.value = c
      else if (!prev) next = 1000 // first sample: percentages need a second one
    } else cpu.value = s.cpu_percent != null ? { busy: s.cpu_percent } : null
    if (s.net && prev?.net) {
      const ms = at - prev.at
      net.value = { rx: rate(prev.net.rx, s.net.rx, ms), tx: rate(prev.net.tx, s.net.tx, ms) }
    }
    if (prev || !s.cpu_ticks) {
      push(hist.cpu, cpu.value?.busy ?? 0, POINTS)
      push(hist.mem, pct(s.mem_used, s.mem_total) ?? 0, POINTS)
      if (net.value) {
        push(hist.rx, net.value.rx ?? 0, POINTS)
        push(hist.tx, net.value.tx ?? 0, POINTS)
      }
    }
    prev = { ticks: s.cpu_ticks, net: s.net, at }
    stats.value = s
    updated.value = new Date()
  } catch (e) {
    if (stopped) return
    error.value = String(e)
  } finally {
    busy = false
  }
  if (!stopped) timer = setTimeout(poll, next)
}
function start() {
  if (!stopped) return
  stopped = false
  prev = null // rates over the time the panel was hidden would be misleading
  poll()
}
function stop() {
  stopped = true
  clearTimeout(timer)
}
watch(
  () => props.active,
  (a) => (a ? start() : stop()),
  { immediate: true },
)
onBeforeUnmount(stop)

const mem = computed(() => {
  const s = stats.value
  if (!s?.mem_total) return null
  const cache = s.mem_cache || 0
  return {
    pct: pct(s.mem_used, s.mem_total),
    used: s.mem_used,
    cache,
    free: Math.max(0, s.mem_total - s.mem_used - cache),
    total: s.mem_total,
  }
})
const swapPct = computed(() => stats.value && pct(stats.value.swap_used, stats.value.swap_total))
/** load average relative to the core count, as a % for colouring */
const loadPct = computed(() => {
  const s = stats.value
  return s?.load && s.cpus ? (100 * s.load[0]) / s.cpus : null
})
const subtitle = computed(() => {
  const s = stats.value
  if (!s) return ''
  const rel = s.release || s.os
  return s.kernel && !rel.includes(s.kernel) ? `${rel} · ${s.kernel}` : rel
})
const time = computed(() =>
  updated.value?.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' }),
)
const w = (p) => Math.max(0, Math.min(100, p || 0)) + '%'
</script>

<template>
  <aside class="hs">
    <header>
      <div class="who">
        <b :title="stats?.hostname || host.host">{{ stats?.hostname || host.label || host.host }}</b>
        <small v-if="subtitle" :title="subtitle">{{ subtitle }}</small>
      </div>
      <button class="iconbtn" title="Hide server stats" @click="emit('close')"><Icon name="x" :size="12" /></button>
    </header>

    <div v-if="error && !stats" class="msg">{{ error }}</div>
    <div v-else-if="!stats" class="msg"><span class="spin"></span> Reading stats…</div>
    <template v-else>
      <div class="chips">
        <span v-if="stats.uptime_secs != null" title="Time since the server started">
          <Icon name="clock" :size="11" /> up {{ fmtUptime(stats.uptime_secs) }}
        </span>
        <span v-if="stats.procs" title="Processes running on the server">{{ stats.procs }} processes</span>
      </div>

      <section>
        <div class="title">
          <span>CPU</span>
          <span class="big" :class="level(cpu?.busy)">{{ cpu ? fmtPct(cpu.busy) : '…' }}</span>
        </div>
        <Sparkline :series="[{ values: hist.cpu, color: 'var(--accent)' }]" :max="100" :points="POINTS" />
        <div v-if="cpu?.user != null" class="split">
          <span><i class="dot user"></i>user {{ fmtPct(cpu.user) }}</span>
          <span><i class="dot sys"></i>system {{ fmtPct(cpu.system) }}</span>
          <span v-if="cpu.iowait >= 1" :class="level(cpu.iowait * 4)" title="Time waiting for disk or network I/O">
            <i class="dot io"></i>iowait {{ fmtPct(cpu.iowait) }}
          </span>
          <span v-if="cpu.steal >= 1" title="Time the hypervisor gave to other virtual machines">
            steal {{ fmtPct(cpu.steal) }}
          </span>
        </div>
        <div v-if="stats.load" class="kv" title="Load average over 1, 5 and 15 minutes">
          <span>Load</span>
          <span :class="level(loadPct)">
            {{ stats.load.map((x) => x.toFixed(2)).join('  ') }}
            <em v-if="stats.cpus">· {{ stats.cpus }} {{ stats.cpus === 1 ? 'core' : 'cores' }}</em>
          </span>
        </div>
      </section>

      <section v-if="mem">
        <div class="title">
          <span>Memory</span>
          <span class="big" :class="level(mem.pct)">{{ fmtPct(mem.pct) }}</span>
        </div>
        <Sparkline :series="[{ values: hist.mem, color: 'var(--ok)' }]" :max="100" :points="POINTS" />
        <div
          class="stack"
          :title="`${fmtBytes(mem.used)} used, ${fmtBytes(mem.cache)} cache, ${fmtBytes(mem.free)} free`"
        >
          <i class="used" :style="{ width: w(pct(mem.used, mem.total)) }"></i>
          <i class="cache" :style="{ width: w(pct(mem.cache, mem.total)) }"></i>
        </div>
        <div class="legend">
          <span><i class="dot used"></i>used {{ fmtBytes(mem.used) }}</span>
          <span v-if="mem.cache" title="Page cache and buffers, given back to programs when needed"
            ><i class="dot cache"></i>cache {{ fmtBytes(mem.cache) }}</span
          >
          <span><i class="dot free"></i>free {{ fmtBytes(mem.free) }}</span>
        </div>
        <div class="kv">
          <span>Total</span><span>{{ fmtBytes(mem.total) }}</span>
        </div>
        <div v-if="stats.swap_total" class="kv">
          <span>Swap</span>
          <span :class="level(swapPct)">{{ fmtBytes(stats.swap_used) }} / {{ fmtBytes(stats.swap_total) }}</span>
        </div>
      </section>

      <section v-if="stats.net">
        <div class="title"><span>Network</span></div>
        <div class="rates">
          <span class="rx" title="Received"><Icon name="down" :size="11" /> {{ fmtRate(net?.rx) }}</span>
          <span class="tx" title="Sent"><Icon name="up" :size="11" /> {{ fmtRate(net?.tx) }}</span>
        </div>
        <Sparkline
          :series="[
            { values: hist.rx, color: 'var(--ok)' },
            { values: hist.tx, color: 'var(--accent)' },
          ]"
          :points="POINTS"
        />
        <div class="kv dim">
          <span>Since boot</span><span>↓ {{ fmtBytes(stats.net.rx) }} · ↑ {{ fmtBytes(stats.net.tx) }}</span>
        </div>
      </section>

      <section v-if="stats.disks.length">
        <div class="title"><span>Disks</span></div>
        <div v-for="d in stats.disks" :key="d.mount" class="disk">
          <div class="kv">
            <span class="mnt" :title="d.mount">{{ d.mount }}</span>
            <span :class="level(pct(d.used, d.size))">{{ fmtPct(pct(d.used, d.size)) }}</span>
          </div>
          <div class="bar" :class="level(pct(d.used, d.size))"><i :style="{ width: w(pct(d.used, d.size)) }"></i></div>
          <div class="sub">{{ fmtBytes(d.size - d.used) }} free of {{ fmtBytes(d.size) }}</div>
        </div>
      </section>

      <section v-if="stats.top.length">
        <div class="title"><span>Top processes</span><span class="cols">CPU · MEM</span></div>
        <div v-for="p in stats.top" :key="p.pid" class="proc" :title="`PID ${p.pid}`">
          <span class="pn">{{ p.name }}</span>
          <span :class="level(p.cpu)">{{ p.cpu.toFixed(1) }}%</span>
          <span>{{ p.mem.toFixed(1) }}%</span>
        </div>
      </section>

      <footer :class="{ err: error }">
        <span v-if="error" class="stale" :title="error">Not updating: {{ error }}</span>
        <span v-else>Every {{ interval() }}s · {{ time }}</span>
      </footer>
    </template>
  </aside>
</template>

<style scoped>
.hs {
  width: 264px;
  flex: none;
  overflow-y: auto;
  padding: 10px 12px 8px;
  font-size: 11.5px;
  line-height: 1.35;
  border-left: 1px solid rgba(127, 127, 127, 0.18);
  display: flex;
  flex-direction: column;
  gap: 12px;
}
header {
  display: flex;
  align-items: flex-start;
  gap: 6px;
}
.who {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
}
.who b {
  font-size: 13px;
}
.who b,
.who small {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.who small {
  opacity: 0.6;
  font-size: 10.5px;
}
header .iconbtn {
  width: 20px;
  height: 20px;
  color: inherit;
  flex: none;
}
.chips {
  display: flex;
  flex-wrap: wrap;
  gap: 4px;
  margin-top: -4px;
}
.chips span {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 1px 7px;
  border-radius: 10px;
  background: rgba(127, 127, 127, 0.14);
  font-size: 10.5px;
}
section {
  display: flex;
  flex-direction: column;
  gap: 5px;
  padding-top: 10px;
  border-top: 1px solid rgba(127, 127, 127, 0.14);
}
.title {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  font-size: 10.5px;
  font-weight: 600;
  letter-spacing: 0.04em;
  text-transform: uppercase;
  opacity: 0.85;
}
.title .big {
  font-size: 16px;
  font-weight: 600;
  letter-spacing: 0;
  font-variant-numeric: tabular-nums;
}
.title .cols {
  font-weight: 400;
  opacity: 0.6;
  text-transform: none;
}
.warn {
  color: var(--warn);
}
.crit {
  color: var(--danger);
}
.kv {
  display: flex;
  justify-content: space-between;
  gap: 8px;
  font-variant-numeric: tabular-nums;
}
.kv > span:first-child {
  opacity: 0.6;
  flex: none;
}
.kv em {
  font-style: normal;
  opacity: 0.6;
}
.kv.dim {
  opacity: 0.7;
  font-size: 10.5px;
}
.split,
.legend {
  display: flex;
  flex-wrap: wrap;
  gap: 2px 10px;
  font-size: 10.5px;
  font-variant-numeric: tabular-nums;
}
.dot {
  display: inline-block;
  width: 7px;
  height: 7px;
  border-radius: 2px;
  margin-right: 4px;
  vertical-align: 0;
}
.dot.user {
  background: var(--accent);
}
.dot.sys {
  background: color-mix(in srgb, var(--accent) 50%, transparent);
}
.dot.io {
  background: var(--warn);
}
.dot.used {
  background: var(--ok);
}
.dot.cache {
  background: color-mix(in srgb, var(--ok) 40%, transparent);
}
.dot.free {
  background: rgba(127, 127, 127, 0.3);
}
.stack,
.bar {
  display: flex;
  height: 6px;
  border-radius: 3px;
  background: rgba(127, 127, 127, 0.2);
  overflow: hidden;
}
.stack i,
.bar i {
  display: block;
  height: 100%;
  transition: width 0.4s;
}
.stack .used {
  background: var(--ok);
}
.stack .cache {
  background: color-mix(in srgb, var(--ok) 40%, transparent);
}
.bar i {
  background: var(--accent);
}
.bar.warn i {
  background: var(--warn);
}
.bar.crit i {
  background: var(--danger);
}
.rates {
  display: flex;
  justify-content: space-between;
  font-size: 13px;
  font-weight: 600;
  font-variant-numeric: tabular-nums;
}
.rates span {
  display: inline-flex;
  align-items: center;
  gap: 3px;
}
.rates .rx :deep(svg) {
  color: var(--ok);
}
.rates .tx :deep(svg) {
  color: var(--accent);
}
.disk {
  display: flex;
  flex-direction: column;
  gap: 3px;
  margin-bottom: 4px;
}
.mnt {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  opacity: 1 !important;
}
.sub {
  opacity: 0.55;
  font-size: 10.5px;
}
.proc {
  display: grid;
  grid-template-columns: 1fr 44px 40px;
  gap: 6px;
  font-variant-numeric: tabular-nums;
}
.proc span:not(.pn) {
  text-align: right;
}
.pn {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
footer {
  margin-top: auto;
  padding-top: 6px;
  font-size: 10px;
  opacity: 0.5;
}
footer.err {
  opacity: 1;
}
footer .stale {
  color: var(--warn);
}
.msg {
  opacity: 0.6;
  line-height: 1.4;
  word-break: break-word;
  display: flex;
  align-items: center;
  gap: 6px;
}
.spin {
  width: 12px;
  height: 12px;
  border: 2px solid rgba(127, 127, 127, 0.3);
  border-top-color: var(--accent);
  border-radius: 50%;
  animation: spin 0.8s linear infinite;
  flex: none;
}
@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}
</style>

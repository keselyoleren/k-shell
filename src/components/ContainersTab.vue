<script setup>
// Docker containers and Kubernetes pods of a host (or this computer): a list on the left, the
// selected one's overview, logs and volumes on the right, and Docker's disk use at the bottom.
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { store, openContainerShell, confirm, toast, err } from '../lib/store'
import { api, fmtSize } from '../lib/api'
import Icon from './Icon.vue'
import LogView from './LogView.vue'

const props = defineProps({ tab: Object, visible: Boolean })
const host = props.tab.host
const where = host ? host.label || host.host : 'this computer'
const engine = computed({
  get: () => props.tab.engine || 'docker',
  set: (v) => (props.tab.engine = v),
})
const q = ref('')

// ---------- helpers ----------
function ago(iso) {
  const t = Date.parse(iso)
  if (!iso || isNaN(t)) return ''
  const s = Math.max(0, (Date.now() - t) / 1000)
  for (const [n, u] of [
    [86400, 'd'],
    [3600, 'h'],
    [60, 'm'],
  ])
    if (s >= n) return Math.floor(s / n) + u
  return Math.floor(s) + 's'
}
/** "1.2MB / 3kB" → ["1.2MB", "3kB"] */
const io = (pair) => (pair || '').split(' / ').concat(['', '']).slice(0, 2)
/** a CPU limit: "1 CPU", "0.5 CPUs", "2 CPUs" */
const cpus = (n) => `${n} CPU${n === 1 ? '' : 's'}`
const when = (iso) => (iso ? new Date(iso).toLocaleString() : '')
const short = (id) => (id || '').replace(/^sha256:/, '').slice(0, 12)
const copy = (text, what) => navigator.clipboard.writeText(text).then(() => toast(`${what} copied`))
const match = (...fields) => {
  const s = q.value.trim().toLowerCase()
  return !s || fields.join(' ').toLowerCase().includes(s)
}
/** green / yellow / red / grey for a state or status word */
function tone(s) {
  s = (s || '').toLowerCase()
  if (/^(running|up|healthy|succeeded|completed)/.test(s)) return 'ok'
  if (/(pending|creating|restarting|starting|paused|init|terminating|waiting)/.test(s)) return 'warn'
  if (/(error|crash|backoff|fail|oom|unhealthy|evicted|dead|err)/.test(s)) return 'bad'
  return 'off'
}

// ================= Docker =================
const dk = ref({ loading: false, error: '', list: null })
const filter = ref('all') // all | running | stopped
const sel = ref('') // container id
const info = ref(null) // { detail, stats }
const infoErr = ref('')
const section = ref('overview')
const disk = ref([])
const busy = ref('')

async function loadDocker(quiet = false) {
  if (!quiet) dk.value = { ...dk.value, loading: true, error: '' }
  try {
    const list = await api.docker_containers({ host })
    dk.value = { loading: false, error: '', list }
    if (!list.some((c) => c.id === sel.value)) {
      sel.value = (list.find((c) => c.state === 'running') || list[0])?.id || ''
    }
  } catch (e) {
    if (!quiet || !dk.value.list) dk.value = { loading: false, error: String(e), list: null }
  }
}
async function loadInfo(quiet = false) {
  const id = sel.value
  if (!id) return (info.value = null)
  if (!quiet) infoErr.value = ''
  try {
    const detail = await api.docker_detail({ host, id })
    if (id !== sel.value) return
    info.value = { detail, stats: detail.running ? info.value?.stats || null : null }
    if (detail.running) {
      const stats = await api.docker_stats({ host, id }).catch(() => null)
      if (id === sel.value && info.value) info.value = { ...info.value, stats }
    }
  } catch (e) {
    if (id === sel.value && !quiet) infoErr.value = String(e)
  }
}
const diskErr = ref('')
const diskLoading = ref(false)
async function loadDisk() {
  diskLoading.value = true
  try {
    disk.value = await api.docker_disk({ host })
    diskErr.value = ''
  } catch (e) {
    diskErr.value = String(e)
  } finally {
    diskLoading.value = false
  }
}
watch(sel, () => {
  info.value = null
  loadInfo()
})

const counts = computed(() => {
  const l = dk.value.list || []
  const running = l.filter((c) => c.state === 'running').length
  return { all: l.length, running, stopped: l.length - running }
})
const groups = computed(() => {
  const out = new Map()
  for (const c of dk.value.list || []) {
    if (filter.value === 'running' && c.state !== 'running') continue
    if (filter.value === 'stopped' && c.state === 'running') continue
    if (!match(c.name, c.image, c.id.slice(0, 12), c.project, c.service, ...c.ports)) continue
    const g = c.project || ''
    if (!out.has(g)) out.set(g, [])
    out.get(g).push(c)
  }
  return [...out.entries()].map(([name, items]) => ({ name, items }))
})
const current = computed(() => (dk.value.list || []).find((c) => c.id === sel.value) || null)
const d = computed(() => info.value?.detail || null)
const stats = computed(() => info.value?.stats || null)

function dockerShell(c) {
  openContainerShell(host, { engine: 'docker', id: c.id }, c.name)
}
function dockerFollow(c) {
  openContainerShell(host, { engine: 'docker-logs', id: c.id }, `logs · ${c.name}`)
}
async function dockerAction(c, action) {
  if (action !== 'start') {
    const ok = await confirm(
      `${action === 'stop' ? 'Stop' : 'Restart'} ${c.name}?`,
      `The container ${action === 'stop' ? 'stops' : 'restarts'} on ${where}; open shells in it are closed.`,
      { okText: action === 'stop' ? 'Stop' : 'Restart' },
    )
    if (!ok) return
  }
  busy.value = action
  try {
    await api.docker_action({ host, id: c.id, action })
    await Promise.all([loadDocker(true), loadInfo(true)])
  } catch (e) {
    err(e)
  } finally {
    busy.value = ''
  }
}
const dockerLogs = (tail) => api.docker_logs({ host, id: sel.value, tail })

const DISK_NAMES = { 'Local Volumes': 'Volumes', 'Build Cache': 'Build cache' }
const reclaim = (r) => {
  const v = (r || '').replace(/\s*\(.*\)$/, '')
  return /^0(\.0+)?B?$/.test(v) ? '' : v
}

// storage: volumes and images by size
const storage = ref(null)
const storageErr = ref('')
const showStorage = ref(false)
async function loadStorage() {
  storageErr.value = ''
  try {
    storage.value = await api.docker_storage({ host })
  } catch (e) {
    storageErr.value = String(e)
  }
}
function openStorage() {
  showStorage.value = true
  loadStorage()
}
function pick(c) {
  sel.value = c.id
  showStorage.value = false
}
watch(section, (s) => s === 'mounts' && engine.value === 'docker' && !storage.value && loadStorage())
/** containers that mount a volume */
const usersOf = (volume) => (dk.value.list || []).filter((c) => c.volumes.includes(volume))
const volumeSize = (name) => storage.value?.volumes.find((v) => v.name === name)?.size || ''
const volumes = computed(() =>
  (storage.value?.volumes || []).filter((v) => match(v.name, ...usersOf(v.name).map((c) => c.name))),
)
const images = computed(() => (storage.value?.images || []).filter((i) => match(i.name)))
/** bytes in Docker's units (decimal: 1 GB = 1000 MB), like the sizes it prints */
function si(n) {
  if (!n) return '0B'
  const u = ['B', 'kB', 'MB', 'GB', 'TB']
  const i = Math.min(u.length - 1, Math.floor(Math.log10(n) / 3))
  const v = n / 1000 ** i
  return `${v >= 100 || i === 0 ? Math.round(v) : v.toFixed(1)}${u[i]}`
}
const unused = (list, key) => {
  const bytes = list.filter((x) => !x[key]).reduce((n, x) => n + x.bytes, 0)
  return bytes ? si(bytes) : ''
}

// ---------- image cleanup ----------
// Dangling images go through `docker image prune`. Unused tagged images are removed one by one,
// exactly the ones ticked, without --force: an image a container still uses is refused.
const imgRef = (i) => (i.dangling ? i.id : i.name)
const danglingImages = computed(() => (storage.value?.images || []).filter((i) => i.dangling && !i.containers))
const unusedImages = computed(() => (storage.value?.images || []).filter((i) => !i.dangling && !i.containers))
const cleaning = ref(false)
const cleanup = ref(null) // { picked: refs[], result }
const picked = computed(() => unusedImages.value.filter((i) => cleanup.value?.picked.includes(imgRef(i))))
function reloadStorage() {
  loadStorage()
  loadDisk()
}
async function pruneDangling() {
  const n = danglingImages.value.length
  const ok = await confirm(
    `Remove ${n} dangling image${n === 1 ? '' : 's'}?`,
    'Dangling images have no tag left (usually older builds of an image you rebuilt). No container uses them.',
    { okText: 'Remove' },
  )
  if (!ok) return
  cleaning.value = true
  try {
    const r = await api.docker_prune_dangling({ host })
    toast(`Removed ${r.deleted} dangling image${r.deleted === 1 ? '' : 's'}, freed ${r.reclaimed || '0B'}`)
  } catch (e) {
    err(e)
  } finally {
    cleaning.value = false
    reloadStorage()
  }
}
async function removeImages(refs) {
  cleaning.value = true
  try {
    return await api.docker_remove_images({ host, images: refs })
  } finally {
    cleaning.value = false
    reloadStorage()
  }
}
async function removeImage(i) {
  const ok = await confirm(
    `Remove ${i.dangling ? 'image ' + short(i.id) : i.name}?`,
    'No container uses it. It has to be pulled or built again before it can be used.',
    { okText: 'Remove' },
  )
  if (!ok) return
  try {
    const r = await removeImages([imgRef(i)])
    if (r.failed.length) err(`Couldn't remove ${r.failed[0][0]}: ${r.failed[0][1]}`)
    else toast(`Removed ${i.dangling ? short(i.id) : i.name}, freed ${si(r.freed_bytes)}`)
  } catch (e) {
    err(e)
  }
}
const openCleanup = () => (cleanup.value = { picked: unusedImages.value.map(imgRef), result: null })
function toggleAll(on) {
  cleanup.value.picked = on ? unusedImages.value.map(imgRef) : []
}
async function runCleanup() {
  try {
    cleanup.value.result = await removeImages(picked.value.map(imgRef))
  } catch (e) {
    err(e)
  }
}

// ================= Kubernetes =================
const kube = ref({ loading: false, error: '', contexts: null, namespaces: [], nsError: '', pods: null })
const context = ref('')
const namespace = ref('default')
const pod = ref('') // pod name
const podInfo = ref(null)
const podErr = ref('')
const logContainer = ref('')
let kseq = 0

async function loadContexts() {
  kube.value = { ...kube.value, loading: true, error: '' }
  try {
    const { contexts, current } = await api.kube_contexts({ host })
    kube.value.contexts = contexts
    if (!contexts.length) {
      kube.value.loading = false
      kube.value.error = `No kubectl contexts are set up on ${where}.`
      return
    }
    context.value = contexts.includes(current) ? current : contexts[0]
    await loadNamespaces()
  } catch (e) {
    kube.value = { ...kube.value, loading: false, error: String(e), contexts: null }
  }
}
async function loadNamespaces() {
  const n = ++kseq
  kube.value = { ...kube.value, loading: true, error: '', nsError: '', pods: null }
  pod.value = ''
  try {
    const list = await api.kube_namespaces({ host, context: context.value })
    if (n !== kseq) return
    kube.value.namespaces = list
    if (list.length && !list.includes(namespace.value)) namespace.value = list.includes('default') ? 'default' : list[0]
  } catch (e) {
    if (n !== kseq) return
    // listing namespaces may be forbidden while the pods of one are not: the name can be typed
    kube.value.namespaces = []
    kube.value.nsError = String(e)
  }
  await loadPods()
}
async function loadPods(quiet = false) {
  const n = ++kseq
  const ns = namespace.value.trim()
  if (!ns) return (kube.value = { ...kube.value, loading: false, pods: null })
  if (!quiet) kube.value = { ...kube.value, loading: true, error: '' }
  try {
    const pods = await api.kube_pods({ host, context: context.value, namespace: ns })
    if (n !== kseq) return
    kube.value = { ...kube.value, loading: false, error: '', pods }
    if (!pods.some((p) => p.name === pod.value)) pod.value = pods[0]?.name || ''
  } catch (e) {
    if (n === kseq && (!quiet || !kube.value.pods))
      kube.value = { ...kube.value, loading: false, error: String(e), pods: null }
  }
}
async function loadPod(quiet = false) {
  const name = pod.value
  if (!name) return (podInfo.value = null)
  if (!quiet) podErr.value = ''
  try {
    const p = await api.kube_pod({ host, context: context.value, namespace: namespace.value.trim(), pod: name })
    if (name !== pod.value) return
    podInfo.value = p
    const main = p.containers_detail.filter((c) => !c.init)
    if (!p.containers_detail.some((c) => c.name === logContainer.value)) logContainer.value = main[0]?.name || ''
  } catch (e) {
    if (name === pod.value && !quiet) podErr.value = String(e)
  }
}
watch(pod, () => {
  podInfo.value = null
  loadPod()
})
const pods = computed(() => (kube.value.pods || []).filter((p) => match(p.name, p.status, p.node, ...p.containers)))
const kubeTarget = (container) => ({
  context: context.value,
  namespace: namespace.value.trim(),
  pod: pod.value,
  container,
})
function kubeShell(container) {
  const p = podInfo.value
  const multi = p && p.containers_detail.filter((c) => !c.init).length > 1
  openContainerShell(
    host,
    { engine: 'kube', ...kubeTarget(container) },
    multi ? `${pod.value}/${container}` : pod.value,
  )
}
function kubeFollow(container) {
  openContainerShell(host, { engine: 'kube-logs', ...kubeTarget(container) }, `logs · ${pod.value}/${container}`)
}
const podLine = (p) =>
  [`${p.ready} ready`, p.started && `${ago(p.started)} old`, p.restarts && `${p.restarts} restarts`]
    .filter(Boolean)
    .join(' · ')
function showLogs(container) {
  logContainer.value = container
  section.value = 'logs'
}
const logC = computed(() => podInfo.value?.containers_detail.find((c) => c.name === logContainer.value))
const kubeLogs = (tail, previous) => api.kube_logs({ host, ...kubeTarget(logContainer.value), tail, previous })
const mountsOf = (volume) =>
  (podInfo.value?.containers_detail || []).flatMap((c) =>
    c.mounts.filter((m) => m.volume === volume).map((m) => ({ container: c.name, ...m })),
  )

// ================= shared =================
function refresh(quiet = false) {
  if (engine.value === 'docker') {
    loadDocker(quiet)
    loadInfo(quiet)
    // the disk summary is not part of the regular refresh, unless it couldn't be read yet
    if (!quiet || (!disk.value.length && !diskLoading.value)) loadDisk()
  } else if (kube.value.contexts) {
    loadPods(quiet)
    loadPod(quiet)
  } else if (!quiet) loadContexts()
}
watch(
  engine,
  (e) => {
    section.value = 'overview'
    if (e === 'docker' && !dk.value.list && !dk.value.loading) {
      // one after the other: on an SSH host the first command opens the connection the rest share
      loadDocker().then(loadDisk)
    }
    if (e === 'kube' && !kube.value.contexts && !kube.value.loading) loadContexts()
  },
  { immediate: true },
)
// keep the state and usage current while the tab is on screen (logs reload on demand)
const timer = setInterval(() => props.visible && !busy.value && refresh(true), 8000)
onBeforeUnmount(() => clearInterval(timer))
const loading = computed(() => (engine.value === 'docker' ? dk.value.loading : kube.value.loading))
</script>

<template>
  <div class="ct">
    <header class="top">
      <div class="seg">
        <button :class="{ on: engine === 'docker' }" @click="engine = 'docker'">Docker</button>
        <button :class="{ on: engine === 'kube' }" @click="engine = 'kube'">Kubernetes</button>
      </div>
      <span class="where dim"><Icon :name="host ? 'server' : 'terminal'" :size="13" /> {{ where }}</span>
      <template v-if="engine === 'kube' && kube.contexts">
        <select class="select sm" v-model="context" @change="loadNamespaces" title="kubectl context">
          <option v-for="c in kube.contexts" :key="c" :value="c">{{ c }}</option>
        </select>
        <input
          class="input sm mono ns"
          v-model="namespace"
          list="kshell-ns"
          spellcheck="false"
          title="Namespace"
          placeholder="namespace"
          @change="loadPods()"
          @keydown.enter="loadPods()"
        />
        <datalist id="kshell-ns">
          <option v-for="n in kube.namespaces" :key="n" :value="n" />
        </datalist>
      </template>
      <div class="grow"></div>
      <div class="find">
        <Icon name="search" :size="13" />
        <input
          v-model="q"
          :placeholder="engine === 'docker' ? 'Search containers…' : 'Search pods…'"
          spellcheck="false"
        />
      </div>
      <button class="iconbtn" :disabled="loading" @click="refresh()" title="Refresh">
        <Icon name="refresh" :size="14" :class="{ spin: loading }" />
      </button>
    </header>

    <!-- ===================== Docker ===================== -->
    <div v-if="engine === 'docker'" class="body">
      <aside class="list">
        <div class="filters">
          <button v-for="f in ['all', 'running', 'stopped']" :key="f" :class="{ on: filter === f }" @click="filter = f">
            {{ f[0].toUpperCase() + f.slice(1) }} <span>{{ counts[f] }}</span>
          </button>
        </div>
        <div class="scroll">
          <pre v-if="dk.error" class="error">{{ dk.error }}</pre>
          <div v-else-if="!dk.list" class="empty dim">Loading containers…</div>
          <div v-else-if="!dk.list.length" class="empty dim">No containers on {{ where }}.</div>
          <div v-else-if="!groups.length" class="empty dim">No match.</div>
          <template v-for="g in groups" :key="g.name">
            <div class="ghead">
              <Icon :name="g.name ? 'grid' : 'cube'" :size="11" /> {{ g.name || 'Standalone' }}
              <span>{{ g.items.length }}</span>
            </div>
            <button
              v-for="c in g.items"
              :key="c.id"
              class="item"
              :class="{ on: c.id === sel }"
              :title="c.name"
              @click="pick(c)"
              @dblclick="c.state === 'running' && dockerShell(c)"
            >
              <span class="dot" :class="tone(c.state)"></span>
              <span class="main">
                <span class="name">{{ c.service && g.name ? c.service : c.name }}</span>
                <span class="sub mono">{{ c.image }}</span>
              </span>
              <span class="side">
                <span class="st" :class="tone(c.state)">{{ c.status }}</span>
                <span v-if="c.ports.length" class="sub mono"
                  >{{ c.ports[0] }}{{ c.ports.length > 1 ? ` +${c.ports.length - 1}` : '' }}</span
                >
              </span>
            </button>
          </template>
        </div>
      </aside>

      <section v-if="showStorage" class="detail">
        <div class="dhead">
          <Icon name="cube" :size="18" />
          <div class="title">
            <h2>Docker storage</h2>
            <div class="sub dim">Disk used by images, containers and volumes on {{ where }}</div>
          </div>
          <div class="acts">
            <button class="iconbtn" @click="loadStorage" title="Reload"><Icon name="refresh" :size="14" /></button>
            <button class="btn sm" @click="showStorage = false" :disabled="!current">Back to container</button>
          </div>
        </div>
        <div class="scroll pad">
          <div class="tiles">
            <div v-for="x in disk" :key="x.kind" class="tile">
              <span>{{ DISK_NAMES[x.kind] || x.kind }}</span
              ><b>{{ x.size }}</b>
              <small
                >{{ x.active }} of {{ x.total }} in use<template v-if="reclaim(x.reclaimable)">
                  · {{ reclaim(x.reclaimable) }} reclaimable</template
                ></small
              >
            </div>
          </div>
          <pre v-if="storageErr" class="error">{{ storageErr }}</pre>
          <div v-else-if="!storage" class="empty dim">Measuring volumes and images…</div>
          <template v-else>
            <div class="stitle">
              Volumes <span>{{ storage.volumes.length }}</span>
              <span v-if="unused(storage.volumes, 'links')" class="tag warn"
                >{{ unused(storage.volumes, 'links') }} in unused volumes</span
              >
            </div>
            <div v-if="!volumes.length" class="empty dim">No volumes.</div>
            <table v-else class="tbl">
              <thead>
                <tr>
                  <th>Volume</th>
                  <th class="num">Size</th>
                  <th>Used by</th>
                  <th>Driver</th>
                </tr>
              </thead>
              <tbody>
                <tr v-for="v in volumes" :key="v.name">
                  <td class="mono">
                    <span v-if="v.anonymous" class="tag off">anonymous</span>
                    <span :title="v.name">{{ v.anonymous ? short(v.name) : v.name }}</span>
                  </td>
                  <td class="num mono">{{ v.size }}</td>
                  <td>
                    <span v-if="!v.links" class="tag warn">unused</span>
                    <span v-else class="chips">
                      <button v-for="c in usersOf(v.name)" :key="c.id" class="chip link-chip" @click="pick(c)">
                        {{ c.name }}
                      </button>
                      <span v-if="!usersOf(v.name).length" class="dim"
                        >{{ v.links }} container{{ v.links === 1 ? '' : 's' }}</span
                      >
                    </span>
                  </td>
                  <td class="dim">{{ v.driver }}</td>
                </tr>
              </tbody>
            </table>
            <div class="shead">
              <div class="stitle">
                Images <span>{{ storage.images.length }}</span>
                <span v-if="unused(storage.images, 'containers')" class="tag warn"
                  >{{ unused(storage.images, 'containers') }} in images no container uses</span
                >
              </div>
              <button
                class="btn sm"
                :disabled="cleaning || !danglingImages.length"
                @click="pruneDangling"
                :title="danglingImages.length ? 'docker image prune' : 'No dangling images'"
              >
                Remove dangling <span class="dim">{{ danglingImages.length }}</span>
              </button>
              <button
                class="btn sm"
                :disabled="cleaning || !unusedImages.length"
                @click="openCleanup"
                title="Choose unused images to remove"
              >
                <Icon name="trash" :size="12" /> Clean up unused… <span class="dim">{{ unusedImages.length }}</span>
              </button>
            </div>
            <table class="tbl">
              <thead>
                <tr>
                  <th>Image</th>
                  <th class="num">Size</th>
                  <th>Containers</th>
                  <th>Created</th>
                  <th></th>
                </tr>
              </thead>
              <tbody>
                <tr v-for="i in images" :key="i.id + i.name">
                  <td class="mono clip" :title="i.name">
                    <span v-if="i.dangling" class="tag off">dangling</span>
                    {{ i.dangling ? short(i.id) : i.name }}
                  </td>
                  <td class="num mono">{{ i.size }}</td>
                  <td>
                    <span v-if="!i.containers" class="tag warn">unused</span
                    ><template v-else>{{ i.containers }}</template>
                  </td>
                  <td class="dim">{{ i.created }}</td>
                  <td class="num">
                    <button
                      v-if="!i.containers"
                      class="iconbtn rm"
                      :disabled="cleaning"
                      @click="removeImage(i)"
                      title="Remove this image"
                    >
                      <Icon name="trash" :size="13" />
                    </button>
                  </td>
                </tr>
              </tbody>
            </table>
          </template>
        </div>
      </section>

      <section v-else class="detail">
        <div v-if="!current" class="empty dim">{{ dk.list?.length ? 'Select a container.' : '' }}</div>
        <template v-else>
          <div class="dhead">
            <span class="dot big" :class="tone(d?.health === 'unhealthy' ? 'unhealthy' : current.state)"></span>
            <div class="title">
              <h2>{{ current.name }}</h2>
              <div class="sub">
                <span :class="['st', tone(current.state)]">{{
                  d?.health ? current.status.replace(/ \((healthy|unhealthy|health: starting)\)/, '') : current.status
                }}</span>
                <span v-if="d?.health" :class="['tag', tone(d.health)]">{{ d.health }}</span>
                <span v-if="d?.oom_killed" class="tag bad">OOM killed</span>
                <span v-if="d?.restart_count" class="tag warn">{{ d.restart_count }} restarts</span>
              </div>
            </div>
            <div class="acts">
              <button
                class="btn sm primary"
                :disabled="current.state !== 'running'"
                @click="dockerShell(current)"
                title="Open a shell in this container"
              >
                <Icon name="terminal" :size="13" /> Shell
              </button>
              <button class="btn sm" @click="dockerFollow(current)" title="Follow the logs live in a terminal tab">
                <Icon name="list" :size="13" /> Follow logs
              </button>
              <button
                v-if="current.state === 'running'"
                class="btn sm"
                :disabled="!!busy"
                @click="dockerAction(current, 'restart')"
              >
                <Icon name="refresh" :size="13" :class="{ spin: busy === 'restart' }" /> Restart
              </button>
              <button
                v-if="current.state === 'running'"
                class="btn sm"
                :disabled="!!busy"
                @click="dockerAction(current, 'stop')"
              >
                <Icon name="stop" :size="13" /> Stop
              </button>
              <button v-else class="btn sm" :disabled="!!busy" @click="dockerAction(current, 'start')">
                <Icon name="play" :size="13" /> Start
              </button>
            </div>
          </div>

          <nav class="sections">
            <button :class="{ on: section === 'overview' }" @click="section = 'overview'">Overview</button>
            <button :class="{ on: section === 'logs' }" @click="section = 'logs'">Logs</button>
            <button :class="{ on: section === 'mounts' }" @click="section = 'mounts'">
              Volumes &amp; mounts <span v-if="d">{{ d.mounts.length }}</span>
            </button>
          </nav>

          <pre v-if="infoErr && section !== 'logs'" class="error">{{ infoErr }}</pre>

          <div v-else-if="section === 'overview'" class="scroll pad">
            <div v-if="!d" class="empty dim">Loading…</div>
            <template v-else>
              <div class="tiles">
                <template v-if="stats">
                  <div class="tile">
                    <span>CPU</span><b>{{ stats.cpu }}</b>
                    <small>{{ d.cpu_limit ? `limit ${cpus(d.cpu_limit)}` : '100% = one core' }}</small>
                  </div>
                  <div class="tile">
                    <span>Memory</span><b>{{ io(stats.mem)[0] }}</b>
                    <small>{{ stats.mem_perc }} of {{ io(stats.mem)[1] }}</small>
                  </div>
                  <div class="tile">
                    <span>Network</span><b>↓ {{ io(stats.net)[0] }}</b
                    ><small>↑ {{ io(stats.net)[1] }} sent</small>
                  </div>
                  <div class="tile">
                    <span>Disk I/O</span><b>{{ io(stats.block)[0] }} read</b
                    ><small>{{ io(stats.block)[1] }} written</small>
                  </div>
                  <div class="tile">
                    <span>Processes</span><b>{{ stats.pids }}</b
                    ><small>in the container</small>
                  </div>
                </template>
                <div v-else-if="!d.running" class="tile wide">
                  <span>Stopped</span>
                  <b>Exit code {{ d.exit_code }}</b>
                  <small v-if="d.finished_at">{{ ago(d.finished_at) }} ago · {{ when(d.finished_at) }}</small>
                </div>
                <div v-else class="tile wide"><span>Usage</span><b class="dim">Reading CPU and memory…</b></div>
                <div class="tile" :title="`Image layers: ${fmtSize(d.size_root - d.size_rw)}`">
                  <span>Disk</span><b>{{ fmtSize(d.size_rw) }}</b
                  ><small>written · {{ fmtSize(d.size_root) }} with image</small>
                </div>
              </div>
              <pre v-if="d.error" class="error">{{ d.error }}</pre>

              <dl class="kv">
                <dt>Image</dt>
                <dd class="mono">{{ d.image }}</dd>
                <dt>ID</dt>
                <dd class="mono">
                  {{ short(d.id) }} <button class="link" @click="copy(d.id, 'Container ID')">copy</button>
                </dd>
                <template v-if="d.project">
                  <dt>Compose</dt>
                  <dd>{{ d.project }} <span class="dim">/</span> {{ d.service }}</dd>
                </template>
                <dt>Command</dt>
                <dd class="mono clip" :title="d.command">{{ d.command || '—' }}</dd>
                <dt>Created</dt>
                <dd>
                  {{ ago(d.created) }} ago <span class="dim">· {{ when(d.created) }}</span>
                </dd>
                <template v-if="d.running && d.started_at">
                  <dt>Started</dt>
                  <dd>
                    {{ ago(d.started_at) }} ago <span class="dim">· {{ when(d.started_at) }}</span>
                  </dd>
                </template>
                <dt>Restart policy</dt>
                <dd>{{ d.restart_policy || 'no' }}</dd>
                <dt>Limits</dt>
                <dd>
                  {{ d.memory_limit ? fmtSize(d.memory_limit) + ' memory' : 'no memory limit' }} ·
                  {{ d.cpu_limit ? cpus(d.cpu_limit) : 'no CPU limit' }}
                </dd>
                <template v-if="d.workdir || d.user">
                  <dt>Runs as</dt>
                  <dd class="mono">
                    {{ d.user || 'root (image default)' }} <span class="dim">in</span> {{ d.workdir || '/' }}
                  </dd>
                </template>
                <dt>Ports</dt>
                <dd class="chips">
                  <span v-if="!d.ports.length" class="dim">none exposed</span>
                  <span v-for="p in d.ports" :key="p.container + p.host" class="chip mono" :class="{ off: !p.host }">
                    <template v-if="p.host">{{ p.host }} → {{ p.container }}</template>
                    <template v-else>{{ p.container }} <span class="dim">not published</span></template>
                  </span>
                </dd>
                <dt>Networks</dt>
                <dd class="chips">
                  <span v-if="!d.networks.length" class="dim">none</span>
                  <span v-for="n in d.networks" :key="n.name" class="chip mono">
                    {{ n.name }}<span v-if="n.ip" class="dim ip">{{ n.ip }}</span>
                  </span>
                </dd>
              </dl>
            </template>
          </div>

          <LogView
            v-else-if="section === 'logs'"
            class="pad grow"
            :source="sel"
            :load="dockerLogs"
            :follow="() => dockerFollow(current)"
          />

          <div v-else class="scroll pad">
            <div v-if="!d" class="empty dim">Loading…</div>
            <div v-else-if="!d.mounts.length" class="empty dim">This container has no volumes or bind mounts.</div>
            <table v-else class="tbl">
              <thead>
                <tr>
                  <th>Type</th>
                  <th>Source</th>
                  <th>Mounted at</th>
                  <th class="num">Size</th>
                  <th>Access</th>
                </tr>
              </thead>
              <tbody>
                <tr v-for="m in d.mounts" :key="m.destination">
                  <td>
                    <span class="tag" :class="m.kind === 'volume' ? 'info' : 'off'">{{ m.kind }}</span>
                  </td>
                  <td class="mono">
                    <div v-if="m.name" class="strong">{{ m.name.length > 40 ? short(m.name) : m.name }}</div>
                    <div class="dim clip" :title="m.source">{{ m.source }}</div>
                  </td>
                  <td class="mono">{{ m.destination }}</td>
                  <td class="num mono">{{ m.kind === 'volume' ? volumeSize(m.name) || '…' : '' }}</td>
                  <td>{{ m.rw ? 'read-write' : 'read-only' }}</td>
                </tr>
              </tbody>
            </table>
          </div>
        </template>
      </section>
    </div>

    <footer v-if="engine === 'docker' && dk.list" class="disk">
      <button class="dlabel" :class="{ on: showStorage }" @click="openStorage" title="Show volumes and images by size">
        <Icon name="cube" :size="12" /> Storage
      </button>
      <button
        v-for="x in disk"
        :key="x.kind"
        class="du"
        @click="openStorage"
        :title="`${x.active} of ${x.total} in use`"
      >
        <b>{{ DISK_NAMES[x.kind] || x.kind }}</b> {{ x.size }}
        <span v-if="reclaim(x.reclaimable)" class="free">{{ reclaim(x.reclaimable) }} reclaimable</span>
      </button>
      <template v-if="!disk.length">
        <span v-if="diskErr" class="derr" :title="diskErr">Couldn't read disk use: {{ diskErr.split('\n')[0] }}</span>
        <span v-else class="dim">Measuring disk use…</span>
        <button v-if="diskErr" class="btn sm" :disabled="diskLoading" @click="loadDisk">Retry</button>
      </template>
    </footer>

    <!-- ===================== Kubernetes ===================== -->
    <div v-if="engine === 'kube'" class="body">
      <aside class="list">
        <p v-if="kube.nsError" class="note dim">Couldn't list namespaces; type one above. {{ kube.nsError }}</p>
        <div class="scroll">
          <pre v-if="kube.error" class="error">{{ kube.error }}</pre>
          <div v-else-if="!kube.pods" class="empty dim">{{ kube.loading ? 'Loading…' : '' }}</div>
          <div v-else-if="!kube.pods.length" class="empty dim">No pods in {{ namespace }}.</div>
          <div v-else-if="!pods.length" class="empty dim">No match.</div>
          <button v-for="p in pods" :key="p.name" class="item" :class="{ on: p.name === pod }" @click="pod = p.name">
            <span class="dot" :class="tone(p.status)"></span>
            <span class="main">
              <span class="name">{{ p.name }}</span>
              <span class="sub">{{ podLine(p) }}</span>
            </span>
            <span class="side"
              ><span class="st" :class="tone(p.status)">{{ p.status }}</span></span
            >
          </button>
        </div>
      </aside>

      <section class="detail">
        <div v-if="!pod" class="empty dim">{{ kube.pods?.length ? 'Select a pod.' : '' }}</div>
        <template v-else>
          <div class="dhead">
            <span class="dot big" :class="tone(podInfo?.status)"></span>
            <div class="title">
              <h2>{{ pod }}</h2>
              <div class="sub" v-if="podInfo">
                <span :class="['st', tone(podInfo.status)]">{{ podInfo.status }}</span>
                <span class="dim">{{ podInfo.ready }} ready · up {{ ago(podInfo.started) }}</span>
                <span v-if="podInfo.restarts" class="tag warn">{{ podInfo.restarts }} restarts</span>
              </div>
            </div>
            <div class="acts" v-if="podInfo">
              <button
                class="btn sm primary"
                @click="kubeShell(logContainer)"
                :title="`Open a shell in ${logContainer}`"
              >
                <Icon name="terminal" :size="13" /> Shell
              </button>
              <button class="btn sm" @click="kubeFollow(logContainer)" title="Follow the logs live in a terminal tab">
                <Icon name="list" :size="13" /> Follow logs
              </button>
            </div>
          </div>

          <nav class="sections">
            <button :class="{ on: section === 'overview' }" @click="section = 'overview'">Overview</button>
            <button :class="{ on: section === 'logs' }" @click="section = 'logs'">Logs</button>
            <button :class="{ on: section === 'mounts' }" @click="section = 'mounts'">
              Volumes <span v-if="podInfo">{{ podInfo.volumes.length }}</span>
            </button>
            <label v-if="podInfo && podInfo.containers_detail.filter((c) => !c.init).length > 1" class="cpick">
              <span class="dim">Container</span>
              <select class="select sm" v-model="logContainer">
                <option v-for="c in podInfo.containers_detail.filter((c) => !c.init)" :key="c.name" :value="c.name">
                  {{ c.name }}
                </option>
              </select>
            </label>
          </nav>

          <pre v-if="podErr && section !== 'logs'" class="error">{{ podErr }}</pre>

          <div v-else-if="section === 'overview'" class="scroll pad">
            <div v-if="!podInfo" class="empty dim">Loading…</div>
            <template v-else>
              <dl class="kv">
                <dt>Namespace</dt>
                <dd>{{ podInfo.namespace }}</dd>
                <dt>Node</dt>
                <dd class="mono">
                  {{ podInfo.node || '—' }} <span v-if="podInfo.host_ip" class="dim">{{ podInfo.host_ip }}</span>
                </dd>
                <dt>Pod IP</dt>
                <dd class="mono">{{ podInfo.ip || '—' }}</dd>
                <template v-if="podInfo.owner">
                  <dt>Owner</dt>
                  <dd class="mono">{{ podInfo.owner }}</dd>
                </template>
                <dt>Created</dt>
                <dd>
                  {{ ago(podInfo.created) }} ago <span class="dim">· {{ when(podInfo.created) }}</span>
                </dd>
                <dt>QoS · account</dt>
                <dd>{{ podInfo.qos || '—' }} <span class="dim">·</span> {{ podInfo.service_account || 'default' }}</dd>
                <template v-if="podInfo.labels.length">
                  <dt>Labels</dt>
                  <dd class="chips">
                    <span v-for="[k, v] in podInfo.labels" :key="k" class="chip mono"
                      >{{ k }}=<b>{{ v }}</b></span
                    >
                  </dd>
                </template>
              </dl>
              <div class="stitle">Containers</div>
              <div v-for="c in podInfo.containers_detail" :key="c.name" class="crow">
                <span
                  class="dot"
                  :class="tone(c.state === 'Running' ? (c.ready ? 'running' : 'waiting') : c.reason || c.state)"
                ></span>
                <div class="main">
                  <div class="name">
                    {{ c.name }} <span v-if="c.init" class="tag off">init</span>
                    <span :class="['st', tone(c.state === 'Running' ? 'running' : c.reason || c.state)]">
                      {{ c.state }}<template v-if="c.reason"> · {{ c.reason }}</template>
                    </span>
                    <span v-if="c.state === 'Running' && !c.ready" class="tag warn">not ready</span>
                  </div>
                  <div class="sub mono">{{ c.image }}</div>
                  <div v-if="c.message" class="msg">{{ c.message }}</div>
                  <div class="sub">
                    <template v-if="c.since"
                      >{{ c.state === 'Running' ? 'up' : 'ended' }} {{ ago(c.since) }} ·
                    </template>
                    {{ c.restarts }} restarts<template v-if="c.last">
                      · last exit: <b :class="tone(c.last)">{{ c.last }}</b></template
                    >
                  </div>
                </div>
                <div v-if="!c.init" class="acts">
                  <button class="iconbtn" :disabled="c.state !== 'Running'" @click="kubeShell(c.name)" title="Shell">
                    <Icon name="terminal" :size="13" />
                  </button>
                  <button class="iconbtn" @click="showLogs(c.name)" title="Logs">
                    <Icon name="list" :size="13" />
                  </button>
                </div>
              </div>
            </template>
          </div>

          <LogView
            v-else-if="section === 'logs'"
            class="pad grow"
            :source="`${pod}/${logContainer}`"
            :load="kubeLogs"
            :can-previous="!!logC?.restarts"
            :follow="() => kubeFollow(logContainer)"
          />

          <div v-else class="scroll pad">
            <div v-if="!podInfo" class="empty dim">Loading…</div>
            <div v-else-if="!podInfo.volumes.length" class="empty dim">This pod has no volumes.</div>
            <table v-else class="tbl">
              <thead>
                <tr>
                  <th>Volume</th>
                  <th>Type</th>
                  <th>Source</th>
                  <th>Mounted at</th>
                </tr>
              </thead>
              <tbody>
                <tr v-for="v in podInfo.volumes" :key="v.name">
                  <td class="mono strong">{{ v.name }}</td>
                  <td>
                    <span class="tag" :class="v.kind === 'PVC' ? 'info' : 'off'">{{ v.kind }}</span>
                  </td>
                  <td class="mono clip" :title="v.source">{{ v.source || '—' }}</td>
                  <td class="mono">
                    <div v-for="m in mountsOf(v.name)" :key="m.container + m.path">
                      {{ m.path }} <span class="dim">{{ m.container }}{{ m.read_only ? ' · ro' : '' }}</span>
                    </div>
                    <span v-if="!mountsOf(v.name).length" class="dim">not mounted</span>
                  </td>
                </tr>
              </tbody>
            </table>
          </div>
        </template>
      </section>
    </div>
    <div v-if="cleanup" class="overlay" @mousedown.self="!cleaning && (cleanup = null)">
      <div class="modal cleanup">
        <h2>Clean up unused images</h2>
        <template v-if="!cleanup.result">
          <p class="muted">
            No container on {{ where }} uses these images. A removed image has to be pulled or built again before it can
            be used. Images that a stopped container uses are kept and not listed.
          </p>
          <label class="pickall">
            <input
              type="checkbox"
              :checked="picked.length === unusedImages.length"
              :indeterminate.prop="picked.length > 0 && picked.length < unusedImages.length"
              @change="toggleAll($event.target.checked)"
            />
            Select all
          </label>
          <div class="clist">
            <label v-for="i in unusedImages" :key="imgRef(i)" class="citem">
              <input type="checkbox" :value="imgRef(i)" v-model="cleanup.picked" />
              <span class="mono cname" :title="i.name">{{ i.name }}</span>
              <span class="dim">{{ i.created }}</span>
              <span class="num mono">{{ i.size }}</span>
            </label>
          </div>
          <p class="dim small">
            {{ picked.length }} selected, {{ si(picked.reduce((n, i) => n + i.bytes, 0)) }} as listed. Images share
            layers and Docker can count sizes differently, so the space actually freed is measured afterwards.
          </p>
          <div class="modal-foot">
            <button class="btn" :disabled="cleaning" @click="cleanup = null">Cancel</button>
            <button class="btn danger" :disabled="cleaning || !picked.length" @click="runCleanup">
              {{ cleaning ? 'Removing…' : `Remove ${picked.length} image${picked.length === 1 ? '' : 's'}` }}
            </button>
          </div>
        </template>
        <template v-else>
          <p>
            Removed <b>{{ cleanup.result.removed.length }}</b> image{{
              cleanup.result.removed.length === 1 ? '' : 's'
            }}, freed <b>{{ si(cleanup.result.freed_bytes) }}</b
            >.
          </p>
          <div v-if="cleanup.result.failed.length" class="fails">
            <div class="dim small">Not removed:</div>
            <div v-for="[name, why] in cleanup.result.failed" :key="name" class="fail">
              <span class="mono">{{ name }}</span> <span class="why">{{ why }}</span>
            </div>
          </div>
          <div class="modal-foot">
            <button class="btn primary" @click="cleanup = null">Done</button>
          </div>
        </template>
      </div>
    </div>
  </div>
</template>

<style scoped>
.ct {
  height: 100%;
  display: flex;
  flex-direction: column;
  background: var(--bg);
  font-size: 13px;
}
.top {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 10px 14px;
  border-bottom: 1px solid var(--line);
}
.seg {
  display: flex;
  background: var(--bg-3);
  border-radius: 8px;
  padding: 2px;
}
.seg button {
  border: 0;
  background: none;
  color: var(--text-2);
  font: inherit;
  font-weight: 600;
  padding: 4px 12px;
  border-radius: 6px;
  cursor: pointer;
}
.seg button.on {
  background: var(--bg-2);
  color: var(--text);
  box-shadow: 0 1px 2px rgba(0, 0, 0, 0.2);
}
.where {
  display: flex;
  align-items: center;
  gap: 5px;
  white-space: nowrap;
}
.grow {
  flex: 1;
}
.select.sm,
.input.sm {
  height: 28px;
  font-size: 12px;
  width: auto;
  max-width: 220px;
}
.ns {
  width: 150px;
}
.find {
  display: flex;
  align-items: center;
  gap: 6px;
  width: 220px;
  height: 28px;
  padding: 0 8px;
  border: 1px solid var(--line);
  border-radius: 7px;
  background: var(--bg-2);
  color: var(--text-3);
}
.find input {
  flex: 1;
  min-width: 0;
  border: 0;
  outline: 0;
  background: none;
  color: var(--text);
  font: inherit;
  font-size: 12px;
}
.body {
  flex: 1;
  min-height: 0;
  display: flex;
}
.list {
  width: 340px;
  flex: none;
  display: flex;
  flex-direction: column;
  border-right: 1px solid var(--line);
  background: var(--bg-2);
}
.filters {
  display: flex;
  gap: 4px;
  padding: 8px 10px;
  border-bottom: 1px solid var(--line);
}
.filters button {
  border: 0;
  background: none;
  color: var(--text-2);
  font: inherit;
  font-size: 12px;
  padding: 3px 9px;
  border-radius: 6px;
  cursor: pointer;
}
.filters button span {
  color: var(--text-3);
  margin-left: 2px;
}
.filters button.on {
  background: var(--bg-4);
  color: var(--text);
}
.scroll {
  flex: 1;
  min-height: 0;
  overflow: auto;
}
.list .scroll {
  padding: 4px 6px 10px;
}
.ghead {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 11px;
  font-weight: 700;
  text-transform: uppercase;
  letter-spacing: 0.05em;
  color: var(--text-3);
  padding: 10px 8px 4px;
}
.ghead span {
  font-weight: 500;
}
.item {
  display: flex;
  align-items: center;
  gap: 9px;
  width: 100%;
  padding: 6px 8px;
  border: 0;
  border-radius: 7px;
  background: none;
  color: var(--text);
  text-align: left;
  cursor: pointer;
  font: inherit;
}
.item:hover {
  background: var(--bg-3);
}
.item.on {
  background: color-mix(in srgb, var(--accent) 14%, transparent);
}
.main {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 1px;
}
.side {
  display: flex;
  flex-direction: column;
  align-items: flex-end;
  gap: 1px;
  max-width: 45%;
  flex: none;
}
.name {
  font-weight: 600;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.sub {
  font-size: 11.5px;
  color: var(--text-3);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  max-width: 100%;
}
.st {
  font-size: 11.5px;
  white-space: nowrap;
  color: var(--text-2);
}
.st.ok {
  color: var(--ok);
}
.st.warn,
b.warn {
  color: var(--warn);
}
.st.bad,
b.bad {
  color: var(--danger);
}
.dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  flex: none;
  background: var(--text-3);
}
.dot.big {
  width: 11px;
  height: 11px;
}
.dot.ok {
  background: var(--ok);
  box-shadow: 0 0 0 3px color-mix(in srgb, var(--ok) 20%, transparent);
}
.dot.warn {
  background: var(--warn);
}
.dot.bad {
  background: var(--danger);
}
.detail {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
}
.dhead {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 14px 18px 10px;
}
.title {
  flex: 1;
  min-width: 0;
}
.title h2 {
  margin: 0;
  font-size: 17px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.title .sub {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-top: 3px;
  font-size: 12px;
}
.acts {
  display: flex;
  gap: 6px;
  flex: none;
}
.tag {
  font-size: 10.5px;
  font-weight: 600;
  padding: 1px 6px;
  border-radius: 5px;
  background: var(--bg-4);
  color: var(--text-2);
  white-space: nowrap;
}
.tag.ok {
  background: color-mix(in srgb, var(--ok) 18%, transparent);
  color: var(--ok);
}
.tag.warn {
  background: color-mix(in srgb, var(--warn) 18%, transparent);
  color: var(--warn);
}
.tag.bad {
  background: color-mix(in srgb, var(--danger) 18%, transparent);
  color: var(--danger);
}
.tag.info {
  background: color-mix(in srgb, var(--accent-2) 20%, transparent);
  color: var(--accent-2);
}
.sections {
  display: flex;
  align-items: center;
  gap: 2px;
  padding: 0 14px;
  border-bottom: 1px solid var(--line);
}
.sections > button {
  border: 0;
  background: none;
  color: var(--text-2);
  font: inherit;
  font-weight: 600;
  padding: 8px 10px;
  border-bottom: 2px solid transparent;
  margin-bottom: -1px;
  cursor: pointer;
}
.sections > button span {
  color: var(--text-3);
  font-weight: 500;
}
.sections > button.on {
  color: var(--text);
  border-bottom-color: var(--accent);
}
.cpick {
  margin-left: auto;
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 12px;
}
.pad {
  padding: 14px 18px;
}
.detail .grow {
  display: flex;
  flex-direction: column;
  min-height: 0;
}
.tiles {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(125px, 1fr));
  gap: 8px;
  margin-bottom: 14px;
}
.tile {
  display: flex;
  flex-direction: column;
  gap: 2px;
  padding: 9px 11px;
  border: 1px solid var(--line);
  border-radius: 9px;
  background: var(--bg-2);
  min-width: 0;
}
.tile.wide {
  grid-column: span 2;
}
.tile span {
  font-size: 11px;
  color: var(--text-3);
  text-transform: uppercase;
  letter-spacing: 0.04em;
}
.tile b {
  font-size: 14px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.tile small {
  font-size: 11px;
  color: var(--text-3);
}
.kv {
  display: grid;
  grid-template-columns: 130px 1fr;
  gap: 7px 12px;
  margin: 0;
}
.kv dt {
  color: var(--text-3);
}
.kv dd {
  margin: 0;
  min-width: 0;
}
.clip {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.chips {
  display: flex;
  flex-wrap: wrap;
  gap: 5px;
}
.chip {
  font-size: 11.5px;
  padding: 1px 7px;
  border-radius: 5px;
  background: var(--bg-3);
  border: 1px solid var(--line);
}
.chip.off {
  opacity: 0.75;
}
.link {
  border: 0;
  background: none;
  color: var(--accent-2);
  cursor: pointer;
  font: inherit;
  font-size: 11.5px;
  padding: 0 4px;
}
.stitle {
  font-size: 11px;
  font-weight: 700;
  color: var(--text-3);
  text-transform: uppercase;
  letter-spacing: 0.06em;
  margin: 18px 0 6px;
}
.crow {
  display: flex;
  align-items: flex-start;
  gap: 10px;
  padding: 9px 10px;
  border: 1px solid var(--line);
  border-radius: 9px;
  background: var(--bg-2);
  margin-bottom: 6px;
}
.crow .dot {
  margin-top: 5px;
}
.crow .name {
  display: flex;
  align-items: center;
  gap: 7px;
}
.msg {
  font-size: 12px;
  color: var(--warn);
  margin: 2px 0;
  white-space: pre-wrap;
}
.tbl {
  width: 100%;
  border-collapse: collapse;
  font-size: 12.5px;
}
.tbl th {
  text-align: left;
  font-size: 11px;
  font-weight: 600;
  color: var(--text-3);
  text-transform: uppercase;
  letter-spacing: 0.04em;
  padding: 0 8px 6px;
  border-bottom: 1px solid var(--line);
}
.tbl td {
  padding: 7px 8px;
  border-bottom: 1px solid var(--line);
  vertical-align: top;
  max-width: 320px;
}
.strong {
  font-weight: 600;
}
.disk {
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 4px 10px;
  border-top: 1px solid var(--line);
  background: var(--bg-2);
  font-size: 12px;
  overflow-x: auto;
  white-space: nowrap;
}
.dlabel {
  display: flex;
  align-items: center;
  gap: 5px;
}
.dlabel,
.du {
  display: flex;
  align-items: center;
  gap: 6px;
  border: 0;
  background: none;
  color: var(--text-2);
  font: inherit;
  padding: 3px 8px;
  border-radius: 6px;
  cursor: pointer;
  white-space: nowrap;
}
.dlabel {
  font-weight: 600;
}
.dlabel:hover,
.du:hover,
.dlabel.on {
  background: var(--bg-3);
  color: var(--text);
}
.du b {
  font-weight: 600;
  color: var(--text);
}
.free {
  color: var(--warn);
}
.derr {
  color: var(--danger);
  overflow: hidden;
  text-overflow: ellipsis;
  max-width: 60%;
}
.ip {
  margin-left: 6px;
}
.num {
  text-align: right;
  white-space: nowrap;
}
.stitle span {
  font-weight: 500;
  margin-left: 4px;
}
.stitle .tag {
  margin-left: 8px;
  text-transform: none;
  letter-spacing: 0;
}
.link-chip {
  cursor: pointer;
  color: var(--text);
  font: inherit;
  font-size: 11.5px;
}
.link-chip:hover {
  border-color: var(--accent);
}
.tbl .tag {
  margin-right: 6px;
}
.shead {
  display: flex;
  align-items: center;
  gap: 6px;
  margin: 18px 0 6px;
}
.shead .stitle {
  margin: 0;
  flex: 1;
}
.iconbtn.rm {
  width: 22px;
  height: 20px;
  margin: -2px 0;
  color: var(--text-3);
}
.iconbtn.rm:hover {
  color: var(--danger);
}
.cleanup {
  width: min(560px, calc(100vw - 40px));
}
.cleanup .muted {
  margin-top: -4px;
  font-size: 12.5px;
  line-height: 1.5;
}
.pickall {
  display: flex;
  align-items: center;
  gap: 8px;
  font-weight: 600;
  padding: 6px 2px;
  border-bottom: 1px solid var(--line);
}
.clist {
  max-height: 300px;
  overflow: auto;
  margin-bottom: 8px;
}
.citem {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 2px;
  border-bottom: 1px solid var(--line);
  font-size: 12.5px;
  cursor: pointer;
}
.cname {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.small {
  font-size: 12px;
  line-height: 1.5;
}
.fails {
  margin-top: 8px;
}
.fail {
  font-size: 12.5px;
  padding: 4px 0;
}
.why {
  color: var(--danger);
}
.note {
  font-size: 12px;
  padding: 8px 10px 0;
  margin: 0;
}
.empty {
  padding: 28px 10px;
  text-align: center;
}
.error {
  white-space: pre-wrap;
  color: var(--danger);
  background: var(--bg-3);
  border-radius: 8px;
  padding: 10px 12px;
  font-size: 12px;
  margin: 10px;
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

<script setup>
// Docker containers and Kubernetes pods of a host (or this computer); a click opens a shell in one.
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { store, openContainerShell } from '../lib/store'
import { api } from '../lib/api'
import Icon from './Icon.vue'

const host = store.containers.host
const where = host ? host.label || host.host : 'this computer'
const tab = ref(store.containers.tab || 'docker')
const q = ref('')
const search = ref(null)

const close = () => (store.containers = null)
const onKey = (e) => e.key === 'Escape' && close()
onMounted(() => {
  window.addEventListener('keydown', onKey)
  search.value?.focus()
})
onBeforeUnmount(() => window.removeEventListener('keydown', onKey))

const match = (...fields) => {
  const s = q.value.trim().toLowerCase()
  return !s || fields.join(' ').toLowerCase().includes(s)
}

// ---------- Docker ----------
const docker = ref({ loading: false, error: '', list: null })
async function loadDocker() {
  docker.value = { ...docker.value, loading: true, error: '' }
  try {
    docker.value = { loading: false, error: '', list: await api.docker_containers({ host }) }
  } catch (e) {
    docker.value = { loading: false, error: String(e), list: null }
  }
}
const containers = computed(() =>
  (docker.value.list || []).filter((c) => match(c.name, c.image, c.id.slice(0, 12), c.ports)),
)
function openDocker(c) {
  openContainerShell(host, { engine: 'docker', id: c.id }, c.name)
  close()
}

// ---------- Kubernetes ----------
const kube = ref({ loading: false, error: '', contexts: null, namespaces: [], nsError: '', pods: null })
const context = ref('')
const namespace = ref('default')
let seq = 0 // only the newest request updates the list

async function loadContexts() {
  kube.value = { ...kube.value, loading: true, error: '' }
  try {
    const { contexts, current } = await api.kube_contexts({ host })
    kube.value.contexts = contexts
    if (!contexts.length) {
      kube.value = {
        ...kube.value,
        loading: false,
        error: `No kubectl contexts are set up ${host ? 'on ' + where : 'here'}.`,
      }
      return
    }
    context.value = contexts.includes(current) ? current : contexts[0]
    await loadNamespaces()
  } catch (e) {
    kube.value = { ...kube.value, loading: false, error: String(e), contexts: null }
  }
}
async function loadNamespaces() {
  const n = ++seq
  kube.value = { ...kube.value, loading: true, error: '', nsError: '', pods: null }
  try {
    const list = await api.kube_namespaces({ host, context: context.value })
    if (n !== seq) return
    kube.value.namespaces = list
    if (list.length && !list.includes(namespace.value)) namespace.value = list.includes('default') ? 'default' : list[0]
  } catch (e) {
    if (n !== seq) return
    // listing namespaces may be forbidden while the pods of one are not: the name can still be typed
    kube.value.namespaces = []
    kube.value.nsError = String(e)
  }
  await loadPods()
}
async function loadPods() {
  const n = ++seq
  const ns = namespace.value.trim()
  if (!ns) {
    kube.value = { ...kube.value, loading: false, pods: null }
    return
  }
  kube.value = { ...kube.value, loading: true, error: '' }
  try {
    const pods = await api.kube_pods({ host, context: context.value, namespace: ns })
    if (n === seq) kube.value = { ...kube.value, loading: false, pods }
  } catch (e) {
    if (n === seq) kube.value = { ...kube.value, loading: false, error: String(e), pods: null }
  }
}
const pods = computed(() => (kube.value.pods || []).filter((p) => match(p.name, p.phase, ...p.containers)))
function openPod(p, container = p.containers[0] || '') {
  const shell = { engine: 'kube', context: context.value, namespace: namespace.value.trim(), pod: p.name, container }
  openContainerShell(host, shell, p.containers.length > 1 ? `${p.name}/${container}` : p.name)
  close()
}

function refresh() {
  if (tab.value === 'docker') return loadDocker()
  return kube.value.contexts ? loadPods() : loadContexts()
}
watch(
  tab,
  (t) => {
    if (t === 'docker' && !docker.value.list && !docker.value.loading) loadDocker()
    if (t === 'kube' && !kube.value.contexts && !kube.value.loading) loadContexts()
  },
  { immediate: true },
)
const loading = computed(() => (tab.value === 'docker' ? docker.value.loading : kube.value.loading))
const running = (status) => /^(up|running)/i.test(status)
</script>

<template>
  <div class="overlay" @mousedown.self="close">
    <div class="modal ctr">
      <div class="head">
        <h2>
          Containers <span class="dim">on {{ where }}</span>
        </h2>
        <button class="iconbtn" :disabled="loading" @click="refresh" title="Refresh">
          <Icon name="refresh" :size="14" :class="{ spin: loading }" />
        </button>
        <button class="iconbtn" @click="close" title="Close"><Icon name="x" :size="14" /></button>
      </div>

      <div class="bar">
        <div class="chips">
          <button class="chip" :class="{ on: tab === 'docker' }" @click="tab = 'docker'">Docker</button>
          <button class="chip" :class="{ on: tab === 'kube' }" @click="tab = 'kube'">Kubernetes</button>
        </div>
        <div class="search">
          <Icon name="search" :size="14" />
          <input
            ref="search"
            class="input"
            v-model="q"
            :placeholder="tab === 'docker' ? 'Search containers…' : 'Search pods…'"
            spellcheck="false"
          />
        </div>
      </div>

      <template v-if="tab === 'kube' && kube.contexts">
        <div class="kpick">
          <label>
            <span class="dim">Context</span>
            <select class="select" v-model="context" @change="loadNamespaces">
              <option v-for="c in kube.contexts" :key="c" :value="c">{{ c }}</option>
            </select>
          </label>
          <label>
            <span class="dim">Namespace</span>
            <input
              class="input mono"
              v-model="namespace"
              list="kshell-namespaces"
              spellcheck="false"
              @change="loadPods"
              @keydown.enter="loadPods"
            />
            <datalist id="kshell-namespaces">
              <option v-for="n in kube.namespaces" :key="n" :value="n" />
            </datalist>
          </label>
        </div>
        <p v-if="kube.nsError" class="note dim">Couldn't list namespaces; type one. {{ kube.nsError }}</p>
      </template>

      <div class="list">
        <template v-if="tab === 'docker'">
          <pre v-if="docker.error" class="error">{{ docker.error }}</pre>
          <div v-else-if="docker.loading && !docker.list" class="dim empty">Loading containers…</div>
          <div v-else-if="docker.list && !docker.list.length" class="dim empty">No running containers.</div>
          <div v-else-if="docker.list && !containers.length" class="dim empty">No match.</div>
          <button
            v-for="c in containers"
            :key="c.id"
            class="row"
            @click="openDocker(c)"
            :title="`Open a shell in ${c.name}`"
          >
            <span class="dot" :class="{ ok: running(c.status) }"></span>
            <span class="main">
              <span class="name">{{ c.name }}</span>
              <span class="sub mono">{{ c.image }} · {{ c.id.slice(0, 12) }}</span>
            </span>
            <span class="meta">
              <span>{{ c.status }}</span>
              <span v-if="c.ports" class="sub mono">{{ c.ports }}</span>
            </span>
            <Icon name="terminal" :size="14" class="go" />
          </button>
        </template>

        <template v-else>
          <pre v-if="kube.error" class="error">{{ kube.error }}</pre>
          <div v-else-if="kube.loading && !kube.pods" class="dim empty">Loading…</div>
          <div v-else-if="kube.pods && !kube.pods.length" class="dim empty">No pods in {{ namespace }}.</div>
          <div v-else-if="kube.pods && !pods.length" class="dim empty">No match.</div>
          <div v-for="p in pods" :key="p.name" class="pod">
            <button class="row" @click="openPod(p)" :title="`Open a shell in ${p.name}`">
              <span class="dot" :class="{ ok: running(p.phase) }"></span>
              <span class="main">
                <span class="name">{{ p.name }}</span>
                <span class="sub">{{ p.phase }} · {{ p.ready }} ready</span>
              </span>
              <Icon name="terminal" :size="14" class="go" />
            </button>
            <div v-if="p.containers.length > 1" class="conts">
              <button
                v-for="c in p.containers"
                :key="c"
                class="chip mono"
                @click="openPod(p, c)"
                :title="`Open a shell in ${c}`"
              >
                {{ c }}
              </button>
            </div>
          </div>
        </template>
      </div>
    </div>
  </div>
</template>

<style scoped>
.ctr {
  width: min(680px, calc(100vw - 40px));
  display: flex;
  flex-direction: column;
  height: min(620px, calc(100vh - 60px));
  overflow: hidden;
}
.head {
  display: flex;
  align-items: center;
  gap: 6px;
}
.head h2 {
  flex: 1;
  margin: 0;
}
.head h2 .dim {
  font-weight: 500;
}
.bar {
  display: flex;
  align-items: center;
  gap: 10px;
  margin: 14px 0 10px;
}
.chips {
  display: flex;
  gap: 6px;
}
.search {
  flex: 1;
  display: flex;
  align-items: center;
  gap: 6px;
  color: var(--text-3);
}
.search .input {
  flex: 1;
}
.kpick {
  display: flex;
  gap: 10px;
  margin-bottom: 8px;
}
.kpick label {
  flex: 1;
  display: flex;
  flex-direction: column;
  gap: 4px;
  font-size: 12px;
  min-width: 0;
}
.note {
  font-size: 12px;
  margin: 0 0 8px;
}
.list {
  flex: 1;
  overflow: auto;
  border-top: 1px solid var(--line);
  padding-top: 6px;
}
.empty {
  padding: 24px 6px;
  text-align: center;
}
.error {
  white-space: pre-wrap;
  color: var(--danger);
  background: var(--bg-3);
  border-radius: 8px;
  padding: 10px 12px;
  font-size: 12px;
  margin: 8px 0;
}
.row {
  display: flex;
  align-items: center;
  gap: 10px;
  width: 100%;
  padding: 8px 10px;
  border: 0;
  border-radius: 8px;
  background: none;
  color: var(--text);
  cursor: pointer;
  text-align: left;
}
.row:hover {
  background: var(--bg-3);
}
.dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: var(--text-3);
  flex: none;
}
.dot.ok {
  background: var(--ok);
}
.main,
.meta {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}
.main {
  flex: 1;
}
.meta {
  align-items: flex-end;
  font-size: 12px;
  color: var(--text-2);
  max-width: 45%;
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
.go {
  color: var(--text-3);
  flex: none;
}
.row:hover .go {
  color: var(--accent);
}
.conts {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  padding: 0 10px 8px 28px;
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

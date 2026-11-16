<script setup>
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { writeText } from '@tauri-apps/plugin-clipboard-manager'
import { openUrl } from '@tauri-apps/plugin-opener'
import { store, loadVault, err, confirm, toast, openMenu } from '../lib/store'
import { api, fmtTime } from '../lib/api'
import { copySecret } from '../lib/lock'
import { strength } from '../lib/passgen'
import { pick } from '../lib/os'
import Icon from '../components/Icon.vue'
import PasswordGenerator from '../components/PasswordGenerator.vue'

const mod = store.platform.os === 'macos' ? '⌘' : 'Ctrl+'

const q = ref('')
const filter = ref('all') // all | weak | reused | tag:<name>
const sort = ref(readPref('passwords.sort', 'name')) // name | recent
const selectedId = ref(null)
const draft = ref(null) // the entry being edited or created
let original = '' // draft as it was opened, to spot unsaved changes
const showPass = ref(false)
const showGen = ref(false)
const revealed = ref(false)
const copied = ref('') // "<id>:<field>" just copied, for the check mark
const searchEl = ref(null)
const nameEl = ref(null)
const listEl = ref(null)

function readPref(k, d) {
  try {
    return localStorage.getItem(k) || d
  } catch {
    return d
  }
}
watch(sort, (v) => {
  try {
    localStorage.setItem('passwords.sort', v)
  } catch {}
})

const all = computed(() => store.vault.passwords || [])

// ---------- health: weak and reused passwords ----------
const useCount = computed(() => {
  const n = new Map()
  const add = (pw) => pw && n.set(pw, (n.get(pw) || 0) + 1)
  all.value.forEach((p) => add(p.password))
  store.vault.hosts.forEach((h) => add(h.password))
  return n
})
const isWeak = (p) => !!p.password && strength(p.password).level <= 2
const isReused = (p) => !!p.password && useCount.value.get(p.password) > 1
const reusedIn = (p) => {
  if (!isReused(p)) return []
  return [
    ...all.value.filter((x) => x.id !== p.id && x.password === p.password).map((x) => x.name),
    ...store.vault.hosts.filter((h) => h.password === p.password).map((h) => `host ${h.label || h.host}`),
  ]
}

const tags = computed(() => [...new Set(all.value.flatMap((p) => p.tags || []))].sort())
const counts = computed(() => ({
  weak: all.value.filter(isWeak).length,
  reused: all.value.filter(isReused).length,
}))

const list = computed(() => {
  const s = q.value.trim().toLowerCase()
  const f = filter.value
  const out = all.value.filter((p) => {
    if (f === 'weak' && !isWeak(p)) return false
    if (f === 'reused' && !isReused(p)) return false
    if (f.startsWith('tag:') && !(p.tags || []).includes(f.slice(4))) return false
    return !s || [p.name, p.username, p.url, p.notes, ...(p.tags || [])].join(' ').toLowerCase().includes(s)
  })
  return sort.value === 'recent'
    ? out.sort((a, b) => (b.updated_at || 0) - (a.updated_at || 0))
    : out.sort((a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: 'base' }))
})

const selected = computed(() => all.value.find((p) => p.id === selectedId.value) || null)
// Keep a sensible selection when the list changes.
watch(
  list,
  (l) => {
    if (draft.value) return
    if (!l.some((p) => p.id === selectedId.value)) selectedId.value = l[0]?.id || null
  },
  { immediate: true },
)
watch(selectedId, () => (revealed.value = false))

// ---------- actions ----------
function flash(key) {
  copied.value = key
  setTimeout(() => copied.value === key && (copied.value = ''), 1500)
}
async function copyPassword(p) {
  if (!p?.password) return
  await copySecret(p.password, 'Password')
  flash(p.id + ':password')
}
async function copyPlain(p, field, what) {
  try {
    await writeText(p[field])
    toast(`${what} copied`)
    flash(p.id + ':' + field)
  } catch (e) {
    err(e)
  }
}
const isLink = (u) => /^https?:\/\//i.test(u || '')
const openLink = (u) => openUrl(u).catch(err)

function select(p) {
  if (draft.value) return
  selectedId.value = p.id
}

async function startEdit(p) {
  if (draft.value && !(await discardOk())) return
  const tag = filter.value.startsWith('tag:') ? filter.value.slice(4) : ''
  draft.value = p
    ? { ...p, tags: (p.tags || []).join(', ') }
    : { id: '', name: '', username: '', password: '', url: '', notes: '', tags: tag }
  original = JSON.stringify(draft.value)
  showPass.value = !p
  showGen.value = false
  await nextTick()
  nameEl.value?.focus()
}
const dirty = () => draft.value && JSON.stringify(draft.value) !== original
async function discardOk() {
  return (
    !dirty() || confirm('Discard changes?', 'What you changed in this password is not saved.', { okText: 'Discard' })
  )
}
async function cancelEdit() {
  if (!(await discardOk())) return
  draft.value = null
  showGen.value = false
}
async function save() {
  if (!draft.value.name.trim()) return nameEl.value?.focus()
  try {
    const saved = await api.password_save({
      password: {
        ...draft.value,
        name: draft.value.name.trim(),
        tags: [
          ...new Set(
            draft.value.tags
              .split(',')
              .map((t) => t.trim())
              .filter(Boolean),
          ),
        ],
      },
    })
    const isNew = !draft.value.id
    draft.value = null
    showGen.value = false
    await loadVault()
    selectedId.value = saved.id
    toast(isNew ? `“${saved.name}” saved` : 'Changes saved')
  } catch (e) {
    err(e)
  }
}
async function remove(p) {
  if (!(await confirm(`Delete “${p.name}”?`, 'The password is removed from the vault. This cannot be undone.'))) return
  try {
    const i = list.value.findIndex((x) => x.id === p.id)
    const next = list.value[i + 1]?.id || list.value[i - 1]?.id || null
    await api.password_delete({ id: p.id })
    await loadVault()
    selectedId.value = next
    toast(`“${p.name}” deleted`)
  } catch (e) {
    err(e)
  }
}
async function duplicate(p) {
  if (draft.value && !(await discardOk())) return
  draft.value = { ...p, id: '', name: `${p.name} copy`, tags: (p.tags || []).join(', '), created_at: 0 }
  original = '' // a copy is unsaved from the start
  showPass.value = false
  showGen.value = false
  await nextTick()
  nameEl.value?.focus()
  nameEl.value?.select()
}
function useGenerated(v) {
  draft.value.password = v
  showPass.value = true
  showGen.value = false
}
function menu(e, p) {
  openMenu(e, [
    ...(p.password ? [{ label: 'Copy password', icon: 'key', action: () => copyPassword(p) }] : []),
    ...(p.username
      ? [{ label: 'Copy username', icon: 'user', action: () => copyPlain(p, 'username', 'Username') }]
      : []),
    ...(isLink(p.url) ? [{ label: 'Open website', icon: 'globe', action: () => openLink(p.url) }] : []),
    {
      label: 'Edit',
      icon: 'edit',
      action: () => {
        selectedId.value = p.id
        startEdit(p)
      },
    },
    { label: 'Duplicate', icon: 'copy', action: () => duplicate(p) },
    '-',
    { label: 'Delete', icon: 'trash', danger: true, action: () => remove(p) },
  ])
}

// ---------- keyboard ----------
function move(d) {
  const l = list.value
  if (!l.length) return
  const i = l.findIndex((p) => p.id === selectedId.value)
  selectedId.value = l[Math.min(l.length - 1, Math.max(0, i + d))].id
  nextTick(() => listEl.value?.querySelector('.item.on')?.scrollIntoView({ block: 'nearest' }))
}
function onKey(e) {
  if (store.active !== 'vault' || store.view !== 'passwords') return
  if (store.dialog || store.palette || store.menu || store.lock.locked || e.defaultPrevented) return
  const cmd = e.metaKey || e.ctrlKey
  if (draft.value) {
    if (e.key === 'Escape') {
      e.preventDefault()
      if (showGen.value) showGen.value = false
      else cancelEdit()
    } else if (cmd && e.key === 'Enter') {
      e.preventDefault()
      save()
    }
    return
  }
  const t = e.target
  const inSearch = t === searchEl.value
  if (!inSearch && t.closest?.('input, textarea, select, [contenteditable]')) return
  if (cmd && e.key.toLowerCase() === 'f') {
    e.preventDefault()
    searchEl.value?.focus()
    searchEl.value?.select()
  } else if (cmd && e.key.toLowerCase() === 'n') {
    e.preventDefault()
    startEdit(null)
  } else if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
    e.preventDefault()
    move(e.key === 'ArrowDown' ? 1 : -1)
  } else if (e.key === 'Enter' && selected.value) {
    e.preventDefault()
    copyPassword(selected.value)
  } else if (e.key === 'Escape' && inSearch && q.value) {
    q.value = ''
  }
}
onMounted(() => window.addEventListener('keydown', onKey))
onBeforeUnmount(() => window.removeEventListener('keydown', onKey))

// Locking K-Shell drops an open editor and any revealed password.
watch(
  () => store.lock.locked,
  (locked) => {
    if (!locked) return
    draft.value = null
    revealed.value = false
    showGen.value = false
  },
)

const initial = (p) => (p.name || '').trim()[0]?.toUpperCase() || '?'
const avatar = (p) => ({ background: pick(p.name || '') })
/** Colour class of one character of a shown password: digits and symbols stand out. */
const charClass = (c) => (/\d/.test(c) ? 'd' : /[a-zA-Z]/.test(c) ? '' : 's')
const draftStrength = computed(() => strength(draft.value?.password || ''))
</script>

<template>
  <div class="page">
    <div class="page-head">
      <h1>Passwords</h1>
      <div v-if="all.length" class="search">
        <Icon name="search" :size="14" />
        <input ref="searchEl" class="input" v-model="q" :placeholder="`Search passwords  (${mod}F)`" />
      </div>
      <button class="btn primary" :title="`New password (${mod}N)`" @click="startEdit(null)">
        <Icon name="plus" :size="14" /> New Password
      </button>
    </div>

    <!-- empty vault -->
    <div v-if="!all.length && !draft" class="page-body">
      <div class="empty first">
        <span class="hero"><Icon name="lock" :size="30" /></span>
        <div class="big">Keep your other passwords here</div>
        <div class="lead">
          Database logins, sudo passwords, web consoles, API tokens: everything that isn't a host. They are stored in
          the same encrypted vault and copied without landing in clipboard history.
        </div>
        <button class="btn primary" @click="startEdit(null)"><Icon name="plus" :size="14" /> Add a password</button>
      </div>
    </div>

    <div v-else class="split" :class="{ detail: draft || selected }">
      <!-- list -->
      <section class="col-list">
        <div class="filters">
          <button class="chip" :class="{ on: filter === 'all' }" @click="filter = 'all'">
            All <span class="n">{{ all.length }}</span>
          </button>
          <button v-if="counts.weak" class="chip warn" :class="{ on: filter === 'weak' }" @click="filter = 'weak'">
            Weak <span class="n">{{ counts.weak }}</span>
          </button>
          <button
            v-if="counts.reused"
            class="chip warn"
            :class="{ on: filter === 'reused' }"
            @click="filter = 'reused'"
          >
            Reused <span class="n">{{ counts.reused }}</span>
          </button>
          <button
            v-for="t in tags"
            :key="t"
            class="chip"
            :class="{ on: filter === 'tag:' + t }"
            @click="filter = filter === 'tag:' + t ? 'all' : 'tag:' + t"
          >
            # {{ t }}
          </button>
        </div>
        <div class="listbar dim">
          <span>{{ list.length }} {{ list.length === 1 ? 'item' : 'items' }}</span>
          <select class="sort" v-model="sort" title="Sort">
            <option value="name">Name A–Z</option>
            <option value="recent">Recently updated</option>
          </select>
        </div>
        <div ref="listEl" class="items">
          <div
            v-for="p in list"
            :key="p.id"
            class="item"
            :class="{ on: !draft && p.id === selectedId, locked: !!draft }"
            @click="select(p)"
            @dblclick="startEdit(p)"
            @contextmenu="menu($event, p)"
          >
            <span class="avatar" :style="avatar(p)">{{ initial(p) }}</span>
            <div class="meta">
              <div class="name">{{ p.name }}</div>
              <div class="sub dim">{{ p.username || p.url || 'No username' }}</div>
            </div>
            <span
              v-if="isWeak(p) || isReused(p)"
              class="flag"
              :title="isReused(p) ? 'Reused password' : 'Weak password'"
            >
              <Icon name="alert" :size="13" />
            </span>
            <button
              v-if="p.password"
              class="iconbtn quick"
              :class="{ done: copied === p.id + ':password' }"
              title="Copy password"
              @click.stop="copyPassword(p)"
            >
              <Icon :name="copied === p.id + ':password' ? 'check' : 'copy'" :size="14" />
            </button>
          </div>
          <div v-if="!list.length" class="none dim">
            <template v-if="q">No passwords match “{{ q }}”</template>
            <template v-else>Nothing here</template>
            <button class="btn sm" @click="((q = ''), (filter = 'all'))">Show all</button>
          </div>
        </div>
        <div class="keys dim">
          <span><kbd>↑</kbd><kbd>↓</kbd> move</span>
          <span><kbd>Enter</kbd> copy password</span>
        </div>
      </section>

      <!-- editor -->
      <section v-if="draft" class="col-detail">
        <form class="pane" @submit.prevent="save">
          <div class="pane-head">
            <button type="button" class="iconbtn back" title="Back" @click="cancelEdit"><Icon name="left" /></button>
            <span class="avatar big" :style="avatar(draft)">{{ initial(draft) }}</span>
            <input
              ref="nameEl"
              class="title-input"
              v-model="draft.name"
              placeholder="Name, e.g. Production Postgres"
              required
            />
          </div>

          <div class="field">
            <label>Username</label>
            <input class="input mono" v-model.trim="draft.username" placeholder="postgres" autocomplete="off" />
          </div>

          <div class="field">
            <label>Password</label>
            <div class="pwrow">
              <input
                class="input mono"
                v-model="draft.password"
                :type="showPass ? 'text' : 'password'"
                autocomplete="new-password"
                spellcheck="false"
              />
              <button
                type="button"
                class="iconbtn"
                :class="{ active: showPass }"
                :title="showPass ? 'Hide' : 'Show'"
                @click="showPass = !showPass"
              >
                <Icon name="eye" />
              </button>
              <button type="button" class="btn" :class="{ primary: !draft.password }" @click="showGen = !showGen">
                <Icon name="dice" :size="14" /> Generate
              </button>
            </div>
            <div v-if="draft.password" class="meter" :data-level="draftStrength.level">
              <i></i><i></i><i></i><i></i>
              <span>{{ draftStrength.label }}</span>
            </div>
            <PasswordGenerator v-if="showGen" @use="useGenerated" @close="showGen = false" />
          </div>

          <div class="field">
            <label>Website or host</label>
            <input
              class="input mono"
              v-model.trim="draft.url"
              placeholder="https://console.example.com  or  db.internal:5432"
            />
          </div>

          <div class="field">
            <label>Tags</label>
            <input class="input" v-model="draft.tags" placeholder="database, prod" list="pw-tags" />
            <datalist id="pw-tags"><option v-for="t in tags" :key="t" :value="t" /></datalist>
          </div>

          <div class="field">
            <label>Notes</label>
            <textarea
              class="textarea"
              rows="4"
              v-model="draft.notes"
              placeholder="Anything worth remembering"
            ></textarea>
          </div>

          <div class="pane-foot">
            <span class="dim keyhint"
              ><kbd>{{ mod }}Enter</kbd> save · <kbd>Esc</kbd> cancel</span
            >
            <button type="button" class="btn" @click="cancelEdit">Cancel</button>
            <button class="btn primary">{{ draft.id ? 'Save changes' : 'Save password' }}</button>
          </div>
        </form>
      </section>

      <!-- details -->
      <section v-else-if="selected" class="col-detail">
        <div class="pane">
          <div class="pane-head">
            <button class="iconbtn back" title="Back" @click="selectedId = null"><Icon name="left" /></button>
            <span class="avatar big" :style="avatar(selected)">{{ initial(selected) }}</span>
            <div class="title">
              <h2>{{ selected.name }}</h2>
              <a v-if="isLink(selected.url)" class="link" href="#" @click.prevent="openLink(selected.url)">{{
                selected.url.replace(/^https?:\/\//i, '')
              }}</a>
              <span v-else-if="selected.url" class="dim mono">{{ selected.url }}</span>
            </div>
            <button class="btn" @click="startEdit(selected)"><Icon name="edit" :size="13" /> Edit</button>
            <button class="iconbtn" title="More" @click="menu($event, selected)"><Icon name="more" /></button>
          </div>

          <div v-if="isReused(selected) || isWeak(selected)" class="notice">
            <Icon name="alert" :size="14" />
            <span v-if="isReused(selected)">
              Same password as {{ reusedIn(selected).slice(0, 3).join(', ')
              }}{{ reusedIn(selected).length > 3 ? ` and ${reusedIn(selected).length - 3} more` : '' }}. Consider giving
              each its own.
            </span>
            <span v-else>This password is easy to guess. Generate a stronger one when you can.</span>
          </div>

          <div class="fields">
            <div v-if="selected.username" class="frow" @click="copyPlain(selected, 'username', 'Username')">
              <label>Username</label>
              <span class="val mono">{{ selected.username }}</span>
              <span class="act" :class="{ done: copied === selected.id + ':username' }">
                <Icon :name="copied === selected.id + ':username' ? 'check' : 'copy'" :size="14" />
                {{ copied === selected.id + ':username' ? 'Copied' : 'Copy' }}
              </span>
            </div>
            <div v-if="selected.password" class="frow" @click="copyPassword(selected)">
              <label>Password</label>
              <span class="val mono pwval">
                <template v-if="revealed">
                  <span v-for="(c, i) in selected.password" :key="i" :class="charClass(c)">{{ c }}</span>
                </template>
                <template v-else>••••••••••••</template>
              </span>
              <button
                class="iconbtn"
                :class="{ active: revealed }"
                :title="revealed ? 'Hide' : 'Show'"
                @click.stop="revealed = !revealed"
              >
                <Icon name="eye" :size="14" />
              </button>
              <span class="act" :class="{ done: copied === selected.id + ':password' }">
                <Icon :name="copied === selected.id + ':password' ? 'check' : 'copy'" :size="14" />
                {{ copied === selected.id + ':password' ? 'Copied' : 'Copy' }}
              </span>
            </div>
            <div v-if="selected.password" class="frow static">
              <label>Strength</label>
              <span class="meter" :data-level="strength(selected.password).level">
                <i></i><i></i><i></i><i></i>
                <span>{{ strength(selected.password).label }} · {{ selected.password.length }} characters</span>
              </span>
            </div>
            <div v-if="selected.url" class="frow" @click="copyPlain(selected, 'url', 'Address')">
              <label>Website or host</label>
              <span class="val mono">{{ selected.url }}</span>
              <button v-if="isLink(selected.url)" class="iconbtn" title="Open" @click.stop="openLink(selected.url)">
                <Icon name="popout" :size="14" />
              </button>
              <span class="act" :class="{ done: copied === selected.id + ':url' }">
                <Icon :name="copied === selected.id + ':url' ? 'check' : 'copy'" :size="14" />
                {{ copied === selected.id + ':url' ? 'Copied' : 'Copy' }}
              </span>
            </div>
          </div>

          <div v-if="selected.notes" class="block">
            <div class="section-title">Notes</div>
            <div class="notes">{{ selected.notes }}</div>
          </div>

          <div v-if="selected.tags?.length" class="block tagrow">
            <button v-for="t in selected.tags" :key="t" class="chip" @click="filter = 'tag:' + t"># {{ t }}</button>
          </div>

          <div class="stamp dim">
            <span v-if="selected.updated_at">Updated {{ fmtTime(selected.updated_at) }}</span>
            <span v-if="selected.created_at">Created {{ fmtTime(selected.created_at) }}</span>
          </div>
        </div>
      </section>

      <section v-else class="col-detail placeholder dim">
        <Icon name="lock" :size="28" />
        <div>Select a password</div>
      </section>
    </div>
  </div>
</template>

<style scoped>
/* ---------- empty vault ---------- */
.first {
  padding-top: 90px;
  max-width: 460px;
  margin: 0 auto;
}
.hero {
  width: 64px;
  height: 64px;
  border-radius: 18px;
  display: grid;
  place-items: center;
  color: var(--accent);
  background: color-mix(in srgb, var(--accent) 14%, transparent);
}
.lead {
  line-height: 1.5;
  margin-bottom: 6px;
}

/* ---------- layout ---------- */
.split {
  flex: 1;
  min-height: 0;
  display: grid;
  grid-template-columns: minmax(250px, 320px) 1fr;
  gap: 14px;
  padding: 4px 22px 22px;
  container-type: inline-size;
}
.col-list,
.col-detail {
  min-height: 0;
  background: var(--bg-2);
  border: 1px solid var(--line);
  border-radius: var(--radius);
  display: flex;
  flex-direction: column;
}
.col-detail {
  overflow: auto;
}
.back {
  display: none;
}
/* narrow window: list or details, one at a time */
@container (max-width: 640px) {
  .col-list,
  .col-detail {
    grid-column: 1 / -1;
    grid-row: 1;
  }
  .split.detail .col-list,
  .split:not(.detail) .col-detail {
    display: none;
  }
  .back {
    display: inline-grid;
  }
}

/* ---------- list ---------- */
.filters {
  display: flex;
  flex-wrap: wrap;
  gap: 5px;
  padding: 10px 10px 4px;
}
.filters .chip {
  border: 0;
  cursor: pointer;
  height: 24px;
  font-size: 12px;
}
.filters .chip .n {
  margin-left: 5px;
  opacity: 0.7;
  font-variant-numeric: tabular-nums;
}
.filters .chip.warn:not(.on) {
  color: var(--warn);
}
.listbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 4px 12px 6px;
  font-size: 11.5px;
}
.sort {
  border: 0;
  background: none;
  color: var(--text-2);
  font: inherit;
  cursor: pointer;
}
.items {
  flex: 1;
  overflow: auto;
  padding: 0 6px 6px;
}
.item {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 7px 6px 7px 8px;
  border-radius: 9px;
  cursor: pointer;
}
.item:hover {
  background: var(--bg-3);
}
.item.on {
  background: color-mix(in srgb, var(--accent) 15%, transparent);
}
.item.locked {
  cursor: default;
  opacity: 0.55;
}
.item .meta {
  flex: 1;
  min-width: 0;
}
.item .name {
  font-weight: 600;
  font-size: 13px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.item .sub {
  font-size: 11.5px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.flag {
  color: var(--warn);
  flex: none;
  display: inline-grid;
}
.quick {
  opacity: 0;
  flex: none;
}
.item:hover .quick,
.item.on .quick,
.quick.done,
.quick:focus-visible {
  opacity: 1;
}
.done {
  color: var(--ok) !important;
}
.none {
  display: grid;
  justify-items: center;
  gap: 8px;
  padding: 30px 10px;
  text-align: center;
  font-size: 12.5px;
}
.keys {
  display: flex;
  gap: 12px;
  padding: 7px 12px;
  border-top: 1px solid var(--line);
  font-size: 11px;
}
kbd {
  font-family: inherit;
  font-size: 10.5px;
  padding: 0 4px;
  margin-right: 2px;
  border-radius: 4px;
  border: 1px solid var(--line);
  background: var(--bg-3);
}

.avatar {
  width: 32px;
  height: 32px;
  flex: none;
  border-radius: 9px;
  display: grid;
  place-items: center;
  color: #fff;
  font-weight: 700;
  font-size: 14px;
}
.avatar.big {
  width: 46px;
  height: 46px;
  border-radius: 12px;
  font-size: 19px;
}

/* ---------- details / editor ---------- */
.pane {
  padding: 18px 20px;
  display: flex;
  flex-direction: column;
  gap: 14px;
  max-width: 640px;
  width: 100%;
}
.pane-head {
  display: flex;
  align-items: center;
  gap: 12px;
}
.title {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.title h2 {
  margin: 0;
  font-size: 17px;
  font-weight: 650;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.title .link,
.title .mono {
  font-size: 12.5px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.link {
  color: var(--accent);
  text-decoration: none;
}
.link:hover {
  text-decoration: underline;
}
.title-input {
  flex: 1;
  min-width: 0;
  font: inherit;
  font-size: 17px;
  font-weight: 650;
  color: var(--text);
  background: transparent;
  border: 0;
  border-bottom: 1px solid var(--line);
  padding: 6px 2px;
  outline: none;
}
.title-input:focus {
  border-bottom-color: var(--accent);
}
.notice {
  display: flex;
  gap: 8px;
  align-items: flex-start;
  padding: 9px 12px;
  border-radius: 9px;
  font-size: 12.5px;
  line-height: 1.45;
  color: var(--text-2);
  background: color-mix(in srgb, var(--warn) 12%, transparent);
}
.notice .icon {
  color: var(--warn);
  flex: none;
  margin-top: 1px;
}
.fields {
  border: 1px solid var(--line);
  border-radius: 10px;
  overflow: hidden;
}
.frow {
  display: flex;
  align-items: center;
  gap: 10px;
  min-height: 46px;
  padding: 4px 8px 4px 14px;
  cursor: pointer;
}
.frow + .frow {
  border-top: 1px solid var(--line);
}
.frow:not(.static):hover {
  background: var(--bg-3);
}
.frow.static {
  cursor: default;
}
.frow label {
  width: 120px;
  flex: none;
  font-size: 12px;
  color: var(--text-3);
  cursor: inherit;
}
.frow .val {
  flex: 1;
  min-width: 0;
  font-size: 13px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.pwval {
  letter-spacing: 0.04em;
}
.pwval .d {
  color: var(--accent);
}
.pwval .s {
  color: var(--warn);
}
.act {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  font-size: 12px;
  color: var(--text-3);
  opacity: 0;
  min-width: 62px;
  justify-content: flex-end;
  padding-right: 4px;
}
.frow:hover .act,
.act.done {
  opacity: 1;
}
.block .notes {
  white-space: pre-wrap;
  font-size: 13px;
  line-height: 1.5;
  color: var(--text-2);
  background: var(--bg);
  border-radius: 9px;
  padding: 10px 12px;
  -webkit-user-select: text;
  user-select: text;
}
.tagrow {
  display: flex;
  flex-wrap: wrap;
  gap: 5px;
}
.tagrow .chip {
  border: 0;
  cursor: pointer;
}
.stamp {
  display: flex;
  gap: 14px;
  font-size: 11.5px;
}
.placeholder {
  display: grid;
  place-content: center;
  justify-items: center;
  gap: 8px;
  font-size: 13px;
}

.pwrow {
  display: flex;
  gap: 6px;
  align-items: center;
}
.meter {
  display: flex;
  align-items: center;
  gap: 4px;
  font-size: 11.5px;
  color: var(--text-3);
  margin-top: 2px;
}
.meter i {
  width: 26px;
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
.pane-foot {
  display: flex;
  align-items: center;
  gap: 8px;
  padding-top: 4px;
}
.keyhint {
  flex: 1;
  font-size: 11.5px;
}
</style>

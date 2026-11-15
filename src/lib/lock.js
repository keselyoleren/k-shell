// App lock: while K-Shell is locked the backend holds no vault key and no decrypted data, and
// the UI drops its copy of the vault too. Open sessions keep running behind the lock screen;
// the host copies their tabs hold lose their secrets until the vault is unlocked again.
// Also: copying secrets to the clipboard (concealed, cleared after a delay).

import { listen } from '@tauri-apps/api/event'
import { store, loadVault, restoreSession, toast, err } from './store'
import { api } from './api'

const LOOK_KEY = 'kshell.lock-look'
const SECRET_FIELDS = ['password', 'secret_key', 'credential']

export async function refreshLock() {
  store.lock = await api.lock_status()
}

/** Theme, accent and zoom, remembered so the lock screen shown at launch looks like the app. */
export function rememberLook() {
  const s = store.vault.settings || {}
  try {
    localStorage.setItem(LOOK_KEY, JSON.stringify({ app_theme: s.app_theme, accent: s.accent, ui_zoom: s.ui_zoom }))
  } catch {}
}
export function rememberedLook() {
  try {
    return JSON.parse(localStorage.getItem(LOOK_KEY)) || {}
  } catch {
    return {}
  }
}

/** Calls fn(host) for every host copy held by an open tab or pane. */
function eachTabHost(fn) {
  const walk = (n) => (n.type === 'leaf' ? n.target?.host && fn(n.target.host) : n.children.forEach(walk))
  for (const t of store.tabs) {
    if (t.host) fn(t.host)
    if (t.root) walk(t.root)
  }
}

let sessionStarted = false
/** Reopen last run's tabs: once, as soon as the vault is readable. */
export function startSession() {
  if (sessionStarted) return
  sessionStarted = true
  restoreSession()
}

/** The vault was locked: forget everything read from it (settings stay, they hold no secrets). */
export function applyLocked() {
  store.lock.locked = true
  store.vault = {
    hosts: [],
    keys: [],
    snippets: [],
    forwards: [],
    known_hosts: [],
    workspaces: [],
    history: [],
    settings: store.vault.settings,
  }
  eachTabHost((h) => SECRET_FIELDS.forEach((f) => h[f] && (h[f] = '')))
  store.editor = null
  store.palette = false
  store.menu = null
  if (store.dialog) {
    const d = store.dialog
    store.dialog = null
    d.resolve?.(d.type === 'confirm' ? false : null)
  }
  document.activeElement?.blur?.()
}

export async function lockNow() {
  if (!store.lock.enabled) return toast('Turn on "Lock K-Shell" in Settings → Security first')
  try {
    await api.vault_lock()
    applyLocked()
  } catch (e) {
    err(e)
  }
}

/** The backend unlocked the vault: read it again and give open tabs their hosts back. */
export async function afterUnlock() {
  await loadVault()
  await refreshLock()
  eachTabHost((h) => {
    const fresh = h.id && store.vault.hosts.find((x) => x.id === h.id)
    if (fresh) SECRET_FIELDS.forEach((f) => (h[f] = fresh[f]))
  })
  rememberLook()
  lastActivity = Date.now()
  startSession()
}

// ---------- auto-lock ----------

let lastActivity = Date.now()
const bump = () => (lastActivity = Date.now())

/** Lock after the idle time set in Settings, and when the backend locks on sleep. */
export function startLockWatch() {
  for (const ev of ['keydown', 'mousedown', 'mousemove', 'wheel', 'touchstart'])
    window.addEventListener(ev, bump, { capture: true, passive: true })
  setInterval(() => {
    const mins = +store.vault.settings?.lock_idle_minutes || 0
    if (!store.lock.enabled || store.lock.locked || !mins) return
    if (Date.now() - lastActivity >= mins * 60000) lockNow()
  }, 10000)
  listen('vault-locked', () => store.lock.locked || applyLocked())
}

// ---------- secrets on the clipboard ----------

/** Copy a password, private key or passphrase: hidden from clipboard history, cleared later. */
export async function copySecret(text, what) {
  try {
    await api.clipboard_copy_secret({ text })
    const secs = +store.vault.settings.clipboard_clear_secs || 0
    toast(secs ? `${what} copied. It is cleared from the clipboard in ${secs} s` : `${what} copied`)
  } catch (e) {
    err(e)
  }
}

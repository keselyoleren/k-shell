<script setup>
import { onMounted, ref, nextTick } from 'vue'
import { open as openDialog } from '@tauri-apps/plugin-dialog'
import { store } from '../lib/store'
import { api } from '../lib/api'
import { afterUnlock } from '../lib/lock'
import Logo from './Logo.vue'
import Icon from './Icon.vue'

const password = ref('')
const error = ref('')
const busy = ref(false)
const input = ref(null)
// Forgotten password: start over from an encrypted backup.
const resetting = ref(false)
const backupPath = ref('')
const backupPass = ref('')

const canTouchId = () => store.lock.touch_id && store.lock.touch_id_available

async function run(fn) {
  busy.value = true
  error.value = ''
  try {
    await fn()
    await afterUnlock()
  } catch (e) {
    const msg = String(e?.message || e)
    if (msg !== 'cancelled') error.value = msg
    await nextTick()
    input.value?.focus()
    input.value?.select()
  } finally {
    busy.value = false
  }
}

function unlock() {
  if (!password.value || busy.value) return
  run(() => api.vault_unlock({ password: password.value }))
}
const touchId = () => run(() => api.vault_unlock_touch_id())

async function pickBackup() {
  const path = await openDialog({ multiple: false, filters: [{ name: 'K-Shell backup', extensions: ['kshell'] }] })
  if (typeof path === 'string') backupPath.value = path
}
function restore() {
  if (!backupPath.value || !backupPass.value || busy.value) return
  run(() => api.vault_reset_from_backup({ path: backupPath.value, passphrase: backupPass.value }))
}
const fileName = (p) => p.split(/[\\/]/).pop()

onMounted(async () => {
  await nextTick()
  input.value?.focus()
  if (canTouchId() && document.hasFocus()) touchId()
})
</script>

<template>
  <div class="lockscreen">
    <div class="drag" data-tauri-drag-region></div>
    <form v-if="!resetting" class="box" @submit.prevent="unlock">
      <Logo :size="56" />
      <h2>K-Shell is locked</h2>
      <p class="dim">Enter your master password to open your hosts and keys.</p>
      <input
        ref="input"
        v-model="password"
        class="input"
        type="password"
        placeholder="Master password"
        autocomplete="current-password"
        :disabled="busy"
      />
      <p v-if="error" class="error">{{ error }}</p>
      <button class="btn primary wide" type="submit" :disabled="busy || !password">
        {{ busy ? 'Unlocking…' : 'Unlock' }}
      </button>
      <button v-if="canTouchId()" class="btn wide" type="button" :disabled="busy" @click="touchId">
        <Icon name="shield" :size="14" /> Unlock with Touch ID
      </button>
      <button class="link" type="button" @click="((resetting = true), (error = ''))">Forgot your password?</button>
    </form>

    <form v-else class="box left" @submit.prevent="restore">
      <h2>Forgot your password?</h2>
      <p class="muted">
        Without the master password this vault can't be opened, and nobody can recover it. You can start again from an
        encrypted backup (Settings → Vault &amp; backup → Export).
      </p>
      <p class="dim">
        The locked vault is not deleted: it stays in K-Shell's data folder as
        <span class="mono">vault-locked-&lt;date&gt;.bin</span>, in case you remember the password later. The app lock
        is off after the restore; turn it on again in Settings → Security.
      </p>
      <div class="field">
        <label>Backup file</label>
        <button class="btn" type="button" @click="pickBackup">
          <Icon name="folder" :size="14" /> {{ backupPath ? fileName(backupPath) : 'Choose…' }}
        </button>
      </div>
      <div class="field">
        <label>Backup passphrase</label>
        <input v-model="backupPass" class="input" type="password" :disabled="busy" />
      </div>
      <p v-if="error" class="error">{{ error }}</p>
      <div class="row end">
        <button class="btn" type="button" :disabled="busy" @click="((resetting = false), (error = ''))">Back</button>
        <button class="btn primary" type="submit" :disabled="busy || !backupPath || !backupPass">
          {{ busy ? 'Restoring…' : 'Restore backup' }}
        </button>
      </div>
    </form>
  </div>
</template>

<style scoped>
.lockscreen {
  position: fixed;
  inset: 0;
  z-index: 90;
  display: grid;
  place-items: center;
  padding: 16px;
  background: var(--bg);
}
.drag {
  position: absolute;
  inset: 0 0 auto 0;
  height: 40px;
}
.box {
  width: min(340px, 100%);
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 10px;
  text-align: center;
}
.box.left {
  width: min(440px, 100%);
  align-items: stretch;
  text-align: left;
}
.box.left .field {
  margin-bottom: 0;
}
.box.left .field .btn {
  align-self: flex-start;
}
h2 {
  margin: 8px 0 0;
  font-size: 18px;
}
p {
  margin: 0;
}
.wide {
  width: 100%;
  justify-content: center;
}
.error {
  color: var(--danger, #e5484d);
  font-size: 12px;
}
.link {
  margin-top: 4px;
  border: 0;
  background: none;
  color: var(--text-3);
  font-size: 12px;
  cursor: pointer;
}
.link:hover {
  color: var(--text);
}
.end {
  justify-content: flex-end;
}
</style>

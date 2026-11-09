// Importing hosts from other tools (shared by the host list and Settings → Vault).
import { open as openDialog } from '@tauri-apps/plugin-dialog'
import { api } from './api'
import { store, loadVault, toast, err, confirm } from './store'

const plural = (n, w) => `${n} ${w}${n === 1 ? '' : 's'}`

export async function importSshConfig() {
  try {
    const n = await api.import_ssh_config()
    await loadVault()
    toast(`Imported ${plural(n, 'host')} from ~/.ssh/config`)
  } catch (e) {
    err(e)
  }
}

/** PuTTY sessions from the Windows registry, or from a .reg export with `file`. */
export async function importPutty({ file = false } = {}) {
  let path = null
  if (file) {
    path = await openDialog({
      multiple: false,
      directory: false,
      filters: [{ name: 'Registry export', extensions: ['reg'] }],
    })
    if (!path) return
  }
  try {
    const r = await api.import_putty({ path })
    await loadVault()
    const title = [`Imported ${plural(r.imported, 'host')} from PuTTY`, r.existing ? `${r.existing} already there` : '']
      .filter(Boolean)
      .join(' · ')
    const details = [...r.skipped.map((s) => `Skipped ${s}`), ...r.notes]
    if (!details.length) return toast(title)
    await confirm(title, 'Some sessions were skipped or came in without part of their settings:', {
      okText: 'OK',
      danger: false,
      detail: details.join('\n'),
    })
  } catch (e) {
    err(e)
  }
}

/** Items for an "Import" menu. */
export function importItems() {
  return [
    { label: 'From ~/.ssh/config', icon: 'import', action: importSshConfig },
    ...(store.platform.os === 'windows'
      ? [{ label: 'PuTTY sessions', icon: 'import', action: () => importPutty() }]
      : []),
    { label: 'PuTTY .reg file…', icon: 'file', action: () => importPutty({ file: true }) },
  ]
}

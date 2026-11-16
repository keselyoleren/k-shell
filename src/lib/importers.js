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

/** Toast for a short import, or a dialog listing what was skipped or partly imported. */
async function showReport(source, r) {
  await loadVault()
  const title = [
    `Imported ${plural(r.imported, 'host')} from ${source}`,
    r.existing ? `${r.existing} already there` : '',
  ]
    .filter(Boolean)
    .join(' · ')
  const details = [...r.skipped.map((s) => `Skipped ${s}`), ...r.notes]
  if (!details.length) return toast(title)
  await confirm(title, 'Some entries were skipped or came in without part of their settings:', {
    okText: 'OK',
    danger: false,
    detail: details.join('\n'),
  })
}

const pickFile = (name, extensions) =>
  openDialog({ multiple: false, directory: false, filters: [{ name, extensions }] })

/** PuTTY sessions from the Windows registry, or from a .reg export with `file`. */
export async function importPutty({ file = false } = {}) {
  let path = null
  if (file) {
    path = await pickFile('Registry export', ['reg'])
    if (!path) return
  }
  try {
    await showReport('PuTTY', await api.import_putty({ path }))
  } catch (e) {
    err(e)
  }
}

/** MobaXterm sessions from a .mxtsessions export or MobaXterm.ini. */
export async function importMobaxterm() {
  const path = await pickFile('MobaXterm sessions', ['mxtsessions', 'ini'])
  if (!path) return
  try {
    await showReport('MobaXterm', await api.import_mobaxterm({ path }))
  } catch (e) {
    err(e)
  }
}

/** Hosts from a CSV file (a spreadsheet export). */
export async function importCsv() {
  const path = await pickFile('CSV file', ['csv', 'txt'])
  if (!path) return
  try {
    await showReport('the CSV file', await api.import_csv({ path }))
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
    { label: 'MobaXterm sessions file…', icon: 'file', action: importMobaxterm },
    { label: 'CSV file…', icon: 'file', action: importCsv },
  ]
}

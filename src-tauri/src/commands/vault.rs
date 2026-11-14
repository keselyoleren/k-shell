//! Hosts, keys, snippets, settings, known hosts, import/backup.

use super::State;
use crate::error::{Ctx, Res};
use crate::vault::{self, upsert, Host, Key, Settings, Snippet, VaultData, Workspace};
use crate::{keys, ssh_config};

#[tauri::command]
pub fn vault_get(st: State) -> Res<VaultData> {
    st.vault.ensure_unlocked()?;
    Ok(st.vault.data.read().clone())
}

#[tauri::command]
pub fn host_save(st: State, mut host: Host) -> Res<Host> {
    if host.created_at == 0 {
        host.created_at = vault::now();
    }
    if host.protocol.is_empty() {
        host.protocol = "ssh".into();
    }
    let h = upsert(&mut st.vault.data.write().hosts, host);
    st.vault.save()?;
    Ok(h)
}

#[tauri::command]
pub fn host_delete(st: State, id: String) -> Res<()> {
    {
        let mut d = st.vault.data.write();
        d.hosts.retain(|h| h.id != id);
        for h in d.hosts.iter_mut().filter(|h| h.jump_host == id) {
            h.jump_host.clear();
        }
        d.forwards.retain(|f| f.host_id != id);
    }
    st.vault.save()
}

#[tauri::command]
pub fn key_save(st: State, key: Key) -> Res<Key> {
    let k = upsert(&mut st.vault.data.write().keys, key);
    st.vault.save()?;
    Ok(k)
}

#[tauri::command]
pub fn key_delete(st: State, id: String) -> Res<()> {
    {
        let mut d = st.vault.data.write();
        d.keys.retain(|k| k.id != id);
        for h in d.hosts.iter_mut().filter(|h| h.key_id == id) {
            h.key_id.clear();
        }
    }
    st.vault.save()
}

#[tauri::command]
pub async fn key_generate(st: State<'_>, name: String, kind: String, passphrase: String) -> Res<Key> {
    // RSA-4096 generation can take a second; keep it off the async runtime.
    let k = tauri::async_runtime::spawn_blocking(move || keys::generate(&name, &kind, &passphrase))
        .await
        .s()??;
    st.vault.data.write().keys.push(k.clone());
    st.vault.save()?;
    Ok(k)
}

#[tauri::command]
pub fn key_import(st: State, name: String, pem: String, passphrase: String) -> Res<Key> {
    let k = keys::import(&name, &pem, &passphrase)?;
    st.vault.data.write().keys.push(k.clone());
    st.vault.save()?;
    Ok(k)
}

#[tauri::command]
pub fn snippet_save(st: State, snippet: Snippet) -> Res<Snippet> {
    let s = upsert(&mut st.vault.data.write().snippets, snippet);
    st.vault.save()?;
    Ok(s)
}

#[tauri::command]
pub fn snippet_delete(st: State, id: String) -> Res<()> {
    st.vault.data.write().snippets.retain(|s| s.id != id);
    st.vault.save()
}

#[tauri::command]
pub fn settings_save(st: State, settings: Settings) -> Res<()> {
    st.vault.data.write().settings = settings;
    st.vault.save()
}

/// Remember the open tabs so they can be reopened on the next launch.
#[tauri::command]
pub fn session_save(st: State, session: serde_json::Value) -> Res<()> {
    st.vault.data.write().session = session;
    st.vault.save()
}

#[tauri::command]
pub fn workspace_save(st: State, mut workspace: Workspace) -> Res<Workspace> {
    if workspace.created_at == 0 {
        workspace.created_at = vault::now();
    }
    for p in workspace.projects.iter_mut().filter(|p| p.id.is_empty()) {
        p.id = vault::new_id();
    }
    let w = upsert(&mut st.vault.data.write().workspaces, workspace);
    st.vault.save()?;
    Ok(w)
}

#[tauri::command]
pub fn workspace_delete(st: State, id: String) -> Res<()> {
    st.vault.data.write().workspaces.retain(|w| w.id != id);
    st.vault.save()
}

#[tauri::command]
pub fn history_add(st: State, cmd: String, target: String) -> Res<()> {
    st.vault.data.write().add_history(&cmd, &target);
    st.vault.save()
}

#[tauri::command]
pub fn history_clear(st: State) -> Res<()> {
    st.vault.data.write().history.clear();
    st.vault.save()
}

#[tauri::command]
pub fn known_host_delete(st: State, host: String, key_type: String) -> Res<()> {
    st.vault.data.write().known_hosts.retain(|k| !(k.host == host && k.key_type == key_type));
    st.vault.save()
}

#[tauri::command]
pub fn import_ssh_config(st: State) -> Res<usize> {
    st.vault.ensure_unlocked()?;
    let n = ssh_config::import(&mut st.vault.data.write())?;
    st.vault.save()?;
    Ok(n)
}

/// Import PuTTY sessions: from the `.reg` file at `path`, or (Windows) from the registry.
#[tauri::command]
pub fn import_putty(st: State, path: Option<String>) -> Res<crate::putty::Report> {
    st.vault.ensure_unlocked()?;
    let sessions = match path.filter(|p| !p.trim().is_empty()) {
        Some(p) => crate::putty::parse_reg(&crate::putty::reg_text(&std::fs::read(&p).ctx(&p)?)),
        None => registry_sessions()?,
    };
    if sessions.is_empty() {
        return Err("No PuTTY sessions found".into());
    }
    let report = crate::putty::import(&mut st.vault.data.write(), sessions);
    st.vault.save()?;
    Ok(report)
}

#[cfg(windows)]
fn registry_sessions() -> Res<Vec<crate::putty::Session>> {
    crate::putty::read_registry()
}

#[cfg(not(windows))]
fn registry_sessions() -> Res<Vec<crate::putty::Session>> {
    Err("PuTTY keeps its sessions in the Windows registry: export them to a .reg file there and import that file".into())
}

#[tauri::command]
pub fn backup_export(st: State, path: String, passphrase: String) -> Res<()> {
    st.vault.ensure_unlocked()?;
    let blob = vault::export_backup(&st.vault.data.read(), &passphrase)?;
    std::fs::write(&path, blob).ctx(&path)
}

/// Merge an encrypted backup into the vault. Returns the number of hosts in it.
#[tauri::command]
pub fn backup_import(st: State, path: String, passphrase: String) -> Res<usize> {
    st.vault.ensure_unlocked()?;
    let blob = std::fs::read(&path).ctx(&path)?;
    let incoming = vault::import_backup(&blob, &passphrase)?;
    let n = incoming.hosts.len();
    st.vault.data.write().merge(incoming);
    st.vault.save()?;
    Ok(n)
}

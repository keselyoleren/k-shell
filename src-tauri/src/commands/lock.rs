//! App lock (master password / Touch ID) and copying secrets to the clipboard.

use tauri::{AppHandle, Emitter, Manager};

use super::State;
use crate::biometric;
use crate::error::{Ctx, Res};
use crate::vault::LockStatus;

/// Event sent to the UI when the backend locks the vault by itself (computer went to sleep).
pub const LOCKED_EVENT: &str = "vault-locked";

#[derive(serde::Serialize)]
pub struct Status {
    #[serde(flatten)]
    lock: LockStatus,
    /// This computer can show a Touch ID prompt.
    touch_id_available: bool,
}

#[tauri::command]
pub fn lock_status(st: State) -> Status {
    Status { lock: st.vault.lock_status(), touch_id_available: biometric::available() }
}

#[tauri::command]
pub fn lock_enable(st: State, password: String) -> Res<()> {
    st.vault.enable_lock(&password)
}

#[tauri::command]
pub fn lock_disable(st: State, password: String) -> Res<()> {
    st.vault.disable_lock(&password)
}

#[tauri::command]
pub fn lock_change_password(st: State, old: String, new: String) -> Res<()> {
    st.vault.change_password(&old, &new)
}

/// Turning Touch ID on asks for a fingerprint first, so it is known to work.
#[tauri::command]
pub async fn lock_set_touch_id(st: State<'_>, enabled: bool) -> Res<()> {
    st.vault.ensure_unlocked()?;
    if enabled {
        biometric::verify("turn on Touch ID for unlocking K-Shell").await?;
    }
    st.vault.set_touch_id(enabled)
}

#[tauri::command]
pub fn vault_lock(st: State) -> Res<()> {
    st.vault.lock()
}

#[tauri::command]
pub async fn vault_unlock(st: State<'_>, password: String) -> Res<()> {
    // Argon2id takes a moment; keep it off the async runtime.
    let vault = st.vault.clone();
    let res = tauri::async_runtime::spawn_blocking(move || vault.unlock(&password)).await.s()?;
    res?;
    after_unlock(&st).await;
    Ok(())
}

#[tauri::command]
pub async fn vault_unlock_touch_id(st: State<'_>) -> Res<()> {
    st.vault.lock_status().touch_id.then_some(()).ok_or("Touch ID is not set up")?;
    biometric::verify("unlock K-Shell").await?;
    st.vault.unlock_touch_id()?;
    after_unlock(&st).await;
    Ok(())
}

/// Forgotten master password: start over from an encrypted backup (the locked vault is kept).
#[tauri::command]
pub async fn vault_reset_from_backup(st: State<'_>, path: String, passphrase: String) -> Res<()> {
    let blob = std::fs::read(&path).ctx(&path)?;
    let vault = st.vault.clone();
    tauri::async_runtime::spawn_blocking(move || vault.reset_from_backup(&blob, &passphrase)).await.s()??;
    after_unlock(&st).await;
    Ok(())
}

/// Rules flagged to start by themselves couldn't when K-Shell started locked.
async fn after_unlock(st: &crate::state::AppState) {
    let active = st.forwards.active();
    let pending = st.vault.data.read().forwards.iter().any(|f| f.auto_start && !active.contains(&f.id));
    if pending && !st.autostarted.swap(true, std::sync::atomic::Ordering::SeqCst) {
        super::forwards::autostart(st).await;
    }
}

/// Copy a password, private key or passphrase: concealed from clipboard history and cleared
/// after the delay set in Settings.
#[tauri::command]
pub fn clipboard_copy_secret(st: State, text: String) -> Res<()> {
    let secs = st.vault.data.read().settings.clipboard_clear_secs;
    crate::clip::copy_secret(text, secs)
}

/// Lock when the computer wakes from sleep, if that is turned on. Sleep shows up as the wall
/// clock moving ahead of the monotonic clock, which stops while the system sleeps.
pub fn watch_sleep(app: AppHandle) {
    use std::time::{Duration, Instant, SystemTime};
    const TICK: Duration = Duration::from_secs(5);
    std::thread::spawn(move || {
        let (mut mono, mut wall) = (Instant::now(), SystemTime::now());
        loop {
            std::thread::sleep(TICK);
            let (m, w) = (Instant::now(), SystemTime::now());
            let slept = w.duration_since(wall).unwrap_or_default().saturating_sub(m - mono);
            (mono, wall) = (m, w);
            if slept < Duration::from_secs(20) {
                continue;
            }
            let st = app.state::<crate::state::AppState>();
            let on = st.vault.data.read().settings.lock_on_sleep;
            if on && st.vault.lock_enabled() && !st.vault.is_locked() && st.vault.lock().is_ok() {
                let _ = app.emit(LOCKED_EVENT, ());
            }
        }
    });
}

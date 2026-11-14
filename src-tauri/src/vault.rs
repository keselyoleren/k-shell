//! Local-first encrypted vault: hosts, keys, snippets, port-forward rules,
//! known hosts and settings. The whole vault is serialized to JSON and sealed
//! with XChaCha20-Poly1305. The 256-bit key lives in the OS keychain
//! (macOS Keychain / Windows Credential Manager / Secret Service) with a
//! 0600 key file as fallback.
//!
//! With the app lock on, the key is not stored at all: it is wrapped with a key derived from
//! the master password (Argon2id) in `vault.lock`, and the vault stays locked (no key, no data
//! in memory) until that password, or Touch ID, unlocks it.

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use chacha20poly1305::{
    aead::{Aead, KeyInit},
    XChaCha20Poly1305, XNonce,
};
use parking_lot::RwLock;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::error::{Ctx, Res};

const MAGIC: &[u8; 4] = b"TMX1";

pub fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

pub fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct Host {
    pub id: String,
    pub label: String,
    pub group: String,
    pub tags: Vec<String>,
    /// ssh | ftp | ftps | s3 | azblob | gcs | telnet | serial
    pub protocol: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub key_id: String,
    pub jump_host: String,
    pub startup_command: String,
    pub color: String,
    pub notes: String,
    pub os: String,
    pub agent_forward: bool,
    // S3 / object storage
    pub endpoint: String,
    pub region: String,
    pub bucket: String,
    pub access_key: String,
    pub secret_key: String,
    pub path_style: bool,
    /// S3 with a custom endpoint uses path-style URLs unless this is set
    /// (needed by Alibaba OSS, Tencent COS and other virtual-hosted-only services).
    pub virtual_hosted: bool,
    /// Google Cloud Storage service-account key (JSON).
    pub credential: String,
    // Serial console (`host` holds the device: /dev/ttyUSB0, COM3…). 0 / "" = defaults.
    pub baud_rate: u32,
    pub data_bits: u8,
    /// none | odd | even
    pub parity: String,
    pub stop_bits: u8,
    /// none | software | hardware
    pub flow_control: String,
    pub created_at: i64,
    pub last_used: i64,
}

impl Host {
    /// Serial settings of a serial-console host.
    pub fn serial(&self) -> crate::terminal::SerialOptions {
        crate::terminal::SerialOptions {
            path: self.host.clone(),
            baud_rate: self.baud_rate,
            data_bits: self.data_bits,
            parity: self.parity.clone(),
            stop_bits: self.stop_bits,
            flow_control: self.flow_control.clone(),
        }
    }

    pub fn addr_port(&self) -> u16 {
        if self.port != 0 {
            return self.port;
        }
        match self.protocol.as_str() {
            "ftp" | "ftps" => 21,
            "telnet" => 23,
            _ => 22,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct Key {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub private_key: String,
    pub public_key: String,
    pub passphrase: String,
    pub fingerprint: String,
    pub created_at: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct Snippet {
    pub id: String,
    pub name: String,
    pub command: String,
    pub tags: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct Forward {
    pub id: String,
    pub name: String,
    pub host_id: String,
    /// local | remote | dynamic
    pub kind: String,
    pub bind_host: String,
    pub bind_port: u16,
    pub dest_host: String,
    pub dest_port: u16,
    pub auto_start: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct KnownHost {
    pub host: String,
    pub key_type: String,
    pub fingerprint: String,
    pub added_at: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Settings {
    pub app_theme: String,
    pub term_theme: String,
    pub font_family: String,
    pub font_size: u16,
    pub line_height: f32,
    pub cursor_style: String,
    pub cursor_blink: bool,
    pub scrollback: u32,
    pub copy_on_select: bool,
    pub right_click_paste: bool,
    pub confirm_close: bool,
    /// Accent colour override (hex); empty uses the theme default.
    pub accent: String,
    /// Webview zoom factor, 1.0 = 100%.
    pub ui_zoom: f32,
    /// Use `term_theme_light` instead of `term_theme` while the app is light.
    pub term_theme_sync: bool,
    pub term_theme_light: String,
    /// Custom shortcuts: "<mac|pc>:<action id>" -> combo ("" = unbound).
    pub keybindings: std::collections::HashMap<String, String>,
    /// Install the K-Shell prompt in new bash/zsh sessions.
    pub prompt_enabled: bool,
    /// Ordered prompt segment ids (see src/lib/prompt.js).
    pub prompt_segments: Vec<String>,
    /// Where the prompt is applied: "all" | "local" | "ssh".
    pub prompt_scope: String,
    /// Show history-based autosuggestions while typing in terminals.
    pub autosuggest: bool,
    /// Keep typed commands in the vault across restarts.
    pub save_history: bool,
    /// Left sidebar hidden (toggled with ⌘B / Ctrl+Shift+B).
    pub sidebar_hidden: bool,
    /// Reconnect an SSH terminal by itself when its connection drops.
    pub auto_reconnect: bool,
    /// Reopen the tabs that were open when the app was closed.
    pub restore_tabs: bool,
    /// Terminal text weight ("normal", "500", "600", "bold") and the weight of bold text.
    pub font_weight: String,
    pub font_weight_bold: String,
    /// Extra pixels between characters.
    pub letter_spacing: f32,
    /// Draw bold text in the bright ANSI colours.
    pub bold_bright: bool,
    /// xterm minimum contrast ratio (1 = off).
    pub min_contrast: f32,
    /// Cursor shape while the terminal is not focused: outline | block | bar | underline | none.
    pub cursor_inactive: String,
    /// What the terminal bell does: none | visual | sound | both.
    pub bell: String,
    /// macOS: the Option key sends Meta (Esc+key) instead of typing special characters.
    pub option_meta: bool,
    /// Ask before pasting text that contains a line break.
    pub paste_warn: bool,
    /// Local terminals: shell to run (empty = the system default) and folder to start in.
    pub local_shell: String,
    pub local_cwd: String,
    /// Follow the shell's working directory (OSC 7; a small hook is added to bash/zsh).
    pub track_cwd: bool,
    /// Write a plain-text log of every terminal session.
    pub session_log: bool,
    /// Folder for session logs ("" = Documents/K-Shell Logs).
    pub log_dir: String,
    /// Colour words matching `highlight_rules` in terminal output.
    pub highlight: bool,
    pub highlight_rules: Vec<HighlightRule>,
    /// Show Sixel and iTerm2 inline images in terminals.
    pub images: bool,
    /// Memory for decoded images, per terminal, in MB (oldest images are dropped first).
    pub image_memory_mb: u32,
    /// Seconds between refreshes of the server stats panel.
    pub stats_interval: u32,
    /// App lock: minutes without keyboard or mouse use before K-Shell locks (0 = never).
    pub lock_idle_minutes: u32,
    /// App lock: lock when the computer goes to sleep.
    pub lock_on_sleep: bool,
    /// Seconds before a copied password or private key is cleared from the clipboard (0 = never).
    pub clipboard_clear_secs: u32,
}

/// Terminal output highlighting: words (comma separated) or a regular expression, drawn in
/// a colour of the terminal theme ("red", "yellow" …) or a hex colour.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct HighlightRule {
    pub pattern: String,
    pub color: String,
    pub regex: bool,
    pub case_sensitive: bool,
    pub enabled: bool,
}

fn default_highlights() -> Vec<HighlightRule> {
    let rule = |pattern: &str, color: &str| HighlightRule {
        pattern: pattern.into(),
        color: color.into(),
        regex: false,
        case_sensitive: false,
        enabled: true,
    };
    vec![
        rule("error, fatal, failed, failure, exception, panic, denied, refused", "red"),
        rule("warn, warning, deprecated", "yellow"),
        rule("success, succeeded, passed", "green"),
    ]
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            app_theme: "dark".into(),
            term_theme: "kshell".into(),
            font_family: "JetBrains Mono, Menlo, Consolas, 'DejaVu Sans Mono', monospace".into(),
            font_size: 13,
            line_height: 1.2,
            cursor_style: "block".into(),
            cursor_blink: true,
            scrollback: 10000,
            copy_on_select: true,
            right_click_paste: true,
            confirm_close: true,
            accent: String::new(),
            ui_zoom: 1.0,
            term_theme_sync: false,
            term_theme_light: "light".into(),
            keybindings: Default::default(),
            prompt_enabled: false,
            prompt_segments: ["venv", "user_host", "cwd", "git", "exit", "symbol"].map(String::from).to_vec(),
            prompt_scope: "all".into(),
            autosuggest: true,
            save_history: true,
            sidebar_hidden: false,
            auto_reconnect: true,
            restore_tabs: true,
            font_weight: "normal".into(),
            font_weight_bold: "bold".into(),
            letter_spacing: 0.0,
            bold_bright: true,
            min_contrast: 1.0,
            cursor_inactive: "outline".into(),
            bell: "visual".into(),
            option_meta: true,
            paste_warn: true,
            local_shell: String::new(),
            local_cwd: String::new(),
            track_cwd: true,
            session_log: false,
            log_dir: String::new(),
            highlight: true,
            highlight_rules: default_highlights(),
            images: true,
            image_memory_mb: 64,
            stats_interval: 3,
            lock_idle_minutes: 15,
            lock_on_sleep: true,
            clipboard_clear_secs: 30,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct VaultData {
    pub version: u32,
    pub hosts: Vec<Host>,
    pub keys: Vec<Key>,
    pub snippets: Vec<Snippet>,
    pub forwards: Vec<Forward>,
    pub known_hosts: Vec<KnownHost>,
    pub settings: Settings,
    /// Commands typed in terminals, newest last; feeds autosuggestions.
    pub history: Vec<HistoryEntry>,
    /// Groups of local project folders.
    pub workspaces: Vec<Workspace>,
    /// Tabs open when the app was last closed (layout written by the UI; hosts by id, no secrets).
    pub session: serde_json::Value,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct Workspace {
    pub id: String,
    pub name: String,
    pub color: String,
    pub projects: Vec<Project>,
    pub created_at: i64,
}

/// A folder opened in a local terminal, optionally running a command first.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub path: String,
    pub startup_command: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct HistoryEntry {
    pub cmd: String,
    /// "local" or the host address the command ran on.
    pub target: String,
    pub at: i64,
}

pub const HISTORY_MAX: usize = 2000;

impl VaultData {
    /// Record a command, moving an existing identical entry to the end.
    pub fn add_history(&mut self, cmd: &str, target: &str) {
        let cmd = cmd.trim();
        if cmd.is_empty() || cmd.len() > 2000 {
            return;
        }
        self.history.retain(|h| !(h.cmd == cmd && h.target == target));
        self.history.push(HistoryEntry { cmd: cmd.into(), target: target.into(), at: now() });
        let over = self.history.len().saturating_sub(HISTORY_MAX);
        self.history.drain(..over);
    }
}

pub struct Vault {
    dir: PathBuf,
    path: PathBuf,
    /// `None` while the app lock is engaged.
    key: RwLock<Option<[u8; 32]>>,
    pub data: RwLock<VaultData>,
}

/// Error returned by anything that needs the vault while it is locked.
pub const LOCKED: &str = "K-Shell is locked";

// In release builds prefer the OS keychain. Debug builds are re-signed on
// every compile which would make macOS prompt for keychain access each run.
fn use_keyring() -> bool {
    !cfg!(debug_assertions) && std::env::var("KSHELL_NO_KEYRING").is_err()
}

fn decode_key(s: &str) -> Option<[u8; 32]> {
    B64.decode(s.trim()).ok()?.try_into().ok()
}

fn load_or_create_key(dir: &Path) -> Res<[u8; 32]> {
    let key_file = dir.join("vault.key");
    let decode = decode_key;
    if use_keyring() {
        if let Ok(entry) = keyring::Entry::new("K-Shell", "vault-key") {
            if let Ok(s) = entry.get_password() {
                if let Some(k) = decode(&s) {
                    return Ok(k);
                }
            }
            // Pre-rename builds stored the key under the "TermX" service.
            let legacy = keyring::Entry::new("TermX", "vault-key")
                .and_then(|e| e.get_password())
                .ok()
                .and_then(|s| decode(&s));
            if let Some(k) = legacy {
                if entry.set_password(&B64.encode(k)).is_ok() {
                    return Ok(k);
                }
            }
            // Migrate an existing file key into the keychain, or create a new one.
            let k = std::fs::read_to_string(&key_file).ok().and_then(|s| decode(&s)).unwrap_or_else(|| {
                let mut k = [0u8; 32];
                rand::thread_rng().fill_bytes(&mut k);
                k
            });
            if entry.set_password(&B64.encode(k)).is_ok() {
                let _ = std::fs::remove_file(&key_file);
                return Ok(k);
            }
            write_key_file(&key_file, &k)?;
            return Ok(k);
        }
    }
    if let Some(k) = std::fs::read_to_string(&key_file).ok().and_then(|s| decode(&s)) {
        return Ok(k);
    }
    let mut k = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut k);
    write_key_file(&key_file, &k)?;
    Ok(k)
}

fn write_key_file(path: &Path, k: &[u8; 32]) -> Res<()> {
    std::fs::write(path, B64.encode(k)).s()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

/// A stored copy of the vault key: an OS keychain entry in release builds, else a file.
struct KeySlot {
    account: &'static str,
    file: &'static str,
}
/// The key K-Shell normally opens the vault with (absent while the app lock is on).
const MAIN_KEY: KeySlot = KeySlot { account: "vault-key", file: "vault.key" };
/// Copy kept for Touch ID unlock, read only after the OS prompt succeeds.
const TOUCH_ID_KEY: KeySlot = KeySlot { account: "vault-key-touchid", file: "vault-touchid.key" };

fn slot_set(dir: &Path, slot: &KeySlot, k: &[u8; 32]) -> Res<()> {
    if use_keyring() {
        if let Ok(e) = keyring::Entry::new("K-Shell", slot.account) {
            if e.set_password(&B64.encode(k)).is_ok() {
                let _ = std::fs::remove_file(dir.join(slot.file));
                return Ok(());
            }
        }
    }
    write_key_file(&dir.join(slot.file), k)
}

fn slot_get(dir: &Path, slot: &KeySlot) -> Option<[u8; 32]> {
    if use_keyring() {
        let k = keyring::Entry::new("K-Shell", slot.account).and_then(|e| e.get_password()).ok();
        if let Some(k) = k.and_then(|s| decode_key(&s)) {
            return Some(k);
        }
    }
    std::fs::read_to_string(dir.join(slot.file)).ok().and_then(|s| decode_key(&s))
}

fn delete_entry(service: &str, account: &str) -> Res<()> {
    match keyring::Entry::new(service, account).and_then(|e| e.delete_credential()) {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

fn slot_delete(dir: &Path, slot: &KeySlot) -> Res<()> {
    if use_keyring() {
        delete_entry("K-Shell", slot.account)?;
        if slot.account == MAIN_KEY.account {
            // Pre-rename builds kept the same key under the "TermX" service.
            delete_entry("TermX", slot.account)?;
        }
    }
    match std::fs::remove_file(dir.join(slot.file)) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
        _ => Ok(()),
    }
}

/// `vault.lock`: the vault key sealed with a key derived from the master password.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
struct LockFile {
    version: u32,
    salt: String,
    wrapped: String,
    /// A copy of the key is kept for Touch ID (see [`TOUCH_ID_KEY`]).
    touch_id: bool,
}

impl LockFile {
    fn new(key: &[u8; 32], password: &str) -> Res<Self> {
        if password.is_empty() {
            return Err("Choose a password".into());
        }
        let mut salt = [0u8; 16];
        rand::thread_rng().fill_bytes(&mut salt);
        let wrapped = seal(&passphrase_key(password, &salt)?, key)?;
        Ok(Self { version: 1, salt: B64.encode(salt), wrapped: B64.encode(wrapped), touch_id: false })
    }

    fn unwrap(&self, password: &str) -> Res<[u8; 32]> {
        let salt = B64.decode(&self.salt).s()?;
        let blob = B64.decode(&self.wrapped).s()?;
        let key = open(&passphrase_key(password, &salt)?, &blob).map_err(|_| "Wrong password".to_string())?;
        key.try_into().map_err(|_| "invalid lock file".to_string())
    }
}

/// What the UI needs to know about the app lock.
#[derive(Serialize, Clone, Debug, Default)]
pub struct LockStatus {
    pub enabled: bool,
    pub locked: bool,
    pub touch_id: bool,
}

pub fn seal(key: &[u8; 32], plain: &[u8]) -> Res<Vec<u8>> {
    let cipher = XChaCha20Poly1305::new(key.into());
    let mut nonce = [0u8; 24];
    rand::thread_rng().fill_bytes(&mut nonce);
    let ct = cipher.encrypt(XNonce::from_slice(&nonce), plain).map_err(|_| "encrypt failed".to_string())?;
    let mut out = Vec::with_capacity(4 + 24 + ct.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    Ok(out)
}

pub fn open(key: &[u8; 32], blob: &[u8]) -> Res<Vec<u8>> {
    if blob.len() < 28 || &blob[..4] != MAGIC {
        return Err("invalid vault file".into());
    }
    let cipher = XChaCha20Poly1305::new(key.into());
    cipher
        .decrypt(XNonce::from_slice(&blob[4..28]), &blob[28..])
        .map_err(|_| "cannot decrypt vault (wrong key or corrupted file)".to_string())
}

const BACKUP_MAGIC: &[u8; 4] = b"TMXB";

/// Derive a key from a user passphrase for portable backups (Argon2id).
fn passphrase_key(pass: &str, salt: &[u8]) -> Res<[u8; 32]> {
    let mut out = [0u8; 32];
    argon2::Argon2::default()
        .hash_password_into(pass.as_bytes(), salt, &mut out)
        .map_err(|e| e.to_string())?;
    Ok(out)
}

/// Serialize and encrypt `data` with a passphrase.
/// Format: `TMXB | salt(16) | TMX1 | nonce(24) | ciphertext`.
pub fn export_backup(data: &VaultData, passphrase: &str) -> Res<Vec<u8>> {
    if passphrase.is_empty() {
        return Err("a passphrase is required".into());
    }
    let json = serde_json::to_vec(data).s()?;
    let mut salt = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut salt);
    let mut out = BACKUP_MAGIC.to_vec();
    out.extend_from_slice(&salt);
    out.extend_from_slice(&seal(&passphrase_key(passphrase, &salt)?, &json)?);
    Ok(out)
}

pub fn import_backup(blob: &[u8], passphrase: &str) -> Res<VaultData> {
    if blob.len() < 20 || &blob[..4] != BACKUP_MAGIC {
        return Err("not a K-Shell backup file".into());
    }
    let key = passphrase_key(passphrase, &blob[4..20])?;
    let json = open(&key, &blob[20..]).map_err(|_| "wrong passphrase or corrupted backup".to_string())?;
    serde_json::from_slice(&json).s()
}

impl VaultData {
    /// Merge another vault into this one. Items with the same id are replaced;
    /// settings are kept as they are.
    pub fn merge(&mut self, other: VaultData) {
        for h in other.hosts {
            upsert(&mut self.hosts, h);
        }
        for k in other.keys {
            upsert(&mut self.keys, k);
        }
        for s in other.snippets {
            upsert(&mut self.snippets, s);
        }
        for f in other.forwards {
            upsert(&mut self.forwards, f);
        }
        for w in other.workspaces {
            upsert(&mut self.workspaces, w);
        }
        for kh in other.known_hosts {
            if !self.known_hosts.iter().any(|x| x.host == kh.host && x.key_type == kh.key_type) {
                self.known_hosts.push(kh);
            }
        }
    }
}

/// Copy vault files from the pre-rename data directory (`…/dev.keselyoleren.termx`)
/// into `dir` the first time K-Shell starts. The old directory is left untouched.
pub fn migrate_legacy_dir(dir: &Path) {
    let Some(old) = dir.parent().map(|p| p.join("dev.keselyoleren.termx")) else { return };
    if !old.is_dir() || dir.join("vault.key").exists() || dir.join("vault.bin").exists() {
        return;
    }
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    for f in ["vault.key", "vault.bin", "vault-dev.bin"] {
        let src = old.join(f);
        if src.exists() {
            let _ = std::fs::copy(&src, dir.join(f));
        }
    }
}

impl Vault {
    /// Open the vault in `dir`. With the app lock on it starts locked: nothing is decrypted
    /// until [`Vault::unlock`].
    pub fn load(dir: PathBuf) -> Res<Self> {
        std::fs::create_dir_all(&dir).s()?;
        let path = dir.join(if cfg!(debug_assertions) { "vault-dev.bin" } else { "vault.bin" });
        let v = Self { dir, path, key: RwLock::new(None), data: RwLock::new(VaultData::default()) };
        if !v.lock_enabled() {
            let key = load_or_create_key(&v.dir)?;
            v.open_with(key)?;
        }
        Ok(v)
    }

    /// Decrypt the vault file with `key` and make it the unlocked vault.
    fn open_with(&self, key: [u8; 32]) -> Res<()> {
        let data = match std::fs::read(&self.path) {
            Ok(blob) => serde_json::from_slice(&open(&key, &blob)?).s()?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                VaultData { version: 1, ..Default::default() }
            }
            Err(e) => return Err(format!("{}: {e}", self.path.display())),
        };
        let mut k = self.key.write();
        *self.data.write() = data;
        *k = Some(key);
        Ok(())
    }

    pub fn save(&self) -> Res<()> {
        // The key guard is held until the file is written, so locking (which takes it for
        // writing before clearing the data) can never let an emptied vault be saved.
        let key = self.key.read();
        let key = key.as_ref().ok_or(LOCKED)?;
        let json = serde_json::to_vec(&*self.data.read()).s()?;
        let blob = seal(key, &json)?;
        let tmp = self.path.with_extension("tmp");
        std::fs::write(&tmp, blob).s()?;
        std::fs::rename(&tmp, &self.path).s()?;
        Ok(())
    }

    // ---------- app lock ----------

    fn lock_path(&self) -> PathBuf {
        self.path.with_extension("lock")
    }

    pub fn lock_enabled(&self) -> bool {
        self.lock_path().exists()
    }

    pub fn is_locked(&self) -> bool {
        self.key.read().is_none()
    }

    pub fn ensure_unlocked(&self) -> Res<()> {
        if self.is_locked() {
            return Err(LOCKED.into());
        }
        Ok(())
    }

    fn current_key(&self) -> Res<[u8; 32]> {
        (*self.key.read()).ok_or_else(|| LOCKED.to_string())
    }

    fn read_lock_file(&self) -> Res<LockFile> {
        let p = self.lock_path();
        serde_json::from_slice(&std::fs::read(&p).ctx(&p.display().to_string())?).s()
    }

    fn write_lock_file(&self, lf: &LockFile) -> Res<()> {
        let tmp = self.path.with_extension("lock.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(lf).s()?).s()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
        }
        std::fs::rename(&tmp, self.lock_path()).s()
    }

    pub fn lock_status(&self) -> LockStatus {
        let touch_id = self.read_lock_file().map(|l| l.touch_id).unwrap_or(false);
        LockStatus { enabled: self.lock_enabled(), locked: self.is_locked(), touch_id }
    }

    /// Turn the app lock on: wrap the key with `password` and remove every stored copy of it.
    pub fn enable_lock(&self, password: &str) -> Res<()> {
        if self.lock_enabled() {
            return Err("The app lock is already on".into());
        }
        let key = self.current_key()?;
        self.save()?;
        self.write_lock_file(&LockFile::new(&key, password)?)?;
        let check = self.read_lock_file().and_then(|l| l.unwrap(password));
        if check.ok() != Some(key) {
            let _ = std::fs::remove_file(self.lock_path());
            return Err("Could not write the lock file".into());
        }
        if let Err(e) = slot_delete(&self.dir, &MAIN_KEY) {
            // Leave things as they were: key stored, lock off.
            let _ = slot_set(&self.dir, &MAIN_KEY, &key);
            let _ = std::fs::remove_file(self.lock_path());
            return Err(format!("Could not remove the vault key from the keychain: {e}"));
        }
        Ok(())
    }

    /// Turn the app lock off: store the key again (keychain or key file) and drop the lock file.
    pub fn disable_lock(&self, password: &str) -> Res<()> {
        let key = self.read_lock_file()?.unwrap(password)?;
        if Some(key) != *self.key.read() {
            return Err(LOCKED.into());
        }
        slot_set(&self.dir, &MAIN_KEY, &key)?;
        if slot_get(&self.dir, &MAIN_KEY) != Some(key) {
            return Err("Could not store the vault key".into());
        }
        let _ = slot_delete(&self.dir, &TOUCH_ID_KEY);
        std::fs::remove_file(self.lock_path()).s()
    }

    pub fn change_password(&self, old: &str, new: &str) -> Res<()> {
        let lf = self.read_lock_file()?;
        let key = lf.unwrap(old)?;
        let mut next = LockFile::new(&key, new)?;
        next.touch_id = lf.touch_id;
        self.write_lock_file(&next)
    }

    /// Keep (or drop) the copy of the key that Touch ID unlocks.
    pub fn set_touch_id(&self, on: bool) -> Res<()> {
        let key = self.current_key()?;
        let mut lf = self.read_lock_file()?;
        if on {
            slot_set(&self.dir, &TOUCH_ID_KEY, &key)?;
        } else {
            slot_delete(&self.dir, &TOUCH_ID_KEY)?;
        }
        lf.touch_id = on;
        self.write_lock_file(&lf)
    }

    /// Engage the lock: forget the key and every decrypted item.
    pub fn lock(&self) -> Res<()> {
        if !self.lock_enabled() {
            return Err("Turn on the app lock in Settings first".into());
        }
        let mut k = self.key.write();
        if let Some(key) = k.as_mut() {
            key.fill(0);
        }
        *k = None;
        *self.data.write() = VaultData::default();
        Ok(())
    }

    pub fn unlock(&self, password: &str) -> Res<()> {
        let key = self.read_lock_file()?.unwrap(password)?;
        if self.is_locked() {
            self.open_with(key)?;
        }
        Ok(())
    }

    /// Unlock with the key copy kept for Touch ID; call only after the OS prompt succeeded.
    pub fn unlock_touch_id(&self) -> Res<()> {
        if !self.read_lock_file()?.touch_id {
            return Err("Touch ID is not set up".into());
        }
        let key =
            slot_get(&self.dir, &TOUCH_ID_KEY).ok_or("Touch ID key not found: unlock with your password")?;
        if self.is_locked() {
            self.open_with(key)?;
        }
        Ok(())
    }

    /// Forgotten master password: start a new vault from an encrypted backup. The locked
    /// vault and its lock file are kept next to it, renamed, never deleted.
    pub fn reset_from_backup(&self, blob: &[u8], passphrase: &str) -> Res<()> {
        if !self.is_locked() {
            return Err("K-Shell is not locked".into());
        }
        let mut data = import_backup(blob, passphrase)?;
        data.version = data.version.max(1);
        let mut key = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut key);
        slot_set(&self.dir, &MAIN_KEY, &key)?;
        let stem = self.path.file_stem().and_then(|s| s.to_str()).unwrap_or("vault").to_string();
        let ts = chrono::Local::now().format("%Y%m%d-%H%M%S");
        if self.path.exists() {
            std::fs::rename(&self.path, self.dir.join(format!("{stem}-locked-{ts}.bin"))).s()?;
        }
        std::fs::rename(self.lock_path(), self.dir.join(format!("{stem}-locked-{ts}.lock"))).s()?;
        let _ = slot_delete(&self.dir, &TOUCH_ID_KEY);
        {
            let mut k = self.key.write();
            *self.data.write() = data;
            *k = Some(key);
        }
        self.save()
    }

    pub fn host(&self, id: &str) -> Option<Host> {
        self.data.read().hosts.iter().find(|h| h.id == id).cloned()
    }

    pub fn key(&self, id: &str) -> Option<Key> {
        self.data.read().keys.iter().find(|k| k.id == id).cloned()
    }

    /// Record that a host was just used (drives the "Recent" list).
    pub fn touch_host(&self, id: &str) {
        let found = {
            let mut d = self.data.write();
            d.hosts.iter_mut().find(|h| h.id == id).map(|h| h.last_used = now()).is_some()
        };
        if found {
            let _ = self.save();
        }
    }
}

/// Items stored in the vault lists with a string id.
pub trait HasId {
    fn id_mut(&mut self) -> &mut String;
    fn id_ref(&self) -> &str;
}
macro_rules! has_id { ($($t:ty),*) => { $(impl HasId for $t {
    fn id_mut(&mut self) -> &mut String { &mut self.id }
    fn id_ref(&self) -> &str { &self.id }
})* } }
has_id!(Host, Key, Snippet, Forward, Workspace, Project);

/// Upsert: assign an id when missing, replace when present.
pub fn upsert<T: HasId + Clone>(list: &mut Vec<T>, mut item: T) -> T {
    if item.id_mut().is_empty() {
        *item.id_mut() = new_id();
    }
    let id = item.id_mut().clone();
    match list.iter_mut().find(|x| x.id_ref() == id) {
        Some(slot) => *slot = item.clone(),
        None => list.push(item.clone()),
    }
    item
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seal_roundtrip_and_tamper_detection() {
        let key = [42u8; 32];
        let mut blob = seal(&key, b"hello").unwrap();
        assert_eq!(open(&key, &blob).unwrap(), b"hello");
        assert!(open(&[0u8; 32], &blob).is_err(), "wrong key must fail");
        *blob.last_mut().unwrap() ^= 1;
        assert!(open(&key, &blob).is_err(), "tampered data must fail");
    }

    #[test]
    fn history_dedupes_and_caps() {
        let mut d = VaultData::default();
        d.add_history("ls", "local");
        d.add_history("  pwd ", "local");
        d.add_history("ls", "local");
        d.add_history("ls", "srv");
        d.add_history("   ", "local");
        let got: Vec<_> = d.history.iter().map(|h| (h.cmd.as_str(), h.target.as_str())).collect();
        assert_eq!(got, [("pwd", "local"), ("ls", "local"), ("ls", "srv")]);
        for i in 0..HISTORY_MAX + 10 {
            d.add_history(&format!("echo {i}"), "local");
        }
        assert_eq!(d.history.len(), HISTORY_MAX);
        assert_eq!(d.history.last().unwrap().cmd, format!("echo {}", HISTORY_MAX + 9));
    }

    #[test]
    fn backup_roundtrip() {
        let mut d = VaultData::default();
        d.hosts.push(Host {
            id: "a".into(),
            label: "web".into(),
            password: "pw".into(),
            ..Default::default()
        });
        let blob = export_backup(&d, "correct horse").unwrap();
        assert!(!String::from_utf8_lossy(&blob).contains("web"));
        assert_eq!(import_backup(&blob, "correct horse").unwrap().hosts[0].password, "pw");
        assert!(import_backup(&blob, "wrong").is_err());
    }

    #[test]
    fn upsert_inserts_then_replaces() {
        let mut list: Vec<Snippet> = Vec::new();
        let s = upsert(&mut list, Snippet { name: "a".into(), ..Default::default() });
        assert!(!s.id.is_empty());
        upsert(&mut list, Snippet { name: "b".into(), ..s.clone() });
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "b");
    }

    #[test]
    fn vault_persists_encrypted() {
        let dir = std::env::temp_dir().join(format!("kshell-test-{}", new_id()));
        let v = Vault::load(dir.clone()).unwrap();
        v.data.write().hosts.push(Host {
            id: "x".into(),
            password: "topsecret".into(),
            ..Default::default()
        });
        v.save().unwrap();
        let raw = std::fs::read(&v.path).unwrap();
        assert!(!String::from_utf8_lossy(&raw).contains("topsecret"));
        assert_eq!(Vault::load(dir.clone()).unwrap().data.read().hosts.len(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    fn temp_vault() -> (PathBuf, Vault) {
        let dir = std::env::temp_dir().join(format!("kshell-test-{}", new_id()));
        let v = Vault::load(dir.clone()).unwrap();
        v.data.write().hosts.push(Host {
            id: "h".into(),
            password: "topsecret".into(),
            ..Default::default()
        });
        v.save().unwrap();
        (dir, v)
    }

    #[test]
    fn app_lock_wraps_key_and_locks() {
        let (dir, v) = temp_vault();
        assert!(v.lock().is_err(), "cannot lock before the lock is on");
        assert!(v.enable_lock("").is_err());
        v.enable_lock("hunter2").unwrap();
        // No stored key any more: the vault can only be opened with the password.
        assert!(slot_get(&dir, &MAIN_KEY).is_none());
        let lock_file = std::fs::read_to_string(v.lock_path()).unwrap();
        assert!(!lock_file.contains("topsecret"));

        v.lock().unwrap();
        assert!(v.is_locked());
        assert!(v.data.read().hosts.is_empty(), "locking drops decrypted data");
        assert_eq!(v.save().unwrap_err(), LOCKED, "a locked vault is never written");
        assert!(v.host("h").is_none());

        // A fresh start is locked too.
        let again = Vault::load(dir.clone()).unwrap();
        assert!(again.is_locked() && again.lock_status().enabled);
        assert_eq!(again.unlock("nope").unwrap_err(), "Wrong password");
        again.unlock("hunter2").unwrap();
        assert_eq!(again.host("h").unwrap().password, "topsecret");

        again.change_password("hunter2", "correct horse").unwrap();
        assert!(again.disable_lock("hunter2").is_err());
        again.disable_lock("correct horse").unwrap();
        assert!(!again.lock_enabled());
        let reopened = Vault::load(dir.clone()).unwrap();
        assert!(!reopened.is_locked());
        assert_eq!(reopened.data.read().hosts.len(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn touch_id_copy_follows_the_setting() {
        let (dir, v) = temp_vault();
        v.enable_lock("pw").unwrap();
        v.set_touch_id(true).unwrap();
        assert!(v.lock_status().touch_id);
        v.lock().unwrap();
        v.unlock_touch_id().unwrap();
        assert_eq!(v.data.read().hosts.len(), 1);
        v.set_touch_id(false).unwrap();
        v.lock().unwrap();
        assert!(v.unlock_touch_id().is_err());
        assert!(slot_get(&dir, &TOUCH_ID_KEY).is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn forgotten_password_restores_from_backup_and_keeps_old_vault() {
        let (dir, v) = temp_vault();
        let backup = export_backup(&v.data.read(), "backup pass").unwrap();
        v.enable_lock("forgotten").unwrap();
        v.lock().unwrap();
        assert!(v.reset_from_backup(&backup, "wrong").is_err());
        assert!(v.lock_enabled(), "a failed restore changes nothing");
        v.reset_from_backup(&backup, "backup pass").unwrap();
        assert!(!v.is_locked() && !v.lock_enabled());
        assert_eq!(v.host("h").unwrap().password, "topsecret");
        let kept: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok()?.file_name().into_string().ok())
            .filter(|n| n.contains("-locked-"))
            .collect();
        assert_eq!(kept.len(), 2, "old vault and lock file are kept: {kept:?}");
        assert_eq!(Vault::load(dir.clone()).unwrap().data.read().hosts.len(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }
}

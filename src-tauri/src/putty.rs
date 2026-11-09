//! Import PuTTY saved sessions, from the Windows registry or from a `.reg` export of
//! `HKEY_CURRENT_USER\Software\SimonTatham\PuTTY\Sessions` (so they can be moved to another
//! computer too).
//!
//! Kept: host, port, user, private key file (`.ppk`, added to the keychain), agent
//! forwarding, and an SSH proxy (PuTTY 0.77+) as the jump host. PuTTY stores no passwords.
//! Sessions for other protocols and other proxy types are reported, not imported.

use std::collections::HashMap;

use crate::vault::{new_id, now, Host, Key, VaultData};

/// Where PuTTY keeps its sessions, under HKEY_CURRENT_USER.
pub const REGISTRY_PATH: &str = r"Software\SimonTatham\PuTTY\Sessions";
/// K-Shell group the imported hosts land in.
const GROUP: &str = "PuTTY";

/// One saved session's values: strings and DWORDs by lowercase value name.
#[derive(Default, Debug)]
pub struct Values {
    pub strings: HashMap<String, String>,
    pub dwords: HashMap<String, u32>,
}

impl Values {
    fn s(&self, k: &str) -> &str {
        self.strings.get(&k.to_lowercase()).map(|s| s.trim()).unwrap_or("")
    }
    fn d(&self, k: &str) -> Option<u32> {
        self.dwords.get(&k.to_lowercase()).copied()
    }
}

#[derive(Default, Debug, Clone, PartialEq)]
pub struct Session {
    pub name: String,
    pub protocol: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub key_file: String,
    pub agent_forward: bool,
    /// PuTTY's ProxyMethod: 0 none, 1 SOCKS 4, 2 SOCKS 5, 3 HTTP, 4 Telnet, 5 local command, 6 SSH.
    pub proxy_method: u32,
    pub proxy_host: String,
    pub proxy_port: u16,
}

/// Session names are stored %XX-escaped (`My%20Server`).
fn unescape_name(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let hex = |c: u8| (c as char).to_digit(16);
            if let (Some(hi), Some(lo)) = (hex(b[i + 1]), hex(b[i + 2])) {
                out.push((hi * 16 + lo) as u8);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A session from its (escaped) registry key name and values; None for PuTTY's
/// "Default Settings" and sessions without a host.
pub fn session(key_name: &str, v: &Values) -> Option<Session> {
    let name = unescape_name(key_name);
    if name == "Default Settings" {
        return None;
    }
    let mut host = v.s("HostName").to_string();
    let mut user = v.s("UserName").to_string();
    // "user@host" in the host field is how many people type it into PuTTY.
    if let Some((u, h)) = host.rsplit_once('@') {
        if user.is_empty() {
            user = u.to_string();
        }
        host = h.to_string();
    }
    if host.is_empty() {
        return None;
    }
    let protocol = match v.s("Protocol") {
        "" => "ssh".to_string(),
        p => p.to_lowercase(),
    };
    Some(Session {
        name,
        protocol,
        host,
        port: v.d("PortNumber").and_then(|p| u16::try_from(p).ok()).unwrap_or(0),
        user,
        key_file: v.s("PublicKeyFile").to_string(),
        agent_forward: v.d("AgentFwd").unwrap_or(0) != 0,
        proxy_method: v.d("ProxyMethod").unwrap_or(0),
        proxy_host: v.s("ProxyHost").to_string(),
        proxy_port: v.d("ProxyPort").and_then(|p| u16::try_from(p).ok()).unwrap_or(0),
    })
}

/// Text of a `.reg` file: regedit writes UTF-16LE with a byte-order mark, `reg export` too;
/// UTF-8 is accepted as well.
pub fn reg_text(bytes: &[u8]) -> String {
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        let units: Vec<u16> = rest.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        return String::from_utf16_lossy(&units);
    }
    let b = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    String::from_utf8_lossy(b).into_owned()
}

/// A quoted .reg string: `"…"` with `\\` and `\"` escapes. Returns the value and what follows.
fn reg_string(s: &str) -> Option<(String, &str)> {
    let s = s.strip_prefix('"')?;
    let mut out = String::new();
    let mut chars = s.char_indices();
    while let Some((i, c)) = chars.next() {
        match c {
            '\\' => {
                if let Some((_, n)) = chars.next() {
                    out.push(n);
                }
            }
            '"' => return Some((out, &s[i + 1..])),
            _ => out.push(c),
        }
    }
    None
}

/// Sessions in a .reg export (other keys in the file are ignored).
pub fn parse_reg(text: &str) -> Vec<Session> {
    let marker = r"\software\simontatham\putty\sessions\";
    let mut out = Vec::new();
    let mut current: Option<(String, Values)> = None;
    let mut continued = false;
    let flush = |cur: &mut Option<(String, Values)>, out: &mut Vec<Session>| {
        if let Some((name, v)) = cur.take() {
            out.extend(session(&name, &v));
        }
    };
    for raw in text.lines() {
        let line = raw.trim();
        if continued {
            continued = line.ends_with('\\'); // rest of a multi-line hex value
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            flush(&mut current, &mut out);
            let key = &line[1..line.len() - 1];
            let lower = key.to_lowercase();
            if let Some(i) = lower.find(marker) {
                let name = &key[i + marker.len()..];
                if !name.is_empty() && !name.contains('\\') && !key.starts_with('-') {
                    current = Some((name.to_string(), Values::default()));
                }
            }
            continue;
        }
        let Some((_, values)) = current.as_mut() else { continue };
        let Some((name, rest)) = reg_string(line) else { continue };
        let Some(rest) = rest.trim_start().strip_prefix('=') else { continue };
        let rest = rest.trim();
        if let Some((value, _)) = reg_string(rest) {
            values.strings.insert(name.to_lowercase(), value);
        } else if let Some(hex) = rest.strip_prefix("dword:") {
            if let Ok(n) = u32::from_str_radix(hex.trim(), 16) {
                values.dwords.insert(name.to_lowercase(), n);
            }
        } else if rest.ends_with('\\') {
            continued = true;
        }
    }
    flush(&mut current, &mut out);
    out
}

/// Sessions saved by PuTTY on this computer.
#[cfg(windows)]
pub fn read_registry() -> Result<Vec<Session>, String> {
    read_registry_at(REGISTRY_PATH)
}

/// Sessions under `path` in HKEY_CURRENT_USER (a parameter so tests can use their own key).
#[cfg(windows)]
pub fn read_registry_at(path: &str) -> Result<Vec<Session>, String> {
    use winreg::enums::{HKEY_CURRENT_USER, REG_DWORD, REG_EXPAND_SZ, REG_SZ};
    use winreg::RegKey;
    let root = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(path)
        .map_err(|_| "No PuTTY sessions found on this computer".to_string())?;
    let mut out = Vec::new();
    for name in root.enum_keys().flatten() {
        let Ok(key) = root.open_subkey(&name) else { continue };
        let mut v = Values::default();
        for (value_name, value) in key.enum_values().flatten() {
            let n = value_name.to_lowercase();
            match value.vtype {
                REG_SZ | REG_EXPAND_SZ => {
                    let units: Vec<u16> =
                        value.bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
                    let s = String::from_utf16_lossy(&units);
                    v.strings.insert(n, s.trim_end_matches('\0').to_string());
                }
                REG_DWORD if value.bytes.len() >= 4 => {
                    let b = &value.bytes;
                    v.dwords.insert(n, u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
                }
                _ => {}
            }
        }
        out.extend(session(&name, &v));
    }
    Ok(out)
}

/// What an import did, for the message shown afterwards.
#[derive(Default, Debug, serde::Serialize)]
pub struct Report {
    pub imported: usize,
    /// Sessions already in the vault (same label in the PuTTY group).
    pub existing: usize,
    /// "name: reason" for sessions that were not imported.
    pub skipped: Vec<String>,
    /// "name: note" for sessions imported without part of their settings.
    pub notes: Vec<String>,
}

/// Add `sessions` to the vault as hosts in the "PuTTY" group.
pub fn import(data: &mut VaultData, sessions: Vec<Session>) -> Report {
    let mut r = Report::default();
    let mut key_by_path: HashMap<String, String> = HashMap::new();
    let mut pending_jump: Vec<(String, Session)> = Vec::new();
    for s in sessions {
        if s.protocol != "ssh" {
            r.skipped.push(format!("{}: {} sessions aren't supported", s.name, s.protocol));
            continue;
        }
        if data.hosts.iter().any(|h| h.group == GROUP && h.label == s.name) {
            r.existing += 1;
            continue;
        }
        let mut key_id = String::new();
        if !s.key_file.is_empty() {
            match key_by_path.get(&s.key_file) {
                Some(id) => key_id = id.clone(),
                None => match std::fs::read_to_string(&s.key_file) {
                    Ok(text) => {
                        let k = key_from_file(&s.key_file, &text);
                        key_by_path.insert(s.key_file.clone(), k.id.clone());
                        key_id = k.id.clone();
                        data.keys.push(k);
                    }
                    Err(e) => r.notes.push(format!("{}: key file {} not read ({e})", s.name, s.key_file)),
                },
            }
        }
        let h = Host {
            id: new_id(),
            label: s.name.clone(),
            group: GROUP.into(),
            protocol: "ssh".into(),
            host: s.host.clone(),
            port: if s.port == 0 { 22 } else { s.port },
            username: s.user.clone(),
            key_id,
            agent_forward: s.agent_forward,
            created_at: now(),
            ..Default::default()
        };
        match s.proxy_method {
            0 => {}
            6 if !s.proxy_host.is_empty() => pending_jump.push((h.id.clone(), s.clone())),
            m => r.notes.push(format!(
                "{}: {} proxy not imported (K-Shell connects through SSH jump hosts only)",
                s.name,
                match m {
                    1 => "SOCKS 4",
                    2 => "SOCKS 5",
                    3 => "HTTP",
                    4 => "Telnet",
                    5 => "local command",
                    _ => "unknown",
                }
            )),
        }
        data.hosts.push(h);
        r.imported += 1;
    }
    // SSH proxies become jump hosts when a host with that address is in the vault.
    for (hid, s) in pending_jump {
        let port = if s.proxy_port == 0 { 22 } else { s.proxy_port };
        let jump = data
            .hosts
            .iter()
            .find(|j| j.id != hid && j.protocol == "ssh" && j.host == s.proxy_host && j.addr_port() == port)
            .map(|j| j.id.clone());
        match jump {
            Some(jid) => {
                if let Some(h) = data.hosts.iter_mut().find(|h| h.id == hid) {
                    h.jump_host = jid;
                }
            }
            None => r.notes.push(format!(
                "{}: jump host {}:{port} isn't a saved host, so it wasn't set",
                s.name, s.proxy_host
            )),
        }
    }
    r
}

/// A keychain entry for a `.ppk` (or OpenSSH) key file; an encrypted one is kept as is and
/// asks for its passphrase when used, like keys imported from ~/.ssh/config.
fn key_from_file(path: &str, text: &str) -> Key {
    let mut k = crate::keys::import(path, text, "").unwrap_or_else(|_| Key {
        id: new_id(),
        kind: "encrypted".into(),
        private_key: text.to_string(),
        created_at: now(),
        ..Default::default()
    });
    k.name = std::path::Path::new(&path.replace('\\', "/"))
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string());
    k
}

#[cfg(test)]
mod tests {
    use super::*;

    const REG: &str = r#"Windows Registry Editor Version 5.00

[HKEY_CURRENT_USER\Software\SimonTatham\PuTTY\Sessions]

[HKEY_CURRENT_USER\Software\SimonTatham\PuTTY\Sessions\Default%20Settings]
"HostName"=""
"PortNumber"=dword:00000016

[HKEY_CURRENT_USER\Software\SimonTatham\PuTTY\Sessions\Web%20Server]
"Present"=dword:00000001
"HostName"="deploy@10.0.0.5"
"PortNumber"=dword:00000898
"Protocol"="ssh"
"PublicKeyFile"="C:\\Users\\me\\keys\\web.ppk"
"AgentFwd"=dword:00000001
"Colour0"="187,187,187"
"WinTitle"="say \"hi\""
"Wordness0"=hex:00,00,00,00,00,00,00,00,00,00,00,00,00,00,00,00,00,00,00,00,00,\
  00,00,00,00,00

[HKEY_CURRENT_USER\Software\SimonTatham\PuTTY\Sessions\bastion]
"HostName"="bastion.example.com"
"UserName"="ops"
"PortNumber"=dword:00000016
"Protocol"="ssh"

[HKEY_CURRENT_USER\Software\SimonTatham\PuTTY\Sessions\Inner]
"HostName"="10.1.0.9"
"PortNumber"=dword:00000016
"ProxyMethod"=dword:00000006
"ProxyHost"="bastion.example.com"
"ProxyPort"=dword:00000016

[HKEY_CURRENT_USER\Software\SimonTatham\PuTTY\Sessions\Behind%20SOCKS]
"HostName"="10.2.0.1"
"ProxyMethod"=dword:00000002
"ProxyHost"="proxy.local"

[HKEY_CURRENT_USER\Software\SimonTatham\PuTTY\Sessions\Switch]
"HostName"="192.168.1.2"
"Protocol"="telnet"
"PortNumber"=dword:00000017

[HKEY_CURRENT_USER\Software\SimonTatham\PuTTY\SshHostKeys]
"ssh-ed25519@22:10.0.0.5"="0x1234"
"#;

    #[test]
    fn parses_sessions_from_a_reg_export() {
        let s = parse_reg(REG);
        let names: Vec<&str> = s.iter().map(|x| x.name.as_str()).collect();
        assert_eq!(names, ["Web Server", "bastion", "Inner", "Behind SOCKS", "Switch"]);
        let web = &s[0];
        assert_eq!((web.host.as_str(), web.user.as_str(), web.port), ("10.0.0.5", "deploy", 2200));
        assert_eq!(web.key_file, r"C:\Users\me\keys\web.ppk");
        assert!(web.agent_forward);
        assert_eq!(s[1].user, "ops");
        assert_eq!(
            (s[2].proxy_method, s[2].proxy_host.as_str(), s[2].proxy_port),
            (6, "bastion.example.com", 22)
        );
        assert_eq!(s[4].protocol, "telnet");
    }

    #[test]
    fn reads_utf16_reg_files() {
        let mut bytes = vec![0xFF, 0xFE];
        for u in REG.encode_utf16() {
            bytes.extend_from_slice(&u.to_le_bytes());
        }
        assert_eq!(parse_reg(&reg_text(&bytes)).len(), 5);
        assert_eq!(parse_reg(&reg_text(REG.as_bytes())).len(), 5);
    }

    #[test]
    fn unescapes_session_names() {
        assert_eq!(unescape_name("My%20Server%25"), "My Server%");
        assert_eq!(unescape_name("caf%C3%A9"), "café");
        assert_eq!(unescape_name("50%"), "50%");
        assert_eq!(unescape_name("a%zzb"), "a%zzb");
    }

    #[test]
    fn imports_into_the_putty_group() {
        let mut data = VaultData::default();
        let r = import(&mut data, parse_reg(REG));
        assert_eq!(r.imported, 4);
        assert_eq!(r.skipped, ["Switch: telnet sessions aren't supported"]);
        assert!(
            r.notes.iter().any(|n| n.starts_with("Behind SOCKS: SOCKS 5 proxy not imported")),
            "{:?}",
            r.notes
        );
        assert!(r.notes.iter().any(|n| n.starts_with("Web Server: key file")), "{:?}", r.notes);
        let by = |l: &str| data.hosts.iter().find(|h| h.label == l).unwrap().clone();
        let web = by("Web Server");
        assert_eq!(
            (web.group.as_str(), web.host.as_str(), web.port, web.username.as_str()),
            ("PuTTY", "10.0.0.5", 2200, "deploy")
        );
        assert!(web.agent_forward && web.password.is_empty());
        assert_eq!(by("Inner").jump_host, by("bastion").id);

        // importing again adds nothing
        let r = import(&mut data, parse_reg(REG));
        assert_eq!((r.imported, r.existing), (0, 4));
        assert_eq!(data.hosts.len(), 4);
    }

    #[test]
    fn ppk_key_files_go_to_the_keychain() {
        let dir = std::env::temp_dir().join(format!("kshell-putty-{}", new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        // made with `puttygen -t ed25519` (PuTTY 0.78), a throwaway key used only here
        let ppk = include_str!("../tests/fixtures/test-key-ed25519.ppk");
        let path = dir.join("srv.ppk");
        std::fs::write(&path, ppk).unwrap();
        let s = Session {
            name: "srv".into(),
            protocol: "ssh".into(),
            host: "h".into(),
            key_file: path.to_string_lossy().into_owned(),
            ..Default::default()
        };
        let mut data = VaultData::default();
        let r = import(&mut data, vec![s.clone(), Session { name: "srv2".into(), ..s }]);
        assert_eq!(r.imported, 2);
        assert!(r.notes.is_empty(), "{:?}", r.notes);
        assert_eq!(data.keys.len(), 1, "one key for both sessions");
        assert_eq!(data.keys[0].name, "srv.ppk");
        assert_ne!(data.keys[0].kind, "encrypted");
        assert!(data.hosts.iter().all(|h| h.key_id == data.keys[0].id));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(windows)]
    #[test]
    fn reads_sessions_from_the_registry() {
        use winreg::enums::HKEY_CURRENT_USER;
        use winreg::RegKey;
        let base = format!(r"Software\KShellTests\{}", new_id());
        let path = format!(r"{base}\Sessions");
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let (k, _) = hkcu.create_subkey(format!(r"{path}\Lab%20Box")).unwrap();
        k.set_value("HostName", &"lab.example.com").unwrap();
        k.set_value("UserName", &"admin").unwrap();
        k.set_value("PortNumber", &2222u32).unwrap();
        k.set_value("Protocol", &"ssh").unwrap();
        let s = read_registry_at(&path);
        let _ = hkcu.delete_subkey_all(&base);
        let s = s.unwrap();
        assert_eq!(s.len(), 1);
        assert_eq!(
            (s[0].name.as_str(), s[0].host.as_str(), s[0].user.as_str(), s[0].port),
            ("Lab Box", "lab.example.com", "admin", 2222)
        );
    }
}

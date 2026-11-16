//! Shared parts of the host importers (PuTTY, MobaXterm, CSV): the report shown after an
//! import, and private key files added to the keychain.

use std::collections::HashMap;

use crate::vault::{new_id, now, Key, VaultData};

/// What an import did, for the message shown afterwards.
#[derive(Default, Debug, serde::Serialize)]
pub struct Report {
    pub imported: usize,
    /// Entries already in the vault.
    pub existing: usize,
    /// "name: reason" for entries that were not imported.
    pub skipped: Vec<String>,
    /// "name: note" for entries imported without part of their settings.
    pub notes: Vec<String>,
}

/// Private key files read during one import, so a file used by many hosts is added once.
#[derive(Default)]
pub struct KeyFiles {
    by_path: HashMap<String, String>,
}

impl KeyFiles {
    /// Keychain id for the key file at `path`, reading and adding it the first time.
    /// A file that can't be read is noted in the report under `name` and gives "".
    pub fn id_for(&mut self, data: &mut VaultData, path: &str, name: &str, r: &mut Report) -> String {
        if path.is_empty() {
            return String::new();
        }
        if let Some(id) = self.by_path.get(path) {
            return id.clone();
        }
        match std::fs::read_to_string(path) {
            Ok(text) => {
                let k = key_from_file(path, &text);
                let id = k.id.clone();
                self.by_path.insert(path.to_string(), id.clone());
                data.keys.push(k);
                id
            }
            Err(e) => {
                r.notes.push(format!("{name}: key file {path} not read ({e})"));
                String::new()
            }
        }
    }
}

/// A keychain entry for a `.ppk` (or OpenSSH) key file; an encrypted one is kept as is and
/// asks for its passphrase when used, like keys imported from ~/.ssh/config.
pub fn key_from_file(path: &str, text: &str) -> Key {
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

/// Text of an imported file: UTF-8 (with or without a byte-order mark), else Windows-1252,
/// which MobaXterm uses for its session files and Excel for "CSV" on many Windows computers.
pub fn decode(bytes: &[u8]) -> String {
    let b = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    match std::str::from_utf8(b) {
        Ok(s) => s.to_string(),
        Err(_) => b.iter().map(|&c| cp1252(c)).collect(),
    }
}

fn cp1252(c: u8) -> char {
    const HIGH: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8d}', 'Ž', '\u{8f}',
        '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9d}', 'ž', 'Ÿ',
    ];
    match c {
        0x80..=0x9F => HIGH[(c - 0x80) as usize],
        _ => c as char,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_windows_1252() {
        assert_eq!(decode(b"caf\xe9 \x80"), "café €");
        assert_eq!(decode("\u{feff}é".as_bytes()), "é");
    }
}

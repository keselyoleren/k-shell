//! Terminal recordings in the asciicast v2 format (`.cast`), the format `asciinema play`,
//! asciinema.org and agg read.
//!
//! The UI captures what a terminal pane shows (output after K-Shell's own hidden prompt hook
//! is taken out, plus resizes) and sends it here in batches. Each event becomes one JSON line:
//! `[seconds, "o", text]` or `[seconds, "r", "COLSxROWS"]`. Typed input is never recorded,
//! so passwords typed at a prompt don't end up in the file.

use parking_lot::Mutex;
use serde::Deserialize;
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use crate::error::{Ctx, Res};

/// Largest recording the player opens.
pub const MAX_READ: u64 = 256 * 1024 * 1024;

/// One event from the UI: seconds since the start, `o` (output) or `r` (resize), data.
#[derive(Deserialize, Debug, Clone, PartialEq)]
pub struct Event(pub f64, pub String, pub String);

pub struct Recording {
    out: BufWriter<File>,
    pub path: PathBuf,
}

impl Recording {
    /// Create `<dir>/<date> <label>.cast` and write the header.
    pub fn create(dir: &Path, label: &str, cols: u16, rows: u16) -> Res<Self> {
        std::fs::create_dir_all(dir).ctx(&dir.to_string_lossy())?;
        let now = chrono::Local::now();
        let safe: String = label
            .chars()
            .map(|c| if c.is_alphanumeric() || "-_.@".contains(c) { c } else { '_' })
            .take(60)
            .collect();
        let path = dir.join(format!("{} {}.cast", now.format("%Y-%m-%d %H-%M-%S"), safe));
        let file = File::create(&path).ctx(&path.to_string_lossy())?;
        let mut out = BufWriter::new(file);
        let header = serde_json::json!({
            "version": 2,
            "width": cols.max(1),
            "height": rows.max(1),
            "timestamp": now.timestamp(),
            "title": label,
            "env": { "TERM": "xterm-256color" },
        });
        writeln!(out, "{header}").ctx("write recording")?;
        Ok(Self { out, path })
    }

    pub fn write(&mut self, events: &[Event]) -> Res<()> {
        for Event(t, kind, data) in events {
            if !(kind == "o" || kind == "r") || !t.is_finite() || *t < 0.0 {
                continue;
            }
            // whole microseconds keep the file small and stable
            let t = (t * 1e6).round() / 1e6;
            let line = serde_json::to_string(&(t, kind, data)).ctx("write recording")?;
            writeln!(self.out, "{line}").ctx("write recording")?;
        }
        self.out.flush().ctx("write recording")
    }
}

/// Recordings in progress, by id.
#[derive(Default)]
pub struct Recordings {
    map: Mutex<HashMap<String, Recording>>,
}

impl Recordings {
    pub fn start(&self, id: String, rec: Recording) -> String {
        let path = rec.path.to_string_lossy().into_owned();
        self.map.lock().insert(id, rec);
        path
    }

    pub fn write(&self, id: &str, events: &[Event]) -> Res<()> {
        let mut m = self.map.lock();
        let rec = m.get_mut(id).ok_or("This recording was already stopped")?;
        rec.write(events)
    }

    /// Finish a recording and return its path.
    pub fn stop(&self, id: &str, events: &[Event]) -> Res<String> {
        let mut rec = self.map.lock().remove(id).ok_or("This recording was already stopped")?;
        rec.write(events)?;
        Ok(rec.path.to_string_lossy().into_owned())
    }
}

/// Read a recording for the player.
pub fn read(path: &str) -> Res<String> {
    let meta = std::fs::metadata(path).ctx(path)?;
    if meta.len() > MAX_READ {
        return Err(format!("This recording is too large to play here ({} MB)", meta.len() / (1024 * 1024)));
    }
    std::fs::read_to_string(path).ctx(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_valid_asciicast_v2() {
        let dir = std::env::temp_dir().join(format!("kshell-cast-test-{}", uuid::Uuid::new_v4()));
        let mut rec = Recording::create(&dir, "web 01/prod", 80, 24).unwrap();
        assert!(rec.path.file_name().unwrap().to_string_lossy().ends_with(" web_01_prod.cast"));
        rec.write(&[
            Event(0.1234567, "o".into(), "héllo \u{1b}[31m\"red\"\u{1b}[0m\r\n".into()),
            Event(1.5, "r".into(), "100x30".into()),
            Event(2.0, "i".into(), "secret".into()), // input is never written
            Event(f64::NAN, "o".into(), "x".into()),
        ])
        .unwrap();
        let path = rec.path.clone();
        drop(rec);
        let text = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<serde_json::Value> = text.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0]["version"], 2);
        assert_eq!(lines[0]["width"], 80);
        assert_eq!(lines[0]["height"], 24);
        assert_eq!(lines[1], serde_json::json!([0.123457, "o", "héllo \u{1b}[31m\"red\"\u{1b}[0m\r\n"]));
        assert_eq!(lines[2], serde_json::json!([1.5, "r", "100x30"]));
        assert!(!text.contains("secret"));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn stop_twice_is_an_error() {
        let dir = std::env::temp_dir().join(format!("kshell-cast-test-{}", uuid::Uuid::new_v4()));
        let recs = Recordings::default();
        recs.start("a".into(), Recording::create(&dir, "x", 10, 5).unwrap());
        assert!(recs.stop("a", &[]).is_ok());
        assert!(recs.stop("a", &[]).is_err());
        assert!(recs.write("a", &[]).is_err());
        std::fs::remove_dir_all(dir).ok();
    }
}

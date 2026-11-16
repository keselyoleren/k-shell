//! Interactive terminals and one-shot SSH commands.

use serde_json::{json, Value};
use tauri::ipc::{Channel, InvokeResponseBody};

use super::State;
use crate::containers::Shell;
use crate::error::Res;
use crate::terminal::{Cmd, LocalOptions, ShellOptions, TelnetOptions};
use crate::vault::Host;

/// Open an SSH shell. Output streams to `on_data` as raw bytes; a JSON
/// `{"type":"exit","code":…}` message is sent when the shell ends.
#[tauri::command]
pub async fn term_open_ssh(
    st: State<'_>,
    id: String,
    host: Host,
    cols: u16,
    rows: u16,
    container: Option<Shell>,
    on_data: Channel<InvokeResponseBody>,
) -> Res<()> {
    let label = if host.label.is_empty() { host.host.clone() } else { host.label.clone() };
    let command = container.map(|c| c.argv()).transpose()?.map(|a| crate::containers::sh_join(&a));
    let log = session_log(&st, &label);
    let conn = st.pool.get(&st.vault, &host).await?;
    st.terminals
        .open_ssh(
            id,
            conn,
            ShellOptions {
                cols,
                rows,
                startup_command: host.startup_command,
                agent_forward: host.agent_forward,
                command,
                log,
            },
            on_data,
        )
        .await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)] // flat IPC arguments
pub fn term_open_local(
    st: State,
    id: String,
    cols: u16,
    rows: u16,
    shell: Option<String>,
    cwd: Option<String>,
    startup_command: Option<String>,
    container: Option<Shell>,
    on_data: Channel<InvokeResponseBody>,
) -> Res<()> {
    let command = container.map(|c| c.argv()).transpose()?;
    let log = session_log(&st, "local");
    let opts = LocalOptions {
        cols,
        rows,
        shell,
        cwd,
        startup_command: startup_command.unwrap_or_default(),
        command,
        log,
    };
    st.terminals.open_local(id, opts, on_data)
}

/// Open a Telnet session to `host` (address and port from the host). Output and the exit
/// message stream to `on_data` like an SSH shell's.
#[tauri::command]
pub async fn term_open_telnet(
    st: State<'_>,
    id: String,
    host: Host,
    cols: u16,
    rows: u16,
    on_data: Channel<InvokeResponseBody>,
) -> Res<()> {
    let label = if host.label.is_empty() { host.host.clone() } else { host.label.clone() };
    let log = session_log(&st, &label);
    let port = host.addr_port();
    let opts = TelnetOptions { host: host.host.trim().to_string(), port, cols, rows, log };
    st.terminals.open_telnet(id, opts, on_data).await
}

/// Open the serial port of a serial-console host (`host.host` is the device).
#[tauri::command]
pub fn term_open_serial(st: State, id: String, host: Host, on_data: Channel<InvokeResponseBody>) -> Res<()> {
    let label = if host.label.is_empty() { host.host.clone() } else { host.label.clone() };
    let log = session_log(&st, &label);
    st.terminals.open_serial(id, host.serial(), log, on_data)
}

/// Serial ports on this computer: `[{ name, description }]`.
#[tauri::command]
pub fn serial_ports() -> Res<Vec<Value>> {
    let ports = serialport::available_ports().map_err(|e| format!("Couldn't list serial ports: {e}"))?;
    Ok(ports
        .into_iter()
        .map(|p| {
            let description = match p.port_type {
                serialport::SerialPortType::UsbPort(u) => {
                    [u.manufacturer, u.product].into_iter().flatten().collect::<Vec<_>>().join(" ")
                }
                serialport::SerialPortType::BluetoothPort => "Bluetooth".into(),
                serialport::SerialPortType::PciPort => "PCI".into(),
                serialport::SerialPortType::Unknown => String::new(),
            };
            json!({ "name": p.port_name, "description": description })
        })
        .collect())
}

/// Check that a Telnet server accepts connections.
#[tauri::command]
pub async fn telnet_test(host: Host) -> Res<String> {
    let port = host.addr_port();
    let started = std::time::Instant::now();
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        tokio::net::TcpStream::connect((host.host.trim(), port)),
    )
    .await
    .map_err(|_| format!("Timed out connecting to {}:{port}", host.host.trim()))?
    .map_err(|e| format!("Couldn't connect to {}:{port}: {e}", host.host.trim()))?;
    Ok(format!("Connected in {} ms", started.elapsed().as_millis()))
}

/// Check that a serial port can be opened with the host's settings.
#[tauri::command]
pub fn serial_test(host: Host) -> Res<String> {
    drop(host.serial().open()?);
    Ok(format!("{} opened", host.host.trim()))
}

/// A new session log when logging is on in the settings (a log that can't be created is
/// skipped rather than blocking the terminal).
fn session_log(st: &State, label: &str) -> Option<crate::sessionlog::SessionLog> {
    let dir = {
        let d = st.vault.data.read();
        if !d.settings.session_log {
            return None;
        }
        crate::sessionlog::dir_from_setting(&d.settings.log_dir)
    };
    crate::sessionlog::SessionLog::create(&dir, label).ok()
}

/// The folder session logs are written to.
#[tauri::command]
pub fn log_dir(st: State) -> String {
    crate::sessionlog::dir_from_setting(&st.vault.data.read().settings.log_dir).to_string_lossy().into_owned()
}

/// Open the session-log folder in the OS file manager (created if needed).
#[tauri::command]
pub fn log_dir_open(app: tauri::AppHandle, st: State) -> Res<()> {
    use tauri_plugin_opener::OpenerExt;
    let dir = crate::sessionlog::dir_from_setting(&st.vault.data.read().settings.log_dir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    app.opener().open_path(dir.to_string_lossy(), None::<&str>).map_err(|e| e.to_string())
}

/// Start recording a terminal pane to a `.cast` file in the session-log folder; returns its path.
#[tauri::command]
pub fn rec_start(st: State, id: String, label: String, cols: u16, rows: u16) -> Res<String> {
    let dir = crate::sessionlog::dir_from_setting(&st.vault.data.read().settings.log_dir);
    let rec = crate::cast::Recording::create(&dir, &label, cols, rows)?;
    Ok(st.recordings.start(id, rec))
}

/// Append a batch of `[seconds, "o" | "r", data]` events to a recording.
#[tauri::command]
pub fn rec_write(st: State, id: String, events: Vec<crate::cast::Event>) -> Res<()> {
    st.recordings.write(&id, &events)
}

/// Write the last events and close the recording; returns its path.
#[tauri::command]
pub fn rec_stop(st: State, id: String, events: Vec<crate::cast::Event>) -> Res<String> {
    st.recordings.stop(&id, &events)
}

/// Show a recording in the OS file manager.
#[tauri::command]
pub fn rec_reveal(app: tauri::AppHandle, path: String) -> Res<()> {
    use tauri_plugin_opener::OpenerExt;
    app.opener().reveal_item_in_dir(&path).map_err(|e| e.to_string())
}

/// Read a `.cast` file for the player.
#[tauri::command]
pub fn cast_read(path: String) -> Res<String> {
    crate::cast::read(&path)
}

#[tauri::command]
pub fn term_write(st: State, id: String, data: String) -> Res<()> {
    st.terminals.send(&id, Cmd::Write(data.into_bytes()))
}

#[tauri::command]
pub fn term_resize(st: State, id: String, cols: u16, rows: u16) -> Res<()> {
    st.terminals.send(&id, Cmd::Resize(cols, rows))
}

#[tauri::command]
pub fn term_close(st: State, id: String) {
    st.terminals.close(&id)
}

#[tauri::command]
pub async fn ssh_exec(st: State<'_>, host: Host, command: String) -> Res<Value> {
    let conn = st.pool.get(&st.vault, &host).await?;
    let (output, code) = conn.exec(&command).await?;
    Ok(json!({ "output": output, "code": code }))
}

/// Names in a folder, for completing paths in the command box; folders end in "/".
/// `host` None: this computer. A folder that can't be read gives an empty list.
#[tauri::command]
pub async fn path_list(st: State<'_>, host: Option<Host>, dir: String) -> Res<Vec<String>> {
    const MAX: usize = 5000;
    match host {
        Some(h) => {
            let conn = st.pool.get(&st.vault, &h).await?;
            // -p marks folders, -L follows links so a link to a folder counts as one
            let cmd = format!("LC_ALL=C ls -1ApL -- {} 2>/dev/null", crate::containers::sh_join(&[dir]));
            let (out, _) = conn.exec(&cmd).await?;
            Ok(out.lines().filter(|l| !l.is_empty()).take(MAX).map(String::from).collect())
        }
        None => tokio::task::spawn_blocking(move || {
            let mut names: Vec<String> = std::fs::read_dir(&dir)
                .map(|rd| {
                    rd.flatten()
                        .take(MAX)
                        .map(|e| {
                            let mut n = e.file_name().to_string_lossy().into_owned();
                            if e.path().is_dir() {
                                n.push('/');
                            }
                            n
                        })
                        .collect()
                })
                .unwrap_or_default();
            names.sort();
            names
        })
        .await
        .map_err(|e| e.to_string()),
    }
}

/// Local branches of the git repository at `dir`, most recently committed first.
/// `host` None: this computer. Not a repository (or no git) gives an empty list.
#[tauri::command]
pub async fn git_branches(st: State<'_>, host: Option<Host>, dir: String) -> Res<Vec<String>> {
    let argv: Vec<String> = [
        "git",
        "-C",
        &dir,
        "for-each-ref",
        "--sort=-committerdate",
        "--format=%(refname:short)",
        "refs/heads",
    ]
    .map(String::from)
    .to_vec();
    let out = match host {
        Some(h) => {
            let conn = st.pool.get(&st.vault, &h).await?;
            let cmd = format!("{} 2>/dev/null", crate::containers::sh_join(&argv));
            conn.exec(&cmd).await?.0
        }
        None => tokio::task::spawn_blocking(move || {
            std::process::Command::new(&argv[0])
                .args(&argv[1..])
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
                .unwrap_or_default()
        })
        .await
        .map_err(|e| e.to_string())?,
    };
    Ok(out.lines().map(str::trim).filter(|l| !l.is_empty()).take(1000).map(String::from).collect())
}

/// Output of a POSIX shell script run on `host` (None: this computer); stderr is not kept.
async fn run_sh(st: &State<'_>, host: Option<Host>, script: String) -> Res<String> {
    match host {
        Some(h) => {
            let conn = st.pool.get(&st.vault, &h).await?;
            Ok(conn.exec(&script).await?.0)
        }
        None => tokio::task::spawn_blocking(move || {
            std::process::Command::new("sh")
                .arg("-c")
                .arg(&script)
                .stderr(std::process::Stdio::null())
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
                .map_err(|e| e.to_string())
        })
        .await
        .map_err(|e| e.to_string())?,
    }
}

/// Shell lines that enter the repository at `dir` and set `$base`: HEAD, or the index in a
/// repository without commits yet. Outside a repository the script ends there.
fn git_prelude(dir: &str) -> String {
    format!(
        "cd -- {} 2>/dev/null || exit 0\n\
         git rev-parse --is-inside-work-tree >/dev/null 2>&1 || exit 0\n\
         g() {{ git -c core.quotePath=false \"$@\" 2>/dev/null; }}\n\
         if git rev-parse -q --verify HEAD >/dev/null 2>&1; then base=HEAD; else base=--cached; fi\n",
        crate::containers::sh_join(&[dir.to_string()])
    )
}

/// Untracked files (not ignored) the change views include, at most this many.
const UNTRACKED_MAX: usize = 200;

/// One changed file of a working tree.
#[derive(serde::Serialize, Debug, PartialEq)]
pub struct Change {
    pub path: String,
    pub added: u32,
    pub removed: u32,
    /// Not tracked by git yet.
    pub untracked: bool,
    pub binary: bool,
}

/// `git diff --numstat` lines, then "\x01" and `added\tremoved\tpath` lines for untracked files.
fn parse_changes(out: &str) -> Vec<Change> {
    let mut untracked = false;
    let mut v = Vec::new();
    for line in out.lines() {
        if line == "\x01" {
            untracked = true;
            continue;
        }
        let mut p = line.splitn(3, '\t');
        let (Some(a), Some(r), Some(path)) = (p.next(), p.next(), p.next()) else { continue };
        v.push(Change {
            path: path.to_string(),
            added: a.parse().unwrap_or(0),
            removed: r.parse().unwrap_or(0),
            untracked,
            binary: a == "-",
        });
    }
    v
}

/// Files changed in the working tree at `dir` against the last commit, untracked ones included.
/// Not a repository: an empty list.
#[tauri::command]
pub async fn git_changes(st: State<'_>, host: Option<Host>, dir: String) -> Res<Vec<Change>> {
    let script = format!(
        "{}g diff $base --numstat --no-renames --no-ext-diff\nprintf '\\001\\n'\n\
         g ls-files --others --exclude-standard | head -n {UNTRACKED_MAX} | while IFS= read -r f; do\n\
         [ -f \"$f\" ] || continue\n\
         if grep -Iq . \"$f\" 2>/dev/null || [ ! -s \"$f\" ]; then printf '%s\\t0\\t%s\\n' \"$(wc -l < \"$f\" | tr -d ' ')\" \"$f\"\n\
         else printf -- '-\\t-\\t%s\\n' \"$f\"; fi\ndone\n",
        git_prelude(&dir)
    );
    Ok(parse_changes(&run_sh(&st, host, script).await?))
}

/// The changes of `git_changes` as a unified diff: tracked files against the last commit,
/// then each untracked file as new. Cut at 8 MB.
#[tauri::command]
pub async fn git_diff(st: State<'_>, host: Option<Host>, dir: String) -> Res<String> {
    const MAX: usize = 8 << 20;
    let script = format!(
        "{}g diff $base --no-color --no-ext-diff --no-renames -U3\n\
         g ls-files --others --exclude-standard | head -n {UNTRACKED_MAX} | while IFS= read -r f; do\n\
         [ -f \"$f\" ] && g diff --no-index --no-color --no-ext-diff -- /dev/null \"$f\"\ndone\n",
        git_prelude(&dir)
    );
    let mut out = run_sh(&st, host, script).await?;
    if out.len() > MAX {
        let mut cut = MAX;
        while !out.is_char_boundary(cut) {
            cut -= 1;
        }
        out.truncate(cut);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_numstat_and_untracked_files() {
        let out = "12\t3\tsrc/a.rs\n-\t-\tlogo.png\n\x01\n40\t0\tsrc/new file.js\n-\t-\tfont.woff\n";
        let v = parse_changes(out);
        assert_eq!(v.len(), 4);
        assert_eq!(
            (v[0].path.as_str(), v[0].added, v[0].removed, v[0].untracked),
            ("src/a.rs", 12, 3, false)
        );
        assert!(v[1].binary && !v[1].untracked);
        assert_eq!((v[2].path.as_str(), v[2].added, v[2].untracked), ("src/new file.js", 40, true));
        assert!(v[3].binary && v[3].untracked);
    }
}

/// Open a fresh connection (bypassing the pool) and report how long it took.
#[tauri::command]
pub async fn ssh_test(st: State<'_>, host: Host) -> Res<String> {
    let started = std::time::Instant::now();
    let conn = st.pool.fresh(&st.vault, &host).await?;
    let ms = started.elapsed().as_millis();
    let _ = conn.handle.disconnect(russh::Disconnect::ByApplication, "", "en").await;
    Ok(format!("Connected in {ms} ms"))
}

/// Detect the remote OS (`ubuntu`, `debian`, `darwin`, …) and store it on the host.
#[tauri::command]
pub async fn host_detect_os(st: State<'_>, host: Host) -> Res<String> {
    let conn = st.pool.get(&st.vault, &host).await?;
    let (out, _) = conn.exec("(. /etc/os-release 2>/dev/null && echo \"$ID\") || uname -s").await?;
    let os = out.lines().next().unwrap_or("").trim().to_lowercase();
    if !os.is_empty() {
        if let Some(h) = st.vault.data.write().hosts.iter_mut().find(|h| h.id == host.id) {
            h.os = os.clone();
        }
        let _ = st.vault.save();
    }
    Ok(os)
}

/// CPU, memory, disk, uptime and load of a host, read over its pooled SSH connection.
#[tauri::command]
pub async fn host_stats(st: State<'_>, host: Host) -> Res<crate::stats::HostStats> {
    let conn = st.pool.get(&st.vault, &host).await?;
    let (out, _) = conn.exec(&crate::stats::command()).await?;
    let stats = crate::stats::parse(&out);
    if stats.is_empty() {
        return Err("Stats aren't available for this host".into());
    }
    Ok(stats)
}

/// Append a keychain public key to `~/.ssh/authorized_keys` (like `ssh-copy-id`).
#[tauri::command]
pub async fn key_install(st: State<'_>, host: Host, key_id: String) -> Res<()> {
    let key = st.vault.key(&key_id).ok_or("key not found")?;
    let pubk = key.public_key.trim().replace('\'', "");
    let conn = st.pool.get(&st.vault, &host).await?;
    let cmd = format!(
        "umask 077; mkdir -p ~/.ssh && touch ~/.ssh/authorized_keys && \
         (grep -qxF '{pubk}' ~/.ssh/authorized_keys || echo '{pubk}' >> ~/.ssh/authorized_keys)"
    );
    let (out, code) = conn.exec(&cmd).await?;
    if code != 0 {
        return Err(format!("failed ({code}): {out}"));
    }
    Ok(())
}

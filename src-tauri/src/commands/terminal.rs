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

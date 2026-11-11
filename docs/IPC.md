# IPC reference

The UI talks to the Rust core only through the Tauri commands below. They are defined in
[`src-tauri/src/commands/`](../src-tauri/src/commands) and called from JavaScript through the `api` proxy in
[`src/lib/api.js`](../src/lib/api.js):

```js
import { api, Channel } from '../lib/api'

const entries = await api.fs_list({ id, path: '/var/log' })
```

**Conventions**

- Argument names are **camelCase in JavaScript** and snake_case in Rust (`src_path` → `srcPath`).
- Every fallible command rejects with a human-readable **string** error, ready to show in a toast.
- Data types (`Host`, `Key`, `Snippet`, `Forward`, `Settings`, `VaultData`) are defined in
  [`src-tauri/src/vault.rs`](../src-tauri/src/vault.rs). Fields are snake_case in both languages.
- Commands that take a full `host` object (not an id) also accept unsaved hosts, which is how quick connect and
  the editor's **Test** button work.

## Vault: `commands/vault.rs`

| Command | Arguments | Returns |
|---|---|---|
| `vault_get` | – | `VaultData` (the whole vault) |
| `host_save` | `host: Host` | `Host` (an empty `id` creates a new host) |
| `host_delete` | `id` | – (also removes its forwards and clears jump-host references) |
| `key_save` | `key: Key` | `Key` |
| `key_delete` | `id` | – |
| `key_generate` | `name, kind: "ed25519" \| "ecdsa" \| "rsa", passphrase` | `Key` |
| `key_import` | `name, pem, passphrase` | `Key` |
| `snippet_save` / `snippet_delete` | `snippet: Snippet` / `id` | `Snippet` / – |
| `settings_save` | `settings: Settings` | – |
| `session_save` | `session` (open tabs as JSON; hosts by id) | – (reopened on the next launch) |
| `history_add` | `cmd, target` (`"local"`, `"local:<project folder>"` or host address) | – (deduplicated, last 2000 kept) |
| `workspace_save` | `workspace: Workspace` | `Workspace` (ids filled in) |
| `workspace_delete` | `id` | – |
| `history_clear` | – | – |
| `known_host_delete` | `host, keyType` | – |
| `import_ssh_config` | – | number of hosts imported |
| `import_putty` | `path?: string` (a `.reg` export; without it, the Windows registry) | `{ imported, existing, skipped: string[], notes: string[] }` |
| `backup_export` | `path, passphrase` | – |
| `backup_import` | `path, passphrase` | number of hosts in the backup (merged) |

## Terminals, SSH, Telnet and serial: `commands/terminal.rs`

| Command | Arguments | Returns |
|---|---|---|
| `term_open_ssh` | `id, host, cols, rows, container?: Shell, onData: Channel` | – (with `container`, runs its `docker exec` / `kubectl exec` instead of the login shell; the startup command is skipped) |
| `term_open_local` | `id, cols, rows, shell?: string, cwd?: string, startupCommand?: string, container?: Shell, onData: Channel` | – (error if `cwd` doesn't exist; `~` is expanded; with `container`, runs its command through a login shell on macOS/Linux, directly on Windows) |
| `term_open_telnet` | `id, host, cols, rows, onData: Channel` | – (address and port from `host`; window size sent with NAWS) |
| `term_open_serial` | `id, host, onData: Channel` | – (`host.host` is the device; `baud_rate`, `data_bits`, `parity`, `stop_bits`, `flow_control` from the host) |
| `serial_ports` | – | `[{ name, description }]` |
| `telnet_test` | `host` | `"Connected in N ms"` |
| `serial_test` | `host` | `"<port> opened"` |
| `term_write` | `id, data: string` | – |
| `term_resize` | `id, cols, rows` | – |
| `term_close` | `id` | – |
| `ssh_exec` | `host, command` | `{ output, code }` |
| `ssh_test` | `host` | `"Connected in N ms"` |
| `host_detect_os` | `host` | OS id such as `ubuntu`, also saved on the host |
| `host_stats` | `host` | `{ os, hostname, kernel, release, cpu_ticks, cpu_percent, cpus, mem_total, mem_used, mem_cache, swap_total, swap_used, uptime_secs, load, procs, net: { rx, tx }, disks: [{ mount, size, used }], top: [{ pid, name, cpu, mem }] }`, read over the pooled connection (Linux `/proc`, macOS/BSD `sysctl`, `vm_stat`, `netstat`, `ps`); errors when nothing could be read |
| `key_install` | `host, keyId` | – (appends the public key to `authorized_keys`) |
| `log_dir` | – | session-log folder (from the settings, or `Documents/K-Shell Logs`) |
| `log_dir_open` | – | – (creates the folder and opens it in the OS file manager) |
| `rec_start` | `id, label, cols, rows` | path of the new `.cast` file in the session-log folder (asciicast v2 header written) |
| `rec_write` | `id, events: [[seconds, "o" \| "r", data]]` | – (appends the events; other event types are dropped) |
| `rec_stop` | `id, events` | path (writes the last events and closes the file) |
| `rec_reveal` | `path` | – (shows the file in the OS file manager) |
| `cast_read` | `path` | file text, for the player (up to 256 MB) |

`id` is chosen by the UI (a UUID). On the `onData` channel:

- terminal output arrives as raw bytes (`ArrayBuffer`); use `asBytes()` from `lib/api.js`;
- when the shell ends, a JSON message `{ "type": "exit", "code": number | null }` arrives.

SSH connections are pooled per host id, so several terminals to one host share one TCP connection.

## Docker and Kubernetes: `commands/containers.rs`

`host: Host | null`: null runs `docker` / `kubectl` on this computer (through a login shell on macOS/Linux, so
PATH matches the user's terminal), a host runs it over its pooled SSH connection.

| Command | Arguments | Returns |
|---|---|---|
| `docker_containers` | `host` | `[{ id, name, image, status, ports }]` from `docker ps` (running containers); errors say when Docker isn't installed, the daemon isn't running or the Docker socket is permission denied |
| `kube_contexts` | `host` | `{ contexts: string[], current }` (`current` empty when none is set) |
| `kube_namespaces` | `host, context` | namespace names (empty `context`: kubectl's current one) |
| `kube_pods` | `host, context, namespace` | `[{ name, phase, ready: "1/2", containers: string[] }]` (`phase` is `Terminating` for a pod being deleted) |

`Shell` (the `container` argument of `term_open_ssh` / `term_open_local`) is
`{ engine: "docker", id }` → `docker exec -it <id> sh -c 'command -v bash >/dev/null && exec bash || exec sh'`, or
`{ engine: "kube", context?, namespace, pod, container? }` →
`kubectl --context=<context> -n <namespace> exec -it <pod> -c <container> -- sh -c '…'` (same shell choice).
Names starting with `-` are rejected.

## Files: `commands/files.rs`

| Command | Arguments | Returns |
|---|---|---|
| `fs_open` | `host: Host \| null` (null = local disk) | `{ id, home, sep }`. Backend picked by `host.protocol`: `ssh` (SFTP), `ftp`, `ftps`, `s3`, `azblob`, `gcs` |
| `fs_close` | `id` | – |
| `fs_list` | `id, path` | `Entry[]`, directories first |
| `fs_mkdir` | `id, path` | – |
| `fs_rename` | `id, from, to` | – |
| `fs_remove` | `id, path, isDir` | – (directories are removed recursively) |
| `fs_chmod` | `id, path, mode` | – (local, SFTP and FTP via `SITE CHMOD`) |
| `fs_read_text` / `fs_write_text` | `id, path` / `id, path, content` | `string` (max 8 MB) / – |
| `fs_read_bytes` | `id, path` | raw bytes as `ArrayBuffer` (max 32 MB, used by the image preview) |
| `fs_open_external` | `id, path, onChange: Channel` | watch id for remote files, `null` for local ones. Opens the file in the app the OS uses for it; remote files are downloaded to a temp copy first and `onChange` gets `{ type: "changed" }` each time that copy is saved. Files that run or install when opened are refused |
| `fs_upload_back` | `wid` | – (uploads the watched copy to where it came from) |
| `fs_unwatch` | `wid` | – |
| `fs_presign` | `id, path, expiry` (seconds) | share URL: S3 pre-signed GET, Azure read-only SAS (needs the account key), GCS signed URL (needs a service-account key; untested against real GCS) |
| `local_stat` | `path` | `{ is_dir, size }` |
| `read_local_text` | `path` | `string` (max 1 MB, used to import key files) |
| `transfer_probe` | `src, srcPath, dst, dstDir` | `{ resumable: false }`, or `{ resumable: true, exists, is_dir, size }` for what is already at the destination |
| `transfer` | `tid, src, srcPath, isDir, size, dst, dstDir, resume?, onProgress: Channel` | resolves when done |
| `transfer_cancel` | `tid` | – (the `transfer` call then rejects with `"cancelled"`) |

`Entry` is `{ name, path, is_dir, is_link, size, mtime, perm }`. `mtime` is in Unix seconds, and `perm` holds POSIX mode
bits or `null`.

`transfer` copies a file or folder from session `src` into folder `dstDir` of session `dst`, between **any** two
backends. `onProgress` receives `{ total, done, resumed, file }` about every 200 ms, then one message with
`finished: true`. `resumed` is the part of `done` that was already at the destination.

With `resume: true`, each destination file that is smaller than its source continues from where it stops and is
checked for the right size afterwards, a file that is already the same size is skipped, and anything else is copied
from the start. Resume works between the local file system and SFTP (either way). For other pairs `transfer_probe`
answers `resumable: false` and `resume` is ignored.

Remote paths always use `/`. Local paths use the OS separator returned as `sep` by `fs_open`. S3 paths look like
`/<bucket>/<key>`, and `/` lists the buckets.

## Port forwarding: `commands/forwards.rs`

| Command | Arguments | Returns |
|---|---|---|
| `forward_save` / `forward_delete` | `forward: Forward` / `id` | `Forward` / – |
| `forward_start` / `forward_stop` | `id` | – |
| `forward_active` | – | ids of running rules |

`Forward.kind` is `local` (`-L`), `remote` (`-R`) or `dynamic` (`-D`, SOCKS5).

## System: `commands/system.rs`

| Command | Returns |
|---|---|
| `platform` | `{ os: "macos" \| "windows" \| "linux", shell }` |

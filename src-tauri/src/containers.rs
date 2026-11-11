//! Docker containers and Kubernetes pods: list them, show their details, logs, mounts and disk
//! use, and build the command that opens a shell (or follows the logs) in one.
//!
//! Everything runs as a plain `docker` / `kubectl` command, either on this computer or on a saved
//! host through its SSH connection, so the same parsing and error messages serve both.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use crate::error::Res;
use crate::ssh::{Pool, SshConn};
use crate::vault::{Host, Vault};

/// Starts bash when the image has it, else sh.
pub const PICK_SHELL: &str = "command -v bash >/dev/null && exec bash || exec sh";

/// Lines a "follow logs" tab starts with.
const FOLLOW_TAIL: &str = "200";

/// A terminal tab that runs inside a container: a shell, or its logs followed live.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "engine", rename_all = "lowercase")]
pub enum Shell {
    Docker {
        id: String,
    },
    Kube {
        /// Empty: kubectl's current context.
        #[serde(default)]
        context: String,
        namespace: String,
        pod: String,
        /// Empty: the pod's default container.
        #[serde(default)]
        container: String,
    },
    #[serde(rename = "docker-logs")]
    DockerLogs {
        id: String,
    },
    #[serde(rename = "kube-logs")]
    KubeLogs {
        #[serde(default)]
        context: String,
        namespace: String,
        pod: String,
        #[serde(default)]
        container: String,
    },
}

impl Shell {
    /// The command line, as separate arguments.
    pub fn argv(&self) -> Res<Vec<String>> {
        match self {
            Shell::Docker { id } => {
                arg_ok("container", id)?;
                Ok(strings(&["docker", "exec", "-it", id, "sh", "-c", PICK_SHELL]))
            }
            Shell::DockerLogs { id } => {
                arg_ok("container", id)?;
                Ok(strings(&["docker", "logs", "--follow", "--tail", FOLLOW_TAIL, id]))
            }
            Shell::Kube { context, namespace, pod, container } => {
                let mut a = kube_pod_cmd(context, namespace, pod, "exec")?;
                a.insert(a.len() - 1, "-it".into());
                container_flag(&mut a, container)?;
                a.extend(strings(&["--", "sh", "-c", PICK_SHELL]));
                Ok(a)
            }
            Shell::KubeLogs { context, namespace, pod, container } => {
                let mut a = kube_pod_cmd(context, namespace, pod, "logs")?;
                a.extend(strings(&["--follow", &format!("--tail={FOLLOW_TAIL}")]));
                container_flag(&mut a, container)?;
                Ok(a)
            }
        }
    }
}

fn strings(a: &[&str]) -> Vec<String> {
    a.iter().map(|s| s.to_string()).collect()
}

/// A name passed to docker/kubectl: not empty, not taken for an option, no control characters.
fn arg_ok(what: &str, s: &str) -> Res<()> {
    if s.is_empty() || s.starts_with('-') || s.chars().any(char::is_control) {
        return Err(format!("Invalid {what} name: “{s}”"));
    }
    Ok(())
}

/// `kubectl`, with `--context` when one is chosen.
fn kubectl(context: &str) -> Res<Vec<String>> {
    let mut a = vec!["kubectl".to_string()];
    if !context.is_empty() {
        arg_ok("context", context)?;
        a.push(format!("--context={context}"));
    }
    Ok(a)
}

/// `kubectl [--context=…] -n <namespace> <verb> <pod>`.
fn kube_pod_cmd(context: &str, namespace: &str, pod: &str, verb: &str) -> Res<Vec<String>> {
    arg_ok("namespace", namespace)?;
    arg_ok("pod", pod)?;
    let mut a = kubectl(context)?;
    a.extend(strings(&["-n", namespace, verb, pod]));
    Ok(a)
}

fn container_flag(a: &mut Vec<String>, container: &str) -> Res<()> {
    if !container.is_empty() {
        arg_ok("container", container)?;
        a.extend(strings(&["-c", container]));
    }
    Ok(())
}

/// Quote one argument for a POSIX shell (and fish).
pub fn sh_quote(s: &str) -> String {
    if !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "-_./=:@,+%".contains(c)) {
        return s.to_string();
    }
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// A command line for a POSIX shell.
pub fn sh_join(argv: &[String]) -> String {
    argv.iter().map(|a| sh_quote(a)).collect::<Vec<_>>().join(" ")
}

/// String at a JSON pointer; numbers are written out, anything else is empty.
fn s(v: &Value, ptr: &str) -> String {
    match v.pointer(ptr) {
        Some(Value::String(x)) => x.clone(),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Bool(b)) => b.to_string(),
        _ => String::new(),
    }
}

fn int(v: &Value, ptr: &str) -> i64 {
    v.pointer(ptr).and_then(Value::as_i64).unwrap_or(0)
}

/// Docker's "never" time (`0001-01-01T00:00:00Z`) as empty.
fn time(v: &Value, ptr: &str) -> String {
    let t = s(v, ptr);
    if t.starts_with("0001-") {
        String::new()
    } else {
        t
    }
}

/// The JSON document in a command's output, after anything a login shell printed first.
fn json_in(out: &str, open: char) -> Res<Value> {
    let start = out.find(open).ok_or_else(|| first_lines(out, 0))?;
    serde_json::from_str(&out[start..]).map_err(|e| format!("Couldn't read the output: {e}"))
}

// ---------- Docker ----------

#[derive(Serialize, Clone, Debug, PartialEq, Default)]
pub struct Container {
    pub id: String,
    pub name: String,
    pub image: String,
    /// running, exited, paused, restarting, created, dead
    pub state: String,
    /// "Up 2 hours", "Exited (0) 3 days ago"
    pub status: String,
    /// Published ports, shortened: `8080→80/tcp`.
    pub ports: Vec<String>,
    /// Docker Compose project and service, from the container's labels.
    pub project: String,
    pub service: String,
    /// Names of the volumes it mounts.
    pub volumes: Vec<String>,
}

/// All containers, stopped ones too.
pub fn docker_ps() -> Vec<String> {
    strings(&["docker", "ps", "--all", "--no-trunc", "--format", "{{json .}}"])
}

/// Containers from `docker ps --format '{{json .}}'`; other lines (a login banner) are skipped.
pub fn parse_docker_ps(out: &str) -> Vec<Container> {
    let mut list: Vec<Container> = out
        .lines()
        .filter(|l| l.trim_start().starts_with('{'))
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .map(|v| {
            let labels = s(&v, "/Labels");
            let label = |k: &str| {
                labels
                    .split(',')
                    .find_map(|kv| kv.strip_prefix(k)?.strip_prefix('='))
                    .unwrap_or("")
                    .to_string()
            };
            let status = s(&v, "/Status");
            // older Docker has no State field
            let state = match s(&v, "/State") {
                st if !st.is_empty() => st,
                _ if status.starts_with("Up") => "running".into(),
                _ => "exited".into(),
            };
            Container {
                id: s(&v, "/ID"),
                // a container can have several names; the first is the one `docker ps` shows
                name: s(&v, "/Names").split(',').next().unwrap_or("").to_string(),
                image: s(&v, "/Image"),
                state,
                status,
                ports: short_ports(&s(&v, "/Ports")),
                project: label("com.docker.compose.project"),
                service: label("com.docker.compose.service"),
                volumes: s(&v, "/Mounts")
                    .split(',')
                    .map(str::trim)
                    .filter(|m| !m.is_empty())
                    .map(String::from)
                    .collect(),
            }
        })
        .filter(|c| !c.id.is_empty())
        .collect();
    list.sort_by(|a, b| {
        (a.project.is_empty(), &a.project, &a.name).cmp(&(b.project.is_empty(), &b.project, &b.name))
    });
    list
}

/// `0.0.0.0:8080->80/tcp, :::8080->80/tcp, 443/tcp` → `["8080→80/tcp", "443/tcp"]`.
fn short_ports(ports: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for p in ports.split(", ").map(str::trim).filter(|p| !p.is_empty()) {
        let p = ["0.0.0.0:", ":::", "[::]:"].iter().fold(p.to_string(), |p, pre| p.replacen(pre, "", 1));
        let p = p.replace("->", "→");
        if !out.contains(&p) {
            out.push(p);
        }
    }
    out
}

#[derive(Serialize, Clone, Debug, PartialEq, Default)]
pub struct Network {
    pub name: String,
    pub ip: String,
}

#[derive(Serialize, Clone, Debug, PartialEq, Default)]
pub struct Port {
    /// `80/tcp`
    pub container: String,
    /// `8080` or `127.0.0.1:8080`; empty when not published.
    pub host: String,
}

#[derive(Serialize, Clone, Debug, PartialEq, Default)]
pub struct Mount {
    /// volume, bind, tmpfs, …
    pub kind: String,
    /// Volume name (empty for a bind mount).
    pub name: String,
    pub source: String,
    pub destination: String,
    pub rw: bool,
}

/// One container from `docker inspect --size`.
#[derive(Serialize, Clone, Debug, PartialEq, Default)]
pub struct DockerDetail {
    pub id: String,
    pub name: String,
    pub image: String,
    pub created: String,
    pub started_at: String,
    pub finished_at: String,
    pub state: String,
    pub running: bool,
    pub exit_code: i64,
    pub oom_killed: bool,
    /// healthy / unhealthy / starting; empty without a health check.
    pub health: String,
    pub error: String,
    pub restart_count: i64,
    pub restart_policy: String,
    pub command: String,
    pub workdir: String,
    pub user: String,
    pub project: String,
    pub service: String,
    pub networks: Vec<Network>,
    pub ports: Vec<Port>,
    pub mounts: Vec<Mount>,
    /// Bytes written in the container's own layer, and with the image.
    pub size_rw: i64,
    pub size_root: i64,
    /// 0: no limit.
    pub memory_limit: i64,
    pub cpu_limit: f64,
}

pub fn docker_inspect_argv(id: &str) -> Res<Vec<String>> {
    arg_ok("container", id)?;
    Ok(strings(&["docker", "inspect", "--size", "--type", "container", id]))
}

pub fn parse_docker_inspect(out: &str) -> Res<DockerDetail> {
    let all = json_in(out, '[')?;
    let v = all.get(0).ok_or("Container not found")?;
    let labels = v.pointer("/Config/Labels");
    let label = |k: &str| labels.and_then(|l| l.get(k)).and_then(Value::as_str).unwrap_or("").to_string();
    let mut command: Vec<String> = vec![s(v, "/Path")];
    if let Some(args) = v.pointer("/Args").and_then(Value::as_array) {
        command.extend(args.iter().filter_map(Value::as_str).map(String::from));
    }
    let mut networks: Vec<Network> = v
        .pointer("/NetworkSettings/Networks")
        .and_then(Value::as_object)
        .map(|m| m.iter().map(|(name, n)| Network { name: name.clone(), ip: s(n, "/IPAddress") }).collect())
        .unwrap_or_default();
    networks.sort_by(|a, b| a.name.cmp(&b.name));
    let mut ports: Vec<Port> = Vec::new();
    if let Some(m) = v.pointer("/NetworkSettings/Ports").and_then(Value::as_object) {
        for (container, binds) in m {
            let mut hosts: Vec<String> = Vec::new();
            for b in binds.as_array().map(|a| a.as_slice()).unwrap_or_default() {
                let (ip, port) = (s(b, "/HostIp"), s(b, "/HostPort"));
                let h = if ip.is_empty() || ip == "0.0.0.0" || ip == "::" {
                    port
                } else {
                    format!("{ip}:{port}")
                };
                if !hosts.contains(&h) {
                    hosts.push(h);
                }
            }
            if hosts.is_empty() {
                ports.push(Port { container: container.clone(), host: String::new() });
            }
            ports.extend(hosts.into_iter().map(|host| Port { container: container.clone(), host }));
        }
    }
    let port_no = |p: &Port| p.container.split('/').next().and_then(|n| n.parse::<u32>().ok()).unwrap_or(0);
    ports.sort_by_key(|p| (port_no(p), p.host.clone()));
    let ports = merge_ports(ports);
    let mounts = v
        .pointer("/Mounts")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .map(|m| Mount {
                    kind: s(m, "/Type"),
                    name: s(m, "/Name"),
                    source: s(m, "/Source"),
                    destination: s(m, "/Destination"),
                    rw: m.pointer("/RW").and_then(Value::as_bool).unwrap_or(true),
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(DockerDetail {
        id: s(v, "/Id"),
        name: s(v, "/Name").trim_start_matches('/').to_string(),
        image: s(v, "/Config/Image"),
        created: time(v, "/Created"),
        started_at: time(v, "/State/StartedAt"),
        finished_at: time(v, "/State/FinishedAt"),
        state: s(v, "/State/Status"),
        running: v.pointer("/State/Running").and_then(Value::as_bool).unwrap_or(false),
        exit_code: int(v, "/State/ExitCode"),
        oom_killed: v.pointer("/State/OOMKilled").and_then(Value::as_bool).unwrap_or(false),
        health: s(v, "/State/Health/Status"),
        error: s(v, "/State/Error"),
        restart_count: int(v, "/RestartCount"),
        restart_policy: s(v, "/HostConfig/RestartPolicy/Name"),
        command: sh_join(&command).trim().to_string(),
        workdir: s(v, "/Config/WorkingDir"),
        user: s(v, "/Config/User"),
        project: label("com.docker.compose.project"),
        service: label("com.docker.compose.service"),
        networks,
        ports,
        mounts,
        size_rw: int(v, "/SizeRw"),
        size_root: int(v, "/SizeRootFs"),
        memory_limit: int(v, "/HostConfig/Memory"),
        cpu_limit: int(v, "/HostConfig/NanoCpus") as f64 / 1e9,
    })
}

/// Consecutive ports published to consecutive host ports (an FTP passive range) as one entry:
/// `21000/tcp → 21000` … `21010/tcp → 21010` becomes `21000-21010/tcp → 21000-21010`.
fn merge_ports(ports: Vec<Port>) -> Vec<Port> {
    /// Host ip and port; None: not published.
    type Bind = Option<(String, u32)>;
    /// First and last container port, protocol, first host port.
    type Run = (u32, u32, String, Bind);
    fn parse(p: &Port) -> Option<(u32, String, Bind)> {
        let (n, proto) = p.container.split_once('/')?;
        let host = if p.host.is_empty() {
            None
        } else {
            let (ip, port) = p.host.rsplit_once(':').unwrap_or(("", &p.host));
            Some((ip.to_string(), port.parse().ok()?))
        };
        Some((n.parse().ok()?, proto.to_string(), host))
    }
    fn flush(run: Run, out: &mut Vec<Port>) {
        let (a, b, proto, host) = run;
        let range = |x: u32, y: u32| if x == y { x.to_string() } else { format!("{x}-{y}") };
        out.push(Port {
            container: format!("{}/{proto}", range(a, b)),
            host: match host {
                None => String::new(),
                Some((ip, h)) if ip.is_empty() => range(h, h + b - a),
                Some((ip, h)) => format!("{ip}:{}", range(h, h + b - a)),
            },
        });
    }
    let mut out = Vec::new();
    let mut run: Option<Run> = None;
    for p in ports {
        let Some((n, proto, host)) = parse(&p) else {
            if let Some(r) = run.take() {
                flush(r, &mut out);
            }
            out.push(p);
            continue;
        };
        if let Some((a, b, rp, rh)) = &mut run {
            let next = n == *b + 1
                && proto == *rp
                && match (&host, &*rh) {
                    (None, None) => true,
                    (Some((ip, h)), Some((rip, rh0))) => ip == rip && *h == *rh0 + (n - *a),
                    _ => false,
                };
            if next {
                *b = n;
                continue;
            }
        }
        if let Some(r) = run.replace((n, n, proto, host)) {
            flush(r, &mut out);
        }
    }
    if let Some(r) = run {
        flush(r, &mut out);
    }
    out
}

/// Live usage of a running container (`docker stats --no-stream`), as Docker prints it.
#[derive(Serialize, Clone, Debug, PartialEq, Default)]
pub struct DockerStats {
    pub cpu: String,
    /// "12.3MiB / 7.6GiB"
    pub mem: String,
    pub mem_perc: String,
    pub net: String,
    pub block: String,
    pub pids: String,
}

pub fn docker_stats_argv(id: &str) -> Res<Vec<String>> {
    arg_ok("container", id)?;
    Ok(strings(&["docker", "stats", "--no-stream", "--format", "{{json .}}", id]))
}

pub fn parse_docker_stats(out: &str) -> Option<DockerStats> {
    let v = json_in(out, '{').ok()?;
    Some(DockerStats {
        cpu: s(&v, "/CPUPerc"),
        mem: s(&v, "/MemUsage"),
        mem_perc: s(&v, "/MemPerc"),
        net: s(&v, "/NetIO"),
        block: s(&v, "/BlockIO"),
        pids: s(&v, "/PIDs"),
    })
}

pub fn docker_logs_argv(id: &str, tail: u32) -> Res<Vec<String>> {
    arg_ok("container", id)?;
    Ok(strings(&["docker", "logs", "--timestamps", "--tail", &tail.clamp(1, 5000).to_string(), id]))
}

/// start / stop / restart a container.
pub fn docker_action_argv(id: &str, action: &str) -> Res<Vec<String>> {
    arg_ok("container", id)?;
    if !["start", "stop", "restart"].contains(&action) {
        return Err(format!("Unknown action: {action}"));
    }
    Ok(strings(&["docker", action, id]))
}

/// One row of `docker system df`: Images, Containers, Local Volumes, Build Cache.
#[derive(Serialize, Clone, Debug, PartialEq, Default)]
pub struct DiskUsage {
    pub kind: String,
    pub total: String,
    pub active: String,
    pub size: String,
    /// "1.2GB (40%)"
    pub reclaimable: String,
}

pub fn docker_df_argv() -> Vec<String> {
    strings(&["docker", "system", "df", "--format", "{{json .}}"])
}

pub fn parse_docker_df(out: &str) -> Vec<DiskUsage> {
    out.lines()
        .filter(|l| l.trim_start().starts_with('{'))
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .map(|v| DiskUsage {
            kind: s(&v, "/Type"),
            total: s(&v, "/TotalCount"),
            active: s(&v, "/Active"),
            size: s(&v, "/Size"),
            reclaimable: s(&v, "/Reclaimable"),
        })
        .filter(|d| !d.kind.is_empty())
        .collect()
}

/// Bytes in a size Docker prints (`23.65MB`, `1.2GB`, `512B`, `8.9kB`); 0 when unreadable.
pub fn size_bytes(s: &str) -> u64 {
    let s = s.trim();
    let split = s.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(s.len());
    let (num, unit) = s.split_at(split);
    let n: f64 = num.parse().unwrap_or(0.0);
    let mult = match unit.trim().to_ascii_lowercase().as_str() {
        "kb" => 1e3,
        "mb" => 1e6,
        "gb" => 1e9,
        "tb" => 1e12,
        "kib" => 1024.0,
        "mib" => 1024.0 * 1024.0,
        "gib" => 1024.0 * 1024.0 * 1024.0,
        _ => 1.0,
    };
    (n * mult) as u64
}

#[derive(Serialize, Clone, Debug, PartialEq, Default)]
pub struct Volume {
    pub name: String,
    pub size: String,
    pub bytes: u64,
    /// Containers using it (running or stopped).
    pub links: i64,
    pub driver: String,
    /// Created without a name (`docker run -v /data`).
    pub anonymous: bool,
}

#[derive(Serialize, Clone, Debug, PartialEq, Default)]
pub struct Image {
    pub id: String,
    /// `nginx:1.27`, or `<none>` for a dangling image.
    pub name: String,
    /// Untagged: no name refers to it any more (an image rebuilt under the same tag).
    pub dangling: bool,
    pub size: String,
    pub bytes: u64,
    /// Containers created from it.
    pub containers: i64,
    pub created: String,
}

#[derive(Serialize, Clone, Debug, PartialEq, Default)]
pub struct Storage {
    /// Largest first.
    pub volumes: Vec<Volume>,
    pub images: Vec<Image>,
}

pub fn docker_storage_argv() -> Vec<String> {
    strings(&["docker", "system", "df", "--verbose", "--format", "{{json .}}"])
}

/// Volumes and images with their sizes, from `docker system df --verbose --format '{{json .}}'`.
pub fn parse_docker_storage(out: &str) -> Res<Storage> {
    let v = json_in(out, '{')?;
    let list = |k: &str| v.get(k).and_then(Value::as_array).cloned().unwrap_or_default();
    let num = |x: &Value, p: &str| s(x, p).parse::<i64>().unwrap_or(0);
    let mut volumes: Vec<Volume> = list("Volumes")
        .iter()
        .map(|x| Volume {
            name: s(x, "/Name"),
            size: s(x, "/Size"),
            bytes: size_bytes(&s(x, "/Size")),
            links: num(x, "/Links"),
            driver: s(x, "/Driver"),
            anonymous: s(x, "/Labels").contains("com.docker.volume.anonymous"),
        })
        .collect();
    volumes.sort_by(|a, b| b.bytes.cmp(&a.bytes).then(a.name.cmp(&b.name)));
    let mut images: Vec<Image> = list("Images")
        .iter()
        .map(|x| {
            let (repo, tag) = (s(x, "/Repository"), s(x, "/Tag"));
            let dangling = repo.is_empty() || repo == "<none>" || tag.is_empty() || tag == "<none>";
            Image {
                id: s(x, "/ID"),
                name: if dangling { "<none>".into() } else { format!("{repo}:{tag}") },
                dangling,
                size: s(x, "/Size"),
                bytes: size_bytes(&s(x, "/Size")),
                containers: num(x, "/Containers"),
                created: s(x, "/CreatedSince"),
            }
        })
        .collect();
    images.sort_by(|a, b| b.bytes.cmp(&a.bytes).then(a.name.cmp(&b.name)));
    Ok(Storage { volumes, images })
}

/// Remove dangling images: Docker's own cleanup, safe because nothing refers to them.
pub fn docker_prune_dangling_argv() -> Vec<String> {
    strings(&["docker", "image", "prune", "--force"])
}

/// What a prune removed: `deleted:` lines and the `Total reclaimed space:` Docker prints.
#[derive(Serialize, Clone, Debug, PartialEq, Default)]
pub struct PruneResult {
    pub deleted: usize,
    pub reclaimed: String,
}

pub fn parse_prune(out: &str) -> PruneResult {
    PruneResult {
        deleted: out.lines().filter(|l| l.trim_start().to_lowercase().starts_with("deleted:")).count(),
        reclaimed: out
            .lines()
            .find_map(|l| l.trim().strip_prefix("Total reclaimed space:"))
            .map(|r| r.trim().to_string())
            .unwrap_or_default(),
    }
}

/// Remove one image by `name:tag` (or ID for a dangling one), without `--force`: Docker refuses
/// when a container, even a stopped one, still uses it.
pub fn docker_image_rm_argv(image: &str) -> Res<Vec<String>> {
    arg_ok("image", image)?;
    Ok(strings(&["docker", "image", "rm", image]))
}

/// Why `docker image rm` refused, in words.
pub fn image_rm_error(out: &str, code: i32, place: &str) -> String {
    let low = out.to_lowercase();
    if low.contains("is being used by") || low.contains("is using its referenced image") {
        return "A container uses it (stopped containers count too); remove that container first.".into();
    }
    if low.contains("referenced in multiple repositories") {
        return "It has several tags; remove each tag.".into();
    }
    if low.contains("no such image") {
        return "Already gone.".into();
    }
    docker_error(out, code, place)
}

/// Bytes Docker counts for images (`docker system df`), to measure what a removal freed.
pub fn images_bytes(df: &[DiskUsage]) -> u64 {
    df.iter().find(|d| d.kind == "Images").map_or(0, |d| size_bytes(&d.size))
}

/// What went wrong running a `docker` command, in words.
pub fn docker_error(out: &str, code: i32, place: &str) -> String {
    let low = out.to_lowercase();
    if code == 127 || low.contains("docker: command not found") || low.contains("docker: not found") {
        return format!("Docker isn't installed {place} (no docker command found).");
    }
    if low.contains("permission denied")
        && (low.contains("docker.sock") || low.contains("docker daemon socket"))
    {
        return format!(
            "Permission denied on the Docker socket {place}. Add the user to the docker group \
             (sudo usermod -aG docker $USER, then log in again) or connect as a user that can run docker."
        );
    }
    if low.contains("cannot connect to the docker daemon")
        || low.contains("is the docker daemon running")
        || low.contains("error during connect")
    {
        return format!("Docker is installed {place} but its daemon isn't running.");
    }
    first_lines(out, code)
}

// ---------- logs ----------

/// Log lines in time order. stdout and stderr arrive one after the other, so lines are sorted by
/// their timestamp (`--timestamps`); a line without one stays with the line before it.
pub fn sort_logs(out: &str) -> String {
    let mut entries: Vec<(chrono::DateTime<chrono::FixedOffset>, usize, String)> = Vec::new();
    let mut head: Vec<String> = Vec::new(); // lines before the first timestamp
    for line in out.lines() {
        let ts = line.split_once(' ').map_or(line, |(t, _)| t);
        match chrono::DateTime::parse_from_rfc3339(ts) {
            Ok(t) => entries.push((t, entries.len(), line.to_string())),
            Err(_) => match entries.last_mut() {
                Some(e) => {
                    e.2.push('\n');
                    e.2.push_str(line);
                }
                None => head.push(line.to_string()),
            },
        }
    }
    entries.sort_by_key(|e| (e.0, e.1));
    head.into_iter().chain(entries.into_iter().map(|e| e.2)).collect::<Vec<_>>().join("\n")
}

// ---------- Kubernetes ----------

#[derive(Serialize, Clone, Debug, PartialEq, Default)]
pub struct KubeContexts {
    pub contexts: Vec<String>,
    /// Empty when no context is set.
    pub current: String,
}

#[derive(Serialize, Clone, Debug, PartialEq, Default)]
pub struct Pod {
    pub name: String,
    /// What `kubectl get pods` shows: Running, Pending, CrashLoopBackOff, Completed, Terminating, …
    pub status: String,
    /// Ready containers, e.g. `1/2`.
    pub ready: String,
    pub restarts: i64,
    /// When the pod started (RFC 3339), else when it was created.
    pub started: String,
    pub node: String,
    pub ip: String,
    pub containers: Vec<String>,
}

/// `kubectl config get-contexts -o name; echo ---; kubectl config current-context`, as one command
/// so a remote host is asked once.
pub fn kube_contexts_cmd() -> String {
    "kubectl config get-contexts -o name && echo '---' && (kubectl config current-context 2>/dev/null || true)".into()
}

/// Local version of [`kube_contexts_cmd`]: two commands.
pub fn kube_contexts_argv() -> (Vec<String>, Vec<String>) {
    (
        strings(&["kubectl", "config", "get-contexts", "-o", "name"]),
        strings(&["kubectl", "config", "current-context"]),
    )
}

pub fn parse_kube_contexts(out: &str) -> KubeContexts {
    let (names, current) = out.split_once("---").unwrap_or((out, ""));
    KubeContexts {
        contexts: names.lines().map(str::trim).filter(|l| !l.is_empty()).map(String::from).collect(),
        current: current.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("").to_string(),
    }
}

pub fn kube_namespaces_argv(context: &str) -> Res<Vec<String>> {
    let mut a = kubectl(context)?;
    a.extend(strings(&[
        "get",
        "namespaces",
        "--request-timeout=15s",
        "-o",
        "jsonpath={.items[*].metadata.name}",
    ]));
    Ok(a)
}

pub fn parse_namespaces(out: &str) -> Vec<String> {
    let mut v: Vec<String> = out.split_whitespace().map(String::from).collect();
    v.sort();
    v
}

pub fn kube_pods_argv(context: &str, namespace: &str) -> Res<Vec<String>> {
    arg_ok("namespace", namespace)?;
    let mut a = kubectl(context)?;
    a.extend(strings(&["-n", namespace, "get", "pods", "--request-timeout=15s", "-o", "json"]));
    Ok(a)
}

pub fn kube_pod_argv(context: &str, namespace: &str, pod: &str) -> Res<Vec<String>> {
    let mut a = kube_pod_cmd(context, namespace, pod, "get")?;
    a.insert(a.len() - 1, "pod".into());
    a.extend(strings(&["--request-timeout=15s", "-o", "json"]));
    Ok(a)
}

pub fn kube_logs_argv(
    context: &str,
    namespace: &str,
    pod: &str,
    container: &str,
    tail: u32,
    previous: bool,
) -> Res<Vec<String>> {
    let mut a = kube_pod_cmd(context, namespace, pod, "logs")?;
    a.extend(strings(&["--timestamps", &format!("--tail={}", tail.clamp(1, 5000)), "--request-timeout=15s"]));
    if previous {
        a.push("--previous".into());
    }
    container_flag(&mut a, container)?;
    Ok(a)
}

/// The status column of `kubectl get pods`: a waiting or terminated container's reason wins over
/// the phase, a pod being deleted is Terminating.
fn pod_status(p: &Value) -> String {
    if p.pointer("/metadata/deletionTimestamp").is_some_and(|d| !d.is_null()) {
        return "Terminating".into();
    }
    let reason = s(p, "/status/reason"); // Evicted, …
    if !reason.is_empty() {
        return reason;
    }
    let statuses = p.pointer("/status/containerStatuses").and_then(Value::as_array);
    for c in statuses.map(|a| a.as_slice()).unwrap_or_default() {
        let waiting = s(c, "/state/waiting/reason");
        if !waiting.is_empty() {
            return waiting;
        }
        let term = s(c, "/state/terminated/reason");
        if !term.is_empty() && s(p, "/status/phase") != "Running" {
            return term;
        }
    }
    s(p, "/status/phase")
}

fn pod_from(p: &Value) -> Pod {
    let containers: Vec<String> = p
        .pointer("/spec/containers")
        .and_then(Value::as_array)
        .map(|a| a.iter().map(|c| s(c, "/name")).filter(|n| !n.is_empty()).collect())
        .unwrap_or_default();
    let statuses = p.pointer("/status/containerStatuses").and_then(Value::as_array);
    let all = statuses.map(|a| a.as_slice()).unwrap_or_default();
    let ready = all.iter().filter(|c| c["ready"].as_bool() == Some(true)).count();
    let started = match s(p, "/status/startTime") {
        t if t.is_empty() => s(p, "/metadata/creationTimestamp"),
        t => t,
    };
    Pod {
        name: s(p, "/metadata/name"),
        status: pod_status(p),
        ready: format!("{ready}/{}", containers.len()),
        restarts: all.iter().map(|c| int(c, "/restartCount")).sum(),
        started,
        node: s(p, "/spec/nodeName"),
        ip: s(p, "/status/podIP"),
        containers,
    }
}

/// Pods from `kubectl get pods -o json`.
pub fn parse_pods(out: &str) -> Res<Vec<Pod>> {
    let v = json_in(out, '{')?;
    let mut pods: Vec<Pod> = v["items"]
        .as_array()
        .map(|a| a.as_slice())
        .unwrap_or_default()
        .iter()
        .map(pod_from)
        .filter(|p| !p.name.is_empty())
        .collect();
    pods.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(pods)
}

#[derive(Serialize, Clone, Debug, PartialEq, Default)]
pub struct VolumeMount {
    pub volume: String,
    pub path: String,
    pub read_only: bool,
}

#[derive(Serialize, Clone, Debug, PartialEq, Default)]
pub struct KubeContainer {
    pub name: String,
    pub image: String,
    /// An init container (runs before the others).
    pub init: bool,
    pub ready: bool,
    pub restarts: i64,
    /// Running, Waiting or Terminated.
    pub state: String,
    /// Why it is waiting or terminated (CrashLoopBackOff, OOMKilled, Completed…), with the exit code.
    pub reason: String,
    pub message: String,
    pub since: String,
    /// Why the previous run ended, when it restarted: `Error (exit 1)`.
    pub last: String,
    pub mounts: Vec<VolumeMount>,
}

#[derive(Serialize, Clone, Debug, PartialEq, Default)]
pub struct KubeVolume {
    pub name: String,
    /// PVC, ConfigMap, Secret, emptyDir, hostPath, …
    pub kind: String,
    /// The claim, config map, secret or path behind it.
    pub source: String,
}

#[derive(Serialize, Clone, Debug, PartialEq, Default)]
pub struct PodDetail {
    #[serde(flatten)]
    pub pod: Pod,
    pub namespace: String,
    pub host_ip: String,
    pub created: String,
    pub qos: String,
    /// `ReplicaSet/web-7d9f…`
    pub owner: String,
    pub service_account: String,
    pub labels: Vec<(String, String)>,
    pub containers_detail: Vec<KubeContainer>,
    pub volumes: Vec<KubeVolume>,
}

fn terminated_text(t: &Value) -> String {
    let reason = s(t, "/reason");
    let code = int(t, "/exitCode");
    match (reason.is_empty(), code) {
        (true, c) => format!("exit {c}"),
        (false, 0) => reason,
        (false, c) => format!("{reason} (exit {c})"),
    }
}

fn kube_container(spec: &Value, status: Option<&Value>, init: bool) -> KubeContainer {
    let mut c = KubeContainer {
        name: s(spec, "/name"),
        image: s(spec, "/image"),
        init,
        mounts: spec
            .pointer("/volumeMounts")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .map(|m| VolumeMount {
                        volume: s(m, "/name"),
                        path: s(m, "/mountPath"),
                        read_only: m["readOnly"].as_bool().unwrap_or(false),
                    })
                    .collect()
            })
            .unwrap_or_default(),
        ..Default::default()
    };
    let Some(st) = status else {
        c.state = "Waiting".into();
        return c;
    };
    c.ready = st["ready"].as_bool().unwrap_or(false);
    c.restarts = int(st, "/restartCount");
    if let Some(t) = st.pointer("/state/running") {
        c.state = "Running".into();
        c.since = s(t, "/startedAt");
    } else if let Some(t) = st.pointer("/state/waiting") {
        c.state = "Waiting".into();
        c.reason = s(t, "/reason");
        c.message = s(t, "/message");
    } else if let Some(t) = st.pointer("/state/terminated") {
        c.state = "Terminated".into();
        c.reason = terminated_text(t);
        c.message = s(t, "/message");
        c.since = s(t, "/finishedAt");
    }
    if let Some(t) = st.pointer("/lastState/terminated") {
        c.last = terminated_text(t);
    }
    c
}

fn kube_volume(v: &Value) -> KubeVolume {
    let name = s(v, "/name");
    let (kind, source) = if v.get("persistentVolumeClaim").is_some() {
        ("PVC", s(v, "/persistentVolumeClaim/claimName"))
    } else if v.get("configMap").is_some() {
        ("ConfigMap", s(v, "/configMap/name"))
    } else if v.get("secret").is_some() {
        ("Secret", s(v, "/secret/secretName"))
    } else if v.get("emptyDir").is_some() {
        ("emptyDir", s(v, "/emptyDir/medium"))
    } else if v.get("hostPath").is_some() {
        ("hostPath", s(v, "/hostPath/path"))
    } else if v.get("nfs").is_some() {
        ("NFS", format!("{}:{}", s(v, "/nfs/server"), s(v, "/nfs/path")))
    } else if v.get("csi").is_some() {
        ("CSI", s(v, "/csi/driver"))
    } else if let Some(p) = v.pointer("/projected/sources").and_then(Value::as_array) {
        let kinds: Vec<String> = p.iter().filter_map(|x| x.as_object()?.keys().next().cloned()).collect();
        ("Projected", kinds.join(", "))
    } else {
        let other = v.as_object().and_then(|o| o.keys().find(|k| *k != "name").cloned()).unwrap_or_default();
        return KubeVolume { name, kind: other, source: String::new() };
    };
    KubeVolume { name, kind: kind.into(), source }
}

/// One pod from `kubectl get pod <name> -o json`.
pub fn parse_pod_detail(out: &str) -> Res<PodDetail> {
    let p = json_in(out, '{')?;
    let status_of = |list: &str, name: &str| -> Option<Value> {
        p.pointer(list)?.as_array()?.iter().find(|c| c["name"].as_str() == Some(name)).cloned()
    };
    let mut containers = Vec::new();
    for (specs, statuses, init) in [
        ("/spec/initContainers", "/status/initContainerStatuses", true),
        ("/spec/containers", "/status/containerStatuses", false),
    ] {
        for spec in p.pointer(specs).and_then(Value::as_array).map(|a| a.as_slice()).unwrap_or_default() {
            let st = status_of(statuses, &s(spec, "/name"));
            containers.push(kube_container(spec, st.as_ref(), init));
        }
    }
    let owner = p
        .pointer("/metadata/ownerReferences/0")
        .map(|o| format!("{}/{}", s(o, "/kind"), s(o, "/name")))
        .unwrap_or_default();
    let mut labels: Vec<(String, String)> = p
        .pointer("/metadata/labels")
        .and_then(Value::as_object)
        .map(|m| m.iter().map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string())).collect())
        .unwrap_or_default();
    labels.sort();
    Ok(PodDetail {
        pod: pod_from(&p),
        namespace: s(&p, "/metadata/namespace"),
        host_ip: s(&p, "/status/hostIP"),
        created: s(&p, "/metadata/creationTimestamp"),
        qos: s(&p, "/status/qosClass"),
        owner,
        service_account: s(&p, "/spec/serviceAccountName"),
        labels,
        containers_detail: containers,
        volumes: p
            .pointer("/spec/volumes")
            .and_then(Value::as_array)
            .map(|a| a.iter().map(kube_volume).collect())
            .unwrap_or_default(),
    })
}

/// What went wrong running a `kubectl` command, in words.
pub fn kube_error(out: &str, code: i32, place: &str) -> String {
    let low = out.to_lowercase();
    if code == 127 || low.contains("kubectl: command not found") || low.contains("kubectl: not found") {
        return format!("kubectl isn't installed {place} (no kubectl command found).");
    }
    first_lines(out, code)
}

fn first_lines(out: &str, code: i32) -> String {
    let text: Vec<&str> = out.lines().map(str::trim).filter(|l| !l.is_empty()).take(4).collect();
    if text.is_empty() {
        format!("Command failed (exit {code})")
    } else {
        text.join("\n")
    }
}

// ---------- SSH connection ----------

/// How long a host's connection stays open after the tab's last command.
const KEEP_FOR: Duration = Duration::from_secs(120);

/// Connections the Containers tab uses, with when they were last used. The pool only keeps a
/// connection while something holds it, and the tab refreshes every few seconds: without this
/// each refresh would be a new SSH handshake (and a new password or 2FA prompt).
static KEPT: LazyLock<Mutex<HashMap<String, Kept>>> = LazyLock::new(Default::default);
/// A connection and when the tab last used it.
type Kept = (Arc<SshConn>, Instant);

/// One connection attempt at a time, so the tab's first commands, sent together, share one
/// connection instead of each opening its own.
static CONNECTING: LazyLock<tokio::sync::Mutex<()>> = LazyLock::new(Default::default);

fn kept(id: &str) -> Option<Arc<SshConn>> {
    let mut k = KEPT.lock();
    match k.get_mut(id) {
        Some((c, used)) if !c.handle.is_closed() => {
            *used = Instant::now();
            Some(c.clone())
        }
        Some(_) => {
            k.remove(id);
            None
        }
        None => None,
    }
}

/// The SSH connection for the Containers tab's commands on `h`: shared, kept open for a while
/// after the last use.
pub async fn shared_conn(pool: &Pool, vault: &Arc<Vault>, h: &Host) -> Res<Arc<SshConn>> {
    if let Some(c) = kept(&h.id) {
        return Ok(c);
    }
    let _one = CONNECTING.lock().await;
    if let Some(c) = kept(&h.id) {
        return Ok(c); // connected while this call waited
    }
    let c = pool.get(vault, h).await?;
    KEPT.lock().insert(h.id.clone(), (c.clone(), Instant::now()));
    // close it once the tab has stopped using it
    let id = h.id.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(KEEP_FOR / 4).await;
            let mut k = KEPT.lock();
            match k.get(&id) {
                Some((c, used)) if used.elapsed() < KEEP_FOR && !c.handle.is_closed() => {}
                _ => {
                    k.remove(&id);
                    break;
                }
            }
        }
    });
    Ok(c)
}

// ---------- running commands on this computer ----------

/// A login shell to run a command line in, so PATH has what the user's terminal has
/// (a macOS app started from the Dock only gets /usr/bin:/bin:/usr/sbin:/sbin).
#[cfg(unix)]
pub fn login_shell() -> String {
    let sh = crate::terminal::default_shell();
    let name = sh.rsplit('/').next().unwrap_or("");
    if ["bash", "zsh", "sh", "ksh", "dash", "fish"].contains(&name) {
        sh
    } else {
        "/bin/sh".into()
    }
}

/// Run `argv` here and return its output (stdout and stderr) and exit code; 127 when the
/// program isn't found.
pub fn run_local(argv: &[String]) -> Res<(String, i32)> {
    #[cfg(unix)]
    let mut cmd = {
        let mut c = std::process::Command::new(login_shell());
        c.arg("-l").arg("-c").arg(sh_join(argv));
        c
    };
    #[cfg(windows)]
    let mut cmd = {
        use std::os::windows::process::CommandExt;
        let mut c = std::process::Command::new(&argv[0]);
        c.args(&argv[1..]).creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        c
    };
    cmd.stdin(std::process::Stdio::null());
    let out = match cmd.output() {
        Ok(o) => o,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok((format!("{}: not found", argv[0]), 127))
        }
        Err(e) => return Err(format!("Couldn't run {}: {e}", argv[0])),
    };
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    Ok((text, out.status.code().unwrap_or(-1)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting() {
        assert_eq!(sh_quote("abc-1.2"), "abc-1.2");
        assert_eq!(sh_quote(""), "''");
        assert_eq!(sh_quote("a b"), "'a b'");
        assert_eq!(sh_quote("it's"), r"'it'\''s'");
        assert_eq!(sh_quote("{{json .}}"), "'{{json .}}'");
        assert_eq!(sh_join(&strings(&["docker", "exec", "x;rm"])), "docker exec 'x;rm'");
    }

    #[test]
    fn docker_argv() {
        let a = Shell::Docker { id: "abc123".into() }.argv().unwrap();
        assert_eq!(
            sh_join(&a),
            "docker exec -it abc123 sh -c 'command -v bash >/dev/null && exec bash || exec sh'"
        );
        assert!(Shell::Docker { id: "".into() }.argv().is_err());
        assert!(Shell::Docker { id: "--privileged".into() }.argv().is_err());
        assert!(Shell::Docker { id: "a\nb".into() }.argv().is_err());
        let a = Shell::DockerLogs { id: "abc".into() }.argv().unwrap();
        assert_eq!(sh_join(&a), "docker logs --follow --tail 200 abc");
        assert!(docker_action_argv("abc", "rm").is_err());
        assert_eq!(docker_action_argv("abc", "restart").unwrap(), strings(&["docker", "restart", "abc"]));
        assert_eq!(docker_logs_argv("abc", 0).unwrap()[4], "1");
    }

    #[test]
    fn kube_argv() {
        let s = Shell::Kube {
            context: "kind-dev".into(),
            namespace: "default".into(),
            pod: "web-1".into(),
            container: "app".into(),
        };
        assert_eq!(
            sh_join(&s.argv().unwrap()),
            "kubectl --context=kind-dev -n default exec -it web-1 -c app -- sh -c \
             'command -v bash >/dev/null && exec bash || exec sh'"
        );
        let s =
            Shell::Kube { context: "".into(), namespace: "ns".into(), pod: "p".into(), container: "".into() };
        assert_eq!(s.argv().unwrap()[..6], strings(&["kubectl", "-n", "ns", "exec", "-it", "p"]));
        let s = Shell::Kube {
            context: "-x".into(),
            namespace: "ns".into(),
            pod: "p".into(),
            container: "".into(),
        };
        assert!(s.argv().is_err());
        let s = Shell::KubeLogs {
            context: "".into(),
            namespace: "ns".into(),
            pod: "p".into(),
            container: "c".into(),
        };
        assert_eq!(sh_join(&s.argv().unwrap()), "kubectl -n ns logs p --follow --tail=200 -c c");
        assert_eq!(
            sh_join(&kube_logs_argv("", "ns", "p", "", 100, true).unwrap()),
            "kubectl -n ns logs p --timestamps --tail=100 --request-timeout=15s --previous"
        );
        assert_eq!(
            sh_join(&kube_pod_argv("", "ns", "p").unwrap()),
            "kubectl -n ns get pod p --request-timeout=15s -o json"
        );
    }

    #[test]
    fn shell_from_json() {
        let s: Shell = serde_json::from_str(r#"{"engine":"docker","id":"x"}"#).unwrap();
        assert_eq!(s, Shell::Docker { id: "x".into() });
        let s: Shell = serde_json::from_str(r#"{"engine":"kube","namespace":"n","pod":"p"}"#).unwrap();
        assert!(matches!(s, Shell::Kube { ref context, .. } if context.is_empty()));
        let s: Shell = serde_json::from_str(r#"{"engine":"docker-logs","id":"x"}"#).unwrap();
        assert_eq!(s, Shell::DockerLogs { id: "x".into() });
    }

    #[test]
    fn docker_ps_output() {
        let out = "Welcome!\n\
            {\"ID\":\"bbb\",\"Image\":\"busybox:1.36\",\"Names\":\"kshell-telnet\",\"Ports\":\"0.0.0.0:2323->2323/tcp, :::2323->2323/tcp\",\"State\":\"running\",\"Status\":\"Up 2 minutes\",\"Labels\":\"com.docker.compose.project=kshell-test,com.docker.compose.service=telnet\",\"Mounts\":\"data,cfg\"}\n\
            {\"ID\":\"aaa\",\"Image\":\"nginx\",\"Names\":\"api,alias\",\"Ports\":\"\",\"Status\":\"Exited (0) 1 hour ago\",\"Labels\":\"\"}\n";
        let list = parse_docker_ps(out);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].name, "kshell-telnet", "compose projects first");
        assert_eq!(list[0].ports, vec!["2323→2323/tcp"]);
        assert_eq!((list[0].project.as_str(), list[0].service.as_str()), ("kshell-test", "telnet"));
        assert_eq!(list[0].volumes, vec!["data", "cfg"]);
        assert_eq!(list[1].name, "api");
        assert_eq!(list[1].state, "exited", "derived from Status on old Docker");
        assert!(parse_docker_ps("").is_empty());
        assert_eq!(
            short_ports("127.0.0.1:5432->5432/tcp, 80/tcp"),
            vec!["127.0.0.1:5432→5432/tcp", "80/tcp"]
        );
    }

    #[test]
    fn docker_inspect_output() {
        let out = r#"[{"Id":"abc","Name":"/web","Path":"nginx","Args":["-g","daemon off;"],
          "Created":"2026-11-01T10:00:00Z","RestartCount":2,"SizeRw":4096,"SizeRootFs":190000000,
          "State":{"Status":"running","Running":true,"ExitCode":0,"StartedAt":"2026-11-02T10:00:00Z",
                   "FinishedAt":"0001-01-01T00:00:00Z","Health":{"Status":"healthy"}},
          "Config":{"Image":"nginx:1.27","WorkingDir":"/","User":"","Labels":{"com.docker.compose.project":"site"}},
          "HostConfig":{"RestartPolicy":{"Name":"unless-stopped"},"Memory":536870912,"NanoCpus":1500000000},
          "NetworkSettings":{"Networks":{"site_default":{"IPAddress":"172.18.0.2"}},
            "Ports":{"80/tcp":[{"HostIp":"0.0.0.0","HostPort":"8080"},{"HostIp":"::","HostPort":"8080"}],"443/tcp":null}},
          "Mounts":[{"Type":"volume","Name":"site_data","Source":"/var/lib/docker/volumes/site_data/_data","Destination":"/data","RW":true},
                    {"Type":"bind","Source":"/srv/conf","Destination":"/etc/nginx/conf.d","RW":false}]}]"#;
        let d = parse_docker_inspect(out).unwrap();
        assert_eq!(d.name, "web");
        assert_eq!(d.command, "nginx -g 'daemon off;'");
        assert_eq!(d.finished_at, "");
        assert_eq!(d.health, "healthy");
        assert_eq!(d.restart_policy, "unless-stopped");
        assert_eq!(d.project, "site");
        assert_eq!(d.networks, vec![Network { name: "site_default".into(), ip: "172.18.0.2".into() }]);
        assert_eq!(d.ports.len(), 2);
        assert_eq!(d.ports[0].container, "80/tcp");
        assert_eq!((d.ports[0].container.as_str(), d.ports[0].host.as_str()), ("80/tcp", "8080"));
        assert_eq!(d.ports[1].host, "");
        assert!(!d.mounts[1].rw);
        assert_eq!(d.cpu_limit, 1.5);
        assert!(parse_docker_inspect("[]").is_err());
        assert!(parse_docker_inspect("Error: No such container: x").is_err());
    }

    #[test]
    fn port_ranges() {
        let p = |c: &str, h: &str| Port { container: c.into(), host: h.into() };
        let mut ports: Vec<Port> = (21000..=21003).map(|n| p(&format!("{n}/tcp"), &n.to_string())).collect();
        ports.insert(0, p("21/tcp", "2121"));
        ports.push(p("30000/tcp", "127.0.0.1:40000"));
        ports.push(p("30001/tcp", "127.0.0.1:40001"));
        ports.push(p("30002/udp", ""));
        ports.push(p("30003/udp", ""));
        assert_eq!(
            merge_ports(ports),
            vec![
                p("21/tcp", "2121"),
                p("21000-21003/tcp", "21000-21003"),
                p("30000-30001/tcp", "127.0.0.1:40000-40001"),
                p("30002-30003/udp", ""),
            ]
        );
        // not consecutive on the host side: kept apart
        assert_eq!(merge_ports(vec![p("80/tcp", "8080"), p("81/tcp", "9090")]).len(), 2);
    }

    #[test]
    fn storage() {
        assert_eq!(size_bytes("23.65MB"), 23_650_000);
        assert_eq!(size_bytes("512B"), 512);
        assert_eq!(size_bytes("1.5GiB"), 1_610_612_736);
        assert_eq!(size_bytes("N/A"), 0);
        let out = r#"{"Images":[{"ID":"sha256:a","Repository":"nginx","Tag":"1.27","Size":"190MB","Containers":"1","CreatedSince":"2 weeks ago"},
            {"ID":"sha256:b","Repository":"<none>","Tag":"<none>","Size":"1.2GB","Containers":"0","CreatedSince":"3 months ago"}],
          "Volumes":[{"Name":"small","Size":"8kB","Links":"1","Driver":"local","Labels":""},
            {"Name":"abc123","Size":"2GB","Links":"0","Driver":"local","Labels":"com.docker.volume.anonymous="}]}"#;
        let st = parse_docker_storage(out).unwrap();
        assert_eq!(st.volumes[0].name, "abc123", "largest first");
        assert!(st.volumes[0].anonymous && st.volumes[0].links == 0);
        assert_eq!(st.images[0].name, "<none>");
        assert!(st.images[0].dangling && !st.images[1].dangling);
        assert_eq!(st.images[1].name, "nginx:1.27");
        assert_eq!(st.images[1].containers, 1);
    }

    #[test]
    fn image_cleanup() {
        let out = "Deleted Images:\nuntagged: foo@sha256:1\ndeleted: sha256:aaa\ndeleted: sha256:bbb\n\nTotal reclaimed space: 1.2GB\n";
        assert_eq!(parse_prune(out), PruneResult { deleted: 2, reclaimed: "1.2GB".into() });
        assert_eq!(
            parse_prune("Total reclaimed space: 0B"),
            PruneResult { deleted: 0, reclaimed: "0B".into() }
        );
        assert_eq!(
            docker_image_rm_argv("nginx:1.27").unwrap(),
            strings(&["docker", "image", "rm", "nginx:1.27"])
        );
        assert!(docker_image_rm_argv("-f").is_err());
        let used = "Error response from daemon: conflict: unable to remove repository reference \"nginx\" (must force) - container 1a2b is using its referenced image 3c4d";
        assert!(image_rm_error(used, 1, "here").contains("A container uses it"));
        assert!(image_rm_error("Error: No such image: x", 1, "here").contains("Already gone"));
        let df = vec![DiskUsage { kind: "Images".into(), size: "1.5GB".into(), ..Default::default() }];
        assert_eq!(images_bytes(&df), 1_500_000_000);
    }

    #[test]
    fn stats_and_df() {
        let st = parse_docker_stats(
            r#"{"CPUPerc":"0.12%","MemUsage":"5MiB / 7.6GiB","MemPerc":"0.06%","NetIO":"1kB / 0B","BlockIO":"0B / 0B","PIDs":"3"}"#,
        )
        .unwrap();
        assert_eq!((st.cpu.as_str(), st.pids.as_str()), ("0.12%", "3"));
        let df = parse_docker_df(
            "{\"Type\":\"Images\",\"TotalCount\":\"12\",\"Active\":\"4\",\"Size\":\"3.2GB\",\"Reclaimable\":\"1.1GB (34%)\"}\n\
             {\"Type\":\"Local Volumes\",\"TotalCount\":3,\"Active\":\"1\",\"Size\":\"20MB\",\"Reclaimable\":\"5MB (25%)\"}",
        );
        assert_eq!(df.len(), 2);
        assert_eq!(df[1].total, "3", "numbers too");
    }

    #[test]
    fn logs_in_time_order() {
        let out =
            "2026-11-11T10:00:01.5Z out 2\n2026-11-11T10:00:00Z out 1\n  continued\n2026-11-11T10:00:01Z err";
        assert_eq!(
            sort_logs(out),
            "2026-11-11T10:00:00Z out 1\n  continued\n2026-11-11T10:00:01Z err\n2026-11-11T10:00:01.5Z out 2"
        );
        assert_eq!(sort_logs("banner\n2026-11-11T10:00:00Z a"), "banner\n2026-11-11T10:00:00Z a");
    }

    #[test]
    fn docker_errors() {
        assert!(
            docker_error("bash: docker: command not found", 127, "on web").contains("isn't installed on web")
        );
        let perm = "permission denied while trying to connect to the Docker daemon socket at unix:///var/run/docker.sock";
        assert!(docker_error(perm, 1, "on web").contains("docker group"));
        let down = "Cannot connect to the Docker daemon at unix:///var/run/docker.sock. Is the docker daemon running?";
        assert!(docker_error(down, 1, "here").contains("daemon isn't running"));
        assert_eq!(docker_error("", 3, "here"), "Command failed (exit 3)");
    }

    #[test]
    fn contexts() {
        let c = parse_kube_contexts("kind-dev\nprod\n---\nprod\n");
        assert_eq!(c.contexts, vec!["kind-dev", "prod"]);
        assert_eq!(c.current, "prod");
        assert_eq!(parse_kube_contexts("a\n---\n").current, "");
        assert_eq!(parse_namespaces("kube-system default"), vec!["default", "kube-system"]);
    }

    #[test]
    fn pods() {
        let out = r#"{"items":[
          {"metadata":{"name":"web-1"},"spec":{"nodeName":"n1","containers":[{"name":"app"},{"name":"sidecar"}]},
           "status":{"phase":"Running","startTime":"2026-11-10T00:00:00Z","podIP":"10.0.0.5","containerStatuses":[
             {"name":"app","ready":true,"restartCount":1},
             {"name":"sidecar","ready":false,"restartCount":4,"state":{"waiting":{"reason":"CrashLoopBackOff"}}}]}},
          {"metadata":{"name":"old","deletionTimestamp":"2026-01-01T00:00:00Z","creationTimestamp":"2025-12-01T00:00:00Z"},
           "spec":{"containers":[{"name":"c"}]},"status":{"phase":"Running"}},
          {"metadata":{"name":"job"},"spec":{"containers":[{"name":"c"}]},"status":{"phase":"Succeeded",
           "containerStatuses":[{"name":"c","ready":false,"state":{"terminated":{"reason":"Completed","exitCode":0}}}]}}
        ]}"#;
        let p = parse_pods(out).unwrap();
        assert_eq!(p.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), vec!["job", "old", "web-1"]);
        assert_eq!(p[0].status, "Completed");
        assert_eq!(p[1].status, "Terminating");
        assert_eq!(p[1].started, "2025-12-01T00:00:00Z", "creation time when not started");
        assert_eq!(p[2].status, "CrashLoopBackOff");
        assert_eq!((p[2].ready.as_str(), p[2].restarts), ("1/2", 5));
        assert_eq!(p[2].node, "n1");
        assert!(parse_pods("No resources found").is_err());
        assert!(parse_pods(r#"{"items":[]}"#).unwrap().is_empty());
        assert!(kube_error("sh: kubectl: not found", 127, "on web").contains("kubectl isn't installed"));
    }

    #[test]
    fn pod_detail() {
        let out = r#"{"metadata":{"name":"db-0","namespace":"prod","creationTimestamp":"2026-11-01T00:00:00Z",
            "labels":{"app":"db"},"ownerReferences":[{"kind":"StatefulSet","name":"db"}]},
          "spec":{"nodeName":"n2","serviceAccountName":"default",
            "initContainers":[{"name":"init","image":"busybox"}],
            "containers":[{"name":"pg","image":"postgres:16","volumeMounts":[{"name":"data","mountPath":"/var/lib/postgresql"},
              {"name":"cfg","mountPath":"/etc/pg","readOnly":true}]}],
            "volumes":[{"name":"data","persistentVolumeClaim":{"claimName":"data-db-0"}},{"name":"cfg","configMap":{"name":"pg-config"}},
              {"name":"tok","projected":{"sources":[{"serviceAccountToken":{}},{"configMap":{}}]}},{"name":"x","gitRepo":{}}]},
          "status":{"phase":"Running","qosClass":"Burstable","hostIP":"192.168.1.2","podIP":"10.1.0.9",
            "initContainerStatuses":[{"name":"init","ready":true,"state":{"terminated":{"reason":"Completed","exitCode":0}}}],
            "containerStatuses":[{"name":"pg","ready":true,"restartCount":3,"state":{"running":{"startedAt":"2026-11-02T00:00:00Z"}},
              "lastState":{"terminated":{"reason":"OOMKilled","exitCode":137}}}]}}"#;
        let d = parse_pod_detail(out).unwrap();
        assert_eq!(d.owner, "StatefulSet/db");
        assert_eq!(d.qos, "Burstable");
        assert_eq!(d.containers_detail.len(), 2);
        assert!(d.containers_detail[0].init);
        assert_eq!(d.containers_detail[0].reason, "Completed");
        let pg = &d.containers_detail[1];
        assert_eq!(
            (pg.state.as_str(), pg.restarts, pg.last.as_str()),
            ("Running", 3, "OOMKilled (exit 137)")
        );
        assert!(pg.mounts[1].read_only);
        assert_eq!(
            d.volumes[0],
            KubeVolume { name: "data".into(), kind: "PVC".into(), source: "data-db-0".into() }
        );
        assert_eq!(d.volumes[2].source, "serviceAccountToken, configMap");
        assert_eq!(d.volumes[3].kind, "gitRepo");
        assert_eq!(d.pod.status, "Running");
        let json = serde_json::to_value(&d).unwrap();
        assert_eq!(json["name"], "db-0", "pod fields are flattened");
    }

    #[cfg(unix)]
    #[test]
    fn local_missing_program() {
        let (_, code) = run_local(&strings(&["kshell-no-such-program-xyz"])).unwrap();
        assert_eq!(code, 127);
    }
}

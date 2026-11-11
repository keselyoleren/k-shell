//! Docker containers and Kubernetes pods: list them, and build the command that opens a shell in one.
//!
//! Everything runs as a plain `docker` / `kubectl` command, either on this computer or on a saved
//! host through its SSH connection, so the same parsing and error messages serve both.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::Res;

/// Starts bash when the image has it, else sh.
pub const PICK_SHELL: &str = "command -v bash >/dev/null && exec bash || exec sh";

/// A shell inside a container, as a terminal target.
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
}

impl Shell {
    /// The command line that opens the shell, as separate arguments.
    pub fn argv(&self) -> Res<Vec<String>> {
        match self {
            Shell::Docker { id } => {
                arg_ok("container", id)?;
                Ok(strings(&["docker", "exec", "-it", id, "sh", "-c", PICK_SHELL]))
            }
            Shell::Kube { context, namespace, pod, container } => {
                arg_ok("namespace", namespace)?;
                arg_ok("pod", pod)?;
                let mut a = kubectl(context)?;
                a.extend(strings(&["-n", namespace, "exec", "-it", pod]));
                if !container.is_empty() {
                    arg_ok("container", container)?;
                    a.extend(strings(&["-c", container]));
                }
                a.extend(strings(&["--", "sh", "-c", PICK_SHELL]));
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

// ---------- Docker ----------

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Container {
    pub id: String,
    pub name: String,
    pub image: String,
    pub status: String,
    pub ports: String,
}

pub fn docker_ps() -> Vec<String> {
    strings(&["docker", "ps", "--no-trunc", "--format", "{{json .}}"])
}

/// Running containers from `docker ps --format '{{json .}}'`; other lines (a login banner) are skipped.
pub fn parse_docker_ps(out: &str) -> Vec<Container> {
    let field = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let mut list: Vec<Container> = out
        .lines()
        .filter(|l| l.trim_start().starts_with('{'))
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .map(|v| Container {
            id: field(&v, "ID"),
            // a container can have several names; the first is the one `docker ps` shows
            name: field(&v, "Names").split(',').next().unwrap_or("").to_string(),
            image: field(&v, "Image"),
            status: field(&v, "Status"),
            ports: field(&v, "Ports"),
        })
        .filter(|c| !c.id.is_empty())
        .collect();
    list.sort_by(|a, b| a.name.cmp(&b.name));
    list
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

// ---------- Kubernetes ----------

#[derive(Serialize, Clone, Debug, PartialEq, Default)]
pub struct KubeContexts {
    pub contexts: Vec<String>,
    /// Empty when no context is set.
    pub current: String,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Pod {
    pub name: String,
    /// Running, Pending, …
    pub phase: String,
    /// Ready containers, e.g. `1/2`.
    pub ready: String,
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

/// Pods from `kubectl get pods -o json`.
pub fn parse_pods(out: &str) -> Res<Vec<Pod>> {
    let json = out.find('{').map(|i| &out[i..]).ok_or("kubectl returned no pod list")?;
    let v: Value = serde_json::from_str(json).map_err(|e| format!("Couldn't read the pod list: {e}"))?;
    let s = |v: &Value, p: &str| v.pointer(p).and_then(Value::as_str).unwrap_or("").to_string();
    let mut pods: Vec<Pod> = v["items"]
        .as_array()
        .map(|a| a.as_slice())
        .unwrap_or_default()
        .iter()
        .map(|p| {
            let containers: Vec<String> = p
                .pointer("/spec/containers")
                .and_then(Value::as_array)
                .map(|a| a.iter().map(|c| s(c, "/name")).filter(|n| !n.is_empty()).collect())
                .unwrap_or_default();
            let statuses = p.pointer("/status/containerStatuses").and_then(Value::as_array);
            let ready =
                statuses.map_or(0, |a| a.iter().filter(|c| c["ready"].as_bool() == Some(true)).count());
            // a pod being deleted still says Running
            let phase = if p.pointer("/metadata/deletionTimestamp").is_some_and(|d| !d.is_null()) {
                "Terminating".to_string()
            } else {
                s(p, "/status/phase")
            };
            Pod {
                name: s(p, "/metadata/name"),
                phase,
                ready: format!("{ready}/{}", containers.len()),
                containers,
            }
        })
        .filter(|p| !p.name.is_empty())
        .collect();
    pods.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(pods)
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
        assert_eq!(s.argv().unwrap()[..5], strings(&["kubectl", "-n", "ns", "exec", "-it"]));
        let s = Shell::Kube {
            context: "-x".into(),
            namespace: "ns".into(),
            pod: "p".into(),
            container: "".into(),
        };
        assert!(s.argv().is_err());
    }

    #[test]
    fn shell_from_json() {
        let s: Shell = serde_json::from_str(r#"{"engine":"docker","id":"x"}"#).unwrap();
        assert_eq!(s, Shell::Docker { id: "x".into() });
        let s: Shell = serde_json::from_str(r#"{"engine":"kube","namespace":"n","pod":"p"}"#).unwrap();
        assert!(matches!(s, Shell::Kube { ref context, .. } if context.is_empty()));
    }

    #[test]
    fn docker_ps_output() {
        let out = "Welcome!\n\
            {\"ID\":\"bbb\",\"Image\":\"busybox:1.36\",\"Names\":\"kshell-telnet\",\"Ports\":\"0.0.0.0:2323->2323/tcp\",\"Status\":\"Up 2 minutes\"}\n\
            {\"ID\":\"aaa\",\"Image\":\"nginx\",\"Names\":\"api,alias\",\"Ports\":\"\",\"Status\":\"Up 1 hour\"}\n";
        let list = parse_docker_ps(out);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].name, "api");
        assert_eq!(list[1].ports, "0.0.0.0:2323->2323/tcp");
        assert!(parse_docker_ps("").is_empty());
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
          {"metadata":{"name":"web-1"},"spec":{"containers":[{"name":"app"},{"name":"sidecar"}]},
           "status":{"phase":"Running","containerStatuses":[{"name":"app","ready":true},{"name":"sidecar","ready":false}]}},
          {"metadata":{"name":"old","deletionTimestamp":"2026-01-01T00:00:00Z"},"spec":{"containers":[{"name":"c"}]},
           "status":{"phase":"Running"}}
        ]}"#;
        let p = parse_pods(out).unwrap();
        assert_eq!(p[0].name, "old");
        assert_eq!(p[0].phase, "Terminating");
        assert_eq!(p[1].containers, vec!["app", "sidecar"]);
        assert_eq!(p[1].ready, "1/2");
        assert!(parse_pods("No resources found").is_err());
        assert!(parse_pods(r#"{"items":[]}"#).unwrap().is_empty());
        assert!(kube_error("sh: kubectl: not found", 127, "on web").contains("kubectl isn't installed"));
    }

    #[cfg(unix)]
    #[test]
    fn local_missing_program() {
        let (_, code) = run_local(&strings(&["kshell-no-such-program-xyz"])).unwrap();
        assert_eq!(code, 127);
    }
}

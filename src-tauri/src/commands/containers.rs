//! Docker containers and Kubernetes pods on this computer (`host` = null) or on a saved SSH host.

use super::State;
use crate::containers::{self as c, Container, KubeContexts, Pod};
use crate::error::Res;
use crate::vault::Host;

/// Where a command runs, for error messages.
fn place(host: Option<&Host>) -> String {
    match host {
        Some(h) => format!("on {}", if h.label.is_empty() { &h.host } else { &h.label }),
        None => "on this computer".into(),
    }
}

/// Run a command line here or over the host's pooled SSH connection.
async fn run(st: &State<'_>, host: Option<&Host>, argv: Vec<String>) -> Res<(String, i32)> {
    match host {
        Some(h) => {
            let conn = st.pool.get(&st.vault, h).await?;
            let (out, code) = conn.exec(&c::sh_join(&argv)).await?;
            Ok((out, code as i32))
        }
        None => tokio::task::spawn_blocking(move || c::run_local(&argv)).await.map_err(|e| e.to_string())?,
    }
}

/// Running Docker containers.
#[tauri::command]
pub async fn docker_containers(st: State<'_>, host: Option<Host>) -> Res<Vec<Container>> {
    let (out, code) = run(&st, host.as_ref(), c::docker_ps()).await?;
    if code != 0 {
        return Err(c::docker_error(&out, code, &place(host.as_ref())));
    }
    Ok(c::parse_docker_ps(&out))
}

/// kubectl contexts and the current one.
#[tauri::command]
pub async fn kube_contexts(st: State<'_>, host: Option<Host>) -> Res<KubeContexts> {
    let where_ = place(host.as_ref());
    match host {
        Some(h) => {
            let conn = st.pool.get(&st.vault, &h).await?;
            let (out, code) = conn.exec(&c::kube_contexts_cmd()).await?;
            if code != 0 {
                return Err(c::kube_error(&out, code as i32, &where_));
            }
            Ok(c::parse_kube_contexts(&out))
        }
        None => {
            let (list, current) = c::kube_contexts_argv();
            let (out, code) = run(&st, None, list).await?;
            if code != 0 {
                return Err(c::kube_error(&out, code, &where_));
            }
            // no current context is not an error
            let cur = run(&st, None, current).await.ok().filter(|(_, code)| *code == 0).map(|(o, _)| o);
            Ok(c::parse_kube_contexts(&format!("{out}\n---\n{}", cur.unwrap_or_default())))
        }
    }
}

/// Namespaces of a context (empty context: the current one).
#[tauri::command]
pub async fn kube_namespaces(st: State<'_>, host: Option<Host>, context: String) -> Res<Vec<String>> {
    let (out, code) = run(&st, host.as_ref(), c::kube_namespaces_argv(&context)?).await?;
    if code != 0 {
        return Err(c::kube_error(&out, code, &place(host.as_ref())));
    }
    Ok(c::parse_namespaces(&out))
}

/// Pods of a namespace with their containers.
#[tauri::command]
pub async fn kube_pods(
    st: State<'_>,
    host: Option<Host>,
    context: String,
    namespace: String,
) -> Res<Vec<Pod>> {
    let (out, code) = run(&st, host.as_ref(), c::kube_pods_argv(&context, &namespace)?).await?;
    if code != 0 {
        return Err(c::kube_error(&out, code, &place(host.as_ref())));
    }
    c::parse_pods(&out)
}

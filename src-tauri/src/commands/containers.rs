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
            let conn = c::shared_conn(&st.pool, &st.vault, h).await?;
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
            let conn = c::shared_conn(&st.pool, &st.vault, &h).await?;
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

/// One container's details (`docker inspect`).
#[tauri::command]
pub async fn docker_detail(st: State<'_>, host: Option<Host>, id: String) -> Res<c::DockerDetail> {
    let (out, code) = run(&st, host.as_ref(), c::docker_inspect_argv(&id)?).await?;
    if code != 0 {
        return Err(c::docker_error(&out, code, &place(host.as_ref())));
    }
    c::parse_docker_inspect(&out)
}

/// Live CPU, memory, network and disk use of a running container (takes about a second:
/// Docker samples the CPU twice).
#[tauri::command]
pub async fn docker_stats(st: State<'_>, host: Option<Host>, id: String) -> Res<Option<c::DockerStats>> {
    let (out, code) = run(&st, host.as_ref(), c::docker_stats_argv(&id)?).await?;
    if code != 0 {
        return Err(c::docker_error(&out, code, &place(host.as_ref())));
    }
    Ok(c::parse_docker_stats(&out))
}

/// The last `tail` log lines of a container, stdout and stderr in time order.
#[tauri::command]
pub async fn docker_logs(st: State<'_>, host: Option<Host>, id: String, tail: u32) -> Res<String> {
    let (out, code) = run(&st, host.as_ref(), c::docker_logs_argv(&id, tail)?).await?;
    if code != 0 {
        return Err(c::docker_error(&out, code, &place(host.as_ref())));
    }
    Ok(c::sort_logs(&out))
}

/// start / stop / restart a container.
#[tauri::command]
pub async fn docker_action(st: State<'_>, host: Option<Host>, id: String, action: String) -> Res<()> {
    let (out, code) = run(&st, host.as_ref(), c::docker_action_argv(&id, &action)?).await?;
    if code != 0 {
        return Err(c::docker_error(&out, code, &place(host.as_ref())));
    }
    Ok(())
}

/// Disk used by images, containers, volumes and the build cache (`docker system df`).
#[tauri::command]
pub async fn docker_disk(st: State<'_>, host: Option<Host>) -> Res<Vec<c::DiskUsage>> {
    let (out, code) = run(&st, host.as_ref(), c::docker_df_argv()).await?;
    if code != 0 {
        return Err(c::docker_error(&out, code, &place(host.as_ref())));
    }
    Ok(c::parse_docker_df(&out))
}

/// One pod with its containers and volumes.
#[tauri::command]
pub async fn kube_pod(
    st: State<'_>,
    host: Option<Host>,
    context: String,
    namespace: String,
    pod: String,
) -> Res<c::PodDetail> {
    let (out, code) = run(&st, host.as_ref(), c::kube_pod_argv(&context, &namespace, &pod)?).await?;
    if code != 0 {
        return Err(c::kube_error(&out, code, &place(host.as_ref())));
    }
    c::parse_pod_detail(&out)
}

/// The last `tail` log lines of a pod's container; `previous`: of its last run before a restart.
#[tauri::command]
#[allow(clippy::too_many_arguments)] // flat IPC arguments
pub async fn kube_logs(
    st: State<'_>,
    host: Option<Host>,
    context: String,
    namespace: String,
    pod: String,
    container: String,
    tail: u32,
    previous: bool,
) -> Res<String> {
    let argv = c::kube_logs_argv(&context, &namespace, &pod, &container, tail, previous)?;
    let (out, code) = run(&st, host.as_ref(), argv).await?;
    if code != 0 {
        return Err(c::kube_error(&out, code, &place(host.as_ref())));
    }
    Ok(c::sort_logs(&out))
}

/// Volumes and images with their sizes (`docker system df --verbose`).
#[tauri::command]
pub async fn docker_storage(st: State<'_>, host: Option<Host>) -> Res<c::Storage> {
    let (out, code) = run(&st, host.as_ref(), c::docker_storage_argv()).await?;
    if code != 0 {
        return Err(c::docker_error(&out, code, &place(host.as_ref())));
    }
    c::parse_docker_storage(&out)
}

/// Remove dangling images (`docker image prune --force`).
#[tauri::command]
pub async fn docker_prune_dangling(st: State<'_>, host: Option<Host>) -> Res<c::PruneResult> {
    let (out, code) = run(&st, host.as_ref(), c::docker_prune_dangling_argv()).await?;
    if code != 0 {
        return Err(c::docker_error(&out, code, &place(host.as_ref())));
    }
    Ok(c::parse_prune(&out))
}

/// Result of removing a list of images: which failed and why, and the space Docker counts as
/// freed (its image total before minus after).
#[derive(serde::Serialize)]
pub struct RemoveResult {
    pub removed: Vec<String>,
    /// `[image, reason]`
    pub failed: Vec<(String, String)>,
    pub freed_bytes: u64,
}

/// Remove exactly these images, one at a time and without `--force`, so an image that a
/// container started using since the list was shown is kept.
#[tauri::command]
pub async fn docker_remove_images(
    st: State<'_>,
    host: Option<Host>,
    images: Vec<String>,
) -> Res<RemoveResult> {
    let where_ = place(host.as_ref());
    let measure =
        |out: &(String, i32)| if out.1 == 0 { c::images_bytes(&c::parse_docker_df(&out.0)) } else { 0 };
    let before = run(&st, host.as_ref(), c::docker_df_argv()).await.map(|o| measure(&o)).unwrap_or(0);
    let mut res = RemoveResult { removed: vec![], failed: vec![], freed_bytes: 0 };
    for image in images {
        match run(&st, host.as_ref(), c::docker_image_rm_argv(&image)?).await {
            Ok((_, 0)) => res.removed.push(image),
            Ok((out, code)) => res.failed.push((image, c::image_rm_error(&out, code, &where_))),
            Err(e) => res.failed.push((image, e)),
        }
    }
    if !res.removed.is_empty() {
        let after = run(&st, host.as_ref(), c::docker_df_argv()).await.map(|o| measure(&o)).unwrap_or(before);
        res.freed_bytes = before.saturating_sub(after);
    }
    Ok(res)
}

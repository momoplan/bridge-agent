use bridge_agent::{
    connector_data_dir, load_config, show_connector, start_connector, stop_connector,
    sync_installed_connector, ConnectorLifecycleResult, ConnectorProcessOwnership,
    ConnectorStartResult, ServiceStartCommand,
};
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use tokio::process::{Child, Command};
use tokio::sync::{oneshot, watch, Mutex};
use tokio::time::{timeout, Duration};

#[cfg(unix)]
use std::os::unix::process::CommandExt as _;
#[cfg(windows)]
const WINDOWS_CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
#[cfg(windows)]
const WINDOWS_CREATE_NO_WINDOW: u32 = 0x0800_0000;
const GRACEFUL_STOP_TIMEOUT: Duration = Duration::from_secs(5);
const FORCE_STOP_TIMEOUT: Duration = Duration::from_secs(15);
const CONNECTOR_RUNTIME_LOG_MAX_BYTES: u64 = 5 * 1024 * 1024;

#[derive(Clone, Default)]
pub(crate) struct ConnectorProcessManager {
    inner: Arc<Mutex<HashMap<String, ManagedConnectorHandle>>>,
    next_generation: Arc<AtomicU64>,
}

struct ManagedConnectorHandle {
    generation: u64,
    pid: u32,
    stop_tx: Option<oneshot::Sender<()>>,
    exit_rx: watch::Receiver<Option<ManagedConnectorExit>>,
}

#[derive(Clone, Debug)]
struct ManagedConnectorExit {
    code: Option<i32>,
    detail: String,
}

impl ConnectorProcessManager {
    pub(crate) async fn start(
        &self,
        connector_id: &str,
        config_path: &Path,
    ) -> Result<ConnectorStartResult, String> {
        let connector_id = connector_id.trim().to_string();
        if connector_id.is_empty() {
            return Err("本地应用 ID 不能为空".to_string());
        }
        sync_installed_connector(config_path, &connector_id).map_err(|err| err.to_string())?;
        let record = show_connector(&connector_id).map_err(|err| err.to_string())?;
        if record
            .manifest
            .runtime
            .as_ref()
            .is_none_or(|runtime| runtime.process_ownership != ConnectorProcessOwnership::Host)
        {
            return run_legacy_start(connector_id, config_path.to_path_buf()).await;
        }

        {
            let mut processes = self.inner.lock().await;
            if let Some(handle) = processes.get(&connector_id) {
                if handle.exit_rx.borrow().is_none() {
                    return Ok(lifecycle_result(
                        &connector_id,
                        Some(0),
                        format!("Bridge Agent 已托管该进程（PID {}）", handle.pid),
                        String::new(),
                    ));
                }
            }
            processes.remove(&connector_id);
        }

        // The process map is intentionally in-memory. After a host crash or an
        // interrupted upgrade, a previous connector process can still be alive
        // even though this Bridge Agent instance has no handle for it. The host
        // ownership contract requires an idempotent stop command, so reconcile
        // that external state before starting the one process we will supervise.
        let cleanup = run_legacy_stop(connector_id.clone(), config_path.to_path_buf()).await?;
        ensure_lifecycle_succeeded(&connector_id, "启动前清理遗留进程", &cleanup)?;

        let command = resolved_start_command(config_path, &connector_id)?;
        let data_dir = connector_data_dir(&connector_id).map_err(|err| err.to_string())?;
        fs::create_dir_all(&data_dir)
            .map_err(|err| format!("创建本地应用运行目录 {} 失败: {err}", data_dir.display()))?;
        let stdout = append_log_file(&data_dir.join("runtime.stdout.log"))?;
        let stderr = append_log_file(&data_dir.join("runtime.stderr.log"))?;
        let mut child = spawn_foreground_process(&connector_id, command, stdout, stderr)?;
        let pid = child
            .id()
            .ok_or_else(|| format!("本地应用 `{connector_id}` 启动后没有进程 ID"))?;
        let generation = self.next_generation.fetch_add(1, Ordering::Relaxed) + 1;
        let (stop_tx, stop_rx) = oneshot::channel();
        let (exit_tx, exit_rx) = watch::channel(None);
        self.inner.lock().await.insert(
            connector_id.clone(),
            ManagedConnectorHandle {
                generation,
                pid,
                stop_tx: Some(stop_tx),
                exit_rx,
            },
        );

        let inner = Arc::clone(&self.inner);
        let supervised_id = connector_id.clone();
        tauri::async_runtime::spawn(async move {
            let exit = supervise_process(&mut child, pid, stop_rx).await;
            let _ = exit_tx.send(Some(exit));
            let mut processes = inner.lock().await;
            if processes
                .get(&supervised_id)
                .is_some_and(|handle| handle.generation == generation)
            {
                processes.remove(&supervised_id);
            }
        });

        Ok(lifecycle_result(
            &connector_id,
            Some(0),
            format!("Bridge Agent 已启动并托管进程（PID {pid}）"),
            String::new(),
        ))
    }

    pub(crate) async fn stop(
        &self,
        connector_id: &str,
        config_path: &Path,
    ) -> Result<ConnectorStartResult, String> {
        let connector_id = connector_id.trim().to_string();
        let record = show_connector(&connector_id).map_err(|err| err.to_string())?;
        if record
            .manifest
            .runtime
            .as_ref()
            .is_none_or(|runtime| runtime.process_ownership != ConnectorProcessOwnership::Host)
        {
            return run_legacy_stop(connector_id, config_path.to_path_buf()).await;
        }

        let Some(mut handle) = self.inner.lock().await.remove(&connector_id) else {
            // The connector shutdown command is deliberately idempotent. Running it also
            // cleans up a process left by an older Bridge Agent version.
            let result = run_legacy_stop(connector_id.clone(), config_path.to_path_buf()).await?;
            ensure_lifecycle_succeeded(&connector_id, "清理未托管的遗留进程", &result)?;
            return Ok(result);
        };

        let graceful = run_legacy_stop(connector_id.clone(), config_path.to_path_buf()).await;
        if wait_for_exit(&mut handle.exit_rx, GRACEFUL_STOP_TIMEOUT)
            .await
            .is_none()
        {
            if let Some(stop_tx) = handle.stop_tx.take() {
                let _ = stop_tx.send(());
            }
        }
        let exit = wait_for_exit(&mut handle.exit_rx, FORCE_STOP_TIMEOUT)
            .await
            .ok_or_else(|| {
                format!(
                    "Bridge Agent 无法在超时时间内停止本地应用 `{connector_id}` 的进程树（PID {}）",
                    handle.pid
                )
            })?;
        let graceful_stderr = match graceful {
            Ok(result) if result.lifecycle.configured && result.lifecycle.exit_code == Some(0) => {
                String::new()
            }
            Ok(result) => {
                let detail = if !result.lifecycle.stderr.trim().is_empty() {
                    result.lifecycle.stderr.trim().to_string()
                } else {
                    format!("退出码 {:?}", result.lifecycle.exit_code)
                };
                format!("优雅停止命令失败，已由宿主回收进程树: {detail}")
            }
            Err(err) => format!("优雅停止命令失败，已由宿主回收进程树: {err}"),
        };
        Ok(lifecycle_result(
            &connector_id,
            Some(0),
            format!("{}；原始退出码 {:?}", exit.detail, exit.code),
            graceful_stderr,
        ))
    }

    pub(crate) async fn stop_if_managed(
        &self,
        connector_id: &str,
        config_path: &Path,
    ) -> Result<bool, String> {
        let record = match show_connector(connector_id) {
            Ok(record) => record,
            Err(_) => return Ok(false),
        };
        let managed =
            record.manifest.runtime.as_ref().is_some_and(|runtime| {
                runtime.process_ownership == ConnectorProcessOwnership::Host
            });
        if managed {
            self.stop(connector_id, config_path).await?;
        }
        Ok(managed)
    }

    pub(crate) async fn stop_all(&self, config_path: &Path) -> Vec<String> {
        let connector_ids = self.inner.lock().await.keys().cloned().collect::<Vec<_>>();
        let mut failures = Vec::new();
        for connector_id in connector_ids {
            if let Err(err) = self.stop(&connector_id, config_path).await {
                failures.push(format!("{connector_id}: {err}"));
            }
        }
        failures
    }
}

fn ensure_lifecycle_succeeded(
    connector_id: &str,
    action: &str,
    result: &ConnectorStartResult,
) -> Result<(), String> {
    if result.lifecycle.configured && result.lifecycle.exit_code == Some(0) {
        return Ok(());
    }
    let detail = if !result.lifecycle.configured {
        "命令未配置".to_string()
    } else if !result.lifecycle.stderr.trim().is_empty() {
        result.lifecycle.stderr.trim().to_string()
    } else {
        format!("退出码 {:?}", result.lifecycle.exit_code)
    };
    Err(format!("本地应用 `{connector_id}` {action}失败: {detail}"))
}

fn resolved_start_command(
    config_path: &Path,
    connector_id: &str,
) -> Result<ServiceStartCommand, String> {
    let config = load_config(config_path).map_err(|err| err.to_string())?;
    config
        .local_apps
        .into_iter()
        .find(|app| app.connector_id == connector_id)
        .and_then(|app| app.start_command)
        .ok_or_else(|| format!("本地应用 `{connector_id}` 没有配置前台启动命令"))
}

fn append_log_file(path: &Path) -> Result<std::fs::File, String> {
    if path
        .metadata()
        .map(|metadata| metadata.len() >= CONNECTOR_RUNTIME_LOG_MAX_BYTES)
        .unwrap_or(false)
    {
        let archived = path.with_extension("log.1");
        if archived.exists() {
            fs::remove_file(&archived).map_err(|err| {
                format!("删除旧的本地应用日志 {} 失败: {err}", archived.display())
            })?;
        }
        fs::rename(path, &archived).map_err(|err| {
            format!(
                "轮转本地应用日志 {} 到 {} 失败: {err}",
                path.display(),
                archived.display()
            )
        })?;
    }
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|err| format!("打开本地应用日志 {} 失败: {err}", path.display()))
}

fn spawn_foreground_process(
    connector_id: &str,
    start_command: ServiceStartCommand,
    stdout: std::fs::File,
    stderr: std::fs::File,
) -> Result<Child, String> {
    let ServiceStartCommand::ShellCommand {
        command,
        cwd,
        env,
        timeout_secs: _,
    } = start_command;
    if command.is_empty() || command[0].trim().is_empty() {
        return Err(format!("本地应用 `{connector_id}` 的前台启动命令为空"));
    }
    let mut process = Command::new(&command[0]);
    process
        .args(command.iter().skip(1))
        .envs(env)
        .env("BAIJIMU_CONNECTOR_PROCESS_OWNER", "bridge-agent")
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    if let Some(cwd) = cwd.as_deref().map(str::trim).filter(|cwd| !cwd.is_empty()) {
        process.current_dir(cwd);
    }
    #[cfg(unix)]
    process.as_std_mut().process_group(0);
    #[cfg(windows)]
    process.creation_flags(WINDOWS_CREATE_NO_WINDOW | WINDOWS_CREATE_NEW_PROCESS_GROUP);
    process
        .spawn()
        .map_err(|err| format!("启动本地应用 `{connector_id}` 的前台进程失败: {err}"))
}

async fn supervise_process(
    child: &mut Child,
    pid: u32,
    mut stop_rx: oneshot::Receiver<()>,
) -> ManagedConnectorExit {
    tokio::select! {
        status = child.wait() => exit_from_wait(status),
        _ = &mut stop_rx => terminate_process_tree(child, pid).await,
    }
}

async fn terminate_process_tree(child: &mut Child, pid: u32) -> ManagedConnectorExit {
    #[cfg(unix)]
    {
        signal_unix_process_group(pid, libc::SIGTERM);
        if let Ok(status) = timeout(GRACEFUL_STOP_TIMEOUT, child.wait()).await {
            return exit_from_wait(status);
        }
        signal_unix_process_group(pid, libc::SIGKILL);
    }
    #[cfg(windows)]
    {
        let mut taskkill = Command::new("taskkill.exe");
        taskkill.creation_flags(WINDOWS_CREATE_NO_WINDOW);
        let _ = taskkill
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .output()
            .await;
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = pid;
        let _ = child.start_kill();
    }
    match timeout(Duration::from_secs(3), child.wait()).await {
        Ok(status) => exit_from_wait(status),
        Err(_) => {
            let _ = child.start_kill();
            let status = child.wait().await;
            exit_from_wait(status)
        }
    }
}

#[cfg(unix)]
fn signal_unix_process_group(pid: u32, signal: libc::c_int) {
    let Ok(process_group) = libc::pid_t::try_from(pid) else {
        return;
    };
    // `killpg(2)` has identical semantics on macOS and Linux and avoids the
    // platform-specific command-line parsing of negative PIDs by /bin/kill.
    unsafe {
        libc::killpg(process_group, signal);
    }
}

fn exit_from_wait(result: std::io::Result<std::process::ExitStatus>) -> ManagedConnectorExit {
    match result {
        Ok(status) => ManagedConnectorExit {
            code: status.code(),
            detail: format!("宿主管理的进程已退出（{status}）"),
        },
        Err(err) => ManagedConnectorExit {
            code: None,
            detail: format!("等待宿主管理的进程退出失败: {err}"),
        },
    }
}

async fn wait_for_exit(
    exit_rx: &mut watch::Receiver<Option<ManagedConnectorExit>>,
    wait: Duration,
) -> Option<ManagedConnectorExit> {
    if let Some(exit) = exit_rx.borrow().clone() {
        return Some(exit);
    }
    if timeout(wait, exit_rx.changed()).await.is_err() {
        return None;
    }
    exit_rx.borrow().clone()
}

async fn run_legacy_start(
    connector_id: String,
    config_path: PathBuf,
) -> Result<ConnectorStartResult, String> {
    tokio::task::spawn_blocking(move || start_connector(&connector_id, &config_path))
        .await
        .map_err(|err| format!("启动本地应用任务失败: {err}"))?
        .map_err(|err| err.to_string())
}

async fn run_legacy_stop(
    connector_id: String,
    config_path: PathBuf,
) -> Result<ConnectorStartResult, String> {
    tokio::task::spawn_blocking(move || stop_connector(&connector_id, &config_path))
        .await
        .map_err(|err| format!("停止本地应用任务失败: {err}"))?
        .map_err(|err| err.to_string())
}

fn lifecycle_result(
    connector_id: &str,
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
) -> ConnectorStartResult {
    ConnectorStartResult {
        connector_id: connector_id.to_string(),
        lifecycle: ConnectorLifecycleResult {
            connector_id: connector_id.to_string(),
            configured: true,
            exit_code,
            stdout,
            stderr,
        },
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::{spawn_foreground_process, supervise_process};
    use bridge_agent::ServiceStartCommand;
    use std::collections::BTreeMap;
    use std::fs::OpenOptions;
    use tokio::sync::oneshot;
    use tokio::time::{timeout, Duration};

    #[tokio::test]
    async fn supervisor_stops_the_complete_foreground_process_group() {
        let temporary = tempfile::tempdir().unwrap();
        let output = temporary.path().join("runtime.log");
        let stdout = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&output)
            .unwrap();
        let stderr = stdout.try_clone().unwrap();
        let command = ServiceStartCommand::ShellCommand {
            command: vec![
                "sh".to_string(),
                "-c".to_string(),
                "trap 'exit 0' TERM; while :; do sleep 1; done".to_string(),
            ],
            cwd: None,
            env: BTreeMap::new(),
            timeout_secs: None,
        };
        let mut child =
            spawn_foreground_process("com.baijimu.connector.test", command, stdout, stderr)
                .unwrap();
        let pid = child.id().unwrap();
        let (stop_tx, stop_rx) = oneshot::channel();
        let supervised =
            tokio::spawn(async move { supervise_process(&mut child, pid, stop_rx).await });

        stop_tx.send(()).unwrap();
        let exit = timeout(Duration::from_secs(10), supervised)
            .await
            .expect("supervisor should stop the process group")
            .unwrap();
        // Unix reports signal-based termination without a numeric exit code.
        // The ownership contract only requires the complete process group to exit.
        assert!(exit.detail.contains("进程已退出"), "{}", exit.detail);
    }
}

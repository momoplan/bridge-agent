fn lifecycle_succeeded(result: &ConnectorStartResult) -> bool {
    result.lifecycle.configured && result.lifecycle.exit_code == Some(0)
}

fn resolved_start_command(config_path: &Path, app_id: &str) -> Result<ServiceStartCommand, String> {
    let config = load_config(config_path).map_err(|err| err.to_string())?;
    config
        .local_apps
        .into_iter()
        .find(|app| app.app_id == app_id)
        .and_then(|app| app.start_command)
        .ok_or_else(|| format!("本地应用 `{app_id}` 没有配置前台启动命令"))
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
    app_id: &str,
    start_command: ServiceStartCommand,
    stdout: std::fs::File,
    stderr: std::fs::File,
) -> Result<Child, String> {
    let ServiceStartCommand::ShellCommand {
        command,
        cwd,
        mut env,
        timeout_secs: _,
    } = start_command;
    if command.is_empty() || command[0].trim().is_empty() {
        return Err(format!("本地应用 `{app_id}` 的前台启动命令为空"));
    }
    enrich_user_command_environment(command.first().map(String::as_str), &mut env);
    let mut process = Command::new(&command[0]);
    process
        .args(command.iter().skip(1))
        .envs(env)
        .env("BAIJIMU_LOCAL_APP_PROCESS_OWNER", "bridge-agent")
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
        .map_err(|err| format!("启动本地应用 `{app_id}` 的前台进程失败: {err}"))
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
    app_id: String,
    config_path: PathBuf,
    dependency_env: std::collections::BTreeMap<String, String>,
) -> Result<ConnectorStartResult, String> {
    tokio::task::spawn_blocking(move || {
        start_connector_with_env(&app_id, &config_path, &dependency_env)
    })
    .await
    .map_err(|err| format!("启动本地应用任务失败: {err}"))?
    .map_err(|err| err.to_string())
}

fn inject_dependency_env(
    command: &mut ServiceStartCommand,
    dependency_env: std::collections::BTreeMap<String, String>,
) {
    let ServiceStartCommand::ShellCommand { env, .. } = command;
    env.extend(dependency_env);
}

async fn run_legacy_stop(
    app_id: String,
    config_path: PathBuf,
) -> Result<ConnectorStartResult, String> {
    tokio::task::spawn_blocking(move || stop_connector(&app_id, &config_path))
        .await
        .map_err(|err| format!("停止本地应用任务失败: {err}"))?
        .map_err(|err| err.to_string())
}

fn lifecycle_result(
    app_id: &str,
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
) -> ConnectorStartResult {
    ConnectorStartResult {
        app_id: app_id.to_string(),
        lifecycle: ConnectorLifecycleResult {
            app_id: app_id.to_string(),
            configured: true,
            exit_code,
            stdout,
            stderr,
        },
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::{inject_dependency_env, spawn_foreground_process, supervise_process};
    use bridge_agent::ServiceStartCommand;
    use std::collections::BTreeMap;
    use std::fs::OpenOptions;
    use tokio::sync::oneshot;
    use tokio::time::{timeout, Duration};

    #[test]
    fn host_dependency_environment_overrides_manifest_environment() {
        let mut command = ServiceStartCommand::ShellCommand {
            command: vec!["example".to_string()],
            cwd: None,
            env: BTreeMap::from([(
                "CODEX_CONNECTOR_BAIJIMU_BINARY".to_string(),
                "relative-baijimu".to_string(),
            )]),
            timeout_secs: None,
        };
        inject_dependency_env(
            &mut command,
            BTreeMap::from([(
                "CODEX_CONNECTOR_BAIJIMU_BINARY".to_string(),
                "/absolute/managed/baijimu".to_string(),
            )]),
        );
        let ServiceStartCommand::ShellCommand { env, .. } = command;
        assert_eq!(
            env.get("CODEX_CONNECTOR_BAIJIMU_BINARY")
                .map(String::as_str),
            Some("/absolute/managed/baijimu")
        );
    }

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

    #[tokio::test]
    async fn foreground_process_receives_the_current_user_command_path() {
        let temporary = tempfile::tempdir().unwrap();
        let output = temporary.path().join("resolved-command.txt");
        let stdout = OpenOptions::new()
            .create(true)
            .append(true)
            .open(temporary.path().join("runtime.stdout.log"))
            .unwrap();
        let stderr = OpenOptions::new()
            .create(true)
            .append(true)
            .open(temporary.path().join("runtime.stderr.log"))
            .unwrap();
        let command = ServiceStartCommand::ShellCommand {
            command: vec![
                "/bin/sh".to_string(),
                "-c".to_string(),
                "command -v env > \"$RESULT_FILE\"".to_string(),
            ],
            cwd: None,
            env: BTreeMap::from([
                ("PATH".to_string(), "/connector-only".to_string()),
                ("RESULT_FILE".to_string(), output.display().to_string()),
            ]),
            timeout_secs: None,
        };

        let mut child =
            spawn_foreground_process("com.baijimu.connector.path-test", command, stdout, stderr)
                .unwrap();
        let status = child.wait().await.unwrap();
        assert!(status.success());
        let resolved = std::fs::read_to_string(output).unwrap();
        assert!(resolved.trim().ends_with("/env"), "{resolved}");
    }
}

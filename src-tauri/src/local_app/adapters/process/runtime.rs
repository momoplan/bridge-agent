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
) -> Result<(Child, bridge_agent::process_tree::Tree), String> {
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
    process.kill_on_drop(true);
    bridge_agent::process_tree::configure(process.as_std_mut());
    let mut child = process
        .spawn()
        .map_err(|err| format!("启动本地应用 `{app_id}` 的前台进程失败: {err}"))?;
    let tree = bridge_agent::process_tree::Tree::attach(child.id().expect("spawned child has pid"))
        .map_err(|err| {
            let _ = child.start_kill();
            format!("纳管本地应用进程树失败: {err}")
        })?;
    Ok((child, tree))
}

async fn supervise_process(
    child: &mut Child,
    tree: bridge_agent::process_tree::Tree,
    mut stop_rx: oneshot::Receiver<()>,
) -> ManagedConnectorExit {
    let status = tokio::select! {
        status = child.wait() => status,
        _ = &mut stop_rx => {
            // Termination itself never launches a second unbounded subprocess.
            let _ = tree.terminate();
            match timeout(Duration::from_secs(2), child.wait()).await {
                Ok(status) => status,
                Err(_) => Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "exit not yet confirmed")),
            }
        }
    };
    // This is a background supervisor, not the stop request. A failed wait must
    // not remove a still-owned process from the registry. Callers wait with a
    // deadline and can report stopping/timeout while supervision continues.
    let mut status = status;
    loop {
        if let Ok(exit) = &status {
            if tree.finish().is_ok() {
                return ManagedConnectorExit {
                    code: exit.code(),
                    detail: format!("宿主管理的进程已退出（{exit}）"),
                };
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        let _ = tree.terminate();
        status = match timeout(Duration::from_secs(2), child.wait()).await {
            Ok(status) => status,
            Err(_) => Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "exit not yet confirmed",
            )),
        };
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
        let (mut child, tree) =
            spawn_foreground_process("com.baijimu.connector.test", command, stdout, stderr)
                .unwrap();
        let (stop_tx, stop_rx) = oneshot::channel();
        let supervised =
            tokio::spawn(async move { supervise_process(&mut child, tree, stop_rx).await });

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

        let (mut child, tree) =
            spawn_foreground_process("com.baijimu.connector.path-test", command, stdout, stderr)
                .unwrap();
        let status = child.wait().await.unwrap();
        tree.terminate().unwrap();
        assert!(status.success());
        let resolved = std::fs::read_to_string(output).unwrap();
        assert!(resolved.trim().ends_with("/env"), "{resolved}");
    }
}

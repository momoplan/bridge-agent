fn run_start_command(
    app_id: &str,
    command: &ServiceStartCommand,
    additional_env: &BTreeMap<String, String>,
) -> Result<ConnectorLifecycleResult> {
    let user_path = current_user_command_path();
    run_start_command_with_user_path(app_id, command, additional_env, user_path.as_deref())
}

fn run_start_command_with_user_path(
    app_id: &str,
    command: &ServiceStartCommand,
    additional_env: &BTreeMap<String, String>,
    user_path: Option<&str>,
) -> Result<ConnectorLifecycleResult> {
    match command {
        ServiceStartCommand::ShellCommand {
            command,
            cwd,
            env,
            timeout_secs,
        } => {
            if command.is_empty() {
                bail!("lifecycle command for local app `{app_id}` is empty");
            }
            let mut lifecycle_env = env.clone();
            lifecycle_env.extend(additional_env.clone());
            enrich_user_command_environment_with_path(
                command.first().map(String::as_str),
                &mut lifecycle_env,
                user_path,
            );

            let mut child = Command::new(&command[0]);
            configure_connector_command(&mut child);
            child.args(&command[1..]);
            if let Some(cwd) = cwd.as_deref().filter(|value| !value.trim().is_empty()) {
                child.current_dir(cwd);
            }
            child.envs(&lifecycle_env);
            let stdout_capture = tempfile::NamedTempFile::new().with_context(|| {
                format!("failed to create stdout capture for local app `{app_id}`")
            })?;
            let stderr_capture = tempfile::NamedTempFile::new().with_context(|| {
                format!("failed to create stderr capture for local app `{app_id}`")
            })?;
            child
                .stdout(Stdio::from(stdout_capture.reopen().with_context(|| {
                    format!("failed to open stdout capture for local app `{app_id}`")
                })?))
                .stderr(Stdio::from(stderr_capture.reopen().with_context(|| {
                    format!("failed to open stderr capture for local app `{app_id}`")
                })?));
            let mut child = child.spawn().with_context(|| {
                format!("failed to run lifecycle command for local app `{app_id}`")
            })?;
            let deadline = Instant::now() + Duration::from_secs(timeout_secs.unwrap_or(20).max(1));
            let (status, timed_out) = loop {
                if let Some(status) = child.try_wait().with_context(|| {
                    format!("failed to wait for local app `{app_id}` lifecycle command")
                })? {
                    break (status, false);
                }
                if Instant::now() >= deadline {
                    terminate_lifecycle_process_tree(&mut child);
                    let status = child.wait().with_context(|| {
                        format!("failed to reap timed out local app `{app_id}` command")
                    })?;
                    break (status, true);
                }
                std::thread::sleep(Duration::from_millis(50));
            };
            // Lifecycle launchers may daemonize a descendant. On Windows that
            // descendant can inherit every inheritable stdio handle even when its
            // own output is redirected. A pipe therefore cannot be drained by
            // waiting for EOF: the launcher has exited, but the descendant still
            // owns the write end. Regular files make launcher completion and
            // output collection independent, and the bounded read prevents an
            // untrusted lifecycle command from exhausting host memory.
            let stdout = read_lifecycle_capture(app_id, "stdout", &stdout_capture)?;
            let mut stderr = read_lifecycle_capture(app_id, "stderr", &stderr_capture)?;
            if timed_out {
                stderr.extend_from_slice(
                    format!(
                        "\nlifecycle command timed out after {} seconds",
                        timeout_secs.unwrap_or(20).max(1)
                    )
                    .as_bytes(),
                );
            }
            Ok(ConnectorLifecycleResult {
                app_id: app_id.to_string(),
                configured: true,
                exit_code: if timed_out { None } else { status.code() },
                stdout: String::from_utf8_lossy(&stdout).to_string(),
                stderr: String::from_utf8_lossy(&stderr).to_string(),
            })
        }
    }
}

fn read_lifecycle_capture(
    app_id: &str,
    stream_name: &str,
    capture: &tempfile::NamedTempFile,
) -> Result<Vec<u8>> {
    let file = capture.reopen().with_context(|| {
        format!("failed to read {stream_name} capture for local app `{app_id}`")
    })?;
    let mut bytes = Vec::new();
    file.take(LIFECYCLE_OUTPUT_MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .with_context(|| format!("failed to collect {stream_name} for local app `{app_id}`"))?;
    if bytes.len() as u64 > LIFECYCLE_OUTPUT_MAX_BYTES {
        bytes.truncate(LIFECYCLE_OUTPUT_MAX_BYTES as usize);
        bytes.extend_from_slice(b"\n[output truncated by Bridge Agent]");
    }
    Ok(bytes)
}

fn terminate_lifecycle_process_tree(child: &mut std::process::Child) {
    #[cfg(windows)]
    {
        let mut terminate = Command::new("taskkill.exe");
        configure_connector_command(&mut terminate);
        let _ = terminate
            .args(["/PID", &child.id().to_string(), "/T", "/F"])
            .output();
    }
    #[cfg(not(windows))]
    {
        let _ = child.kill();
    }
}

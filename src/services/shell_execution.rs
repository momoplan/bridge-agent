async fn run_shell_command(
    prepared: PreparedShellExec,
    cancel_rx: Option<oneshot::Receiver<()>>,
    live_output: Option<ShellLiveOutput>,
) -> CompletedShellExec {
    let started = Instant::now();
    let completed_at = || current_epoch_ms();

    let mut command = Command::new(&prepared.command_args[0]);
    #[cfg(windows)]
    command.creation_flags(WINDOWS_CREATE_NO_WINDOW);
    command
        .args(prepared.command_args.iter().skip(1))
        .current_dir(&prepared.cwd)
        .env_clear()
        .envs(prepared.env)
        .stdin(if prepared.stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(err) => {
            let executable = &prepared.command_args[0];
            let message = format!(
                "failed to spawn `{executable}` in {}: {err}",
                prepared.cwd.display()
            );
            let stderr = if err.kind() == ErrorKind::NotFound {
                format!("{message}\nPATH={}", prepared.path_for_diagnostics)
            } else {
                message.clone()
            };

            return CompletedShellExec {
                status: ShellExecutionStatus::Failed,
                exit_code: None,
                stdout: String::new(),
                stderr,
                timed_out: false,
                success: false,
                error: Some(InvokeError {
                    code: "COMMAND_SPAWN_FAILED".to_string(),
                    message,
                }),
                duration_ms: started.elapsed().as_millis() as u64,
                completed_at_epoch_ms: completed_at(),
            };
        }
    };

    if let Some(stdin) = prepared.stdin {
        if let Some(mut child_stdin) = child.stdin.take() {
            tokio::spawn(async move {
                let _ = child_stdin.write_all(stdin.as_bytes()).await;
            });
        }
    }

    wait_for_shell_command(
        child,
        prepared.timeout_secs,
        cancel_rx,
        started,
        live_output,
    )
    .await
}

async fn wait_for_shell_command(
    mut child: Child,
    timeout_secs: Option<u64>,
    cancel_rx: Option<oneshot::Receiver<()>>,
    started: Instant,
    live_output: Option<ShellLiveOutput>,
) -> CompletedShellExec {
    let stdout_task = child.stdout.take().map(|stdout| {
        let live_output = live_output.clone();
        tokio::spawn(async move {
            read_shell_output(stdout, live_output, ShellOutputStream::Stdout).await
        })
    });
    let stderr_task = child.stderr.take().map(|stderr| {
        tokio::spawn(async move {
            read_shell_output(stderr, live_output, ShellOutputStream::Stderr).await
        })
    });

    let wait_result = await_shell_exit(&mut child, timeout_secs, cancel_rx).await;

    let stdout = join_output(stdout_task).await;
    let stderr = join_output(stderr_task).await;
    complete_shell_wait(wait_result, stdout, stderr, started)
}

async fn await_shell_exit(
    child: &mut Child,
    timeout_secs: Option<u64>,
    cancel_rx: Option<oneshot::Receiver<()>>,
) -> ShellWaitResult {
    match (timeout_secs, cancel_rx) {
        (Some(timeout_secs), Some(mut cancel_rx)) => {
            tokio::select! {
                result = child.wait() => ShellWaitResult::Exited(result),
                _ = sleep(Duration::from_secs(timeout_secs)) => {
                    let _ = child.kill().await;
                    ShellWaitResult::TimedOut(timeout_secs, child.wait().await)
                }
                _ = &mut cancel_rx => {
                    let _ = child.kill().await;
                    ShellWaitResult::Canceled(child.wait().await)
                }
            }
        }
        (Some(timeout_secs), None) => {
            tokio::select! {
                result = child.wait() => ShellWaitResult::Exited(result),
                _ = sleep(Duration::from_secs(timeout_secs)) => {
                    let _ = child.kill().await;
                    ShellWaitResult::TimedOut(timeout_secs, child.wait().await)
                }
            }
        }
        (None, Some(mut cancel_rx)) => {
            tokio::select! {
                result = child.wait() => ShellWaitResult::Exited(result),
                _ = &mut cancel_rx => {
                    let _ = child.kill().await;
                    ShellWaitResult::Canceled(child.wait().await)
                }
            }
        }
        (None, None) => ShellWaitResult::Exited(child.wait().await),
    }
}

fn complete_shell_wait(
    wait_result: ShellWaitResult,
    stdout: String,
    stderr: String,
    started: Instant,
) -> CompletedShellExec {
    let duration_ms = started.elapsed().as_millis() as u64;
    let completed_at_epoch_ms = current_epoch_ms();

    match wait_result {
        ShellWaitResult::Exited(Ok(status)) => CompletedShellExec {
            status: if status.success() {
                ShellExecutionStatus::Succeeded
            } else {
                ShellExecutionStatus::Failed
            },
            exit_code: status.code(),
            stdout,
            stderr,
            timed_out: false,
            success: status.success(),
            error: None,
            duration_ms,
            completed_at_epoch_ms,
        },
        ShellWaitResult::TimedOut(timeout_secs, status) => CompletedShellExec {
            status: ShellExecutionStatus::TimedOut,
            exit_code: status.ok().and_then(|status| status.code()),
            stdout,
            stderr,
            timed_out: true,
            success: false,
            error: Some(InvokeError {
                code: "TIMEOUT".to_string(),
                message: format!("timed out after {timeout_secs}s"),
            }),
            duration_ms,
            completed_at_epoch_ms,
        },
        ShellWaitResult::Canceled(status) => CompletedShellExec {
            status: ShellExecutionStatus::Canceled,
            exit_code: status.ok().and_then(|status| status.code()),
            stdout,
            stderr,
            timed_out: false,
            success: false,
            error: Some(InvokeError {
                code: "CANCELED".to_string(),
                message: "execution was canceled".to_string(),
            }),
            duration_ms,
            completed_at_epoch_ms,
        },
        ShellWaitResult::Exited(Err(err)) => CompletedShellExec {
            status: ShellExecutionStatus::Failed,
            exit_code: None,
            stdout,
            stderr,
            timed_out: false,
            success: false,
            error: Some(InvokeError {
                code: "COMMAND_WAIT_FAILED".to_string(),
                message: err.to_string(),
            }),
            duration_ms,
            completed_at_epoch_ms,
        },
    }
}

enum ShellWaitResult {
    Exited(std::io::Result<std::process::ExitStatus>),
    TimedOut(u64, std::io::Result<std::process::ExitStatus>),
    Canceled(std::io::Result<std::process::ExitStatus>),
}

async fn read_shell_output<R>(
    mut stream: R,
    live_output: Option<ShellLiveOutput>,
    stream_kind: ShellOutputStream,
) -> Vec<u8>
where
    R: AsyncRead + Unpin,
{
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 8192];

    loop {
        match stream.read(&mut chunk).await {
            Ok(0) => break,
            Ok(read) => {
                buffer.extend_from_slice(&chunk[..read]);
                if let Some(live_output) = live_output.as_ref() {
                    let text = String::from_utf8_lossy(&chunk[..read]);
                    live_output
                        .store
                        .append_output(&live_output.execution_id, stream_kind, &text)
                        .await;
                }
            }
            Err(_) => break,
        }
    }

    buffer
}

async fn join_output(task: Option<tokio::task::JoinHandle<Vec<u8>>>) -> String {
    match task {
        Some(task) => String::from_utf8_lossy(&task.await.unwrap_or_default()).into_owned(),
        None => String::new(),
    }
}

fn current_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

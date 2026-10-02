async fn run_shell_command(
    prepared: PreparedShellExec,
    cancel_rx: Option<oneshot::Receiver<()>>,
    live_output: Option<ShellLiveOutput>,
) -> CompletedShellExec {
    let started = Instant::now();
    let completed_at = || current_epoch_ms();

    let mut command = Command::new(&prepared.command_args[0]);
    crate::process_tree::configure(command.as_std_mut());
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

    let tree = match crate::process_tree::Tree::attach(child.id().expect("spawned child has pid")) {
        Ok(tree) => tree,
        Err(error) => {
            let _ = child.start_kill();
            let _ = tokio::time::timeout(Duration::from_secs(2), child.wait()).await;
            return complete_shell_wait(
                ShellWaitResult::Exited(Err(std::io::Error::other(error.to_string()))),
                String::new(),
                String::new(),
                started,
            );
        }
    };
    let mut stdin_task = None;
    if let Some(stdin) = prepared.stdin {
        if let Some(mut child_stdin) = child.stdin.take() {
            stdin_task = Some(ShellIoTask(tokio::spawn(async move {
                let _ = child_stdin.write_all(stdin.as_bytes()).await;
            })));
        }
    }

    let result = wait_for_shell_command(
        child,
        tree,
        prepared.timeout_secs,
        cancel_rx,
        started,
        live_output,
    )
    .await;
    drop(stdin_task);
    result
}

async fn wait_for_shell_command(
    mut child: Child,
    tree: crate::process_tree::Tree,
    timeout_secs: Option<u64>,
    cancel_rx: Option<oneshot::Receiver<()>>,
    started: Instant,
    live_output: Option<ShellLiveOutput>,
) -> CompletedShellExec {
    let stdout_task = child.stdout.take().map(|stdout| {
        let live_output = live_output.clone();
        ShellIoTask(tokio::spawn(async move {
            read_shell_output(stdout, live_output, ShellOutputStream::Stdout).await
        }))
    });
    let stderr_task = child.stderr.take().map(|stderr| {
        ShellIoTask(tokio::spawn(async move {
            read_shell_output(stderr, live_output, ShellOutputStream::Stderr).await
        }))
    });

    let mut wait_result = await_shell_exit(&mut child, &tree, timeout_secs, cancel_rx).await;
    // A completed parent may still have descendants holding inherited pipes.
    let cleanup = tokio::task::spawn_blocking(move || tree.finish()).await;
    if let Err(error) = cleanup
        .map_err(anyhow::Error::from)
        .and_then(|result| result)
    {
        wait_result = ShellWaitResult::Exited(Err(std::io::Error::other(error.to_string())));
    }

    let (stdout, stderr) = tokio::join!(join_output(stdout_task), join_output(stderr_task));
    if let Some(error) = stdout.as_ref().err().or_else(|| stderr.as_ref().err()) {
        wait_result = ShellWaitResult::Exited(Err(std::io::Error::other(format!(
            "output collection failed: {error}"
        ))));
    }
    complete_shell_wait(
        wait_result,
        stdout.unwrap_or_default(),
        stderr.unwrap_or_default(),
        started,
    )
}

async fn await_shell_exit(
    child: &mut Child,
    tree: &crate::process_tree::Tree,
    timeout_secs: Option<u64>,
    cancel_rx: Option<oneshot::Receiver<()>>,
) -> ShellWaitResult {
    match (timeout_secs, cancel_rx) {
        (Some(timeout_secs), Some(mut cancel_rx)) => {
            tokio::select! {
                result = child.wait() => ShellWaitResult::Exited(result),
                _ = sleep(Duration::from_secs(timeout_secs)) => {
                    ShellWaitResult::TimedOut(timeout_secs, stop_shell_tree(child, tree).await)
                }
                _ = &mut cancel_rx => {
                    ShellWaitResult::Canceled(stop_shell_tree(child, tree).await)
                }
            }
        }
        (Some(timeout_secs), None) => {
            tokio::select! {
                result = child.wait() => ShellWaitResult::Exited(result),
                _ = sleep(Duration::from_secs(timeout_secs)) => {
                    ShellWaitResult::TimedOut(timeout_secs, stop_shell_tree(child, tree).await)
                }
            }
        }
        (None, Some(mut cancel_rx)) => {
            tokio::select! {
                result = child.wait() => ShellWaitResult::Exited(result),
                _ = &mut cancel_rx => {
                    ShellWaitResult::Canceled(stop_shell_tree(child, tree).await)
                }
            }
        }
        (None, None) => ShellWaitResult::Exited(child.wait().await),
    }
}

async fn stop_shell_tree(
    child: &mut Child,
    tree: &crate::process_tree::Tree,
) -> std::io::Result<std::process::ExitStatus> {
    tree.terminate()
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    tokio::time::timeout(Duration::from_secs(2), child.wait())
        .await
        .map_err(|_| {
            std::io::Error::new(
                ErrorKind::TimedOut,
                "process tree termination was requested but exit could not be confirmed",
            )
        })?
}

fn complete_shell_wait(
    wait_result: ShellWaitResult,
    stdout: String,
    stderr: String,
    started: Instant,
) -> CompletedShellExec {
    let duration_ms = started.elapsed().as_millis() as u64;
    let completed_at_epoch_ms = current_epoch_ms();

    let wait_result = match wait_result {
        ShellWaitResult::TimedOut(_, Err(error)) | ShellWaitResult::Canceled(Err(error)) => {
            ShellWaitResult::Exited(Err(error))
        }
        other => other,
    };
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
) -> std::io::Result<Vec<u8>>
where
    R: AsyncRead + Unpin,
{
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 8192];

    loop {
        match stream.read(&mut chunk).await {
            Ok(0) => break,
            Ok(read) => {
                const MAX_CAPTURE: usize = 16 * 1024 * 1024;
                if buffer.len() < MAX_CAPTURE {
                    let remaining = MAX_CAPTURE - buffer.len();
                    buffer.extend_from_slice(&chunk[..read.min(remaining)]);
                    if buffer.len() == MAX_CAPTURE {
                        buffer.extend_from_slice(b"\n[output truncated by Bridge Agent]");
                    }
                }
                if let Some(live_output) = live_output.as_ref() {
                    let text = String::from_utf8_lossy(&chunk[..read]);
                    live_output
                        .store
                        .append_output(&live_output.execution_id, stream_kind, &text)
                        .await;
                }
            }
            Err(error) => return Err(error),
        }
    }

    Ok(buffer)
}

async fn join_output(
    task: Option<ShellIoTask<std::io::Result<Vec<u8>>>>,
) -> std::io::Result<String> {
    match task {
        Some(mut task) => match tokio::time::timeout(Duration::from_secs(2), &mut task.0).await {
            Ok(result) => result
                .map_err(std::io::Error::other)?
                .map(|bytes| String::from_utf8_lossy(&bytes).into_owned()),
            Err(_) => Err(std::io::Error::new(
                ErrorKind::TimedOut,
                "output pipe remained open after process tree termination",
            )),
        },
        None => Ok(String::new()),
    }
}

fn current_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Dropping the invocation must also cancel its stdin/output tasks.
struct ShellIoTask<T>(tokio::task::JoinHandle<T>);
impl<T> Drop for ShellIoTask<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

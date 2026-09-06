pub fn terminate_runtime_lock_owner(
    lock_path: &Path,
    expected_pid: u32,
    expected_agent_id: &str,
    expected_config_path: &str,
) -> Result<()> {
    let document = read_runtime_lock(lock_path)?;
    if document.pid != expected_pid
        || document.agent_id != expected_agent_id
        || document.config_path != expected_config_path
    {
        bail!("runtime lock changed; please retry with the latest conflict information");
    }
    if document.pid == std::process::id() {
        bail!("runtime lock is owned by this 百积木 process; use the normal stop or restart action instead");
    }

    let process = describe_process(document.pid);
    if process.running {
        if !process_looks_like_bridge_agent(&process) {
            bail!(
                "pid {} is running but does not look like a 百积木 process",
                document.pid
            );
        }
        terminate_process(document.pid)?;
        wait_for_process_exit(document.pid, Duration::from_secs(5))?;
    }

    match fs::remove_file(lock_path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err).with_context(|| {
            format!(
                "failed to remove runtime lock after terminating owner {}",
                lock_path.display()
            )
        }),
    }
}

#[cfg(windows)]
fn describe_process(pid: u32) -> RuntimeProcessInfo {
    describe_process_windows(pid)
}

#[cfg(unix)]
fn describe_process(pid: u32) -> RuntimeProcessInfo {
    describe_process_unix(pid)
}

#[cfg(not(any(unix, windows)))]
fn describe_process(pid: u32) -> RuntimeProcessInfo {
    RuntimeProcessInfo {
        pid,
        parent_pid: None,
        name: None,
        executable_path: None,
        command_line: None,
        running: process_is_running(pid),
    }
}

#[cfg(windows)]
fn describe_process_windows(pid: u32) -> RuntimeProcessInfo {
    match inspect_windows_process(pid) {
        Ok(Some(process)) => RuntimeProcessInfo {
            pid,
            parent_pid: process.parent_pid,
            name: Some(process.image_name),
            executable_path: process.executable_path,
            command_line: None,
            running: true,
        },
        Ok(None) => RuntimeProcessInfo {
            pid,
            parent_pid: None,
            name: None,
            executable_path: None,
            command_line: None,
            running: false,
        },
        Err(_) => RuntimeProcessInfo {
            pid,
            parent_pid: None,
            name: None,
            executable_path: None,
            command_line: None,
            running: true,
        },
    }
}

#[cfg(unix)]
fn describe_process_unix(pid: u32) -> RuntimeProcessInfo {
    if let Ok(output) = std::process::Command::new("ps")
        .args([
            "-p",
            &pid.to_string(),
            "-o",
            "ppid=",
            "-o",
            "comm=",
            "-o",
            "args=",
        ])
        .output()
    {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if let Some(line) = stdout.lines().map(str::trim).find(|line| !line.is_empty()) {
                let mut parts = line.splitn(3, char::is_whitespace);
                let parent_pid = parts.next().and_then(|value| value.trim().parse().ok());
                let name = parts
                    .next()
                    .map(str::trim)
                    .filter(|value| !value.is_empty());
                let command_line = parts
                    .next()
                    .map(str::trim)
                    .filter(|value| !value.is_empty());
                return RuntimeProcessInfo {
                    pid,
                    parent_pid,
                    name: name.map(ToOwned::to_owned),
                    executable_path: None,
                    command_line: command_line.map(ToOwned::to_owned),
                    running: true,
                };
            }
        }
    }

    RuntimeProcessInfo {
        pid,
        parent_pid: None,
        name: None,
        executable_path: None,
        command_line: None,
        running: process_is_running(pid),
    }
}

fn process_looks_like_bridge_agent(process: &RuntimeProcessInfo) -> bool {
    process
        .name
        .as_deref()
        .is_some_and(is_bridge_agent_process_name)
        || process
            .executable_path
            .as_deref()
            .is_some_and(is_bridge_agent_process_name)
        || process
            .command_line
            .as_deref()
            .is_some_and(command_line_starts_with_bridge_agent)
}

fn command_line_starts_with_bridge_agent(command_line: &str) -> bool {
    let command_line = command_line.trim();
    if command_line.is_empty() {
        return false;
    }

    if let Some(rest) = command_line.strip_prefix('"') {
        if let Some((executable, _)) = rest.split_once('"') {
            return is_bridge_agent_process_name(executable);
        }
        return false;
    }

    command_line
        .split_whitespace()
        .next()
        .is_some_and(is_bridge_agent_process_name)
}

#[cfg(windows)]
fn terminate_process(pid: u32) -> Result<()> {
    let Some(process) = inspect_windows_process(pid)? else {
        return Ok(());
    };
    if !is_bridge_agent_process_name(&process.image_name)
        && !process
            .executable_path
            .as_deref()
            .is_some_and(is_bridge_agent_process_name)
    {
        bail!("pid {pid} changed owner before termination");
    }
    terminate_windows_process(&process)
}

#[cfg(unix)]
fn terminate_process(pid: u32) -> Result<()> {
    let _ = std::process::Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .status();
    if wait_for_process_exit(pid, Duration::from_secs(2)).is_ok() {
        return Ok(());
    }
    let status = std::process::Command::new("kill")
        .args(["-KILL", &pid.to_string()])
        .status()
        .with_context(|| format!("failed to run kill for pid {pid}"))?;
    if status.success() || !process_is_running(pid) {
        return Ok(());
    }
    bail!("failed to terminate runtime owner pid {pid}");
}

#[cfg(not(any(unix, windows)))]
fn terminate_process(pid: u32) -> Result<()> {
    bail!("terminating pid {pid} is not supported on this platform");
}

fn wait_for_process_exit(pid: u32, timeout: Duration) -> Result<()> {
    let started = SystemTime::now();
    while process_is_running(pid) {
        if started.elapsed().unwrap_or_default() >= timeout {
            bail!("runtime owner pid {pid} is still running");
        }
        std::thread::sleep(std::time::Duration::from_millis(150));
    }
    Ok(())
}

#[cfg(unix)]
fn process_is_running(pid: u32) -> bool {
    // Unix process APIs use a signed pid_t. Passing a larger u32 through the
    // `kill` utility can wrap to a negative process-group selector (notably
    // u32::MAX -> -1 on Linux) and incorrectly report an unrelated process.
    if pid == 0 || pid > i32::MAX as u32 {
        return false;
    }
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

#[cfg(windows)]
fn process_is_running(pid: u32) -> bool {
    windows_process_is_running(pid)
}

#[cfg(not(any(unix, windows)))]
fn process_is_running(_pid: u32) -> bool {
    true
}

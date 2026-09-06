async fn audit_http_request(
    State(state): State<EventServerState>,
    request: Request,
    next: Next,
) -> Response {
    let started = Instant::now();
    let method = request.method().as_str().to_string();
    let path = request.uri().path().to_string();
    let response = next.run(request).await;
    let status = response.status();
    let status_code = status.as_u16();
    let level = if status.is_server_error() {
        "error"
    } else if status.is_client_error() {
        "warn"
    } else {
        "info"
    };
    let outcome = if status.is_success() {
        "succeeded"
    } else {
        "failed"
    };

    emit_audit_log(
        &state,
        level,
        format!("local api {method} {path} -> {status_code}"),
        LogMetadata::category("local_api")
            .http(method, path, status_code)
            .duration_ms(started.elapsed().as_millis() as u64)
            .outcome(outcome),
    );

    response
}

async fn bind_event_listener(bind: SocketAddr) -> Result<TcpListener> {
    let first_err = match TcpListener::bind(bind).await {
        Ok(listener) => return Ok(listener),
        Err(err) => err,
    };

    if first_err.kind() != ErrorKind::AddrInUse {
        return Err(first_err.into());
    }

    let Some(reclaimed) = reclaim_occupied_event_port(bind).await? else {
        return Err(first_err.into());
    };

    for _ in 0..PORT_RECLAIM_BIND_RETRIES {
        match TcpListener::bind(bind).await {
            Ok(listener) => return Ok(listener),
            Err(err) if err.kind() == ErrorKind::AddrInUse => {
                sleep(PORT_RECLAIM_RETRY_DELAY).await;
            }
            Err(err) => return Err(err.into()),
        }
    }

    TcpListener::bind(bind).await.with_context(|| {
        format!(
            "local event server port is still occupied after stopping {} (pid {})",
            reclaimed.image_name, reclaimed.pid
        )
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OccupiedPortOwner {
    pid: u32,
    image_name: String,
    parent_pid: Option<u32>,
    executable_path: Option<String>,
}

async fn reclaim_occupied_event_port(bind: SocketAddr) -> Result<Option<OccupiedPortOwner>> {
    if !bind.ip().is_loopback() || bind.port() == 0 {
        return Ok(None);
    }

    let Some(owner) = find_occupied_tcp_listener(bind)? else {
        return Ok(None);
    };

    if owner.pid == std::process::id() {
        return Ok(None);
    }

    if !is_bridge_agent_process_name(&owner.image_name) {
        let process_details = owner
            .executable_path
            .as_deref()
            .map(|path| format!(", path {path}"))
            .unwrap_or_default();
        anyhow::bail!(
            "local event server port {bind} is already occupied by {} (pid {}, parent pid {:?}{}), not a 百积木 process",
            owner.image_name,
            owner.pid,
            owner.parent_pid,
            process_details
        );
    }

    terminate_process(owner.pid, &owner.image_name)?;
    Ok(Some(owner))
}

#[cfg(windows)]
fn find_occupied_tcp_listener(bind: SocketAddr) -> Result<Option<OccupiedPortOwner>> {
    let Some(pid) = find_windows_tcp_listener_pid(bind)? else {
        return Ok(None);
    };
    // The listener can exit between the TCP table snapshot and the process
    // snapshot. Treat that as a normal race and let the runtime bind retry run.
    let Some(process) = inspect_windows_process(pid)? else {
        return Ok(None);
    };
    Ok(Some(OccupiedPortOwner {
        pid,
        image_name: process.image_name,
        parent_pid: process.parent_pid,
        executable_path: process.executable_path,
    }))
}

#[cfg(unix)]
fn find_occupied_tcp_listener(bind: SocketAddr) -> Result<Option<OccupiedPortOwner>> {
    let port_filter = format!("-iTCP:{}", bind.port());
    let lsof = std::process::Command::new("lsof")
        .args(["-nP", &port_filter, "-sTCP:LISTEN", "-F", "pcn"])
        .output()
        .context("failed to inspect TCP listeners with lsof")?;
    if !lsof.status.success() {
        if lsof.stdout.is_empty() && lsof.stderr.is_empty() {
            return Ok(None);
        }
        anyhow::bail!(
            "lsof failed while inspecting local event server port: {}",
            String::from_utf8_lossy(&lsof.stderr).trim()
        );
    }

    let stdout = String::from_utf8_lossy(&lsof.stdout);
    Ok(parse_lsof_listening_owner(&stdout, bind))
}

#[cfg(not(any(windows, unix)))]
fn find_occupied_tcp_listener(_bind: SocketAddr) -> Result<Option<OccupiedPortOwner>> {
    Ok(None)
}

#[cfg(windows)]
fn terminate_process(pid: u32, image_name: &str) -> Result<()> {
    let Some(process) = inspect_windows_process(pid)? else {
        return Ok(());
    };
    if !process.image_name.eq_ignore_ascii_case(image_name) {
        anyhow::bail!(
            "pid {pid} changed owner from {image_name} to {}; refusing to terminate a reused pid",
            process.image_name
        );
    }
    terminate_windows_process(&process)
}

#[cfg(unix)]
fn terminate_process(pid: u32, image_name: &str) -> Result<()> {
    let pid_arg = pid.to_string();
    let kill = std::process::Command::new("kill")
        .args(["-TERM", &pid_arg])
        .output()
        .with_context(|| format!("failed to stop {image_name} (pid {pid}) with kill"))?;
    if !kill.status.success() {
        anyhow::bail!(
            "failed to stop {} (pid {}): {}",
            image_name,
            pid,
            String::from_utf8_lossy(&kill.stderr).trim()
        );
    }
    Ok(())
}

#[cfg(not(any(windows, unix)))]
fn terminate_process(pid: u32, image_name: &str) -> Result<()> {
    anyhow::bail!("cannot stop {image_name} (pid {pid}) on this platform")
}

#[cfg(any(unix, test))]
fn parse_lsof_listening_owner(lsof_output: &str, bind: SocketAddr) -> Option<OccupiedPortOwner> {
    #[derive(Default)]
    struct CurrentOwner {
        pid: Option<u32>,
        image_name: Option<String>,
        matches_bind: bool,
    }

    impl CurrentOwner {
        fn into_match(self) -> Option<OccupiedPortOwner> {
            if !self.matches_bind {
                return None;
            }
            Some(OccupiedPortOwner {
                pid: self.pid?,
                image_name: self.image_name?,
                parent_pid: None,
                executable_path: None,
            })
        }
    }

    let mut current = CurrentOwner::default();
    for line in lsof_output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        if let Some(pid) = line.strip_prefix('p') {
            if let Some(owner) = current.into_match() {
                return Some(owner);
            }
            current = CurrentOwner {
                pid: pid.parse().ok(),
                ..CurrentOwner::default()
            };
            continue;
        }

        if let Some(command) = line.strip_prefix('c') {
            current.image_name = Some(command.to_string());
            continue;
        }

        if let Some(name) = line.strip_prefix('n') {
            if lsof_name_covers_bind(name, bind) {
                current.matches_bind = true;
            }
        }
    }

    current.into_match()
}

#[cfg(any(unix, test))]
fn lsof_name_covers_bind(name: &str, bind: SocketAddr) -> bool {
    name.split_whitespace()
        .map(|token| token.trim_end_matches(',').trim_end_matches(';'))
        .any(|token| local_endpoint_covers_bind(token, bind))
}

#[cfg(any(unix, test))]
fn local_endpoint_covers_bind(endpoint: &str, bind: SocketAddr) -> bool {
    let Some((host, port)) = split_endpoint(endpoint) else {
        return false;
    };
    if port != bind.port() {
        return false;
    }
    let host = host.trim_matches(['[', ']']);
    if host == "*" {
        return true;
    }
    let Ok(endpoint_ip) = host.parse::<std::net::IpAddr>() else {
        return false;
    };
    if endpoint_ip.is_unspecified() {
        return true;
    }
    endpoint_ip == bind.ip()
}

#[cfg(any(unix, test))]
fn split_endpoint(endpoint: &str) -> Option<(&str, u16)> {
    let endpoint = endpoint.trim();
    if let Some(rest) = endpoint.strip_prefix('[') {
        let close = rest.rfind(']')?;
        let host = &rest[..close];
        let port = rest.get(close + 1..)?.strip_prefix(':')?.parse().ok()?;
        return Some((host, port));
    }

    let (host, port) = endpoint.rsplit_once(':')?;
    Some((host, port.parse().ok()?))
}

#[derive(Debug)]
struct RuntimeInstanceLock {
    path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RuntimeLockDocument {
    pid: u32,
    agent_id: String,
    config_path: String,
    started_at_ms: u64,
}

impl RuntimeInstanceLock {
    fn acquire(config_path: &Path, agent_id: &str) -> Result<Self> {
        let lock_path = runtime_lock_path(config_path);
        if let Some(parent) = lock_path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("failed to create runtime lock dir {}", parent.display())
            })?;
            if let Some(conflict) =
                find_legacy_runtime_lock_conflict(parent, &lock_path, config_path)?
            {
                return Err(conflict.into());
            }
        }

        for _ in 0..3 {
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&lock_path)
            {
                Ok(mut file) => {
                    let document = RuntimeLockDocument {
                        pid: std::process::id(),
                        agent_id: agent_id.to_string(),
                        config_path: config_path.display().to_string(),
                        started_at_ms: now_ms(),
                    };
                    let content = serde_json::to_vec_pretty(&document)?;
                    file.write_all(&content).with_context(|| {
                        format!("failed to write runtime lock {}", lock_path.display())
                    })?;
                    file.write_all(b"\n").with_context(|| {
                        format!("failed to write runtime lock {}", lock_path.display())
                    })?;
                    file.sync_all().with_context(|| {
                        format!("failed to flush runtime lock {}", lock_path.display())
                    })?;
                    return Ok(Self { path: lock_path });
                }
                Err(err) if err.kind() == ErrorKind::AlreadyExists => {
                    if remove_stale_runtime_lock(&lock_path)? {
                        continue;
                    }
                    if let Ok(existing) = read_runtime_lock(&lock_path) {
                        return Err(RuntimeLockConflict {
                            pid: existing.pid,
                            agent_id: existing.agent_id,
                            config_path: existing.config_path,
                            lock_path: lock_path.display().to_string(),
                            process: describe_process(existing.pid),
                        }
                        .into());
                    }
                    anyhow::bail!(
                        "bridge-agent runtime lock already exists at {}",
                        lock_path.display()
                    );
                }
                Err(err) => {
                    return Err(err).with_context(|| {
                        format!("failed to create runtime lock {}", lock_path.display())
                    });
                }
            }
        }

        anyhow::bail!(
            "failed to acquire runtime lock after removing stale lock {}",
            lock_path.display()
        )
    }
}

impl Drop for RuntimeInstanceLock {
    fn drop(&mut self) {
        if let Err(err) = fs::remove_file(&self.path) {
            if err.kind() != ErrorKind::NotFound {
                warn!(
                    "failed to remove runtime lock {}: {err:#}",
                    self.path.display()
                );
            }
        }
    }
}

fn runtime_lock_path(config_path: &Path) -> PathBuf {
    let config_base_dir = resolve_config_base_dir(config_path);
    let config_file = config_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("agent-config.json");
    let resolved_path = config_path
        .canonicalize()
        .unwrap_or_else(|_| config_path.to_path_buf());
    let digest = Sha256::digest(resolved_path.to_string_lossy().as_bytes());
    let fingerprint = digest[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    // The runtime owns a configuration instance, not an agent identity. Agent
    // identity can be migrated while a stale process is still alive, so putting
    // agent_id in the filename would allow both identities to acquire a lock.
    let name = format!(
        "{}-{fingerprint}.lock",
        sanitize_lock_component(config_file)
    );
    config_base_dir.join(RUNTIME_LOCK_DIR).join(name)
}

fn find_legacy_runtime_lock_conflict(
    lock_dir: &Path,
    canonical_lock_path: &Path,
    config_path: &Path,
) -> Result<Option<RuntimeLockConflict>> {
    let entries = fs::read_dir(lock_dir)
        .with_context(|| format!("failed to inspect runtime lock dir {}", lock_dir.display()))?;
    for entry in entries {
        let path = entry
            .with_context(|| format!("failed to read runtime lock dir {}", lock_dir.display()))?
            .path();
        if path == canonical_lock_path
            || path.extension().and_then(|value| value.to_str()) != Some("lock")
        {
            continue;
        }
        let Ok(document) = read_runtime_lock(&path) else {
            continue;
        };
        if !runtime_config_paths_match(Path::new(&document.config_path), config_path) {
            continue;
        }
        if remove_stale_runtime_lock(&path)? {
            continue;
        }
        return Ok(Some(RuntimeLockConflict {
            pid: document.pid,
            agent_id: document.agent_id,
            config_path: document.config_path,
            lock_path: path.display().to_string(),
            process: describe_process(document.pid),
        }));
    }
    Ok(None)
}

fn runtime_config_paths_match(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

fn sanitize_lock_component(value: &str) -> String {
    let sanitized = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '.' || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    let sanitized = sanitized.trim_matches('_');
    if sanitized.is_empty() {
        "runtime".to_string()
    } else {
        sanitized.chars().take(48).collect()
    }
}

fn remove_stale_runtime_lock(path: &Path) -> Result<bool> {
    let Some(document) = read_runtime_lock(path).ok() else {
        fs::remove_file(path).with_context(|| {
            format!(
                "failed to remove unreadable runtime lock {}",
                path.display()
            )
        })?;
        return Ok(true);
    };
    if document.pid == std::process::id() {
        return Ok(false);
    }
    let process = describe_process(document.pid);
    if runtime_lock_owner_is_active(&process) {
        return Ok(false);
    }
    fs::remove_file(path).with_context(|| {
        format!(
            "failed to remove stale runtime lock {} for pid {}",
            path.display(),
            document.pid
        )
    })?;
    Ok(true)
}

fn runtime_lock_owner_is_active(process: &RuntimeProcessInfo) -> bool {
    if !process.running {
        return false;
    }
    if process_looks_like_bridge_agent(process) {
        return true;
    }

    // If the platform cannot describe a running PID, keep the lock rather than
    // risking a second runtime. Identifiable non-Bridge-Agent PIDs are stale
    // lock reuse and can be reclaimed.
    process.name.is_none() && process.executable_path.is_none() && process.command_line.is_none()
}

fn read_runtime_lock(path: &Path) -> Result<RuntimeLockDocument> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("failed to read runtime lock {}", path.display()))?;
    serde_json::from_str(&content)
        .with_context(|| format!("failed to parse runtime lock {}", path.display()))
}

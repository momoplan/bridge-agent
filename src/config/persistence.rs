pub fn load_config(path: &Path) -> Result<AgentConfig> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("failed to read config {}", path.display()))?;
    let has_legacy_codex_binary_path = config_has_legacy_codex_binary_path(&content);
    let mut config: AgentConfig = serde_json::from_str(&content)
        .with_context(|| format!("failed to parse config {}", path.display()))?;
    let mut changed = has_legacy_codex_binary_path;
    changed |= config.normalize();
    changed |= migrate_legacy_defaults(&mut config);
    config.validate()?;
    let legacy_token = config.relay.token.trim().to_string();
    if legacy_token.is_empty() {
        if let Some(token) = load_relay_token(path)? {
            config.relay.token = token;
        }
    } else {
        store_relay_token(path, &legacy_token)?;
        config.relay.token = legacy_token;
        changed = true;
    }
    if changed {
        write_public_config(path, &config)?;
    }
    Ok(config)
}

fn config_has_legacy_codex_binary_path(content: &str) -> bool {
    serde_json::from_str::<Value>(content)
        .ok()
        .and_then(|config| config.get("runtime").cloned())
        .and_then(|runtime| runtime.as_object().cloned())
        .is_some_and(|runtime| runtime.contains_key("codex_binary_path"))
}

fn remove_legacy_codex_binary_overrides(config: &mut AgentConfig) -> bool {
    fn remove(command: &mut Option<ServiceStartCommand>) -> bool {
        let Some(ServiceStartCommand::ShellCommand { env, .. }) = command.as_mut() else {
            return false;
        };
        env.remove("CODEX_CONNECTOR_CODEX_BINARY").is_some()
    }

    let mut changed = false;
    for service in &mut config.services {
        changed |= remove(&mut service.start_command);
        changed |= remove(&mut service.stop_command);
    }
    for app in &mut config.local_apps {
        changed |= remove(&mut app.start_command);
        changed |= remove(&mut app.stop_command);
    }
    changed
}

pub fn save_config(path: &Path, config: &AgentConfig) -> Result<()> {
    let mut config = config.clone();
    config.normalize();
    config.validate()?;
    if !config.relay.token.trim().is_empty() {
        store_relay_token(path, &config.relay.token)?;
    }
    write_public_config(path, &config)
}

fn write_public_config(path: &Path, config: &AgentConfig) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create config dir {}", parent.display()))?;
        set_private_directory_permissions(parent)?;
    }
    let mut public_config = config.clone();
    public_config.relay.token.clear();
    let content = serde_json::to_string_pretty(&public_config)?;
    write_config_atomically(path, format!("{content}\n").as_bytes())?;
    set_private_file_permissions(path)?;
    Ok(())
}

pub fn ensure_config_exists(path: &Path) -> Result<()> {
    if !path.exists() {
        delete_relay_token(path)?;
        save_config(path, &AgentConfig::example())?;
    }
    Ok(())
}

pub fn reset_invalid_config(path: &Path) -> Result<ConfigRecovery> {
    let archived_path = if path.exists() {
        Some(archive_existing_config(path)?)
    } else {
        None
    };
    delete_relay_token(path)?;
    let config = AgentConfig::example();
    save_config(path, &config)?;
    Ok(ConfigRecovery {
        archived_path,
        config,
    })
}

pub fn clear_relay_credentials(path: &Path) -> Result<()> {
    delete_relay_token(path)
}

#[cfg(unix)]
fn set_private_directory_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .with_context(|| format!("failed to secure config directory {}", path.display()))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_private_directory_permissions(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn set_private_file_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .with_context(|| format!("failed to secure config file {}", path.display()))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_private_file_permissions(_path: &Path) -> Result<()> {
    Ok(())
}

fn write_config_atomically(path: &Path, content: &[u8]) -> Result<()> {
    let temp_path = unique_sibling_path(path, TEMP_CONFIG_MARKER);
    let write_result = (|| -> Result<()> {
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp_path)
            .with_context(|| format!("failed to create temp config {}", temp_path.display()))?;
        file.write_all(content)
            .with_context(|| format!("failed to write temp config {}", temp_path.display()))?;
        file.sync_all()
            .with_context(|| format!("failed to flush temp config {}", temp_path.display()))?;
        Ok(())
    })();

    if let Err(err) = write_result {
        let _ = fs::remove_file(&temp_path);
        return Err(err);
    }

    if path.exists() {
        let backup_path = backup_config_path(path);
        write_sanitized_config_backup(path, &backup_path)?;
    }

    if let Err(err) = replace_file(&temp_path, path) {
        let _ = fs::remove_file(&temp_path);
        return Err(err);
    }

    Ok(())
}

fn backup_config_path(path: &Path) -> PathBuf {
    sibling_path(path, CONFIG_BACKUP_SUFFIX)
}

fn write_sanitized_config_backup(source: &Path, destination: &Path) -> Result<()> {
    let content = fs::read(source)
        .with_context(|| format!("failed to read config backup source {}", source.display()))?;
    let sanitized = serde_json::from_slice::<Value>(&content)
        .ok()
        .and_then(|mut value| {
            value
                .get_mut("relay")?
                .as_object_mut()?
                .insert("token".to_string(), Value::String(String::new()));
            serde_json::to_vec_pretty(&value).ok()
        })
        .unwrap_or(content);
    fs::write(destination, sanitized).with_context(|| {
        format!(
            "failed to backup config {} to {}",
            source.display(),
            destination.display()
        )
    })?;
    set_private_file_permissions(destination)
}

fn archive_existing_config(path: &Path) -> Result<PathBuf> {
    let archived_path = unique_sibling_path(path, INVALID_CONFIG_MARKER);
    fs::rename(path, &archived_path).with_context(|| {
        format!(
            "failed to archive config {} to {}",
            path.display(),
            archived_path.display()
        )
    })?;
    set_private_file_permissions(&archived_path)?;
    Ok(archived_path)
}

fn unique_sibling_path(path: &Path, marker: &str) -> PathBuf {
    let timestamp = current_timestamp_millis();
    for attempt in 0..1000 {
        let suffix = if attempt == 0 {
            format!("{marker}-{timestamp}")
        } else {
            format!("{marker}-{timestamp}-{attempt}")
        };
        let candidate = sibling_path(path, &suffix);
        if !candidate.exists() {
            return candidate;
        }
    }
    sibling_path(
        path,
        &format!("{marker}-{timestamp}-{}", uuid::Uuid::new_v4().simple()),
    )
}

fn sibling_path(path: &Path, suffix: &str) -> PathBuf {
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_else(|| DEFAULT_CONFIG_FILE_NAME.into());
    let name = format!("{file_name}.{suffix}");
    path.with_file_name(name)
}

fn current_timestamp_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}

#[cfg(windows)]
fn replace_file(from: &Path, to: &Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    fn wide_path(path: &Path) -> Vec<u16> {
        path.as_os_str().encode_wide().chain(Some(0)).collect()
    }

    let from_wide = wide_path(from);
    let to_wide = wide_path(to);
    let replaced = unsafe {
        MoveFileExW(
            from_wide.as_ptr(),
            to_wide.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if replaced == 0 {
        return Err(std::io::Error::last_os_error()).with_context(|| {
            format!(
                "failed to replace config {} with {}",
                to.display(),
                from.display()
            )
        });
    }
    Ok(())
}

#[cfg(not(windows))]
fn replace_file(from: &Path, to: &Path) -> Result<()> {
    fs::rename(from, to).with_context(|| {
        format!(
            "failed to replace config {} with {}",
            to.display(),
            from.display()
        )
    })
}

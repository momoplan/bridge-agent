#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ConnectorRuntimePreparation {
    Deferred,
    Prepare,
}

fn resolve_installed_start_commands(
    services: &mut [ServiceConfig],
    package_path: &Path,
    runtime_config: &RuntimeConfig,
    manifest: &ConnectorManifest,
    runtime_preparation: ConnectorRuntimePreparation,
) -> Result<()> {
    let data_dir = connector_data_dir(&manifest.app_id)?;
    let start_policy = manifest
        .runtime
        .as_ref()
        .map(|runtime| runtime.start_policy.trim())
        .filter(|value| !value.is_empty())
        .unwrap_or("automatic");
    if !matches!(start_policy, "automatic" | "manual") {
        bail!(
            "connector `{}` has unsupported runtime.startPolicy `{start_policy}`",
            manifest.app_id
        );
    }
    let (event_token_path, local_app_token_path, asset_upload_token_path) =
        prepare_connector_runtime_paths(manifest, &data_dir, runtime_preparation)?;
    let package_bins = read_package_bins(package_path)?;
    let python_scripts = read_python_project_scripts(package_path)?;
    let python_env = if python_scripts.is_empty() {
        None
    } else if runtime_preparation == ConnectorRuntimePreparation::Prepare {
        ensure_python_project_environment(package_path, &python_scripts, runtime_config)?
    } else {
        Some(package_path.join(CONNECTOR_PYTHON_ENV_DIR))
    };
    let node_path = if runtime_preparation == ConnectorRuntimePreparation::Prepare {
        resolve_command_path("node", runtime_config)
    } else {
        configured_runtime_command_path("node", runtime_config)
    };
    let command_runtime = InstalledCommandRuntime {
        package_path,
        package_bins: &package_bins,
        python_scripts: &python_scripts,
        python_env: python_env.as_deref(),
        node_path: &node_path,
    };
    configure_service_runtime_commands(
        services,
        manifest,
        &data_dir,
        start_policy,
        &event_token_path,
        &local_app_token_path,
        asset_upload_token_path.as_deref(),
        &command_runtime,
    );
    Ok(())
}

fn prepare_connector_runtime_paths(
    manifest: &ConnectorManifest,
    data_dir: &Path,
    runtime_preparation: ConnectorRuntimePreparation,
) -> Result<(PathBuf, PathBuf, Option<PathBuf>)> {
    let prepare = runtime_preparation == ConnectorRuntimePreparation::Prepare;
    if prepare {
        fs::create_dir_all(data_dir).with_context(|| {
            format!("failed to create connector data directory {}", data_dir.display())
        })?;
        #[cfg(unix)]
        fs::set_permissions(data_dir, fs::Permissions::from_mode(0o700))?;
    }
    let event_token_path = if prepare {
        ensure_connector_event_token(&manifest.app_id)?
    } else {
        connector_event_token_path(&manifest.app_id)?
    };
    let local_app_token_path = if prepare {
        ensure_connector_management_token(&manifest.app_id)?
    } else {
        connector_management_token_path(&manifest.app_id)?
    };
    let asset_upload_token_path = connector_permission_is_active(
        manifest,
        CONNECTOR_ASSET_UPLOAD_PERMISSION,
    )
    .then(|| {
        if prepare {
            ensure_connector_asset_upload_token(&manifest.app_id)
        } else {
            connector_asset_upload_token_path(&manifest.app_id)
        }
    })
    .transpose()?;
    Ok((event_token_path, local_app_token_path, asset_upload_token_path))
}

#[allow(clippy::too_many_arguments)]
fn configure_service_runtime_commands(
    services: &mut [ServiceConfig],
    manifest: &ConnectorManifest,
    data_dir: &Path,
    start_policy: &str,
    event_token_path: &Path,
    local_app_token_path: &Path,
    asset_upload_token_path: Option<&Path>,
    command_runtime: &InstalledCommandRuntime<'_>,
) {
    for service in services {
        if service.stop_command.is_none() {
            service.stop_command = derive_stop_command_from_start(service.start_command.as_ref());
        }
        for command_config in [&mut service.start_command, &mut service.stop_command] {
            let Some(ServiceStartCommand::ShellCommand {
                command, cwd, env, ..
            }) = command_config.as_mut()
            else {
                continue;
            };
            env.insert(LOCAL_APP_ID_ENV.to_string(), manifest.app_id.clone());
            env.insert(
                LOCAL_APP_DATA_DIR_ENV.to_string(),
                data_dir.display().to_string(),
            );
            env.insert(
                LOCAL_APP_START_POLICY_ENV.to_string(),
                start_policy.to_string(),
            );
            env.insert(
                LOCAL_APP_TOKEN_FILE_ENV.to_string(),
                local_app_token_path.display().to_string(),
            );
            env.insert(
                LOCAL_APP_EVENT_TOKEN_FILE_ENV.to_string(),
                event_token_path.display().to_string(),
            );
            env.insert(
                LOCAL_APP_EVENT_ENDPOINT_ENV.to_string(),
                DEFAULT_LOCAL_APP_EVENT_ENDPOINT.to_string(),
            );
            if let Some(asset_upload_token_path) = asset_upload_token_path {
                env.insert(
                    LOCAL_APP_ASSET_UPLOAD_TOKEN_FILE_ENV.to_string(),
                    asset_upload_token_path.display().to_string(),
                );
                env.insert(
                    LOCAL_APP_ASSET_UPLOAD_ENDPOINT_ENV.to_string(),
                    DEFAULT_LOCAL_APP_ASSET_UPLOAD_ENDPOINT.to_string(),
                );
            }
            resolve_installed_shell_command(command, cwd, env, command_runtime);
        }
    }
}

fn derive_stop_command_from_start(
    start_command: Option<&ServiceStartCommand>,
) -> Option<ServiceStartCommand> {
    let ServiceStartCommand::ShellCommand {
        command,
        cwd,
        env,
        timeout_secs,
    } = start_command?;
    let start_index = command.iter().position(|part| part == "start")?;
    if !command.iter().any(|part| part == "--daemon") {
        return None;
    }
    let mut stop_command = command.clone();
    stop_command[start_index] = "stop".to_string();
    stop_command.retain(|part| part != "--daemon");
    Some(ServiceStartCommand::ShellCommand {
        command: stop_command,
        cwd: cwd.clone(),
        env: env.clone(),
        timeout_secs: *timeout_secs,
    })
}

struct InstalledCommandRuntime<'a> {
    package_path: &'a Path,
    package_bins: &'a BTreeMap<String, String>,
    python_scripts: &'a BTreeMap<String, String>,
    python_env: Option<&'a Path>,
    node_path: &'a Option<PathBuf>,
}

fn resolve_installed_shell_command(
    command: &mut Vec<String>,
    cwd: &mut Option<String>,
    env: &mut BTreeMap<String, String>,
    runtime: &InstalledCommandRuntime<'_>,
) {
    if command.is_empty() {
        return;
    }
    let executable = command[0].trim();
    if executable.is_empty() || Path::new(executable).is_absolute() {
        return;
    }

    if let Some(direct_path) = native_command_path(runtime.package_path, executable) {
        command[0] = direct_path.display().to_string();
        if cwd.as_deref().map(str::trim).unwrap_or_default().is_empty() {
            *cwd = Some(runtime.package_path.display().to_string());
        }
        return;
    }

    if runtime.python_scripts.contains_key(executable) {
        if let Some(env_path) = runtime.python_env {
            command[0] = python_script_path(env_path, executable)
                .display()
                .to_string();
            prepend_path_entry(env, python_bin_dir(env_path));
            if cwd.as_deref().map(str::trim).unwrap_or_default().is_empty() {
                *cwd = Some(runtime.package_path.display().to_string());
            }
            return;
        }
    }

    if let Some(relative_bin) = runtime.package_bins.get(executable) {
        command[0] = runtime
            .node_path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "node".to_string());
        command.insert(
            1,
            runtime
                .package_path
                .join(relative_bin)
                .display()
                .to_string(),
        );
        if cwd.as_deref().map(str::trim).unwrap_or_default().is_empty() {
            *cwd = Some(runtime.package_path.display().to_string());
        }
    }
}

fn native_command_path(package_path: &Path, executable: &str) -> Option<PathBuf> {
    let executable_names = if cfg!(windows) && !executable.to_ascii_lowercase().ends_with(".exe") {
        vec![format!("{executable}.exe"), executable.to_string()]
    } else {
        vec![executable.to_string()]
    };
    for platform_dir in native_platform_bin_dirs() {
        for executable_name in &executable_names {
            let candidate = package_path
                .join("bin")
                .join(&platform_dir)
                .join(executable_name);
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }
    for executable_name in executable_names {
        let direct_path = package_path.join(&executable_name);
        if direct_path.exists() {
            return Some(direct_path);
        }
        let bin_path = package_path.join("bin").join(executable_name);
        if bin_path.exists() {
            return Some(bin_path);
        }
    }
    None
}

fn native_platform_bin_dirs() -> Vec<String> {
    let os = env::consts::OS;
    let arch = match env::consts::ARCH {
        "aarch64" => "arm64",
        "x86_64" => "x86_64",
        other => other,
    };
    vec![format!("{os}-{arch}"), os.to_string()]
}

fn prepend_path_entry(env_vars: &mut BTreeMap<String, String>, entry: PathBuf) {
    let mut path_entries = vec![entry];
    append_split_path(&mut path_entries, env_vars.get("PATH"));
    if let Ok(joined_path) = env::join_paths(path_entries) {
        env_vars.insert(
            "PATH".to_string(),
            joined_path.to_string_lossy().to_string(),
        );
    }
}

fn append_split_path(entries: &mut Vec<PathBuf>, value: Option<&String>) {
    let Some(value) = value else {
        return;
    };
    for entry in env::split_paths(value) {
        push_unique_path_entry(entries, entry);
    }
}

fn push_unique_path_entry(entries: &mut Vec<PathBuf>, entry: PathBuf) {
    if entry.as_os_str().is_empty() {
        return;
    }
    if !entries.iter().any(|candidate| candidate == &entry) {
        entries.push(entry);
    }
}

fn resolve_command_path(executable: &str, runtime_config: &RuntimeConfig) -> Option<PathBuf> {
    configured_runtime_command_path(executable, runtime_config)
        .or_else(|| discover_command_path(executable))
}

fn discover_command_path(executable: &str) -> Option<PathBuf> {
    find_command_in_path(executable, env::var("PATH").ok().as_ref())
        .or_else(|| {
            current_user_command_path()
                .and_then(|path| find_command_in_path(executable, Some(&path)))
        })
        .or_else(|| bundled_runtime_command_path(executable))
}

fn configured_runtime_command_path(
    executable: &str,
    runtime_config: &RuntimeConfig,
) -> Option<PathBuf> {
    let path = (executable == "node")
        .then_some(runtime_config.node_path.as_deref())
        .flatten()?;
    resolve_command_file(
        &PathBuf::from(path.trim()),
        cfg!(windows),
        &windows_command_extensions(),
    )
}

fn bundled_runtime_command_path(executable: &str) -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let candidates: &[&str] = match executable {
            "node" => &[
                "/Applications/ChatGPT.app/Contents/Resources/cua_node/bin/node",
                "/Applications/Codex.app/Contents/Resources/cua_node/bin/node",
            ],
            _ => &[],
        };
        candidates
            .iter()
            .map(PathBuf::from)
            .find(|candidate| candidate.is_file())
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = executable;
        None
    }
}

fn find_command_in_path(executable: &str, path: Option<&String>) -> Option<PathBuf> {
    find_command_in_path_for_platform(
        executable,
        path,
        cfg!(windows),
        &windows_command_extensions(),
    )
}

fn find_command_in_path_for_platform(
    executable: &str,
    path: Option<&String>,
    windows: bool,
    windows_extensions: &[String],
) -> Option<PathBuf> {
    let path = path?;
    env::split_paths(path)
        .find_map(|dir| resolve_command_file(&dir.join(executable), windows, windows_extensions))
}

fn resolve_command_file(
    path: &Path,
    windows: bool,
    windows_extensions: &[String],
) -> Option<PathBuf> {
    if !windows {
        return path.is_file().then(|| path.to_path_buf());
    }
    if path.extension().is_some() {
        return has_supported_windows_command_extension(path)
            .then(|| path.is_file().then(|| path.to_path_buf()))
            .flatten();
    }
    windows_extensions.iter().find_map(|extension| {
        let mut candidate = path.as_os_str().to_os_string();
        candidate.push(extension);
        let candidate = PathBuf::from(candidate);
        candidate.is_file().then_some(candidate)
    })
}

fn has_supported_windows_command_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            ["com", "exe", "bat", "cmd"]
                .iter()
                .any(|candidate| candidate.eq_ignore_ascii_case(extension))
        })
}

fn windows_command_extensions() -> Vec<String> {
    windows_command_extensions_from(env::var("PATHEXT").ok().as_deref())
}

fn windows_command_extensions_from(value: Option<&str>) -> Vec<String> {
    const SUPPORTED: [&str; 4] = [".COM", ".EXE", ".BAT", ".CMD"];
    let mut extensions = Vec::new();
    for extension in value
        .into_iter()
        .flat_map(|value| value.split(';'))
        .map(str::trim)
        .filter(|value| {
            SUPPORTED
                .iter()
                .any(|item| item.eq_ignore_ascii_case(value))
        })
    {
        if !extensions
            .iter()
            .any(|existing: &String| existing.eq_ignore_ascii_case(extension))
        {
            extensions.push(extension.to_string());
        }
    }
    if extensions.is_empty() {
        extensions = SUPPORTED.iter().map(|value| (*value).to_string()).collect();
    }
    extensions
}

fn read_package_bins(package_path: &Path) -> Result<BTreeMap<String, String>> {
    let path = package_path.join("package.json");
    if !path.exists() {
        return Ok(BTreeMap::new());
    }
    let content = fs::read_to_string(&path)
        .with_context(|| format!("failed to read package metadata {}", path.display()))?;
    let package: Value = serde_json::from_str(&content)
        .with_context(|| format!("failed to parse package metadata {}", path.display()))?;
    let mut bins = BTreeMap::new();
    match package.get("bin") {
        Some(Value::String(bin)) => {
            if let Some(name) = package.get("name").and_then(Value::as_str) {
                bins.insert(name.to_string(), bin.to_string());
            }
        }
        Some(Value::Object(map)) => {
            for (name, bin) in map {
                if let Some(bin) = bin.as_str() {
                    bins.insert(name.to_string(), bin.to_string());
                }
            }
        }
        _ => {}
    }
    Ok(bins)
}

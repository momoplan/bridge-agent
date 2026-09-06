fn prepare_connector_package_destination(package_path: &Path, replace: bool) -> Result<PathBuf> {
    if !replace {
        bail!(
            "connector package already exists at {}; pass replace to overwrite it",
            package_path.display()
        );
    }
    release_connector_package_processes(package_path)?;
    quarantine_connector_package_path(package_path).with_context(|| {
        format!(
            "failed to move aside existing connector package {} before replacement",
            package_path.display()
        )
    })
}

fn quarantine_connector_package_path(package_path: &Path) -> Result<PathBuf> {
    let parent = package_path
        .parent()
        .with_context(|| format!("failed to resolve parent for {}", package_path.display()))?;
    let name = package_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("package");
    for attempt in 0..100 {
        let suffix = if attempt == 0 {
            String::new()
        } else {
            format!("-{attempt}")
        };
        let quarantine_path = parent.join(format!("{name}.replaced-{}{}", now_ms(), suffix));
        if quarantine_path.exists() {
            continue;
        }
        rename_connector_path_after_handles_close(package_path, &quarantine_path)?;
        return Ok(quarantine_path);
    }
    bail!(
        "failed to choose replacement path for existing connector package {}",
        package_path.display()
    )
}

fn restore_replaced_connector_package(
    package_path: &Path,
    replaced_package: Option<&Path>,
) -> Result<()> {
    if package_path.exists() {
        fs::remove_dir_all(package_path).with_context(|| {
            format!(
                "failed to remove incomplete connector package {}",
                package_path.display()
            )
        })?;
    }
    if let Some(replaced_package) = replaced_package {
        fs::rename(replaced_package, package_path).with_context(|| {
            format!(
                "failed to restore {} to {}",
                replaced_package.display(),
                package_path.display()
            )
        })?;
    }
    Ok(())
}

fn quarantine_connector_install_root(install_root: &Path) -> Result<PathBuf> {
    let parent = install_root
        .parent()
        .with_context(|| format!("failed to resolve parent for {}", install_root.display()))?;
    let name = install_root
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("connector");
    for attempt in 0..100 {
        let suffix = if attempt == 0 {
            String::new()
        } else {
            format!("-{attempt}")
        };
        let quarantine_path = parent.join(format!(".{name}.uninstalling-{}{}", now_ms(), suffix));
        if quarantine_path.exists() {
            continue;
        }
        rename_connector_path_after_handles_close(install_root, &quarantine_path).with_context(
            || {
                format!(
                    "failed to move connector installation {} to {} before uninstall",
                    install_root.display(),
                    quarantine_path.display()
                )
            },
        )?;
        return Ok(quarantine_path);
    }
    bail!(
        "failed to choose uninstall path for connector installation {}",
        install_root.display()
    )
}

const CONNECTOR_PATH_RENAME_ATTEMPTS: usize = 30;
const CONNECTOR_PATH_RENAME_RETRY_DELAY: Duration = Duration::from_millis(100);

fn rename_connector_path_after_handles_close(source: &Path, destination: &Path) -> Result<()> {
    rename_connector_path_with_retry(
        source,
        destination,
        CONNECTOR_PATH_RENAME_ATTEMPTS,
        CONNECTOR_PATH_RENAME_RETRY_DELAY,
    )
}

fn rename_connector_path_with_retry(
    source: &Path,
    destination: &Path,
    attempts: usize,
    retry_delay: Duration,
) -> Result<()> {
    let attempts = attempts.max(1);
    for attempt in 0..attempts {
        match fs::rename(source, destination) {
            Ok(()) => return Ok(()),
            Err(err) if connector_path_error_is_retryable(&err) && attempt + 1 < attempts => {
                std::thread::sleep(retry_delay);
            }
            Err(err) => {
                return Err(err).with_context(|| {
                    format!(
                        "failed to move {} to {} after waiting for process handles to close",
                        source.display(),
                        destination.display()
                    )
                });
            }
        }
    }
    unreachable!("connector path rename loop always returns")
}

fn connector_path_error_is_retryable(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::PermissionDenied
            | std::io::ErrorKind::WouldBlock
            | std::io::ErrorKind::Interrupted
    ) || matches!(error.raw_os_error(), Some(32 | 33))
}

fn stop_connector_for_package_change(
    app_id: &str,
    config: &crate::config::AgentConfig,
) -> Result<()> {
    let Some(app) = config.local_apps.iter().find(|app| app.app_id == app_id) else {
        return Ok(());
    };
    match (&app.start_command, &app.stop_command) {
        (_, Some(command)) => {
            let result = run_start_command(app_id, command, &BTreeMap::new()).map_err(|error| {
                anyhow::Error::new(ConnectorPackageStopError {
                    message: format!(
                        "failed to stop connector `{app_id}` before package change: {error:#}"
                    ),
                })
            })?;
            if result.exit_code != Some(0) {
                let detail = if !result.stderr.trim().is_empty() {
                    result.stderr.trim().to_string()
                } else {
                    format!("exit code {:?}", result.exit_code)
                };
                return Err(anyhow::Error::new(ConnectorPackageStopError {
                    message: format!(
                        "failed to stop connector `{app_id}` before package change: {detail}"
                    ),
                }));
            }
            Ok(())
        }
        (Some(_), None) => Err(anyhow::Error::new(ConnectorPackageStopError {
            message: format!(
                "connector `{app_id}` has a start command but no stop command; refusing to change a package that may still be running"
            ),
        })),
        (None, None) => Ok(()),
    }
}

#[cfg(windows)]
fn release_connector_package_processes(package_path: &Path) -> Result<()> {
    let package_path = package_path
        .canonicalize()
        .unwrap_or_else(|_| package_path.to_path_buf());
    let pids = connector_package_processes(&package_path)?;
    for pid in pids {
        let mut terminate = Command::new("taskkill");
        configure_connector_command(&mut terminate);
        let taskkill = terminate
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .output()
            .with_context(|| format!("failed to terminate connector process tree {pid}"))?;
        if !taskkill.status.success() {
            bail!(
                "failed to terminate connector process tree {pid}\nstdout:\n{}\nstderr:\n{}",
                String::from_utf8_lossy(&taskkill.stdout),
                String::from_utf8_lossy(&taskkill.stderr)
            );
        }
    }
    for _ in 0..30 {
        let remaining = connector_package_processes(&package_path)?;
        if remaining.is_empty() {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let remaining = connector_package_processes(&package_path)?;
    bail!(
        "connector package processes did not stop for {}: {:?}",
        package_path.display(),
        remaining
    )
}

#[cfg(windows)]
fn connector_package_processes(package_path: &Path) -> Result<Vec<u32>> {
    let package_path = package_path
        .canonicalize()
        .unwrap_or_else(|_| package_path.to_path_buf());
    let script = r#"
$prefix = [System.IO.Path]::GetFullPath(
  $env:BAIJIMU_LOCAL_APP_PACKAGE_TO_RELEASE
).TrimEnd([System.IO.Path]::DirectorySeparatorChar) +
  [System.IO.Path]::DirectorySeparatorChar
Get-CimInstance Win32_Process -ErrorAction Stop |
  Where-Object {
    $_.ExecutablePath -and
    [System.IO.Path]::GetFullPath($_.ExecutablePath).StartsWith(
      $prefix,
      [System.StringComparison]::OrdinalIgnoreCase
    )
  } |
  Select-Object -ExpandProperty ProcessId
"#;
    let mut inspect = Command::new("powershell");
    configure_connector_command(&mut inspect);
    let output = inspect
        .args(["-NoProfile", "-Command", script])
        .env(
            "BAIJIMU_LOCAL_APP_PACKAGE_TO_RELEASE",
            package_path.as_os_str(),
        )
        .output()
        .with_context(|| {
            format!(
                "failed to inspect Windows processes using connector package {}",
                package_path.display()
            )
        })?;
    if !output.status.success() {
        bail!(
            "failed to inspect Windows processes using connector package {}\nstdout:\n{}\nstderr:\n{}",
            package_path.display(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let mut pids = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.trim().parse::<u32>().ok())
        .collect::<Vec<_>>();
    pids.sort_unstable();
    pids.dedup();
    Ok(pids)
}

#[cfg(unix)]
fn release_connector_package_processes(package_path: &Path) -> Result<()> {
    for _ in 0..30 {
        let remaining = connector_package_processes(package_path)?;
        if remaining.is_empty() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let remaining = connector_package_processes(package_path)?;
    bail!(
        "connector package processes are still running for {}: {:?}",
        package_path.display(),
        remaining
    )
}

#[cfg(all(not(windows), not(unix)))]
fn release_connector_package_processes(_package_path: &Path) -> Result<()> {
    bail!("connector package process inspection is unsupported on this platform")
}

#[cfg(windows)]
fn force_release_connector_package_processes(package_path: &Path) -> Result<()> {
    release_connector_package_processes(package_path)
}

#[cfg(unix)]
fn force_release_connector_package_processes(package_path: &Path) -> Result<()> {
    signal_connector_package_processes(package_path, libc::SIGTERM)?;
    for _ in 0..50 {
        if connector_package_processes(package_path)?.is_empty() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    signal_connector_package_processes(package_path, libc::SIGKILL)?;
    for _ in 0..30 {
        if connector_package_processes(package_path)?.is_empty() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let remaining = connector_package_processes(package_path)?;
    bail!(
        "connector package processes did not stop for {}: {:?}",
        package_path.display(),
        remaining
    )
}

#[cfg(unix)]
fn signal_connector_package_processes(package_path: &Path, signal: i32) -> Result<()> {
    for pid in connector_package_processes(package_path)? {
        let result = unsafe { libc::kill(pid as libc::pid_t, signal) };
        if result == 0 {
            continue;
        }
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) {
            continue;
        }
        return Err(error).with_context(|| {
            format!(
                "failed to signal connector process {pid} using package {}",
                package_path.display()
            )
        });
    }
    Ok(())
}

#[cfg(unix)]
fn connector_package_processes(package_path: &Path) -> Result<Vec<u32>> {
    let output = Command::new(unix_ps_binary())
        .args(["-axo", "pid=,command="])
        .output()
        .context("failed to inspect Unix processes for connector package use")?;
    if !output.status.success() {
        bail!(
            "failed to inspect Unix processes for connector package use\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(parse_unix_connector_package_processes(
        &String::from_utf8_lossy(&output.stdout),
        package_path,
        std::process::id(),
    ))
}

#[cfg(target_os = "macos")]
fn unix_ps_binary() -> &'static str {
    "/bin/ps"
}

#[cfg(all(unix, not(target_os = "macos")))]
fn unix_ps_binary() -> &'static str {
    "/usr/bin/ps"
}

#[cfg(unix)]
fn parse_unix_connector_package_processes(
    process_list: &str,
    package_path: &Path,
    current_pid: u32,
) -> Vec<u32> {
    let mut package_paths = vec![package_path.to_path_buf()];
    if let Ok(canonical_path) = package_path.canonicalize() {
        if canonical_path != package_path {
            package_paths.push(canonical_path);
        }
    }
    let package_prefixes = package_paths
        .iter()
        .map(|path| {
            format!(
                "{}{}",
                path.to_string_lossy()
                    .trim_end_matches(std::path::MAIN_SEPARATOR),
                std::path::MAIN_SEPARATOR
            )
        })
        .collect::<Vec<_>>();
    let mut pids = process_list
        .lines()
        .filter_map(|line| {
            let line = line.trim_start();
            let separator = line.find(char::is_whitespace)?;
            let pid = line[..separator].parse::<u32>().ok()?;
            let command = line[separator..].trim_start();
            (pid != current_pid
                && package_prefixes
                    .iter()
                    .any(|package_prefix| command.contains(package_prefix)))
            .then_some(pid)
        })
        .collect::<Vec<_>>();
    pids.sort_unstable();
    pids.dedup();
    pids
}

#[cfg(all(not(windows), not(unix)))]
fn force_release_connector_package_processes(_package_path: &Path) -> Result<()> {
    bail!("forced connector uninstall is unsupported on this platform")
}

fn copy_connector_package(source: &Path, destination: &Path) -> Result<()> {
    if source.is_file() {
        fs::create_dir_all(destination)
            .with_context(|| format!("failed to create connector dir {}", destination.display()))?;
        fs::copy(source, destination.join(CONNECTOR_MANIFEST_FILE)).with_context(|| {
            format!(
                "failed to copy connector manifest {} to {}",
                source.display(),
                destination.display()
            )
        })?;
        return Ok(());
    }

    copy_dir_recursive(source, destination)
}

fn connector_package_sha256(package_path: &Path) -> Result<String> {
    let mut files = Vec::new();
    collect_connector_package_files(package_path, package_path, &mut files)?;
    files.sort();

    let mut digest = Sha256::new();
    digest.update(b"bridge-agent-connector-package-v1\0");
    let mut buffer = [0_u8; 64 * 1024];
    for relative_path in files {
        let relative = relative_path.to_string_lossy();
        digest.update((relative.len() as u64).to_le_bytes());
        digest.update(relative.as_bytes());
        let path = package_path.join(&relative_path);
        let metadata = fs::metadata(&path)
            .with_context(|| format!("failed to inspect connector file {}", path.display()))?;
        digest.update(metadata.len().to_le_bytes());
        let mut file = fs::File::open(&path)
            .with_context(|| format!("failed to hash connector file {}", path.display()))?;
        loop {
            let read = file
                .read(&mut buffer)
                .with_context(|| format!("failed to hash connector file {}", path.display()))?;
            if read == 0 {
                break;
            }
            digest.update(&buffer[..read]);
        }
    }
    Ok(format!("sha256:{:x}", digest.finalize()))
}

fn collect_connector_package_files(
    root: &Path,
    directory: &Path,
    files: &mut Vec<PathBuf>,
) -> Result<()> {
    for entry in fs::read_dir(directory)
        .with_context(|| format!("failed to read connector package {}", directory.display()))?
    {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            collect_connector_package_files(root, &entry.path(), files)?;
        } else if file_type.is_file() {
            files.push(
                entry
                    .path()
                    .strip_prefix(root)
                    .with_context(|| {
                        format!(
                            "connector file {} escaped package {}",
                            entry.path().display(),
                            root.display()
                        )
                    })?
                    .to_path_buf(),
            );
        }
    }
    Ok(())
}

fn copy_dir_recursive(source: &Path, destination: &Path) -> Result<()> {
    fs::create_dir_all(destination)
        .with_context(|| format!("failed to create directory {}", destination.display()))?;
    for entry in
        fs::read_dir(source).with_context(|| format!("failed to read {}", source.display()))?
    {
        let entry = entry?;
        if should_skip_connector_package_entry(&entry.file_name()) {
            continue;
        }
        let from = entry.path();
        let to = destination.join(entry.file_name());
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            copy_dir_recursive(&from, &to)?;
        } else if file_type.is_file() {
            fs::copy(&from, &to).with_context(|| {
                format!("failed to copy {} to {}", from.display(), to.display())
            })?;
        }
    }
    Ok(())
}

fn should_skip_connector_package_entry(name: &std::ffi::OsStr) -> bool {
    matches!(
        name.to_str(),
        Some(
            CONNECTOR_PYTHON_ENV_DIR
                | ".venv"
                | "__pycache__"
                | ".pytest_cache"
                | ".git"
                | "target"
        )
    )
}

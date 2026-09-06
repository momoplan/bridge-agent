fn write_executable(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = fs::File::create(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    set_executable(path)
}

#[cfg(unix)]
fn set_executable(path: &Path) -> Result<()> {
    let mut permissions = fs::metadata(path)?.permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions)?;
    Ok(())
}

#[cfg(not(unix))]
fn set_executable(_path: &Path) -> Result<()> {
    Ok(())
}

fn extract_binary(bytes: &[u8], source: &str, archive_path: Option<&str>) -> Result<Vec<u8>> {
    let lower = source
        .split(['?', '#'])
        .next()
        .unwrap_or(source)
        .to_ascii_lowercase();
    if lower.ends_with(".zip") {
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes))?;
        if let Some(path) = archive_path.map(str::trim).filter(|path| !path.is_empty()) {
            let expected = normalize_archive_entry_name(path);
            let mut matching_index = None;
            for index in 0..archive.len() {
                let file = archive.by_index(index)?;
                if normalize_archive_entry_name(file.name()) != expected {
                    continue;
                }
                if matching_index.replace(index).is_some() {
                    bail!("managed tool archive contains multiple entries matching {path}");
                }
            }
            let index = matching_index
                .with_context(|| format!("managed tool archive does not contain {path}"))?;
            let mut file = archive.by_index(index)?;
            return read_archive_entry(&mut file);
        }
        for index in 0..archive.len() {
            let mut file = archive.by_index(index)?;
            let name = normalize_archive_entry_name(file.name());
            if name == binary_name() || name.ends_with(&format!("/{}", binary_name())) {
                return read_archive_entry(&mut file);
            }
        }
        bail!("managed tool archive does not contain {}", binary_name());
    }
    if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
        let decoder = flate2::read::GzDecoder::new(Cursor::new(bytes));
        let mut archive = tar::Archive::new(decoder);
        let expected = archive_path
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .map(normalize_archive_entry_name);
        for entry in archive.entries()? {
            let mut entry = entry?;
            let path = normalize_archive_entry_name(&entry.path()?.to_string_lossy());
            let matches = expected
                .as_deref()
                .map(|expected| path == expected)
                .unwrap_or_else(|| {
                    path == binary_name() || path.ends_with(&format!("/{}", binary_name()))
                });
            if matches {
                return read_archive_entry(&mut entry);
            }
        }
        bail!("managed tool archive does not contain {}", binary_name());
    }
    Ok(bytes.to_vec())
}

fn normalize_archive_entry_name(path: &str) -> String {
    path.replace('\\', "/")
}

fn read_archive_entry(reader: &mut impl Read) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    reader
        .take(MAX_DOWNLOAD_BYTES + 1)
        .read_to_end(&mut output)?;
    if output.len() as u64 > MAX_DOWNLOAD_BYTES {
        bail!("managed tool binary exceeds 128 MiB");
    }
    Ok(output)
}

#[cfg(any(target_os = "macos", windows))]
fn verify_platform_signature(path: &Path) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        let output = Command::new("/usr/bin/codesign")
            .args(["--verify", "--strict", "--verbose=2"])
            .arg(path)
            .output()
            .context("failed to run codesign verification")?;
        if !output.status.success() {
            bail!(
                "managed CLI signature verification failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
    }
    #[cfg(windows)]
    {
        let escaped = path.display().to_string().replace('\'', "''");
        let script = format!(
            "$s=Get-AuthenticodeSignature -LiteralPath '{escaped}'; if($s.Status -ne 'Valid'){{Write-Error $s.Status; exit 1}}"
        );
        let mut command = Command::new("powershell.exe");
        command.creation_flags(WINDOWS_CREATE_NO_WINDOW);
        let output = command
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .output()
            .context("failed to run Authenticode verification")?;
        if !output.status.success() {
            bail!(
                "managed CLI Authenticode verification failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
    }
    Ok(())
}

#[cfg(not(any(target_os = "macos", windows)))]
fn verify_platform_signature(_path: &Path) -> Result<()> {
    Ok(())
}

fn validate_cli(path: &Path, expected_version: Option<&str>) -> Result<String> {
    if !path.is_file() {
        bail!("CLI binary does not exist: {}", path.display());
    }
    let mut command = Command::new(path);
    #[cfg(windows)]
    command.creation_flags(WINDOWS_CREATE_NO_WINDOW);
    let output = command
        .args(["--version", "--json"])
        .output()
        .with_context(|| format!("failed to execute {}", path.display()))?;
    if !output.status.success() {
        bail!(
            "CLI version check failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let parsed: CliVersionOutput =
        serde_json::from_slice(&output.stdout).context("CLI version output is not valid JSON")?;
    if !parsed.name.is_empty() && parsed.name != "baijimu" {
        bail!("unexpected CLI identity: {}", parsed.name);
    }
    if !parsed.implementation.is_empty() && parsed.implementation != "rust-native" {
        bail!("unexpected CLI implementation: {}", parsed.implementation);
    }
    if let Some(expected) = expected_version {
        if parsed.version != expected {
            bail!(
                "CLI version mismatch: expected {expected}, got {}",
                parsed.version
            );
        }
    }
    Ok(parsed.version)
}

fn normalize_sha256(value: &str) -> Result<String> {
    let value = value.trim().strip_prefix("sha256:").unwrap_or(value.trim());
    if value.len() != 64 || !value.chars().all(|character| character.is_ascii_hexdigit()) {
        bail!("managed tool checksum must be a 64 character SHA-256 hex value");
    }
    Ok(value.to_ascii_lowercase())
}

fn hex_sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn load_state() -> Result<Option<ManagedToolState>> {
    let path = state_path();
    match fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .with_context(|| format!("failed to parse {}", path.display()))
            .map(Some),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn save_state(state: &ManagedToolState) -> Result<()> {
    let path = state_path();
    let parent = path
        .parent()
        .context("managed tool state path has no parent")?;
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(".{STATE_FILE_NAME}-{}.tmp", now_ms()));
    let bytes = serde_json::to_vec_pretty(state)?;
    let mut file = fs::File::create(&temporary)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    #[cfg(windows)]
    {
        let backup = parent.join(format!(".{STATE_FILE_NAME}-{}.bak", now_ms()));
        if path.exists() {
            fs::rename(&path, &backup)?;
        }
        if let Err(error) = fs::rename(&temporary, &path) {
            if backup.exists() {
                let _ = fs::rename(&backup, &path);
            }
            return Err(error.into());
        }
        let _ = fs::remove_file(backup);
    }
    #[cfg(not(windows))]
    fs::rename(temporary, path)?;
    Ok(())
}

fn managed_root() -> PathBuf {
    if let Some(root) = std::env::var_os("BAIJIMU_MANAGED_TOOL_ROOT") {
        return PathBuf::from(root);
    }
    #[cfg(windows)]
    if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
        return PathBuf::from(local_app_data)
            .join("Baijimu")
            .join("apps")
            .join(TOOL_ID);
    }
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("baijimu")
        .join("apps")
        .join(TOOL_ID)
}

fn versions_dir() -> PathBuf {
    managed_root().join("versions")
}

fn version_binary_path(version: &str) -> PathBuf {
    versions_dir().join(version).join(binary_name())
}

fn state_path() -> PathBuf {
    managed_root().join(STATE_FILE_NAME)
}

fn launcher_path() -> PathBuf {
    if let Some(bin_dir) = std::env::var_os("BAIJIMU_MANAGED_BIN_DIR") {
        return PathBuf::from(bin_dir).join(binary_name());
    }
    #[cfg(windows)]
    if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
        return PathBuf::from(local_app_data)
            .join("Baijimu")
            .join("bin")
            .join(binary_name());
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".local")
        .join("bin")
        .join(binary_name())
}

#[derive(Debug, Clone, Copy)]
struct PathInstallationStatus {
    configured: bool,
    restart_required: bool,
}

#[cfg(not(windows))]
fn ensure_launcher_on_user_path() -> Result<PathInstallationStatus> {
    Ok(PathInstallationStatus {
        configured: true,
        restart_required: false,
    })
}

#[cfg(windows)]
fn ensure_launcher_on_user_path() -> Result<PathInstallationStatus> {
    let launcher_dir = launcher_path()
        .parent()
        .context("managed CLI launcher has no parent directory")?
        .to_path_buf();
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (environment, _) = hkcu
        .create_subkey("Environment")
        .context("failed to open current user environment registry key")?;

    if ensure_windows_user_path_in_registry(&environment, &launcher_dir)? {
        broadcast_environment_change();
    }

    let process_path = std::env::var("PATH").unwrap_or_default();
    Ok(PathInstallationStatus {
        configured: true,
        restart_required: !windows_path_contains_with(&process_path, &launcher_dir, |name| {
            std::env::var(name).ok()
        }),
    })
}

#[cfg(windows)]
fn ensure_windows_user_path_in_registry(environment: &RegKey, launcher_dir: &Path) -> Result<bool> {
    let existing_raw = match environment.get_raw_value("Path") {
        Ok(value) => Some(value),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error).context("failed to read current user PATH"),
    };
    let existing = match existing_raw.as_ref() {
        Some(value) => String::from_reg_value(value)
            .context("current user PATH is not a string registry value")?,
        None => String::new(),
    };
    let merged =
        merge_windows_user_path_with(&existing, launcher_dir, |name| std::env::var(name).ok());
    if merged != existing {
        let value_type = existing_raw
            .as_ref()
            .map(|value| value.vtype.clone())
            .filter(|value_type| *value_type == REG_SZ || *value_type == REG_EXPAND_SZ)
            .unwrap_or(REG_EXPAND_SZ);
        let value = RegValue {
            bytes: merged
                .encode_utf16()
                .chain(std::iter::once(0))
                .flat_map(u16::to_le_bytes)
                .collect(),
            vtype: value_type,
        };
        environment
            .set_raw_value("Path", &value)
            .context("failed to add Baijimu CLI to current user PATH")?;
        return Ok(true);
    }
    Ok(false)
}

#[cfg(windows)]
fn broadcast_environment_change() {
    let environment = "Environment"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let mut result = 0usize;
    unsafe {
        SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            0,
            environment.as_ptr() as isize,
            SMTO_ABORTIFHUNG,
            5_000,
            &mut result,
        );
    }
}

#[cfg(any(windows, test))]
fn merge_windows_user_path_with<F>(existing: &str, launcher_dir: &Path, lookup: F) -> String
where
    F: Fn(&str) -> Option<String> + Copy,
{
    let launcher = launcher_dir.to_string_lossy();
    if existing.is_empty() {
        return launcher.into_owned();
    }
    let mut entries = vec![launcher.into_owned()];
    entries.extend(existing.split(';').filter_map(|entry| {
        if windows_path_entries_equal_with(entry, launcher_dir, lookup) {
            None
        } else {
            Some(entry.to_string())
        }
    }));
    entries.join(";")
}

#[cfg(any(windows, test))]
fn windows_path_contains_with<F>(path: &str, expected: &Path, lookup: F) -> bool
where
    F: Fn(&str) -> Option<String> + Copy,
{
    path.split(';')
        .any(|entry| windows_path_entries_equal_with(entry, expected, lookup))
}

#[cfg(any(windows, test))]
fn windows_path_entries_equal_with<F>(entry: &str, expected: &Path, lookup: F) -> bool
where
    F: Fn(&str) -> Option<String> + Copy,
{
    normalize_windows_path(&expand_windows_env_vars(entry, lookup))
        == normalize_windows_path(&expected.to_string_lossy())
}

#[cfg(any(windows, test))]
fn normalize_windows_path(value: &str) -> String {
    value
        .trim()
        .trim_matches('"')
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_lowercase()
}

#[cfg(any(windows, test))]
fn expand_windows_env_vars<F>(value: &str, lookup: F) -> String
where
    F: Fn(&str) -> Option<String> + Copy,
{
    let mut output = String::with_capacity(value.len());
    let mut remainder = value;
    while let Some(start) = remainder.find('%') {
        output.push_str(&remainder[..start]);
        let after_start = &remainder[start + 1..];
        let Some(end) = after_start.find('%') else {
            output.push_str(&remainder[start..]);
            return output;
        };
        let name = &after_start[..end];
        if let Some(replacement) = lookup(name) {
            output.push_str(&replacement);
        } else {
            output.push('%');
            output.push_str(name);
            output.push('%');
        }
        remainder = &after_start[end + 1..];
    }
    output.push_str(remainder);
    output
}

fn binary_name() -> &'static str {
    if cfg!(windows) {
        "baijimu.exe"
    } else {
        "baijimu"
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests;

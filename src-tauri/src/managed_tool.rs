use anyhow::{bail, Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{Cursor, Read, Write};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(windows)]
use std::os::windows::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const TOOL_ID: &str = "com.baijimu.cli";
const TOOL_NAME: &str = "Baijimu CLI";
const TOOL_DESCRIPTION: &str =
    "百积木官方命令行工具，用于在本机管理工作区、项目、智能体和平台能力。";
const STATE_FILE_NAME: &str = "state.json";
const MAX_DOWNLOAD_BYTES: u64 = 128 * 1024 * 1024;
#[cfg(windows)]
const WINDOWS_CREATE_NO_WINDOW: u32 = 0x08000000;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedToolStatus {
    pub id: String,
    pub name: String,
    pub description: String,
    pub state: String,
    pub installed_version: Option<String>,
    pub bundled_version: Option<String>,
    pub previous_version: Option<String>,
    pub active_path: String,
    pub launcher_path: String,
    pub can_rollback: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManagedToolState {
    schema_version: u32,
    active_version: String,
    previous_version: Option<String>,
    source: String,
    checksum: String,
    installed_at_epoch_ms: u64,
    updated_at_epoch_ms: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CliVersionOutput {
    version: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    implementation: String,
}

pub fn bootstrap_bundled(source: Option<&Path>) -> Result<ManagedToolStatus> {
    fs::create_dir_all(versions_dir())?;

    if let Some(mut state) = load_state()? {
        if let Ok(version) = validate_cli(&version_binary_path(&state.active_version), None) {
            if version != state.active_version {
                state.active_version = version;
                state.updated_at_epoch_ms = now_ms();
                save_state(&state)?;
            }
            repair_launcher(&version_binary_path(&state.active_version))?;
            return inspect(source);
        }
        if let Some(previous) = state.previous_version.clone() {
            if validate_cli(&version_binary_path(&previous), Some(&previous)).is_ok() {
                state.active_version = previous;
                state.previous_version = None;
                state.source = "automatic-recovery".to_string();
                state.updated_at_epoch_ms = now_ms();
                save_state(&state)?;
                repair_launcher(&version_binary_path(&state.active_version))?;
                return inspect(source);
            }
        }
    }

    let launcher = launcher_path();
    if launcher.is_file() {
        if let Ok(version) = validate_cli(&launcher, None) {
            import_binary(&launcher, &version, "legacy-launcher", None)?;
            return inspect(source);
        }
    }

    let source = source.context("bundled baijimu CLI resource not found")?;
    let version = validate_cli(source, None)
        .with_context(|| format!("bundled baijimu CLI is invalid: {}", source.display()))?;
    import_binary(source, &version, "bundled", None)?;
    inspect(Some(source))
}

pub fn inspect(bundled_source: Option<&Path>) -> Result<ManagedToolStatus> {
    let bundled_version = bundled_source.and_then(|path| validate_cli(path, None).ok());
    let launcher = launcher_path();
    let fallback_active_path = managed_root().join("versions");
    let Some(state) = load_state()? else {
        return Ok(ManagedToolStatus {
            id: TOOL_ID.to_string(),
            name: TOOL_NAME.to_string(),
            description: TOOL_DESCRIPTION.to_string(),
            state: "missing".to_string(),
            installed_version: None,
            bundled_version,
            previous_version: None,
            active_path: fallback_active_path.display().to_string(),
            launcher_path: launcher.display().to_string(),
            can_rollback: false,
            detail: "CLI 尚未完成托管安装".to_string(),
        });
    };

    let active = version_binary_path(&state.active_version);
    let active_result = validate_cli(&active, Some(&state.active_version));
    let launcher_result = validate_cli(&launcher, Some(&state.active_version));
    let previous_valid = state
        .previous_version
        .as_deref()
        .is_some_and(|version| validate_cli(&version_binary_path(version), Some(version)).is_ok());
    let (status, detail) = match (active_result, launcher_result) {
        (Ok(_), Ok(_)) => (
            "ready",
            format!("CLI {} 已安装并可从稳定命令路径调用", state.active_version),
        ),
        (Ok(_), Err(error)) => (
            "broken",
            format!("CLI 版本文件正常，但稳定命令入口异常：{error:#}"),
        ),
        (Err(error), _) => ("broken", format!("当前 CLI 版本不可用：{error:#}")),
    };

    Ok(ManagedToolStatus {
        id: TOOL_ID.to_string(),
        name: TOOL_NAME.to_string(),
        description: TOOL_DESCRIPTION.to_string(),
        state: status.to_string(),
        installed_version: Some(state.active_version),
        bundled_version,
        previous_version: state.previous_version,
        active_path: active.display().to_string(),
        launcher_path: launcher.display().to_string(),
        can_rollback: previous_valid,
        detail,
    })
}

pub async fn install_update(
    source: &str,
    expected_version: &str,
    expected_checksum: &str,
    archive_path: Option<&str>,
) -> Result<ManagedToolStatus> {
    let source = source.trim();
    let expected_version = expected_version.trim();
    if expected_version.is_empty() {
        bail!("managed tool version cannot be empty");
    }
    let url = reqwest::Url::parse(source).context("managed tool source must be a valid URL")?;
    if url.scheme() != "https" {
        bail!("managed tool source must use HTTPS");
    }
    let checksum = normalize_sha256(expected_checksum)?;
    let client = Client::builder()
        .connect_timeout(Duration::from_secs(20))
        .timeout(Duration::from_secs(10 * 60))
        .user_agent(concat!(
            "bridge-agent-managed-tool/",
            env!("CARGO_PKG_VERSION")
        ))
        .build()?;
    let response = client
        .get(url)
        .send()
        .await
        .context("failed to download managed tool")?
        .error_for_status()
        .context("managed tool download returned an error")?;
    if response
        .content_length()
        .is_some_and(|length| length > MAX_DOWNLOAD_BYTES)
    {
        bail!("managed tool package exceeds 128 MiB");
    }
    let bytes = response
        .bytes()
        .await
        .context("failed to read managed tool package")?;
    if bytes.len() as u64 > MAX_DOWNLOAD_BYTES {
        bail!("managed tool package exceeds 128 MiB");
    }
    let actual_checksum = hex_sha256(bytes.as_ref());
    if actual_checksum != checksum {
        bail!("managed tool checksum mismatch: expected {checksum}, got {actual_checksum}");
    }

    let binary = extract_binary(bytes.as_ref(), source, archive_path)?;
    let staging_dir = managed_root().join("staging");
    fs::create_dir_all(&staging_dir)?;
    let candidate = staging_dir.join(format!("{}-{}.tmp", binary_name(), now_ms()));
    write_executable(&candidate, &binary)?;
    verify_platform_signature(&candidate)?;
    validate_cli(&candidate, Some(expected_version))?;
    activate_candidate(&candidate, expected_version, source, &actual_checksum)?;
    let _ = fs::remove_file(candidate);
    inspect(None)
}

pub fn rollback() -> Result<ManagedToolStatus> {
    let mut state = load_state()?.context("managed CLI is not installed")?;
    let previous = state
        .previous_version
        .clone()
        .context("no previous CLI version is available")?;
    let previous_binary = version_binary_path(&previous);
    validate_cli(&previous_binary, Some(&previous))?;
    let current = state.active_version;
    state.active_version = previous;
    state.previous_version = Some(current);
    state.source = "rollback".to_string();
    state.updated_at_epoch_ms = now_ms();
    save_state(&state)?;
    repair_launcher(&previous_binary)?;
    inspect(None)
}

fn import_binary(
    source: &Path,
    version: &str,
    source_label: &str,
    checksum: Option<&str>,
) -> Result<()> {
    let bytes = fs::read(source)?;
    let checksum = checksum
        .map(str::to_string)
        .unwrap_or_else(|| hex_sha256(&bytes));
    let staging_dir = managed_root().join("staging");
    fs::create_dir_all(&staging_dir)?;
    let candidate = staging_dir.join(format!("{}-{}.tmp", binary_name(), now_ms()));
    write_executable(&candidate, &bytes)?;
    validate_cli(&candidate, Some(version))?;
    activate_candidate(&candidate, version, source_label, &checksum)?;
    let _ = fs::remove_file(candidate);
    Ok(())
}

fn activate_candidate(candidate: &Path, version: &str, source: &str, checksum: &str) -> Result<()> {
    let version_dir = versions_dir().join(version);
    fs::create_dir_all(&version_dir)?;
    let version_binary = version_dir.join(binary_name());
    replace_file(candidate, &version_binary)?;
    validate_cli(&version_binary, Some(version))?;

    let previous_state = load_state()?;
    let now = now_ms();
    let previous_version = previous_state
        .as_ref()
        .and_then(|state| (state.active_version != version).then(|| state.active_version.clone()));
    let state = ManagedToolState {
        schema_version: 1,
        active_version: version.to_string(),
        previous_version: previous_version.or_else(|| {
            previous_state
                .as_ref()
                .and_then(|state| state.previous_version.clone())
        }),
        source: source.to_string(),
        checksum: checksum.to_string(),
        installed_at_epoch_ms: previous_state
            .as_ref()
            .map(|state| state.installed_at_epoch_ms)
            .unwrap_or(now),
        updated_at_epoch_ms: now,
    };
    save_state(&state)?;
    repair_launcher(&version_binary)
}

fn repair_launcher(active_binary: &Path) -> Result<()> {
    let launcher = launcher_path();
    if let Some(parent) = launcher.parent() {
        fs::create_dir_all(parent)?;
    }
    replace_file(active_binary, &launcher)?;
    validate_cli(&launcher, None)?;
    Ok(())
}

fn replace_file(source: &Path, target: &Path) -> Result<()> {
    let parent = target
        .parent()
        .context("managed tool target has no parent")?;
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(".{}-{}.tmp", binary_name(), now_ms()));
    fs::copy(source, &temporary)?;
    set_executable(&temporary)?;
    #[cfg(windows)]
    {
        let backup = parent.join(format!(".{}-{}.bak", binary_name(), now_ms()));
        if target.exists() {
            fs::rename(target, &backup).with_context(|| {
                format!(
                    "failed to prepare managed tool replacement: {}",
                    target.display()
                )
            })?;
        }
        if let Err(error) = fs::rename(&temporary, target) {
            if backup.exists() {
                let _ = fs::rename(&backup, target);
            }
            return Err(error.into());
        }
        let _ = fs::remove_file(backup);
    }
    #[cfg(not(windows))]
    fs::rename(&temporary, target)?;
    Ok(())
}

fn write_executable(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = fs::File::create(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    set_executable(path)
}

fn set_executable(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        let mut permissions = fs::metadata(path)?.permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions)?;
    }
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
            let mut file = archive
                .by_name(path)
                .with_context(|| format!("managed tool archive does not contain {path}"))?;
            return read_archive_entry(&mut file);
        }
        for index in 0..archive.len() {
            let mut file = archive.by_index(index)?;
            let name = file.name().replace('\\', "/");
            if name == binary_name() || name.ends_with(&format!("/{}", binary_name())) {
                return read_archive_entry(&mut file);
            }
        }
        bail!("managed tool archive does not contain {}", binary_name());
    }
    if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
        let decoder = flate2::read::GzDecoder::new(Cursor::new(bytes));
        let mut archive = tar::Archive::new(decoder);
        let expected = archive_path.map(str::trim).filter(|path| !path.is_empty());
        for entry in archive.entries()? {
            let mut entry = entry?;
            let path = entry.path()?.to_string_lossy().replace('\\', "/");
            let matches = expected
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
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn checksum_accepts_prefixed_sha256() {
        let value = "a".repeat(64);
        assert_eq!(normalize_sha256(&format!("sha256:{value}")).unwrap(), value);
    }

    #[test]
    fn zip_extracts_explicit_cli_path() {
        let cursor = Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(cursor);
        writer
            .start_file::<_, ()>(
                format!("bin/{}", binary_name()),
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        writer.write_all(b"test-cli").unwrap();
        let bytes = writer.finish().unwrap().into_inner();
        assert_eq!(
            extract_binary(
                &bytes,
                "https://example.test/baijimu.zip",
                Some(&format!("bin/{}", binary_name()))
            )
            .unwrap(),
            b"test-cli"
        );
    }

    #[cfg(unix)]
    #[test]
    fn managed_bootstrap_never_downgrades_and_supports_rollback() {
        let _guard = ENV_LOCK.lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("managed");
        let bin = temp.path().join("bin");
        std::env::set_var("BAIJIMU_MANAGED_TOOL_ROOT", &root);
        std::env::set_var("BAIJIMU_MANAGED_BIN_DIR", &bin);

        let bundled = temp.path().join("baijimu-bundled");
        let newer = temp.path().join("baijimu-newer");
        write_fake_cli(&bundled, "0.1.0");
        write_fake_cli(&newer, "0.2.0");

        let initial = bootstrap_bundled(Some(&bundled)).unwrap();
        assert_eq!(initial.installed_version.as_deref(), Some("0.1.0"));

        import_binary(&newer, "0.2.0", "test-update", None).unwrap();
        let after_restart = bootstrap_bundled(Some(&bundled)).unwrap();
        assert_eq!(after_restart.installed_version.as_deref(), Some("0.2.0"));
        assert_eq!(after_restart.previous_version.as_deref(), Some("0.1.0"));

        let rolled_back = rollback().unwrap();
        assert_eq!(rolled_back.installed_version.as_deref(), Some("0.1.0"));
        assert_eq!(rolled_back.previous_version.as_deref(), Some("0.2.0"));

        std::env::remove_var("BAIJIMU_MANAGED_TOOL_ROOT");
        std::env::remove_var("BAIJIMU_MANAGED_BIN_DIR");
    }

    #[cfg(unix)]
    fn write_fake_cli(path: &Path, version: &str) {
        fs::write(
            path,
            format!(
                "#!/bin/sh\nprintf '%s\\n' '{{\"name\":\"baijimu\",\"version\":\"{version}\",\"implementation\":\"rust-native\"}}'\n"
            ),
        )
        .unwrap();
        set_executable(path).unwrap();
    }
}

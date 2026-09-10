use anyhow::{bail, Context, Result};
use semver::Version;
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
use std::sync::{Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};
#[cfg(windows)]
use windows_sys::Win32::UI::WindowsAndMessaging::{
    SendMessageTimeoutW, HWND_BROADCAST, SMTO_ABORTIFHUNG, WM_SETTINGCHANGE,
};
#[cfg(windows)]
use winreg::enums::{HKEY_CURRENT_USER, REG_EXPAND_SZ, REG_SZ};
#[cfg(windows)]
use winreg::types::FromRegValue;
#[cfg(windows)]
use winreg::{RegKey, RegValue};

pub const TOOL_ID: &str = "baijimu-cli";
const TOOL_NAME: &str = "Baijimu CLI";
const TOOL_DESCRIPTION: &str =
    "百积木官方命令行工具，用于在本机管理工作区、项目、智能体和平台能力。";
const STATE_FILE_NAME: &str = "state.json";
const MAX_DOWNLOAD_BYTES: u64 = 128 * 1024 * 1024;
static MANAGED_TOOL_LOCK: Mutex<()> = Mutex::new(());
#[cfg(windows)]
const WINDOWS_CREATE_NO_WINDOW: u32 = 0x08000000;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedToolStatus {
    pub install_source: Option<local_app_contract::InstallSource>,
    pub id: String,
    pub name: String,
    pub description: String,
    pub state: String,
    pub installed_version: Option<String>,
    pub bundled_version: Option<String>,
    pub previous_version: Option<String>,
    pub active_path: String,
    pub launcher_path: String,
    pub path_configured: bool,
    pub restart_required: bool,
    pub can_rollback: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManagedToolState {
    #[serde(default)]
    install_source: Option<local_app_contract::InstallSource>,
    #[serde(default)]
    previous_install_source: Option<local_app_contract::InstallSource>,
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
    let _guard = lock_managed_tool()?;
    bootstrap_bundled_inner(source)
}

fn bootstrap_bundled_inner(source: Option<&Path>) -> Result<ManagedToolStatus> {
    fs::create_dir_all(versions_dir())?;

    if let Some(state) = load_state()? {
        if validate_cli(
            &version_binary_path(&state.active_version),
            Some(&state.active_version),
        )
        .is_ok()
        {
            let launcher = launcher_path();
            if launcher.is_file() {
                if let Ok(version) = validate_cli(&launcher, None) {
                    if version_is_newer(&version, &state.active_version)? {
                        import_binary(&launcher, &version, "newer-stable-launcher", None, None)?;
                        return bootstrap_bundled_inner(source);
                    }
                }
            }
            if let Some(bundled) = source {
                if let Ok(version) = validate_cli(bundled, None) {
                    if version_is_newer(&version, &state.active_version)? {
                        import_binary(
                            bundled,
                            &version,
                            "bundled-upgrade",
                            None,
                            bundled_market_source(bundled, &version)?,
                        )?;
                        return inspect_inner(Some(bundled));
                    }
                }
            }
            repair_launcher(&version_binary_path(&state.active_version))?;
            return inspect_inner(source);
        }
        let launcher = launcher_path();
        if launcher.is_file() {
            if let Ok(version) = validate_cli(&launcher, None) {
                import_binary(&launcher, &version, "recovered-stable-launcher", None, None)?;
                return bootstrap_bundled_inner(source);
            }
        }
        if let Some(previous) = state.previous_version.clone() {
            if validate_cli(&version_binary_path(&previous), Some(&previous)).is_ok() {
                let current = state.active_version.clone();
                let mut recovered = state;
                recovered.active_version = previous;
                recovered.previous_version = Some(current);
                std::mem::swap(
                    &mut recovered.install_source,
                    &mut recovered.previous_install_source,
                );
                recovered.source = "automatic-recovery".to_string();
                recovered.updated_at_epoch_ms = now_ms();
                save_state(&recovered)?;
                repair_launcher(&version_binary_path(&recovered.active_version))?;
                return bootstrap_bundled_inner(source);
            }
        }
    }

    let launcher = launcher_path();
    if launcher.is_file() {
        if let Ok(version) = validate_cli(&launcher, None) {
            import_binary(&launcher, &version, "legacy-launcher", None, None)?;
            return bootstrap_bundled_inner(source);
        }
    }

    let source = source.context("bundled baijimu CLI resource not found")?;
    let version = validate_cli(source, None)
        .with_context(|| format!("bundled baijimu CLI is invalid: {}", source.display()))?;
    import_binary(
        source,
        &version,
        "bundled",
        None,
        bundled_market_source(source, &version)?,
    )?;
    inspect_inner(Some(source))
}

fn version_is_newer(candidate: &str, current: &str) -> Result<bool> {
    let candidate = Version::parse(candidate)
        .with_context(|| format!("invalid managed CLI version: {candidate}"))?;
    let current = Version::parse(current)
        .with_context(|| format!("invalid managed CLI version: {current}"))?;
    Ok(candidate > current)
}

pub fn inspect(bundled_source: Option<&Path>) -> Result<ManagedToolStatus> {
    let _guard = lock_managed_tool()?;
    inspect_inner(bundled_source)
}

fn inspect_inner(bundled_source: Option<&Path>) -> Result<ManagedToolStatus> {
    let bundled_version = bundled_source.and_then(|path| validate_cli(path, None).ok());
    let launcher = launcher_path();
    let fallback_active_path = managed_root().join("versions");
    let Some(state) = load_state()? else {
        return Ok(ManagedToolStatus {
            install_source: None,
            id: TOOL_ID.to_string(),
            name: TOOL_NAME.to_string(),
            description: TOOL_DESCRIPTION.to_string(),
            state: "missing".to_string(),
            installed_version: None,
            bundled_version,
            previous_version: None,
            active_path: fallback_active_path.display().to_string(),
            launcher_path: launcher.display().to_string(),
            path_configured: false,
            restart_required: false,
            can_rollback: false,
            detail: "CLI 尚未完成托管安装".to_string(),
        });
    };

    let active = version_binary_path(&state.active_version);
    let active_result = validate_cli(&active, Some(&state.active_version));
    let launcher_detected = validate_cli(&launcher, None);
    if let Ok(launcher_version) = &launcher_detected {
        if active_result.is_err() || version_is_newer(launcher_version, &state.active_version)? {
            import_binary(
                &launcher,
                launcher_version,
                "external-stable-launcher",
                None,
                None,
            )?;
            return inspect_inner(bundled_source);
        }
        if launcher_version != &state.active_version {
            repair_launcher(&active)?;
            return inspect_inner(bundled_source);
        }
    } else if active_result.is_ok() {
        repair_launcher(&active)?;
        return inspect_inner(bundled_source);
    }

    let launcher_result = validate_cli(&launcher, Some(&state.active_version));
    let previous_valid = state
        .previous_version
        .as_deref()
        .is_some_and(|version| validate_cli(&version_binary_path(version), Some(version)).is_ok());
    let path_status = ensure_launcher_on_user_path()?;
    let (status, mut detail) = match (active_result, launcher_result) {
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
    if status == "ready" && path_status.restart_required {
        detail.push_str("；稳定命令已写入当前用户 PATH，请重新启动已打开的 Codex 或终端");
    }

    Ok(ManagedToolStatus {
        install_source: state.install_source,
        id: TOOL_ID.to_string(),
        name: TOOL_NAME.to_string(),
        description: TOOL_DESCRIPTION.to_string(),
        state: status.to_string(),
        installed_version: Some(state.active_version),
        bundled_version,
        previous_version: state.previous_version,
        active_path: active.display().to_string(),
        launcher_path: launcher.display().to_string(),
        path_configured: path_status.configured,
        restart_required: path_status.restart_required,
        can_rollback: previous_valid,
        detail,
    })
}

pub fn ensure_bundled_dependency_ready(
    tool_id: &str,
    minimum_version: &str,
    bundled_source: Option<&Path>,
) -> Result<ManagedToolStatus> {
    if tool_id != TOOL_ID {
        bail!("unsupported managed tool dependency: {tool_id}");
    }
    let required_version = Version::parse(minimum_version)
        .with_context(|| format!("invalid minimum managed tool version: {minimum_version}"))?;
    let _guard = lock_managed_tool()?;
    let status = bootstrap_bundled_inner(bundled_source)?;
    if status.state != "ready" {
        bail!("managed tool {tool_id} is not ready: {}", status.detail);
    }
    let installed_version_text = status
        .installed_version
        .as_deref()
        .context("managed tool ready state is missing installed version")?;
    let installed_version = Version::parse(installed_version_text).with_context(|| {
        format!("invalid installed managed tool version: {installed_version_text}")
    })?;
    if installed_version < required_version {
        bail!(
            "managed tool {tool_id} requires version {minimum_version} or newer, installed version is {installed_version}"
        );
    }
    validate_cli(
        Path::new(&status.launcher_path),
        Some(installed_version_text),
    )
    .with_context(|| {
        format!(
            "managed tool launcher is not executable: {}",
            status.launcher_path
        )
    })?;
    Ok(status)
}

pub fn install_market_package(
    bytes: &[u8],
    artifact: &local_app_contract::Artifact,
    selection: local_app_contract::InstallSource,
    listing: &local_app_contract::MarketListing,
    archive_path: Option<&str>,
) -> Result<ManagedToolStatus> {
    bridge_agent::market_distribution::resolve_exact(&selection, listing)?;
    if listing.frozen_version.source.application.app_id.as_str() != TOOL_ID
        || listing.frozen_version.content.application_type
            != local_app_contract::ApplicationType::ManagedTool
        || bytes.len() as u64 != artifact.size_bytes
        || bytes.len() as u64 > MAX_DOWNLOAD_BYTES
        || !listing.frozen_version.content.artifacts.contains(artifact)
    {
        bail!("managed tool package does not match selected frozen version");
    }
    let expected_version = listing.frozen_version.source.version.to_string();
    let binary = extract_binary(bytes, artifact.file_name.as_str(), archive_path)?;
    fs::create_dir_all(managed_root())?;
    let staging = tempfile::tempdir_in(managed_root()).context("cannot stage managed tool")?;
    let candidate = staging.path().join(binary_name());
    write_executable(&candidate, &binary)?;
    verify_platform_signature(&candidate)?;
    validate_cli(&candidate, Some(&expected_version))?;
    let _guard = lock_managed_tool()?;
    if let Some(state) = load_state()? {
        if state.install_source.is_some()
            && bridge_agent::market_distribution::select_upgrade(
                state.install_source.as_ref(),
                listing,
            )?
            .is_none()
        {
            bail!("selected tool release is not newer than the installed release");
        }
    }
    // Preserve the existing package checksum used by managed-tool recovery.
    activate_candidate(
        &candidate,
        &expected_version,
        "market",
        &hex_sha256(bytes),
        Some(selection),
    )?;
    inspect_inner(None)
}

pub fn rollback() -> Result<ManagedToolStatus> {
    let _guard = lock_managed_tool()?;
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
    std::mem::swap(
        &mut state.install_source,
        &mut state.previous_install_source,
    );
    state.source = "rollback".to_string();
    state.updated_at_epoch_ms = now_ms();
    save_state(&state)?;
    repair_launcher(&previous_binary)?;
    inspect_inner(None)
}

fn lock_managed_tool() -> Result<MutexGuard<'static, ()>> {
    MANAGED_TOOL_LOCK
        .lock()
        .map_err(|_| anyhow::anyhow!("managed tool operation lock is poisoned"))
}

fn import_binary(
    source: &Path,
    version: &str,
    source_label: &str,
    checksum: Option<&str>,
    install_source: Option<local_app_contract::InstallSource>,
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
    activate_candidate(&candidate, version, source_label, &checksum, install_source)?;
    let _ = fs::remove_file(candidate);
    Ok(())
}

fn activate_candidate(
    candidate: &Path,
    version: &str,
    source: &str,
    checksum: &str,
    install_source: Option<local_app_contract::InstallSource>,
) -> Result<()> {
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
        install_source,
        previous_install_source: previous_state
            .as_ref()
            .and_then(|state| state.install_source.clone()),
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

include!("managed_tool/implementation.rs");

fn bundled_market_source(
    binary: &Path,
    version: &str,
) -> Result<Option<local_app_contract::InstallSource>> {
    let file_name = binary
        .file_name()
        .context("bundled binary has no file name")?
        .to_string_lossy();
    let path = binary.with_file_name(format!("{file_name}.market.json"));
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let selection: local_app_contract::InstallSource = local_app_contract::decode(&bytes)?;
    match &selection {
        local_app_contract::InstallSource::Market {
            listing_id,
            version: selected,
            source,
            ..
        } if !listing_id.is_nil()
            && source.application.app_id.as_str() == TOOL_ID
            && selected == &source.version
            && selected.to_string() == version =>
        {
            Ok(Some(selection))
        }
        _ => bail!("bundled CLI market source does not match its immutable version"),
    }
}

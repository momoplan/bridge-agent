use crate::config::{
    ensure_config_exists, load_config, save_config, RuntimeConfig, ServiceConfig,
    ServiceRegistration, ServiceStartCommand,
};
use anyhow::{bail, Context, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::Read;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

const CONNECTOR_MANIFEST_FILE: &str = "connector.json";
const CONNECTOR_INSTALL_RECORD_FILE: &str = "install.json";
const CONNECTOR_PYTHON_ENV_DIR: &str = ".bridge-agent-python";
const CONNECTOR_PYTHON_REQUIREMENT: &str = ">=3.12,<3.13";
const CONNECTOR_PYTHON_ENV_MARKER: &str = ".install-ok";
const CONNECTOR_DATA_DIR_ENV: &str = "BAIJIMU_CONNECTOR_DATA_DIR";
const CONNECTOR_START_POLICY_ENV: &str = "BAIJIMU_CONNECTOR_START_POLICY";
const CONNECTOR_MANAGEMENT_TOKEN_FILE: &str = "management-token";

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PythonRuntimeStatus {
    pub requirement: String,
    pub configured_path: Option<String>,
    pub detected_path: Option<String>,
    pub version: Option<String>,
    pub compatible: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorManifest {
    pub schema_version: String,
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub publisher: Option<ConnectorPublisher>,
    #[serde(default)]
    pub source: Option<ConnectorSource>,
    #[serde(default)]
    pub runtime: Option<ConnectorRuntime>,
    #[serde(default)]
    pub management: Option<ConnectorManagement>,
    #[serde(default)]
    pub ui: Option<ConnectorUi>,
    #[serde(default)]
    pub config_schema: Option<Value>,
    #[serde(default)]
    pub remote_capabilities: Vec<ConnectorRemoteCapability>,
    #[serde(default)]
    pub permissions: Vec<ConnectorPermission>,
    #[serde(default)]
    pub legacy_autostart_labels: Vec<String>,
    #[serde(default)]
    pub services: Vec<ServiceRegistration>,
    #[serde(default)]
    pub service_registration_files: Vec<String>,
    #[serde(default)]
    pub hooks: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorUi {
    #[serde(rename = "type")]
    pub ui_type: String,
    pub entry: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub default_view: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectorPublisher {
    pub name: String,
    #[serde(default)]
    pub homepage: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectorSource {
    #[serde(rename = "type")]
    pub source_type: String,
    #[serde(default)]
    pub repo: Option<String>,
    #[serde(default)]
    pub revision: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorRuntime {
    #[serde(rename = "type")]
    pub runtime_type: String,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub health_check: Option<Value>,
    #[serde(default = "default_connector_start_policy")]
    pub start_policy: String,
}

fn default_connector_start_policy() -> String {
    "automatic".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorPermission {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub platforms: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorManagement {
    #[serde(rename = "type")]
    pub management_type: String,
    pub base_url: String,
    pub auth: ConnectorManagementAuth,
    #[serde(default)]
    pub operations: BTreeMap<String, ConnectorManagementOperation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectorManagementAuth {
    #[serde(rename = "type")]
    pub auth_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectorManagementOperation {
    pub method: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectorRemoteCapability {
    pub name: String,
    #[serde(default)]
    pub risk: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorInstallRecord {
    pub manifest: ConnectorManifest,
    pub package_path: String,
    pub source_path: String,
    #[serde(default)]
    pub source_reference: Option<String>,
    #[serde(default)]
    pub trust_level: ConnectorTrustLevel,
    #[serde(default)]
    pub market_app_id: Option<String>,
    #[serde(default)]
    pub source_checksum: Option<String>,
    #[serde(default)]
    pub package_checksum: Option<String>,
    pub service_names: Vec<String>,
    pub installed_at_epoch_ms: u64,
    #[serde(default)]
    pub last_synced_at_epoch_ms: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectorTrustLevel {
    PlatformTrusted,
    #[default]
    UserTrusted,
}

#[derive(Debug, Clone, Default)]
pub struct ConnectorInstallProvenance {
    source_reference: Option<String>,
    trust_level: ConnectorTrustLevel,
    market_app_id: Option<String>,
    source_checksum: Option<String>,
}

impl ConnectorInstallProvenance {
    pub fn user_trusted(source_reference: Option<&str>) -> Self {
        Self {
            source_reference: normalized_optional_text(source_reference),
            ..Self::default()
        }
    }

    pub fn platform_trusted(
        source_reference: &str,
        market_app_id: &str,
        source_checksum: &str,
    ) -> Result<Self> {
        let source_reference = normalized_optional_text(Some(source_reference))
            .context("platform-trusted connector source is required")?;
        let market_app_id = normalized_optional_text(Some(market_app_id))
            .context("platform-trusted connector market app id is required")?;
        let source_checksum = normalize_sha256_checksum(source_checksum)?;
        Ok(Self {
            source_reference: Some(source_reference),
            trust_level: ConnectorTrustLevel::PlatformTrusted,
            market_app_id: Some(market_app_id),
            source_checksum: Some(source_checksum),
        })
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorInstallResult {
    pub connector_id: String,
    pub name: String,
    pub version: String,
    pub package_path: String,
    pub service_names: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorSummary {
    pub id: String,
    pub name: String,
    pub version: String,
    pub package_path: String,
    pub source_path: String,
    pub source_reference: Option<String>,
    pub trust_level: ConnectorTrustLevel,
    pub market_app_id: Option<String>,
    pub source_checksum: Option<String>,
    pub package_checksum: Option<String>,
    pub ui: Option<ConnectorUi>,
    pub permissions: Vec<ConnectorPermission>,
    pub start_policy: String,
    pub service_names: Vec<String>,
    pub installed_at_epoch_ms: u64,
    pub last_synced_at_epoch_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorSyncFailure {
    pub connector_id: String,
    pub name: String,
    pub error: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorSyncReport {
    pub summaries: Vec<ConnectorSummary>,
    pub failures: Vec<ConnectorSyncFailure>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorStartResult {
    pub connector_id: String,
    pub services: Vec<ConnectorServiceStartResult>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorServiceStartResult {
    pub service: String,
    pub configured: bool,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

pub fn connectors_dir() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("BRIDGE_AGENT_CONNECTORS_DIR") {
        return Ok(PathBuf::from(path));
    }
    if let Some(dirs) = ProjectDirs::from("com", "baijimu", "bridge-agent") {
        return Ok(dirs.config_dir().join("connectors"));
    }
    bail!("failed to resolve BridgeAgent config directory")
}

pub fn connector_data_dir(connector_id: &str) -> Result<PathBuf> {
    validate_connector_id(connector_id)?;
    if let Some(path) = std::env::var_os("BRIDGE_AGENT_CONNECTOR_DATA_DIR") {
        return Ok(PathBuf::from(path).join(connector_id));
    }
    if let Some(dirs) = ProjectDirs::from("com", "baijimu", "bridge-agent") {
        return Ok(dirs.config_dir().join("connector-data").join(connector_id));
    }
    bail!("failed to resolve BridgeAgent connector data directory")
}

pub fn connector_management_token_path(connector_id: &str) -> Result<PathBuf> {
    Ok(connector_data_dir(connector_id)?.join(CONNECTOR_MANAGEMENT_TOKEN_FILE))
}

pub fn load_connector_manifest(source: &Path) -> Result<ConnectorManifest> {
    let manifest_path = connector_manifest_path(source);
    let content = fs::read_to_string(&manifest_path).with_context(|| {
        format!(
            "failed to read connector manifest {}",
            manifest_path.display()
        )
    })?;
    let manifest: ConnectorManifest = serde_json::from_str(&content).with_context(|| {
        format!(
            "failed to parse connector manifest {}",
            manifest_path.display()
        )
    })?;
    validate_manifest(&manifest)?;
    if let Some(ui) = manifest.ui.as_ref() {
        resolve_connector_ui_entry(source, ui)?;
    }
    Ok(manifest)
}

pub fn resolve_connector_ui_entry(package_path: &Path, ui: &ConnectorUi) -> Result<PathBuf> {
    resolve_connector_ui_asset(package_path, ui, None)
}

pub fn resolve_connector_ui_asset(
    package_path: &Path,
    ui: &ConnectorUi,
    asset_path: Option<&str>,
) -> Result<PathBuf> {
    let package_root = package_path.canonicalize().with_context(|| {
        format!(
            "failed to resolve connector package {}",
            package_path.display()
        )
    })?;
    if !package_root.is_dir() {
        bail!("connector UI requires a package directory");
    }

    let entry_relative = validated_connector_ui_relative_path(&ui.entry, "entry")?;
    let entry = package_root.join(&entry_relative);
    let ui_root = entry
        .parent()
        .with_context(|| "connector UI entry must have a parent directory")?
        .canonicalize()
        .with_context(|| format!("failed to resolve connector UI root {}", entry.display()))?;
    if !ui_root.starts_with(&package_root) {
        bail!("connector UI root escapes the connector package");
    }

    let candidate = match asset_path {
        Some(path) if !path.trim().is_empty() => {
            ui_root.join(validated_connector_ui_relative_path(path, "asset path")?)
        }
        _ => entry,
    };
    let resolved = candidate.canonicalize().with_context(|| {
        format!(
            "failed to resolve connector UI asset {}",
            candidate.display()
        )
    })?;
    if !resolved.starts_with(&ui_root) || !resolved.is_file() {
        bail!("connector UI asset is outside the declared UI directory");
    }
    Ok(resolved)
}

pub fn install_connector_from_path(
    source: &Path,
    config_path: &Path,
    replace: bool,
) -> Result<ConnectorInstallResult> {
    install_connector_from_path_with_source_reference(source, config_path, replace, None)
}

pub fn install_connector_from_path_with_source_reference(
    source: &Path,
    config_path: &Path,
    replace: bool,
    source_reference: Option<&str>,
) -> Result<ConnectorInstallResult> {
    install_connector_from_path_with_provenance(
        source,
        config_path,
        replace,
        ConnectorInstallProvenance::user_trusted(source_reference),
    )
}

pub fn install_connector_from_path_with_provenance(
    source: &Path,
    config_path: &Path,
    replace: bool,
    provenance: ConnectorInstallProvenance,
) -> Result<ConnectorInstallResult> {
    ensure_config_exists(config_path)?;
    let mut config = load_config(config_path)?;
    let source = source
        .canonicalize()
        .unwrap_or_else(|_| source.to_path_buf());
    let manifest = load_connector_manifest(&source)?;
    let registrations = load_connector_service_registrations(&source, &manifest)?;
    if registrations.is_empty() {
        bail!("connector `{}` does not declare any services", manifest.id);
    }

    let service_names = registrations
        .iter()
        .map(|registration| registration.name.trim().to_string())
        .collect::<Vec<_>>();
    let mut services = registrations
        .into_iter()
        .map(ServiceRegistration::into_service_config)
        .collect::<Result<Vec<_>>>()?;

    let package_path = installed_connector_package_path(&manifest)?;
    if package_path.exists() {
        prepare_connector_package_destination(&package_path, replace)?;
    }
    copy_connector_package(&source, &package_path)?;
    let package_checksum = connector_package_sha256(&package_path)?;
    resolve_installed_start_commands(&mut services, &package_path, &config.runtime, &manifest)?;
    cleanup_legacy_connector_autostarts_for_manifest(&manifest);

    for service in &services {
        upsert_service(&mut config.services, service.clone(), replace)?;
    }
    save_config(config_path, &config)?;

    let now = now_ms();
    let installed_at_epoch_ms = load_install_record(&manifest.id)
        .map(|record| record.installed_at_epoch_ms)
        .unwrap_or(now);
    let record = ConnectorInstallRecord {
        manifest: manifest.clone(),
        package_path: package_path.display().to_string(),
        source_path: source.display().to_string(),
        source_reference: provenance.source_reference,
        trust_level: provenance.trust_level,
        market_app_id: provenance.market_app_id,
        source_checksum: provenance.source_checksum,
        package_checksum: Some(package_checksum),
        service_names: service_names.clone(),
        installed_at_epoch_ms,
        last_synced_at_epoch_ms: now,
    };
    save_install_record(&record)?;

    Ok(ConnectorInstallResult {
        connector_id: manifest.id,
        name: manifest.name,
        version: manifest.version,
        package_path: package_path.display().to_string(),
        service_names,
    })
}

fn normalized_optional_text(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn normalize_sha256_checksum(value: &str) -> Result<String> {
    let value = value.trim().strip_prefix("sha256:").unwrap_or(value.trim());
    if value.len() != 64 || !value.chars().all(|character| character.is_ascii_hexdigit()) {
        bail!("platform-trusted connector source requires a valid SHA-256 checksum");
    }
    Ok(format!("sha256:{}", value.to_ascii_lowercase()))
}

pub fn uninstall_connector(connector_id: &str, config_path: &Path) -> Result<ConnectorSummary> {
    ensure_config_exists(config_path)?;
    let record = load_install_record(connector_id)?;
    let _ = stop_connector(connector_id, config_path);
    cleanup_legacy_connector_autostarts_for_manifest(&record.manifest);

    let mut config = load_config(config_path)?;
    config.services.retain(|service| {
        !record
            .service_names
            .iter()
            .any(|name| name == &service.name)
    });
    save_config(config_path, &config)?;

    let install_root = install_record_path(connector_id)?
        .parent()
        .with_context(|| format!("connector `{connector_id}` install root is missing"))?
        .to_path_buf();
    if install_root.exists() {
        fs::remove_dir_all(&install_root).with_context(|| {
            format!(
                "failed to remove connector installation {}",
                install_root.display()
            )
        })?;
    }

    Ok(summary_from_record(record))
}

pub fn list_connectors() -> Result<Vec<ConnectorSummary>> {
    let mut connectors = load_install_records()?
        .into_iter()
        .map(summary_from_record)
        .collect::<Vec<_>>();
    connectors.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(connectors)
}

pub fn show_connector(connector_id: &str) -> Result<ConnectorInstallRecord> {
    load_install_record(connector_id)
}

pub fn sync_installed_connectors(config_path: &Path) -> Result<Vec<ConnectorSummary>> {
    let report = sync_installed_connectors_report(config_path)?;
    if !report.failures.is_empty() {
        bail!("{}", format_connector_sync_failures(&report.failures));
    }
    Ok(report.summaries)
}

pub fn sync_installed_connector(
    config_path: &Path,
    connector_id: &str,
) -> Result<ConnectorSummary> {
    ensure_config_exists(config_path)?;
    let mut config = load_config(config_path)?;
    let record = load_install_record(connector_id)?;
    let summary = sync_installed_connector_record(&mut config, record, now_ms())?;
    save_config(config_path, &config)?;
    Ok(summary)
}

pub fn sync_installed_connectors_report(config_path: &Path) -> Result<ConnectorSyncReport> {
    ensure_config_exists(config_path)?;
    let records = load_install_records()?;
    if records.is_empty() {
        return Ok(ConnectorSyncReport {
            summaries: Vec::new(),
            failures: Vec::new(),
        });
    }

    let mut config = load_config(config_path)?;
    let now = now_ms();
    let mut summaries = Vec::new();
    let mut failures = Vec::new();

    for record in records {
        let connector_id = record.manifest.id.clone();
        let name = record.manifest.name.clone();
        match sync_installed_connector_record(&mut config, record, now) {
            Ok(summary) => summaries.push(summary),
            Err(err) => {
                let error = format!("{err:#}");
                tracing::warn!(
                    connector_id = %connector_id,
                    name = %name,
                    error = %error,
                    "failed to sync installed connector"
                );
                failures.push(ConnectorSyncFailure {
                    connector_id,
                    name,
                    error,
                });
            }
        }
    }

    save_config(config_path, &config)?;
    Ok(ConnectorSyncReport {
        summaries,
        failures,
    })
}

pub fn format_connector_sync_failures(failures: &[ConnectorSyncFailure]) -> String {
    if failures.is_empty() {
        return "no connector sync failures".to_string();
    }
    let details = failures
        .iter()
        .map(|failure| {
            format!(
                "{} ({}) failed: {}",
                failure.name, failure.connector_id, failure.error
            )
        })
        .collect::<Vec<_>>()
        .join("; ");
    format!(
        "failed to sync {} installed connector(s): {details}",
        failures.len()
    )
}

fn sync_installed_connector_record(
    config: &mut crate::config::AgentConfig,
    mut record: ConnectorInstallRecord,
    now: u64,
) -> Result<ConnectorSummary> {
    cleanup_legacy_connector_autostarts_for_manifest(&record.manifest);
    let package_path = PathBuf::from(&record.package_path);
    let connector_id = record.manifest.id.clone();
    let registrations = load_connector_service_registrations(&package_path, &record.manifest)
        .with_context(|| {
            format!("failed to reload service registrations for connector `{connector_id}`")
        })?;
    let mut services = registrations
        .into_iter()
        .map(ServiceRegistration::into_service_config)
        .collect::<Result<Vec<_>>>()?;
    resolve_installed_start_commands(
        &mut services,
        &package_path,
        &config.runtime,
        &record.manifest,
    )?;

    for service in services {
        upsert_synced_service(&mut config.services, service);
    }

    record.last_synced_at_epoch_ms = now;
    save_install_record(&record)?;
    Ok(summary_from_record(record))
}

pub fn start_connector(connector_id: &str, config_path: &Path) -> Result<ConnectorStartResult> {
    ensure_config_exists(config_path)?;
    sync_installed_connector(config_path, connector_id)?;
    let record = load_install_record(connector_id)?;
    let config = load_config(config_path)?;
    let mut results = Vec::new();
    for service_name in &record.service_names {
        let service = config
            .services
            .iter()
            .find(|service| &service.name == service_name);
        let result = match service.and_then(|service| service.start_command.as_ref()) {
            Some(command) => run_start_command(service_name, command)?,
            None => ConnectorServiceStartResult {
                service: service_name.clone(),
                configured: false,
                exit_code: None,
                stdout: String::new(),
                stderr: "start command is not configured".to_string(),
            },
        };
        results.push(result);
    }
    Ok(ConnectorStartResult {
        connector_id: record.manifest.id,
        services: results,
    })
}

pub fn stop_connector(connector_id: &str, config_path: &Path) -> Result<ConnectorStartResult> {
    ensure_config_exists(config_path)?;
    let record = load_install_record(connector_id)?;
    let config = load_config(config_path)?;
    let mut results = Vec::new();
    for service_name in &record.service_names {
        let service = config
            .services
            .iter()
            .find(|service| &service.name == service_name);
        let result = match service.and_then(|service| service.stop_command.as_ref()) {
            Some(command) => run_start_command(service_name, command)?,
            None => ConnectorServiceStartResult {
                service: service_name.clone(),
                configured: false,
                exit_code: None,
                stdout: String::new(),
                stderr: "stop command is not configured".to_string(),
            },
        };
        results.push(result);
    }
    Ok(ConnectorStartResult {
        connector_id: record.manifest.id,
        services: results,
    })
}

fn validate_manifest(manifest: &ConnectorManifest) -> Result<()> {
    if !matches!(manifest.schema_version.as_str(), "1.0" | "1.1" | "1.2") {
        bail!(
            "connector schemaVersion `{}` is not supported; expected 1.0, 1.1 or 1.2",
            manifest.schema_version
        );
    }
    if manifest.id.trim().is_empty() {
        bail!("connector id cannot be empty");
    }
    if manifest.name.trim().is_empty() {
        bail!("connector name cannot be empty");
    }
    if manifest.version.trim().is_empty() {
        bail!("connector version cannot be empty");
    }
    if manifest.services.is_empty() && manifest.service_registration_files.is_empty() {
        bail!("connector must declare services or serviceRegistrationFiles");
    }
    validate_connector_id(&manifest.id)?;
    if let Some(management) = manifest.management.as_ref() {
        validate_management(management)?;
    }
    if let Some(ui) = manifest.ui.as_ref() {
        if manifest.schema_version == "1.0" {
            bail!("connector ui requires schemaVersion 1.1 or newer");
        }
        validate_connector_ui(ui)?;
    }
    let start_policy = manifest
        .runtime
        .as_ref()
        .map(|runtime| runtime.start_policy.trim())
        .unwrap_or("automatic");
    if !matches!(start_policy, "automatic" | "manual") {
        bail!("connector runtime.startPolicy must be automatic or manual");
    }
    if manifest.schema_version != "1.2"
        && (!manifest.permissions.is_empty()
            || !manifest.legacy_autostart_labels.is_empty()
            || start_policy == "manual")
    {
        bail!(
            "connector permissions, legacyAutostartLabels and manual startPolicy require schemaVersion 1.2"
        );
    }
    for permission in &manifest.permissions {
        if permission.id.trim().is_empty() || permission.title.trim().is_empty() {
            bail!("connector permission id and title cannot be empty");
        }
    }
    Ok(())
}

fn validate_connector_ui(ui: &ConnectorUi) -> Result<()> {
    if ui.ui_type != "embedded" {
        bail!("connector ui.type must be embedded");
    }
    let entry = validated_connector_ui_relative_path(&ui.entry, "entry")?;
    if entry
        .parent()
        .is_none_or(|parent| parent.as_os_str().is_empty())
    {
        bail!("connector ui.entry must be inside a dedicated UI directory");
    }
    if entry.extension().and_then(|value| value.to_str()) != Some("html") {
        bail!("connector ui.entry must point to an .html file");
    }
    if ui
        .title
        .as_deref()
        .is_some_and(|title| title.trim().is_empty() || title.chars().count() > 64)
    {
        bail!("connector ui.title must contain 1 to 64 characters");
    }
    Ok(())
}

fn validated_connector_ui_relative_path(value: &str, field: &str) -> Result<PathBuf> {
    let normalized = value.trim();
    if normalized.is_empty() || normalized.contains('\\') {
        bail!("connector UI {field} must be a non-empty forward-slash relative path");
    }
    let path = Path::new(normalized);
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        bail!("connector UI {field} must stay inside the UI directory");
    }
    Ok(path.to_path_buf())
}

fn validate_connector_id(connector_id: &str) -> Result<()> {
    let connector_id = connector_id.trim();
    if connector_id.is_empty()
        || connector_id == "."
        || connector_id == ".."
        || sanitize_path_component(connector_id) != connector_id
    {
        bail!("connector id contains unsupported characters")
    }
    Ok(())
}

fn validate_management(management: &ConnectorManagement) -> Result<()> {
    if !management.management_type.eq_ignore_ascii_case("http") {
        bail!("connector management.type must be http");
    }
    if management.auth.auth_type != "connector_token" {
        bail!("connector management.auth.type must be connector_token");
    }
    let url = url::Url::parse(&management.base_url)
        .with_context(|| "connector management.baseUrl must be a valid URL")?;
    if url.scheme() != "http" || !url.has_host() {
        bail!("connector management.baseUrl must use local HTTP");
    }
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
    {
        bail!("connector management.baseUrl must be an origin-only URL");
    }
    let host = url.host_str().unwrap_or_default();
    if host != "localhost"
        && host != "127.0.0.1"
        && host != "::1"
        && url.host().is_none_or(|host| match host {
            url::Host::Ipv4(address) => !address.is_loopback(),
            url::Host::Ipv6(address) => !address.is_loopback(),
            url::Host::Domain(_) => true,
        })
    {
        bail!("connector management.baseUrl must be loopback-only");
    }
    if management.operations.is_empty() {
        bail!("connector management.operations cannot be empty");
    }
    for (name, operation) in &management.operations {
        if name.trim().is_empty() || sanitize_path_component(name) != *name {
            bail!("connector management operation name is invalid");
        }
        if !matches!(operation.method.as_str(), "GET" | "POST") {
            bail!("connector management operation `{name}` method must be GET or POST");
        }
        if !operation.path.starts_with("/management/")
            || operation.path.contains('?')
            || operation.path.contains('#')
        {
            bail!("connector management operation `{name}` path is invalid");
        }
    }
    Ok(())
}

fn connector_manifest_path(source: &Path) -> PathBuf {
    if source.is_file() {
        source.to_path_buf()
    } else {
        source.join(CONNECTOR_MANIFEST_FILE)
    }
}

fn load_connector_service_registrations(
    source: &Path,
    manifest: &ConnectorManifest,
) -> Result<Vec<ServiceRegistration>> {
    let mut registrations = manifest.services.clone();
    for file in &manifest.service_registration_files {
        let path = source.join(file);
        let content = fs::read_to_string(&path)
            .with_context(|| format!("failed to read service registration {}", path.display()))?;
        let registration: ServiceRegistration = serde_json::from_str(&content)
            .with_context(|| format!("failed to parse service registration {}", path.display()))?;
        registrations.push(registration);
    }
    Ok(registrations)
}

fn installed_connector_package_path(manifest: &ConnectorManifest) -> Result<PathBuf> {
    Ok(connectors_dir()?
        .join(sanitize_path_component(&manifest.id))
        .join("package"))
}

fn install_record_path(connector_id: &str) -> Result<PathBuf> {
    Ok(connectors_dir()?
        .join(sanitize_path_component(connector_id))
        .join(CONNECTOR_INSTALL_RECORD_FILE))
}

fn save_install_record(record: &ConnectorInstallRecord) -> Result<()> {
    let path = install_record_path(&record.manifest.id)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create connector dir {}", parent.display()))?;
    }
    fs::write(
        &path,
        format!("{}\n", serde_json::to_string_pretty(record)?),
    )
    .with_context(|| {
        format!(
            "failed to write connector install record {}",
            path.display()
        )
    })?;
    Ok(())
}

fn load_install_records() -> Result<Vec<ConnectorInstallRecord>> {
    let dir = connectors_dir()?;
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut records = Vec::new();
    for entry in fs::read_dir(&dir).with_context(|| format!("failed to read {}", dir.display()))? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let id = entry.file_name().to_string_lossy().to_string();
        match load_install_record(&id) {
            Ok(record) => records.push(record),
            Err(err) => tracing::warn!("failed to load connector `{id}`: {err:#}"),
        }
    }
    records.sort_by(|left, right| left.manifest.id.cmp(&right.manifest.id));
    Ok(records)
}

fn load_install_record(connector_id: &str) -> Result<ConnectorInstallRecord> {
    let path = install_record_path(connector_id)?;
    let content = fs::read_to_string(&path)
        .with_context(|| format!("failed to read connector install record {}", path.display()))?;
    serde_json::from_str(&content).with_context(|| {
        format!(
            "failed to parse connector install record {}",
            path.display()
        )
    })
}

fn prepare_connector_package_destination(package_path: &Path, replace: bool) -> Result<()> {
    if !replace {
        bail!(
            "connector package already exists at {}; pass replace to overwrite it",
            package_path.display()
        );
    }
    match fs::remove_dir_all(package_path) {
        Ok(()) => Ok(()),
        Err(remove_err) => {
            let quarantine_path =
                quarantine_connector_package_path(package_path).with_context(|| {
                    format!(
                        "failed to replace connector {}; also failed to move aside the existing package after remove failed: {remove_err}",
                        package_path.display()
                    )
                })?;
            tracing::warn!(
                "moved undeletable connector package {} to {} after remove failed: {remove_err}",
                package_path.display(),
                quarantine_path.display()
            );
            Ok(())
        }
    }
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
        fs::rename(package_path, &quarantine_path).with_context(|| {
            format!(
                "failed to move {} to {}",
                package_path.display(),
                quarantine_path.display()
            )
        })?;
        return Ok(quarantine_path);
    }
    bail!(
        "failed to choose replacement path for existing connector package {}",
        package_path.display()
    )
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

fn resolve_installed_start_commands(
    services: &mut [ServiceConfig],
    package_path: &Path,
    runtime_config: &RuntimeConfig,
    manifest: &ConnectorManifest,
) -> Result<()> {
    let data_dir = connector_data_dir(&manifest.id)?;
    let start_policy = manifest
        .runtime
        .as_ref()
        .map(|runtime| runtime.start_policy.trim())
        .filter(|value| !value.is_empty())
        .unwrap_or("automatic");
    if !matches!(start_policy, "automatic" | "manual") {
        bail!(
            "connector `{}` has unsupported runtime.startPolicy `{start_policy}`",
            manifest.id
        );
    }
    fs::create_dir_all(&data_dir).with_context(|| {
        format!(
            "failed to create connector data directory {}",
            data_dir.display()
        )
    })?;
    #[cfg(unix)]
    fs::set_permissions(&data_dir, fs::Permissions::from_mode(0o700))?;
    let package_bins = read_package_bins(package_path)?;
    let python_scripts = read_python_project_scripts(package_path)?;
    let python_env =
        ensure_python_project_environment(package_path, &python_scripts, runtime_config)?;
    let node_path = resolve_command_path("node", runtime_config);
    let codex_path = resolve_command_path("codex", runtime_config);
    let command_runtime = InstalledCommandRuntime {
        package_path,
        package_bins: &package_bins,
        python_scripts: &python_scripts,
        python_env: python_env.as_deref(),
        node_path: &node_path,
        codex_path: &codex_path,
    };
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
            env.insert(
                CONNECTOR_DATA_DIR_ENV.to_string(),
                data_dir.display().to_string(),
            );
            env.insert(
                CONNECTOR_START_POLICY_ENV.to_string(),
                start_policy.to_string(),
            );
            resolve_installed_shell_command(command, cwd, env, &command_runtime);
        }
    }
    Ok(())
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
    codex_path: &'a Option<PathBuf>,
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
        enrich_start_command_env(env, [runtime.node_path, runtime.codex_path]);
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
            enrich_start_command_env(env, [runtime.node_path, runtime.codex_path]);
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
        enrich_start_command_env(env, [runtime.node_path, runtime.codex_path]);
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

fn enrich_start_command_env<'a>(
    env_vars: &mut BTreeMap<String, String>,
    executable_paths: impl IntoIterator<Item = &'a Option<PathBuf>>,
) {
    let mut path_entries = Vec::new();
    let mut codex_binary = None;
    for executable_path in executable_paths
        .into_iter()
        .filter_map(|path| path.as_ref())
    {
        if executable_path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name == "codex")
        {
            codex_binary = Some(executable_path.display().to_string());
        }
        if let Some(parent) = executable_path.parent() {
            push_unique_path_entry(&mut path_entries, parent.to_path_buf());
        }
    }
    append_split_path(&mut path_entries, env_vars.get("PATH"));
    append_split_path(&mut path_entries, env::var("PATH").ok().as_ref());
    if let Some(shell_path) = login_shell_path() {
        append_split_path(&mut path_entries, Some(&shell_path));
    }
    if let Ok(joined_path) = env::join_paths(path_entries) {
        env_vars.insert(
            "PATH".to_string(),
            joined_path.to_string_lossy().to_string(),
        );
    }
    if let Some(codex_binary) = codex_binary {
        env_vars
            .entry("CODEX_CONNECTOR_CODEX_BINARY".to_string())
            .or_insert(codex_binary);
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
        .or_else(|| find_command_in_path(executable, env::var("PATH").ok().as_ref()))
        .or_else(|| {
            login_shell_path().and_then(|path| find_command_in_path(executable, Some(&path)))
        })
        .or_else(|| bundled_runtime_command_path(executable))
}

fn configured_runtime_command_path(
    executable: &str,
    runtime_config: &RuntimeConfig,
) -> Option<PathBuf> {
    let path = match executable {
        "node" => runtime_config.node_path.as_deref(),
        "codex" => runtime_config.codex_binary_path.as_deref(),
        _ => None,
    }?;
    let path = PathBuf::from(path.trim());
    path.is_file().then_some(path)
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
    let path = path?;
    env::split_paths(path)
        .map(|dir| dir.join(executable))
        .find(|candidate| candidate.is_file())
}

fn login_shell_path() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        let output = Command::new("/bin/zsh")
            .args(["-lc", "printf %s \"$PATH\""])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if path.is_empty() {
            None
        } else {
            Some(path)
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
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

fn read_python_project_scripts(package_path: &Path) -> Result<BTreeMap<String, String>> {
    let path = package_path.join("pyproject.toml");
    if !path.exists() {
        return Ok(BTreeMap::new());
    }
    let content = fs::read_to_string(&path)
        .with_context(|| format!("failed to read Python project metadata {}", path.display()))?;
    let project: toml::Value = toml::from_str(&content)
        .with_context(|| format!("failed to parse Python project metadata {}", path.display()))?;
    let mut scripts = BTreeMap::new();
    let Some(table) = project
        .get("project")
        .and_then(|value| value.get("scripts"))
        .and_then(toml::Value::as_table)
    else {
        return Ok(scripts);
    };
    for (name, value) in table {
        let Some(entrypoint) = value.as_str() else {
            continue;
        };
        let entrypoint = entrypoint.trim();
        let Some(module) = entrypoint.split(':').next() else {
            continue;
        };
        let module = module.trim();
        if !name.trim().is_empty() && !module.is_empty() && !entrypoint.is_empty() {
            scripts.insert(name.trim().to_string(), entrypoint.to_string());
        }
    }
    Ok(scripts)
}

fn ensure_python_project_environment(
    package_path: &Path,
    scripts: &BTreeMap<String, String>,
    runtime_config: &RuntimeConfig,
) -> Result<Option<PathBuf>> {
    if scripts.is_empty() {
        return Ok(None);
    }
    let env_path = package_path.join(CONNECTOR_PYTHON_ENV_DIR);
    let python = python_env_executable(&env_path);
    let requires_python = read_python_requires_python(package_path)?;
    let base_python = resolve_python_for_project(requires_python.as_deref(), runtime_config)?;
    if python.exists() && !python_env_uses_base_interpreter(&python, Path::new(&base_python)) {
        fs::remove_dir_all(&env_path).with_context(|| {
            format!(
                "failed to recreate Python connector environment {} after interpreter change",
                env_path.display()
            )
        })?;
    }
    if !python.exists() {
        create_python_env(&env_path, &base_python)?;
    }
    if python_project_install_needed(package_path, &env_path)? {
        install_python_project_dependencies(package_path, &python)?;
        write_python_project_scripts(package_path, &env_path, scripts)?;
        mark_python_project_installed(package_path, &env_path)?;
    } else {
        write_python_project_scripts(package_path, &env_path, scripts)?;
    }
    Ok(Some(env_path))
}

fn python_project_install_needed(package_path: &Path, env_path: &Path) -> Result<bool> {
    let marker = env_path.join(CONNECTOR_PYTHON_ENV_MARKER);
    if !marker.exists() {
        return Ok(true);
    }
    let marker_modified = marker
        .metadata()
        .and_then(|metadata| metadata.modified())
        .with_context(|| format!("failed to inspect {}", marker.display()))?;
    for relative in [
        "pyproject.toml",
        "requirements.lock",
        "setup.py",
        "setup.cfg",
    ] {
        let path = package_path.join(relative);
        if !path.exists() {
            continue;
        }
        let modified = path
            .metadata()
            .and_then(|metadata| metadata.modified())
            .with_context(|| format!("failed to inspect {}", path.display()))?;
        if modified > marker_modified {
            return Ok(true);
        }
    }
    Ok(false)
}

fn create_python_env(env_path: &Path, base_python: &str) -> Result<()> {
    let parent = env_path
        .parent()
        .with_context(|| format!("failed to resolve parent for {}", env_path.display()))?;
    fs::create_dir_all(parent).with_context(|| format!("failed to create {}", parent.display()))?;
    let output = Command::new(base_python)
        .args(["-m", "venv"])
        .arg(env_path)
        .output()
        .with_context(|| {
            format!("failed to create Python environment with `{base_python} -m venv`")
        })?;
    if !output.status.success() {
        bail!(
            "failed to create Python connector environment {}\nstdout:\n{}\nstderr:\n{}",
            env_path.display(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}

fn python_env_uses_base_interpreter(env_python: &Path, base_python: &Path) -> bool {
    let output = Command::new(env_python)
        .args([
            "-I",
            "-c",
            "import os,sys; print(os.path.realpath(sys._base_executable))",
        ])
        .output();
    let Ok(output) = output else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    let current = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
    let expected = base_python
        .canonicalize()
        .unwrap_or_else(|_| base_python.to_path_buf());
    let current = current.canonicalize().unwrap_or(current);
    current == expected
}

fn read_python_requires_python(package_path: &Path) -> Result<Option<String>> {
    let Some(project) = read_python_project_metadata(package_path)? else {
        return Ok(None);
    };
    Ok(project
        .get("project")
        .and_then(|value| value.get("requires-python"))
        .and_then(toml::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string))
}

fn read_python_project_dependencies(package_path: &Path) -> Result<Vec<String>> {
    let Some(project) = read_python_project_metadata(package_path)? else {
        return Ok(Vec::new());
    };
    let Some(dependencies) = project
        .get("project")
        .and_then(|value| value.get("dependencies"))
        .and_then(toml::Value::as_array)
    else {
        return Ok(Vec::new());
    };
    Ok(dependencies
        .iter()
        .filter_map(toml::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect())
}

fn read_python_project_metadata(package_path: &Path) -> Result<Option<toml::Value>> {
    let path = package_path.join("pyproject.toml");
    if !path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(&path)
        .with_context(|| format!("failed to read Python project metadata {}", path.display()))?;
    let project = toml::from_str(&content)
        .with_context(|| format!("failed to parse Python project metadata {}", path.display()))?;
    Ok(Some(project))
}

fn resolve_python_for_project(
    requires_python: Option<&str>,
    runtime_config: &RuntimeConfig,
) -> Result<String> {
    let requirement = requires_python.unwrap_or(CONNECTOR_PYTHON_REQUIREMENT);
    for candidate in python_candidates(runtime_config) {
        if python_matches_requirement(&candidate, requires_python) {
            return Ok(candidate.display().to_string());
        }
    }
    bail!(
        "failed to find a Python interpreter matching {requirement}. Install Python 3.12, then set runtime.python_path to its absolute executable path.",
    )
}

pub fn inspect_python_runtime(runtime_config: &RuntimeConfig) -> PythonRuntimeStatus {
    let requirement = CONNECTOR_PYTHON_REQUIREMENT.to_string();
    let configured_path = runtime_config
        .python_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);

    for candidate in python_candidates(runtime_config) {
        let Some(version) = python_version(&candidate) else {
            continue;
        };
        let version_text = format!("{}.{}.{}", version.0, version.1, version.2);
        let detected_path = candidate
            .canonicalize()
            .unwrap_or(candidate)
            .display()
            .to_string();
        let compatible = python_version_satisfies_requirement(version, &requirement);
        let message = if compatible {
            format!("已检测到 Python {version_text}，可供本地应用使用。")
        } else {
            format!(
                "当前解释器是 Python {version_text}，不满足 {requirement}；请安装并选择 Python 3.12。"
            )
        };
        return PythonRuntimeStatus {
            requirement,
            configured_path,
            detected_path: Some(detected_path),
            version: Some(version_text),
            compatible,
            message,
        };
    }

    PythonRuntimeStatus {
        requirement,
        configured_path,
        detected_path: None,
        version: None,
        compatible: false,
        message: "未检测到可执行的 Python 3.12。请先安装 Python 3.12，再选择其可执行文件。"
            .to_string(),
    }
}

fn python_candidates(runtime_config: &RuntimeConfig) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(value) = runtime_config
        .python_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        push_unique_path_entry(&mut candidates, PathBuf::from(value));
        return candidates;
    }
    if let Some(value) = env::var("BRIDGE_AGENT_PYTHON")
        .ok()
        .filter(|value| !value.trim().is_empty())
    {
        push_unique_path_entry(&mut candidates, PathBuf::from(value));
    }
    for executable in ["python3.12", "python3", "python"] {
        for path in find_commands_in_paths(executable) {
            push_unique_path_entry(&mut candidates, path);
        }
    }
    for path in [
        "/opt/homebrew/bin/python3",
        "/usr/local/bin/python3",
        "/opt/anaconda3/bin/python",
        "/usr/bin/python3",
    ] {
        let path = PathBuf::from(path);
        if path.is_file() {
            push_unique_path_entry(&mut candidates, path);
        }
    }
    candidates
}

fn find_commands_in_paths(executable: &str) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Ok(path) = env::var("PATH") {
        append_split_path(&mut paths, Some(&path));
    }
    if let Some(path) = login_shell_path() {
        append_split_path(&mut paths, Some(&path));
    }
    paths
        .into_iter()
        .map(|dir| dir.join(executable))
        .filter(|candidate| candidate.is_file())
        .collect()
}

fn python_matches_requirement(candidate: &Path, requires_python: Option<&str>) -> bool {
    let Some(version) = python_version(candidate) else {
        return false;
    };
    python_version_satisfies_requirement(version, CONNECTOR_PYTHON_REQUIREMENT)
        && requires_python
            .map(|requirement| python_version_satisfies_requirement(version, requirement))
            .unwrap_or(true)
}

fn python_version_satisfies_requirement(version: (u32, u32, u32), requirement: &str) -> bool {
    requirement.split(',').map(str::trim).all(|part| {
        if let Some(required) = part.strip_prefix(">=").and_then(parse_python_version) {
            return version >= required;
        }
        if let Some(required) = part.strip_prefix("<=").and_then(parse_python_version) {
            return version <= required;
        }
        if let Some(required) = part.strip_prefix("==").and_then(parse_python_version) {
            return version == required;
        }
        if let Some(required) = part.strip_prefix('>').and_then(parse_python_version) {
            return version > required;
        }
        if let Some(required) = part.strip_prefix('<').and_then(parse_python_version) {
            return version < required;
        }
        false
    })
}

fn python_version(candidate: &Path) -> Option<(u32, u32, u32)> {
    let output = Command::new(candidate).arg("--version").output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = if output.stdout.is_empty() {
        String::from_utf8_lossy(&output.stderr)
    } else {
        String::from_utf8_lossy(&output.stdout)
    };
    parse_python_version(&text)
}

fn parse_python_version(value: &str) -> Option<(u32, u32, u32)> {
    let version = value
        .split_whitespace()
        .find(|part| part.chars().next().is_some_and(|ch| ch.is_ascii_digit()))?;
    let mut parts = version.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts
        .next()
        .and_then(|part| {
            part.chars()
                .take_while(|ch| ch.is_ascii_digit())
                .collect::<String>()
                .parse()
                .ok()
        })
        .unwrap_or(0);
    Some((major, minor, patch))
}

fn install_python_project_dependencies(package_path: &Path, python: &Path) -> Result<()> {
    let lock_path = package_path.join("requirements.lock");
    if lock_path.is_file() {
        let output = Command::new(python)
            .args([
                "-I",
                "-m",
                "pip",
                "install",
                "--disable-pip-version-check",
                "--requirement",
            ])
            .arg(&lock_path)
            .output()
            .with_context(|| {
                format!(
                    "failed to install locked Python connector dependencies with {}",
                    python.display()
                )
            })?;
        if !output.status.success() {
            bail!(
                "failed to install locked Python connector dependencies for {}\nstdout:\n{}\nstderr:\n{}",
                package_path.display(),
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        return Ok(());
    }
    let dependencies = read_python_project_dependencies(package_path)?;
    if dependencies.is_empty() {
        return Ok(());
    }
    let output = Command::new(python)
        .args(["-I", "-m", "pip", "install", "--disable-pip-version-check"])
        .args(&dependencies)
        .output()
        .with_context(|| {
            format!(
                "failed to install Python connector dependencies with {}",
                python.display()
            )
        })?;
    if !output.status.success() {
        bail!(
            "failed to install Python connector dependencies for {}\nstdout:\n{}\nstderr:\n{}",
            package_path.display(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}

fn write_python_project_scripts(
    package_path: &Path,
    env_path: &Path,
    scripts: &BTreeMap<String, String>,
) -> Result<()> {
    if scripts.is_empty() {
        return Ok(());
    }
    let bin_dir = python_bin_dir(env_path);
    fs::create_dir_all(&bin_dir)
        .with_context(|| format!("failed to create {}", bin_dir.display()))?;
    for (script, module) in scripts {
        write_python_project_script(package_path, env_path, script, module)?;
    }
    Ok(())
}

fn write_python_project_script(
    package_path: &Path,
    env_path: &Path,
    script: &str,
    entrypoint: &str,
) -> Result<()> {
    let target = python_script_path(env_path, script);
    let (module, function) = python_entrypoint_parts(entrypoint);
    #[cfg(windows)]
    {
        let python = python_env_executable(env_path);
        let runner = format!(
            "@echo off\r\nset PYTHONNOUSERSITE=1\r\nset PYTHONPATH=\r\n\"{}\" -I -c \"import sys; sys.path.insert(0, r'{}'); from {} import {}; raise SystemExit({}())\" %*\r\n",
            python.display(),
            package_path.display(),
            module,
            function,
            function
        );
        fs::write(&target, runner.as_bytes()).with_context(|| {
            format!(
                "failed to write Python connector script {}",
                target.display()
            )
        })?;
    }
    #[cfg(not(windows))]
    {
        use std::os::unix::fs::PermissionsExt;

        let python = python_env_executable(env_path);
        let runner = format!(
            "#!/bin/sh\nunset PYTHONPATH\nexport PYTHONNOUSERSITE=1\nexec {} -I - \"$@\" <<'PY'\nimport sys\nsys.path.insert(0, {:?})\nfrom {} import {}\nif __name__ == '__main__':\n    raise SystemExit({}())\nPY\n",
            shell_single_quote(&python.display().to_string()),
            package_path.display().to_string(),
            module,
            function,
            function
        );
        fs::write(&target, runner.as_bytes()).with_context(|| {
            format!(
                "failed to write Python connector script {}",
                target.display()
            )
        })?;
        let mut permissions = fs::metadata(&target)
            .with_context(|| format!("failed to inspect {}", target.display()))?
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&target, permissions)
            .with_context(|| format!("failed to make {} executable", target.display()))?;
    }
    Ok(())
}

#[cfg(not(windows))]
fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn python_entrypoint_parts(entrypoint: &str) -> (&str, &str) {
    let (module, function) = entrypoint.split_once(':').unwrap_or((entrypoint, "main"));
    let module = module.trim();
    let function = function.trim();
    if function.is_empty() {
        (module, "main")
    } else {
        (module, function)
    }
}

fn mark_python_project_installed(package_path: &Path, env_path: &Path) -> Result<()> {
    let marker = env_path.join(CONNECTOR_PYTHON_ENV_MARKER);
    fs::write(
        &marker,
        format!("package={}\n", package_path.display()).as_bytes(),
    )
    .with_context(|| format!("failed to write {}", marker.display()))?;
    Ok(())
}

fn python_env_executable(env_path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        env_path.join("Scripts").join("python.exe")
    }
    #[cfg(not(windows))]
    {
        env_path.join("bin").join("python")
    }
}

fn python_script_path(env_path: &Path, script: &str) -> PathBuf {
    #[cfg(windows)]
    {
        env_path.join("Scripts").join(format!("{script}.cmd"))
    }
    #[cfg(not(windows))]
    {
        env_path.join("bin").join(script)
    }
}

fn python_bin_dir(env_path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        env_path.join("Scripts")
    }
    #[cfg(not(windows))]
    {
        env_path.join("bin")
    }
}

fn upsert_service(
    services: &mut Vec<ServiceConfig>,
    service: ServiceConfig,
    replace: bool,
) -> Result<()> {
    match services
        .iter()
        .position(|candidate| candidate.name == service.name)
    {
        Some(index) if replace => {
            services[index] = service;
            Ok(())
        }
        Some(_) => bail!(
            "service `{}` already exists; pass --replace to overwrite",
            service.name
        ),
        None => {
            services.push(service);
            Ok(())
        }
    }
}

fn upsert_synced_service(services: &mut Vec<ServiceConfig>, mut service: ServiceConfig) {
    match services
        .iter()
        .position(|candidate| candidate.name == service.name)
    {
        Some(index) => {
            service.enabled = services[index].enabled;
            services[index] = service;
        }
        None => services.push(service),
    }
}

fn cleanup_legacy_connector_autostarts_for_manifest(manifest: &ConnectorManifest) {
    for label in legacy_autostart_labels_for_manifest(manifest) {
        cleanup_legacy_autostart_label(&label);
    }
}

fn legacy_autostart_labels_for_manifest(manifest: &ConnectorManifest) -> Vec<String> {
    let mut labels = manifest.legacy_autostart_labels.clone();
    // WeChat Connector <= 0.3.x installed this fixed LaunchAgent label but did
    // not record it in connector.json. Keep the migration explicit so an
    // upgraded Bridge Agent can remove the old unsigned Python launcher before
    // the Connector package itself is upgraded.
    if manifest.id == "com.baijimu.connector.wechat"
        && !labels
            .iter()
            .any(|label| label == "com.baijimu.wechat-bridge-collector")
    {
        labels.push("com.baijimu.wechat-bridge-collector".to_string());
    }
    for value in manifest.hooks.values() {
        if value.contains("install-autostart") {
            if !labels.contains(&manifest.id) {
                labels.push(manifest.id.clone());
            }
            break;
        }
    }
    labels
}

fn cleanup_legacy_autostart_label(label: &str) {
    #[cfg(target_os = "macos")]
    {
        let label = label.trim();
        if label.is_empty() {
            return;
        }
        let uid = Command::new("id")
            .arg("-u")
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
            .filter(|value| !value.is_empty());
        if let Some(uid) = uid {
            let target = format!("gui/{uid}/{label}");
            let _ = Command::new("launchctl")
                .args(["bootout", &target])
                .output();
        }

        if let Some(home) = env::var_os("HOME") {
            let plist = PathBuf::from(home)
                .join("Library")
                .join("LaunchAgents")
                .join(format!("{label}.plist"));
            match fs::remove_file(&plist) {
                Ok(()) => tracing::info!("removed legacy connector autostart {}", plist.display()),
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => tracing::warn!(
                    "failed to remove legacy connector autostart {}: {err:#}",
                    plist.display()
                ),
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = label;
    }
}

fn run_start_command(
    service_name: &str,
    command: &ServiceStartCommand,
) -> Result<ConnectorServiceStartResult> {
    match command {
        ServiceStartCommand::ShellCommand {
            command,
            cwd,
            env,
            timeout_secs: _,
        } => {
            if command.is_empty() {
                bail!("start command for service `{service_name}` is empty");
            }
            let mut child = Command::new(&command[0]);
            child.args(&command[1..]);
            if let Some(cwd) = cwd.as_deref().filter(|value| !value.trim().is_empty()) {
                child.current_dir(cwd);
            }
            child.envs(env);
            let output = child
                .output()
                .with_context(|| format!("failed to start service `{service_name}`"))?;
            Ok(ConnectorServiceStartResult {
                service: service_name.to_string(),
                configured: true,
                exit_code: output.status.code(),
                stdout: String::from_utf8_lossy(&output.stdout).to_string(),
                stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            })
        }
    }
}

fn summary_from_record(record: ConnectorInstallRecord) -> ConnectorSummary {
    let last_synced_at_epoch_ms = if record.last_synced_at_epoch_ms == 0 {
        record.installed_at_epoch_ms
    } else {
        record.last_synced_at_epoch_ms
    };
    let start_policy = record
        .manifest
        .runtime
        .as_ref()
        .map(|runtime| runtime.start_policy.clone())
        .unwrap_or_else(default_connector_start_policy);
    ConnectorSummary {
        id: record.manifest.id,
        name: record.manifest.name,
        version: record.manifest.version,
        package_path: record.package_path,
        source_path: record.source_path,
        source_reference: record.source_reference,
        trust_level: record.trust_level,
        market_app_id: record.market_app_id,
        source_checksum: record.source_checksum,
        package_checksum: record.package_checksum,
        ui: record.manifest.ui,
        permissions: record.manifest.permissions,
        start_policy,
        service_names: record.service_names,
        installed_at_epoch_ms: record.installed_at_epoch_ms,
        last_synced_at_epoch_ms,
    }
}

fn sanitize_path_component(value: &str) -> String {
    value
        .chars()
        .map(|ch| match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '.' | '-' | '_' => ch,
            _ => '_',
        })
        .collect()
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AgentConfig, MethodBinding};
    use serde_json::json;
    use std::ffi::OsString;
    use std::sync::{Mutex, MutexGuard};
    use tempfile::tempdir;

    static CONNECTOR_ENV_LOCK: Mutex<()> = Mutex::new(());

    struct ConnectorTestEnvGuard {
        _lock: MutexGuard<'static, ()>,
        previous: Option<OsString>,
        previous_data: Option<OsString>,
    }

    impl Drop for ConnectorTestEnvGuard {
        fn drop(&mut self) {
            if let Some(previous) = self.previous.as_ref() {
                std::env::set_var("BRIDGE_AGENT_CONNECTORS_DIR", previous);
            } else {
                std::env::remove_var("BRIDGE_AGENT_CONNECTORS_DIR");
            }
            if let Some(previous) = self.previous_data.as_ref() {
                std::env::set_var("BRIDGE_AGENT_CONNECTOR_DATA_DIR", previous);
            } else {
                std::env::remove_var("BRIDGE_AGENT_CONNECTOR_DATA_DIR");
            }
        }
    }

    fn connector_test_env(path: impl AsRef<Path>) -> ConnectorTestEnvGuard {
        let lock = CONNECTOR_ENV_LOCK.lock().unwrap();
        let previous = std::env::var_os("BRIDGE_AGENT_CONNECTORS_DIR");
        let previous_data = std::env::var_os("BRIDGE_AGENT_CONNECTOR_DATA_DIR");
        std::env::set_var("BRIDGE_AGENT_CONNECTORS_DIR", path.as_ref());
        std::env::set_var(
            "BRIDGE_AGENT_CONNECTOR_DATA_DIR",
            path.as_ref().join("data"),
        );
        ConnectorTestEnvGuard {
            _lock: lock,
            previous,
            previous_data,
        }
    }

    #[test]
    fn connector_manifest_loads_service_registration_files() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join(CONNECTOR_MANIFEST_FILE),
            serde_json::to_string_pretty(&json!({
                "schemaVersion": "1.0",
                "id": "com.baijimu.connector.test",
                "name": "Test Connector",
                "version": "0.1.0",
                "serviceRegistrationFiles": ["service-registration.json"]
            }))
            .unwrap(),
        )
        .unwrap();
        fs::write(
            dir.path().join("service-registration.json"),
            serde_json::to_string_pretty(&json!({
                "name": "testService",
                "description": "Test service.",
                "transport": {
                    "type": "http",
                    "baseUrl": "http://127.0.0.1:18082"
                },
                "methods": [{
                    "name": "ping",
                    "description": "Ping.",
                    "path": "/invoke/ping"
                }]
            }))
            .unwrap(),
        )
        .unwrap();

        let manifest = load_connector_manifest(dir.path()).unwrap();
        let registrations = load_connector_service_registrations(dir.path(), &manifest).unwrap();
        assert_eq!(registrations.len(), 1);
        let service = registrations
            .into_iter()
            .next()
            .unwrap()
            .into_service_config()
            .unwrap();
        assert_eq!(service.name, "testService");
        match &service.methods[0].binding {
            MethodBinding::Http(binding) => {
                assert_eq!(binding.url, "http://127.0.0.1:18082/invoke/ping");
            }
            other => panic!("unexpected binding: {other:?}"),
        }
    }

    #[test]
    fn connector_manifest_accepts_loopback_management_operations() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join(CONNECTOR_MANIFEST_FILE),
            serde_json::to_string_pretty(&json!({
                "schemaVersion": "1.0",
                "id": "com.baijimu.connector.managed",
                "name": "Managed Connector",
                "version": "0.1.0",
                "management": {
                    "type": "http",
                    "baseUrl": "http://127.0.0.1:18110",
                    "auth": { "type": "connector_token" },
                    "operations": {
                        "state": { "method": "GET", "path": "/management/v1/state" }
                    }
                },
                "services": [{
                    "name": "managedService",
                    "description": "Managed service.",
                    "transport": { "type": "http", "baseUrl": "http://127.0.0.1:18110" },
                    "methods": [{ "name": "ping", "description": "Ping.", "path": "/invoke/ping" }]
                }]
            }))
            .unwrap(),
        )
        .unwrap();

        let manifest = load_connector_manifest(dir.path()).unwrap();
        let management = manifest.management.unwrap();
        assert_eq!(management.auth.auth_type, "connector_token");
        assert_eq!(management.operations["state"].path, "/management/v1/state");
    }

    #[test]
    fn connector_manifest_accepts_embedded_ui_and_resolves_only_ui_assets() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("ui/assets")).unwrap();
        fs::write(dir.path().join("ui/index.html"), "<main>settings</main>").unwrap();
        fs::write(dir.path().join("ui/assets/app.js"), "window.loaded = true;").unwrap();
        fs::write(dir.path().join("private.txt"), "not a UI asset").unwrap();
        fs::write(
            dir.path().join(CONNECTOR_MANIFEST_FILE),
            serde_json::to_string_pretty(&json!({
                "schemaVersion": "1.1",
                "id": "com.baijimu.connector.with-ui",
                "name": "Connector With UI",
                "version": "0.1.0",
                "ui": {
                    "type": "embedded",
                    "entry": "ui/index.html",
                    "title": "个性化设置",
                    "defaultView": true
                },
                "services": [{
                    "name": "withUiService",
                    "description": "UI test service.",
                    "transport": { "type": "http", "baseUrl": "http://127.0.0.1:18110" },
                    "methods": [{ "name": "ping", "description": "Ping.", "path": "/invoke/ping" }]
                }]
            }))
            .unwrap(),
        )
        .unwrap();

        let manifest = load_connector_manifest(dir.path()).unwrap();
        let ui = manifest.ui.unwrap();
        assert!(ui.default_view);
        assert_eq!(ui.title.as_deref(), Some("个性化设置"));
        assert_eq!(
            resolve_connector_ui_entry(dir.path(), &ui).unwrap(),
            dir.path().join("ui/index.html").canonicalize().unwrap()
        );
        assert_eq!(
            resolve_connector_ui_asset(dir.path(), &ui, Some("assets/app.js")).unwrap(),
            dir.path().join("ui/assets/app.js").canonicalize().unwrap()
        );
        assert!(resolve_connector_ui_asset(dir.path(), &ui, Some("../private.txt")).is_err());
    }

    #[test]
    fn connector_manifest_rejects_missing_or_escaping_ui_entry() {
        let dir = tempdir().unwrap();
        let manifest_path = dir.path().join(CONNECTOR_MANIFEST_FILE);
        let base = json!({
            "schemaVersion": "1.1",
            "id": "com.baijimu.connector.bad-ui",
            "name": "Bad UI Connector",
            "version": "0.1.0",
            "services": [{
                "name": "badUiService",
                "description": "Bad UI service.",
                "transport": { "type": "http", "baseUrl": "http://127.0.0.1:18110" },
                "methods": [{ "name": "ping", "description": "Ping.", "path": "/invoke/ping" }]
            }]
        });

        let mut missing = base.clone();
        missing["ui"] = json!({ "type": "embedded", "entry": "ui/missing.html" });
        fs::write(
            &manifest_path,
            serde_json::to_string_pretty(&missing).unwrap(),
        )
        .unwrap();
        assert!(load_connector_manifest(dir.path())
            .unwrap_err()
            .to_string()
            .contains("failed to resolve connector UI root"));

        let mut escaping = base.clone();
        escaping["ui"] = json!({ "type": "embedded", "entry": "../outside.html" });
        fs::write(
            &manifest_path,
            serde_json::to_string_pretty(&escaping).unwrap(),
        )
        .unwrap();
        assert!(load_connector_manifest(dir.path())
            .unwrap_err()
            .to_string()
            .contains("must stay inside"));

        fs::write(dir.path().join("index.html"), "<main>root UI</main>").unwrap();
        let mut root_entry = base.clone();
        root_entry["ui"] = json!({ "type": "embedded", "entry": "index.html" });
        fs::write(
            &manifest_path,
            serde_json::to_string_pretty(&root_entry).unwrap(),
        )
        .unwrap();
        assert!(load_connector_manifest(dir.path())
            .unwrap_err()
            .to_string()
            .contains("dedicated UI directory"));

        let mut legacy_schema = base;
        legacy_schema["schemaVersion"] = json!("1.0");
        legacy_schema["ui"] = json!({ "type": "embedded", "entry": "ui/index.html" });
        fs::write(
            &manifest_path,
            serde_json::to_string_pretty(&legacy_schema).unwrap(),
        )
        .unwrap();
        assert!(load_connector_manifest(dir.path())
            .unwrap_err()
            .to_string()
            .contains("requires schemaVersion 1.1 or newer"));
    }

    #[test]
    fn installed_connector_summary_exposes_embedded_ui_metadata() {
        let dir = tempdir().unwrap();
        let _env = connector_test_env(dir.path().join("installed-connectors"));
        let source = dir.path().join("source");
        fs::create_dir_all(source.join("ui")).unwrap();
        fs::write(
            source.join("ui/index.html"),
            "<!doctype html><title>UI</title>",
        )
        .unwrap();
        fs::write(
            source.join(CONNECTOR_MANIFEST_FILE),
            serde_json::to_string_pretty(&json!({
                "schemaVersion": "1.1",
                "id": "com.baijimu.connector.installed-ui",
                "name": "Installed UI Connector",
                "version": "0.1.0",
                "ui": {
                    "type": "embedded",
                    "entry": "ui/index.html",
                    "title": "设置",
                    "defaultView": true
                },
                "services": [{
                    "name": "installedUiService",
                    "description": "Installed UI service.",
                    "transport": { "type": "http", "baseUrl": "http://127.0.0.1:18110" },
                    "methods": [{ "name": "ping", "description": "Ping.", "path": "/invoke/ping" }]
                }]
            }))
            .unwrap(),
        )
        .unwrap();
        let config_path = dir.path().join("agent-config.json");
        save_config(&config_path, &AgentConfig::example()).unwrap();

        install_connector_from_path(&source, &config_path, false).unwrap();
        let summary = list_connectors().unwrap().remove(0);
        let serialized = serde_json::to_value(&summary).unwrap();
        assert_eq!(serialized["ui"]["type"], "embedded");
        assert!(serialized["ui"].get("uiType").is_none());
        assert_eq!(serialized["ui"]["defaultView"], true);
        let ui = summary.ui.expect("installed UI metadata");
        assert_eq!(ui.ui_type, "embedded");
        assert_eq!(ui.entry, "ui/index.html");
        assert_eq!(ui.title.as_deref(), Some("设置"));
        assert!(ui.default_view);
        assert_eq!(summary.trust_level, ConnectorTrustLevel::UserTrusted);
        assert!(summary
            .package_checksum
            .as_deref()
            .is_some_and(|value| value.starts_with("sha256:") && value.len() == 71));

        let market_checksum = format!("sha256:{}", "a".repeat(64));
        let provenance = ConnectorInstallProvenance::platform_trusted(
            "https://downloads.example.test/connector.zip",
            "market-app-1",
            &market_checksum,
        )
        .unwrap();
        install_connector_from_path_with_provenance(&source, &config_path, true, provenance)
            .unwrap();
        let trusted = list_connectors().unwrap().remove(0);
        assert_eq!(trusted.trust_level, ConnectorTrustLevel::PlatformTrusted);
        assert_eq!(trusted.market_app_id.as_deref(), Some("market-app-1"));
        assert_eq!(
            trusted.source_checksum.as_deref(),
            Some(market_checksum.as_str())
        );

        let mut legacy = serde_json::to_value(show_connector(&trusted.id).unwrap()).unwrap();
        let legacy = legacy.as_object_mut().unwrap();
        legacy.remove("trustLevel");
        legacy.remove("marketAppId");
        legacy.remove("sourceChecksum");
        legacy.remove("packageChecksum");
        let legacy: ConnectorInstallRecord =
            serde_json::from_value(serde_json::Value::Object(legacy.clone())).unwrap();
        assert_eq!(legacy.trust_level, ConnectorTrustLevel::UserTrusted);
        assert!(legacy.market_app_id.is_none());
    }

    #[test]
    fn platform_trusted_provenance_requires_complete_sha256_evidence() {
        assert!(ConnectorInstallProvenance::platform_trusted(
            "https://downloads.example.test/connector.zip",
            "market-app-1",
            "invalid",
        )
        .is_err());
        assert!(
            ConnectorInstallProvenance::platform_trusted("", "market-app-1", &"a".repeat(64),)
                .is_err()
        );
    }

    #[test]
    fn connector_manifest_rejects_remote_management_url() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join(CONNECTOR_MANIFEST_FILE),
            serde_json::to_string_pretty(&json!({
                "schemaVersion": "1.0",
                "id": "com.baijimu.connector.unsafe",
                "name": "Unsafe Connector",
                "version": "0.1.0",
                "management": {
                    "type": "http",
                    "baseUrl": "https://example.com",
                    "auth": { "type": "connector_token" },
                    "operations": {
                        "state": { "method": "GET", "path": "/management/v1/state" }
                    }
                },
                "services": [{
                    "name": "unsafeService",
                    "description": "Unsafe service.",
                    "transport": { "type": "http", "baseUrl": "http://127.0.0.1:18110" },
                    "methods": [{ "name": "ping", "description": "Ping.", "path": "/invoke/ping" }]
                }]
            }))
            .unwrap(),
        )
        .unwrap();

        let error = load_connector_manifest(dir.path()).unwrap_err();
        assert!(error.to_string().contains("management.baseUrl"));
    }

    #[test]
    fn connector_manifest_rejects_management_base_url_with_path() {
        let management = ConnectorManagement {
            management_type: "http".to_string(),
            base_url: "http://127.0.0.1:18110/untrusted".to_string(),
            auth: ConnectorManagementAuth {
                auth_type: "connector_token".to_string(),
            },
            operations: BTreeMap::from([(
                "state".to_string(),
                ConnectorManagementOperation {
                    method: "GET".to_string(),
                    path: "/management/v1/state".to_string(),
                },
            )]),
        };

        let error = validate_management(&management).unwrap_err();
        assert!(error.to_string().contains("origin-only"));
    }

    #[test]
    fn install_connector_updates_agent_config() {
        let dir = tempdir().unwrap();
        let _env = connector_test_env(dir.path().join("connectors"));
        let config_path = dir.path().join("agent-config.json");
        save_config(&config_path, &AgentConfig::example()).unwrap();
        let source = dir.path().join("connector");
        fs::create_dir_all(&source).unwrap();
        fs::write(
            source.join(CONNECTOR_MANIFEST_FILE),
            serde_json::to_string_pretty(&json!({
                "schemaVersion": "1.0",
                "id": "com.baijimu.connector.inline",
                "name": "Inline Connector",
                "version": "0.1.0",
                "services": [{
                    "name": "inlineService",
                    "description": "Inline service.",
                    "transport": {
                        "type": "http",
                        "baseUrl": "http://127.0.0.1:18082"
                    },
                    "methods": [{
                        "name": "invoke",
                        "description": "Invoke.",
                        "path": "/invoke"
                    }]
                }]
            }))
            .unwrap(),
        )
        .unwrap();

        let result = install_connector_from_path(&source, &config_path, false).unwrap();
        assert_eq!(result.connector_id, "com.baijimu.connector.inline");
        assert_eq!(result.service_names, vec!["inlineService".to_string()]);
        let config = load_config(&config_path).unwrap();
        assert!(config
            .services
            .iter()
            .any(|service| service.name == "inlineService"));
    }

    #[test]
    fn uninstall_connector_removes_package_and_install_record() {
        let dir = tempdir().unwrap();
        let connectors_dir = dir.path().join("connectors");
        let _env = connector_test_env(connectors_dir.clone());
        let config_path = dir.path().join("agent-config.json");
        save_config(&config_path, &AgentConfig::example()).unwrap();
        let source = dir.path().join("connector");
        fs::create_dir_all(&source).unwrap();
        fs::write(
            source.join(CONNECTOR_MANIFEST_FILE),
            serde_json::to_string_pretty(&json!({
                "schemaVersion": "1.0",
                "id": "com.baijimu.connector.uninstall-test",
                "name": "Uninstall Test Connector",
                "version": "0.1.0",
                "services": [{
                    "name": "uninstallTestService",
                    "description": "Uninstall test service.",
                    "transport": {
                        "type": "http",
                        "baseUrl": "http://127.0.0.1:18121"
                    },
                    "methods": [{
                        "name": "ping",
                        "description": "Ping.",
                        "path": "/invoke/ping"
                    }]
                }]
            }))
            .unwrap(),
        )
        .unwrap();

        install_connector_from_path(&source, &config_path, false).unwrap();
        let install_root = connectors_dir.join("com.baijimu.connector.uninstall-test");
        assert!(install_root.join("package").is_dir());
        assert!(install_root.join(CONNECTOR_INSTALL_RECORD_FILE).is_file());

        uninstall_connector("com.baijimu.connector.uninstall-test", &config_path).unwrap();

        assert!(!install_root.exists());
        assert!(list_connectors().unwrap().is_empty());
        assert!(!load_config(&config_path)
            .unwrap()
            .services
            .iter()
            .any(|service| service.name == "uninstallTestService"));
    }

    #[test]
    fn reinstall_preserves_install_time_and_updates_sync_source() {
        let dir = tempdir().unwrap();
        let _env = connector_test_env(dir.path().join("connectors"));
        let config_path = dir.path().join("agent-config.json");
        save_config(&config_path, &AgentConfig::example()).unwrap();
        let source = dir.path().join("connector");
        fs::create_dir_all(&source).unwrap();
        fs::write(
            source.join(CONNECTOR_MANIFEST_FILE),
            serde_json::to_string_pretty(&json!({
                "schemaVersion": "1.0",
                "id": "com.baijimu.connector.resync",
                "name": "Resync Connector",
                "version": "0.1.0",
                "services": [{
                    "name": "resyncService",
                    "description": "Resync service.",
                    "transport": {
                        "type": "http",
                        "baseUrl": "http://127.0.0.1:18082"
                    },
                    "methods": [{
                        "name": "invoke",
                        "description": "Invoke.",
                        "path": "/invoke"
                    }]
                }]
            }))
            .unwrap(),
        )
        .unwrap();

        install_connector_from_path_with_source_reference(
            &source,
            &config_path,
            false,
            Some("https://example.test/connector.git#main"),
        )
        .unwrap();
        let first = show_connector("com.baijimu.connector.resync").unwrap();

        install_connector_from_path_with_source_reference(
            &source,
            &config_path,
            true,
            Some("https://example.test/connector.git#next"),
        )
        .unwrap();
        let second = show_connector("com.baijimu.connector.resync").unwrap();
        assert_eq!(second.installed_at_epoch_ms, first.installed_at_epoch_ms);
        assert_eq!(
            second.source_reference.as_deref(),
            Some("https://example.test/connector.git#next")
        );
        assert!(second.last_synced_at_epoch_ms >= first.last_synced_at_epoch_ms);

        let summary = list_connectors()
            .unwrap()
            .into_iter()
            .find(|connector| connector.id == "com.baijimu.connector.resync")
            .unwrap();
        assert_eq!(summary.source_reference, second.source_reference);
        assert_eq!(
            summary.last_synced_at_epoch_ms,
            second.last_synced_at_epoch_ms
        );
    }

    #[test]
    fn install_connector_resolves_package_bin_start_command() {
        let dir = tempdir().unwrap();
        let _env = connector_test_env(dir.path().join("connectors"));
        let config_path = dir.path().join("agent-config.json");
        save_config(&config_path, &AgentConfig::example()).unwrap();
        let source = dir.path().join("connector");
        fs::create_dir_all(source.join("bin")).unwrap();
        fs::write(
            source.join("package.json"),
            serde_json::to_string_pretty(&json!({
                "name": "test-connector",
                "bin": {
                    "test-connector": "./bin/start.js"
                }
            }))
            .unwrap(),
        )
        .unwrap();
        fs::write(source.join("bin").join("start.js"), "console.log('ok');\n").unwrap();
        fs::write(
            source.join(CONNECTOR_MANIFEST_FILE),
            serde_json::to_string_pretty(&json!({
                "schemaVersion": "1.0",
                "id": "com.baijimu.connector.bin",
                "name": "Bin Connector",
                "version": "0.1.0",
                "services": [{
                    "name": "binService",
                    "description": "Bin service.",
                    "transport": {
                        "type": "http",
                        "baseUrl": "http://127.0.0.1:18082"
                    },
                    "startCommand": {
                        "type": "shell_command",
                        "command": ["test-connector", "--port", "18082"]
                    },
                    "methods": [{
                        "name": "invoke",
                        "description": "Invoke.",
                        "path": "/invoke"
                    }]
                }]
            }))
            .unwrap(),
        )
        .unwrap();

        let result = install_connector_from_path(&source, &config_path, false).unwrap();
        let package_path = PathBuf::from(result.package_path);
        let config = load_config(&config_path).unwrap();
        let service = config
            .services
            .iter()
            .find(|service| service.name == "binService")
            .unwrap();
        let ServiceStartCommand::ShellCommand {
            command, cwd, env, ..
        } = service.start_command.as_ref().unwrap();
        let expected_node = resolve_command_path("node", &config.runtime)
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "node".to_string());
        assert_eq!(command[0], expected_node);
        assert_eq!(
            command[1],
            package_path.join("./bin/start.js").display().to_string()
        );
        assert_eq!(command[2], "--port");
        assert_eq!(cwd.as_deref(), Some(package_path.to_str().unwrap()));
        assert_eq!(
            env.get(CONNECTOR_DATA_DIR_ENV).map(String::as_str),
            Some(
                connector_data_dir("com.baijimu.connector.bin")
                    .unwrap()
                    .to_str()
                    .unwrap()
            )
        );
        if let Some(node_dir) = Path::new(&expected_node)
            .parent()
            .filter(|dir| !dir.as_os_str().is_empty())
        {
            assert!(env
                .get("PATH")
                .is_some_and(|path| env::split_paths(path).any(|entry| entry == node_dir)));
        }
    }

    #[test]
    fn install_connector_resolves_platform_native_bin_start_command() {
        let dir = tempdir().unwrap();
        let _env = connector_test_env(dir.path().join("connectors"));
        let config_path = dir.path().join("agent-config.json");
        save_config(&config_path, &AgentConfig::example()).unwrap();

        let source = dir.path().join("connector");
        let platform_dir = native_platform_bin_dirs().remove(0);
        fs::create_dir_all(source.join("bin").join(&platform_dir)).unwrap();
        fs::write(
            source.join("package.json"),
            serde_json::to_string_pretty(&json!({
                "name": "native-connector",
                "bin": {
                    "native-connector": "./bin/legacy.js"
                }
            }))
            .unwrap(),
        )
        .unwrap();
        fs::write(
            source.join("bin").join("legacy.js"),
            "console.log('legacy');\n",
        )
        .unwrap();
        fs::write(source.join("native-connector"), "#!/bin/sh\n").unwrap();
        fs::write(
            source
                .join("bin")
                .join(&platform_dir)
                .join("native-connector"),
            "#!/bin/sh\n",
        )
        .unwrap();
        fs::write(
            source.join(CONNECTOR_MANIFEST_FILE),
            serde_json::to_string_pretty(&json!({
                "schemaVersion": "1.0",
                "id": "com.baijimu.connector.native",
                "name": "Native Connector",
                "version": "0.1.0",
                "services": [{
                    "name": "nativeService",
                    "description": "Native service.",
                    "transport": {
                        "type": "http",
                        "baseUrl": "http://127.0.0.1:18082"
                    },
                    "startCommand": {
                        "type": "shell_command",
                        "command": ["native-connector", "start"]
                    },
                    "methods": [{
                        "name": "invoke",
                        "description": "Invoke.",
                        "path": "/invoke"
                    }]
                }]
            }))
            .unwrap(),
        )
        .unwrap();

        let result = install_connector_from_path(&source, &config_path, false).unwrap();
        let package_path = PathBuf::from(result.package_path);
        let config = load_config(&config_path).unwrap();
        let service = config
            .services
            .iter()
            .find(|service| service.name == "nativeService")
            .unwrap();
        let ServiceStartCommand::ShellCommand { command, cwd, .. } =
            service.start_command.as_ref().unwrap();
        assert_eq!(
            command[0],
            package_path
                .join("bin")
                .join(platform_dir)
                .join("native-connector")
                .display()
                .to_string()
        );
        assert_eq!(cwd.as_deref(), Some(package_path.to_str().unwrap()));
    }

    #[test]
    fn install_connector_prefers_configured_node_path() {
        let dir = tempdir().unwrap();
        let _env = connector_test_env(dir.path().join("connectors"));
        let config_path = dir.path().join("agent-config.json");
        let configured_node = dir.path().join("custom-node");
        fs::write(&configured_node, "").unwrap();
        let mut agent_config = AgentConfig::example();
        agent_config.runtime.node_path = Some(configured_node.display().to_string());
        save_config(&config_path, &agent_config).unwrap();

        let source = dir.path().join("connector");
        fs::create_dir_all(source.join("bin")).unwrap();
        fs::write(
            source.join("package.json"),
            serde_json::to_string_pretty(&json!({
                "name": "test-connector",
                "bin": {
                    "test-connector": "./bin/start.js"
                }
            }))
            .unwrap(),
        )
        .unwrap();
        fs::write(source.join("bin").join("start.js"), "console.log('ok');\n").unwrap();
        fs::write(
            source.join(CONNECTOR_MANIFEST_FILE),
            serde_json::to_string_pretty(&json!({
                "schemaVersion": "1.0",
                "id": "com.baijimu.connector.configured-node",
                "name": "Configured Node Connector",
                "version": "0.1.0",
                "services": [{
                    "name": "configuredNodeService",
                    "description": "Configured Node service.",
                    "transport": {
                        "type": "http",
                        "baseUrl": "http://127.0.0.1:18082"
                    },
                    "startCommand": {
                        "type": "shell_command",
                        "command": ["test-connector", "--port", "18082"]
                    },
                    "methods": [{
                        "name": "invoke",
                        "description": "Invoke.",
                        "path": "/invoke"
                    }]
                }]
            }))
            .unwrap(),
        )
        .unwrap();

        install_connector_from_path(&source, &config_path, false).unwrap();
        let config = load_config(&config_path).unwrap();
        let service = config
            .services
            .iter()
            .find(|service| service.name == "configuredNodeService")
            .unwrap();
        let ServiceStartCommand::ShellCommand { command, env, .. } =
            service.start_command.as_ref().unwrap();
        assert_eq!(command[0], configured_node.display().to_string());
        assert!(env
            .get("PATH")
            .is_some_and(|path| { env::split_paths(path).any(|entry| entry == dir.path()) }));
    }

    #[test]
    fn install_connector_derives_package_bin_stop_command() {
        let dir = tempdir().unwrap();
        let _env = connector_test_env(dir.path().join("connectors"));
        let config_path = dir.path().join("agent-config.json");
        save_config(&config_path, &AgentConfig::example()).unwrap();
        let source = dir.path().join("connector");
        fs::create_dir_all(source.join("bin")).unwrap();
        fs::write(
            source.join("package.json"),
            serde_json::to_string_pretty(&json!({
                "name": "test-connector",
                "bin": {
                    "test-connector": "./bin/start.js"
                }
            }))
            .unwrap(),
        )
        .unwrap();
        fs::write(source.join("bin").join("start.js"), "console.log('ok');\n").unwrap();
        fs::write(
            source.join(CONNECTOR_MANIFEST_FILE),
            serde_json::to_string_pretty(&json!({
                "schemaVersion": "1.0",
                "id": "com.baijimu.connector.daemon",
                "name": "Daemon Connector",
                "version": "0.1.0",
                "services": [{
                    "name": "daemonService",
                    "description": "Daemon service.",
                    "transport": {
                        "type": "http",
                        "baseUrl": "http://127.0.0.1:18082"
                    },
                    "startCommand": {
                        "type": "shell_command",
                        "command": ["test-connector", "start", "--daemon", "--port", "18082"]
                    },
                    "methods": [{
                        "name": "invoke",
                        "description": "Invoke.",
                        "path": "/invoke"
                    }]
                }]
            }))
            .unwrap(),
        )
        .unwrap();

        let result = install_connector_from_path(&source, &config_path, false).unwrap();
        let package_path = PathBuf::from(result.package_path);
        let config = load_config(&config_path).unwrap();
        let service = config
            .services
            .iter()
            .find(|service| service.name == "daemonService")
            .unwrap();
        let ServiceStartCommand::ShellCommand { command, cwd, .. } =
            service.stop_command.as_ref().unwrap();
        let expected_node = resolve_command_path("node", &config.runtime)
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "node".to_string());
        assert_eq!(command[0], expected_node);
        assert_eq!(
            command[1],
            package_path.join("./bin/start.js").display().to_string()
        );
        assert_eq!(command[2], "stop");
        assert_eq!(command[3], "--port");
        assert_eq!(cwd.as_deref(), Some(package_path.to_str().unwrap()));
    }

    #[test]
    fn reads_python_project_scripts() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("pyproject.toml"),
            r#"
[project.scripts]
wechat-bridge-collector = "wechat_bridge_collector.app:main"
"#,
        )
        .unwrap();

        let scripts = read_python_project_scripts(dir.path()).unwrap();
        assert_eq!(
            scripts.get("wechat-bridge-collector").map(String::as_str),
            Some("wechat_bridge_collector.app:main")
        );
    }

    #[test]
    fn writes_python_project_script_for_source_tree() {
        let dir = tempdir().unwrap();
        let package_path = dir.path().join("package");
        let env_path = package_path.join(CONNECTOR_PYTHON_ENV_DIR);
        fs::create_dir_all(python_bin_dir(&env_path)).unwrap();
        fs::create_dir_all(package_path.join("sample_connector")).unwrap();
        fs::write(
            package_path.join("sample_connector").join("app.py"),
            "def run():\n    print('ok')\n    return 0\n",
        )
        .unwrap();

        write_python_project_script(
            &package_path,
            &env_path,
            "sample-connector",
            "sample_connector.app:run",
        )
        .unwrap();

        let script = fs::read_to_string(python_script_path(&env_path, "sample-connector")).unwrap();
        assert!(script.contains("sys.path.insert"));
        assert!(script.contains("sample_connector.app"));
        assert!(script.contains("run()"));
    }

    #[test]
    fn resolves_python_project_script_to_connector_environment() {
        let dir = tempdir().unwrap();
        let env_path = dir.path().join(CONNECTOR_PYTHON_ENV_DIR);
        fs::create_dir_all(python_bin_dir(&env_path)).unwrap();
        let mut command = vec!["wechat-bridge-collector".to_string(), "start".to_string()];
        let mut cwd = None;
        let mut env_vars = BTreeMap::new();
        let scripts = BTreeMap::from([(
            "wechat-bridge-collector".to_string(),
            "wechat_bridge_collector.app".to_string(),
        )]);

        let command_runtime = InstalledCommandRuntime {
            package_path: dir.path(),
            package_bins: &BTreeMap::new(),
            python_scripts: &scripts,
            python_env: Some(&env_path),
            node_path: &None,
            codex_path: &None,
        };
        resolve_installed_shell_command(&mut command, &mut cwd, &mut env_vars, &command_runtime);

        assert_eq!(
            command[0],
            python_script_path(&env_path, "wechat-bridge-collector")
                .display()
                .to_string()
        );
        assert_eq!(command[1], "start");
        assert_eq!(cwd.as_deref(), Some(dir.path().to_str().unwrap()));
        assert!(env_vars
            .get("PATH")
            .is_some_and(|path| env::split_paths(path)
                .next()
                .is_some_and(|entry| entry == python_bin_dir(&env_path))));
    }

    #[test]
    fn parses_python_version_requirements() {
        assert_eq!(parse_python_version("Python 3.12.7"), Some((3, 12, 7)));
        assert_eq!(parse_python_version("3.10"), Some((3, 10, 0)));
        assert!(python_version_satisfies_requirement(
            (3, 12, 7),
            ">=3.12,<3.13"
        ));
        assert!(!python_version_satisfies_requirement(
            (3, 11, 9),
            ">=3.12,<3.13"
        ));
        assert!(!python_version_satisfies_requirement(
            (3, 13, 0),
            ">=3.12,<3.13"
        ));
    }

    #[test]
    fn sync_installed_connectors_restores_lifecycle_commands_and_preserves_enabled() {
        let dir = tempdir().unwrap();
        let _env = connector_test_env(dir.path().join("connectors"));
        let config_path = dir.path().join("agent-config.json");
        save_config(&config_path, &AgentConfig::example()).unwrap();
        let source = dir.path().join("connector");
        fs::create_dir_all(&source).unwrap();
        fs::write(
            source.join(CONNECTOR_MANIFEST_FILE),
            serde_json::to_string_pretty(&json!({
                "schemaVersion": "1.0",
                "id": "com.baijimu.connector.sync",
                "name": "Sync Connector",
                "version": "0.1.0",
                "services": [{
                    "name": "syncService",
                    "description": "Sync service.",
                    "transport": {
                        "type": "http",
                        "baseUrl": "http://127.0.0.1:18082"
                    },
                    "startCommand": {
                        "type": "shell_command",
                        "command": ["/bin/sh", "-c", "sleep 60"]
                    },
                    "methods": [{
                        "name": "invoke",
                        "description": "Invoke.",
                        "path": "/invoke"
                    }]
                }]
            }))
            .unwrap(),
        )
        .unwrap();

        install_connector_from_path(&source, &config_path, false).unwrap();
        let mut config = load_config(&config_path).unwrap();
        let service = config
            .services
            .iter_mut()
            .find(|service| service.name == "syncService")
            .unwrap();
        service.enabled = false;
        service.start_command = None;
        save_config(&config_path, &config).unwrap();

        sync_installed_connectors(&config_path).unwrap();
        let config = load_config(&config_path).unwrap();
        let service = config
            .services
            .iter()
            .find(|service| service.name == "syncService")
            .unwrap();
        assert!(!service.enabled);
        assert!(service.start_command.is_some());
    }

    #[test]
    fn sync_report_records_bad_connector_without_blocking_good_connectors() {
        let dir = tempdir().unwrap();
        let _env = connector_test_env(dir.path().join("connectors"));
        let config_path = dir.path().join("agent-config.json");
        save_config(&config_path, &AgentConfig::example()).unwrap();

        let good_source = dir.path().join("good-connector");
        fs::create_dir_all(&good_source).unwrap();
        fs::write(
            good_source.join(CONNECTOR_MANIFEST_FILE),
            serde_json::to_string_pretty(&json!({
                "schemaVersion": "1.0",
                "id": "com.baijimu.connector.good",
                "name": "Good Connector",
                "version": "0.1.0",
                "services": [{
                    "name": "goodService",
                    "description": "Good service.",
                    "transport": {
                        "type": "http",
                        "baseUrl": "http://127.0.0.1:18082"
                    },
                    "methods": [{
                        "name": "invoke",
                        "description": "Invoke.",
                        "path": "/invoke"
                    }]
                }]
            }))
            .unwrap(),
        )
        .unwrap();
        install_connector_from_path(&good_source, &config_path, false).unwrap();

        let bad_manifest: ConnectorManifest = serde_json::from_value(json!({
            "schemaVersion": "1.0",
            "id": "com.baijimu.connector.bad-python",
            "name": "Bad Python Connector",
            "version": "0.1.0",
            "services": [{
                "name": "badPythonService",
                "description": "Bad Python service.",
                "transport": {
                    "type": "http",
                    "baseUrl": "http://127.0.0.1:18083"
                },
                "startCommand": {
                    "type": "shell_command",
                    "command": ["bad-python-connector", "start"]
                },
                "methods": [{
                    "name": "invoke",
                    "description": "Invoke.",
                    "path": "/invoke"
                }]
            }]
        }))
        .unwrap();
        let bad_package_path = installed_connector_package_path(&bad_manifest).unwrap();
        fs::create_dir_all(&bad_package_path).unwrap();
        fs::write(
            bad_package_path.join("pyproject.toml"),
            r#"[project]
name = "bad-python-connector"
version = "0.1.0"
requires-python = ">=999.0"

[project.scripts]
bad-python-connector = "bad_python_connector.app:main"
"#,
        )
        .unwrap();
        save_install_record(&ConnectorInstallRecord {
            manifest: bad_manifest,
            package_path: bad_package_path.display().to_string(),
            source_path: dir
                .path()
                .join("bad-connector-source")
                .display()
                .to_string(),
            source_reference: None,
            trust_level: ConnectorTrustLevel::UserTrusted,
            market_app_id: None,
            source_checksum: None,
            package_checksum: None,
            service_names: vec!["badPythonService".to_string()],
            installed_at_epoch_ms: 1,
            last_synced_at_epoch_ms: 1,
        })
        .unwrap();

        let report = sync_installed_connectors_report(&config_path).unwrap();
        assert!(report
            .summaries
            .iter()
            .any(|summary| summary.id == "com.baijimu.connector.good"));
        assert_eq!(report.failures.len(), 1);
        assert_eq!(
            report.failures[0].connector_id,
            "com.baijimu.connector.bad-python"
        );
        assert!(report.failures[0]
            .error
            .contains("failed to find a Python interpreter matching >=999.0"));

        let strict_error = sync_installed_connectors(&config_path).unwrap_err();
        assert!(strict_error
            .to_string()
            .contains("failed to sync 1 installed connector"));
    }

    #[test]
    fn legacy_autostart_cleanup_uses_manifest_labels() {
        let manifest = ConnectorManifest {
            schema_version: "1.0".to_string(),
            id: "com.baijimu.connector.wechat".to_string(),
            name: "WeChat Connector".to_string(),
            version: "0.1.0".to_string(),
            description: String::new(),
            publisher: None,
            source: None,
            runtime: None,
            management: None,
            ui: None,
            config_schema: None,
            remote_capabilities: Vec::new(),
            permissions: Vec::new(),
            legacy_autostart_labels: Vec::new(),
            services: Vec::new(),
            service_registration_files: Vec::new(),
            hooks: BTreeMap::from([(
                "installAutostart".to_string(),
                "wechat-bridge-collector install-autostart".to_string(),
            )]),
        };

        assert_eq!(
            legacy_autostart_labels_for_manifest(&manifest),
            vec![
                "com.baijimu.wechat-bridge-collector".to_string(),
                "com.baijimu.connector.wechat".to_string(),
            ]
        );
    }
}

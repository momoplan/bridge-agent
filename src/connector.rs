use crate::config::{
    ensure_config_exists, load_config, save_config, RuntimeConfig, ServiceConfig,
    ServiceRegistration, ServiceStartCommand,
};
use anyhow::{bail, Context, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::env;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

const CONNECTOR_MANIFEST_FILE: &str = "connector.json";
const CONNECTOR_INSTALL_RECORD_FILE: &str = "install.json";
const CONNECTOR_PYTHON_ENV_DIR: &str = ".bridge-agent-python";
const CONNECTOR_PYTHON_ENV_MARKER: &str = ".install-ok";
const CONNECTOR_DATA_DIR_ENV: &str = "BAIJIMU_CONNECTOR_DATA_DIR";
const CONNECTOR_MANAGEMENT_TOKEN_FILE: &str = "management-token";

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
    pub config_schema: Option<Value>,
    #[serde(default)]
    pub remote_capabilities: Vec<ConnectorRemoteCapability>,
    #[serde(default)]
    pub services: Vec<ServiceRegistration>,
    #[serde(default)]
    pub service_registration_files: Vec<String>,
    #[serde(default)]
    pub hooks: BTreeMap<String, String>,
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
    pub service_names: Vec<String>,
    pub installed_at_epoch_ms: u64,
    #[serde(default)]
    pub last_synced_at_epoch_ms: u64,
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
    Ok(manifest)
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
    resolve_installed_start_commands(&mut services, &package_path, &config.runtime, &manifest.id)?;
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
        source_reference: source_reference
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
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

    let package_path = PathBuf::from(&record.package_path);
    if package_path.exists() {
        fs::remove_dir_all(&package_path).with_context(|| {
            format!(
                "failed to remove connector package {}",
                package_path.display()
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
        &record.manifest.id,
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
    if manifest.schema_version.trim().is_empty() {
        bail!("connector schemaVersion cannot be empty");
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
    Ok(())
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
    connector_id: &str,
) -> Result<()> {
    let data_dir = connector_data_dir(connector_id)?;
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
    let python_env = ensure_python_project_environment(package_path, &python_scripts)?;
    let node_path = resolve_command_path("node", runtime_config);
    let codex_path = resolve_command_path("codex", runtime_config);
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
            resolve_installed_shell_command(
                command,
                cwd,
                env,
                package_path,
                &package_bins,
                &python_scripts,
                python_env.as_deref(),
                &node_path,
                &codex_path,
            );
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

fn resolve_installed_shell_command(
    command: &mut Vec<String>,
    cwd: &mut Option<String>,
    env: &mut BTreeMap<String, String>,
    package_path: &Path,
    package_bins: &BTreeMap<String, String>,
    python_scripts: &BTreeMap<String, String>,
    python_env: Option<&Path>,
    node_path: &Option<PathBuf>,
    codex_path: &Option<PathBuf>,
) {
    if command.is_empty() {
        return;
    }
    let executable = command[0].trim();
    if executable.is_empty() || Path::new(executable).is_absolute() {
        return;
    }

    if let Some(direct_path) = native_command_path(package_path, executable) {
        command[0] = direct_path.display().to_string();
        enrich_start_command_env(env, [node_path, codex_path]);
        if cwd.as_deref().map(str::trim).unwrap_or_default().is_empty() {
            *cwd = Some(package_path.display().to_string());
        }
        return;
    }

    if python_scripts.contains_key(executable) {
        if let Some(env_path) = python_env {
            command[0] = python_script_path(env_path, executable)
                .display()
                .to_string();
            enrich_start_command_env(env, [node_path, codex_path]);
            prepend_path_entry(env, python_bin_dir(env_path));
            if cwd.as_deref().map(str::trim).unwrap_or_default().is_empty() {
                *cwd = Some(package_path.display().to_string());
            }
            return;
        }
    }

    if let Some(relative_bin) = package_bins.get(executable) {
        command[0] = node_path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "node".to_string());
        command.insert(1, package_path.join(relative_bin).display().to_string());
        enrich_start_command_env(env, [node_path, codex_path]);
        if cwd.as_deref().map(str::trim).unwrap_or_default().is_empty() {
            *cwd = Some(package_path.display().to_string());
        }
        return;
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
    let os = match env::consts::OS {
        "macos" => "macos",
        "windows" => "windows",
        "linux" => "linux",
        other => other,
    };
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
        return candidates
            .iter()
            .map(PathBuf::from)
            .find(|candidate| candidate.is_file());
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
) -> Result<Option<PathBuf>> {
    if scripts.is_empty() {
        return Ok(None);
    }
    let env_path = package_path.join(CONNECTOR_PYTHON_ENV_DIR);
    let python = python_env_executable(&env_path);
    if !python.exists() {
        create_python_env(package_path, &env_path)?;
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
    for relative in ["pyproject.toml", "setup.py", "setup.cfg"] {
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

fn create_python_env(package_path: &Path, env_path: &Path) -> Result<()> {
    let parent = env_path
        .parent()
        .with_context(|| format!("failed to resolve parent for {}", env_path.display()))?;
    fs::create_dir_all(parent).with_context(|| format!("failed to create {}", parent.display()))?;
    let requires_python = read_python_requires_python(package_path)?;
    let python = resolve_python_for_project(requires_python.as_deref())?;
    let output = Command::new(&python)
        .args(["-m", "venv"])
        .arg(env_path)
        .output()
        .with_context(|| format!("failed to create Python environment with `{python} -m venv`"))?;
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

fn resolve_python_for_project(requires_python: Option<&str>) -> Result<String> {
    for candidate in python_candidates() {
        if python_matches_requirement(&candidate, requires_python) {
            return Ok(candidate.display().to_string());
        }
    }
    bail!(
        "failed to find a Python interpreter matching {}. Install Python 3.10+ and make it available in PATH.",
        requires_python.unwrap_or("the connector requirement")
    )
}

fn python_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(value) = env::var("BRIDGE_AGENT_PYTHON")
        .ok()
        .filter(|value| !value.trim().is_empty())
    {
        push_unique_path_entry(&mut candidates, PathBuf::from(value));
    }
    for executable in [
        "python3.12",
        "python3.11",
        "python3.10",
        "python3",
        "python",
    ] {
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
    match minimum_python_version(requires_python) {
        Some(minimum) => version >= minimum,
        None => true,
    }
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

fn minimum_python_version(requires_python: Option<&str>) -> Option<(u32, u32, u32)> {
    let requires_python = requires_python?;
    requires_python
        .split(',')
        .filter_map(|part| part.trim().strip_prefix(">="))
        .filter_map(parse_python_version)
        .max()
}

fn install_python_project_dependencies(package_path: &Path, python: &Path) -> Result<()> {
    let dependencies = read_python_project_dependencies(package_path)?;
    if dependencies.is_empty() {
        return Ok(());
    }
    let output = Command::new(python)
        .args(["-m", "pip", "install", "--disable-pip-version-check"])
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
            "@echo off\r\n\"{}\" -c \"import sys; sys.path.insert(0, r'{}'); from {} import {}; raise SystemExit({}())\" %*\r\n",
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
            "#!/bin/sh\nexec {} - \"$@\" <<'PY'\nimport sys\nsys.path.insert(0, {:?})\nfrom {} import {}\nif __name__ == '__main__':\n    raise SystemExit({}())\nPY\n",
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
    let mut labels = Vec::new();
    for value in manifest.hooks.values() {
        if value.contains("install-autostart") {
            labels.push(manifest.id.clone());
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
    ConnectorSummary {
        id: record.manifest.id,
        name: record.manifest.name,
        version: record.manifest.version,
        package_path: record.package_path,
        source_path: record.source_path,
        source_reference: record.source_reference,
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

        resolve_installed_shell_command(
            &mut command,
            &mut cwd,
            &mut env_vars,
            dir.path(),
            &BTreeMap::new(),
            &scripts,
            Some(&env_path),
            &None,
            &None,
        );

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
        assert_eq!(minimum_python_version(Some(">=3.10,<4")), Some((3, 10, 0)));
        assert_eq!(
            minimum_python_version(Some(">=3.10,>=3.11")),
            Some((3, 11, 0))
        );
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
    fn legacy_autostart_cleanup_does_not_remove_current_wechat_collector_label() {
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
            config_schema: None,
            remote_capabilities: Vec::new(),
            services: Vec::new(),
            service_registration_files: Vec::new(),
            hooks: BTreeMap::from([(
                "installAutostart".to_string(),
                "wechat-bridge-collector install-autostart".to_string(),
            )]),
        };

        assert_eq!(
            legacy_autostart_labels_for_manifest(&manifest),
            vec!["com.baijimu.connector.wechat".to_string()]
        );
    }
}

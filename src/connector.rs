use crate::config::{
    ensure_config_exists, load_config, save_config, ServiceConfig, ServiceRegistration,
    ServiceStartCommand,
};
use anyhow::{bail, Context, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

const CONNECTOR_MANIFEST_FILE: &str = "connector.json";
const CONNECTOR_INSTALL_RECORD_FILE: &str = "install.json";

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
    pub service_names: Vec<String>,
    pub installed_at_epoch_ms: u64,
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
    pub service_names: Vec<String>,
    pub installed_at_epoch_ms: u64,
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
    if let Some(dirs) = ProjectDirs::from("com", "Baijimu", "BridgeAgent") {
        return Ok(dirs.config_dir().join("connectors"));
    }
    bail!("failed to resolve BridgeAgent config directory")
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
    ensure_config_exists(config_path)?;
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
    let services = registrations
        .into_iter()
        .map(ServiceRegistration::into_service_config)
        .collect::<Result<Vec<_>>>()?;

    let package_path = installed_connector_package_path(&manifest)?;
    if package_path.exists() {
        fs::remove_dir_all(&package_path)
            .with_context(|| format!("failed to replace connector {}", package_path.display()))?;
    }
    copy_connector_package(&source, &package_path)?;

    let mut config = load_config(config_path)?;
    for service in &services {
        upsert_service(&mut config.services, service.clone(), replace)?;
    }
    save_config(config_path, &config)?;

    let record = ConnectorInstallRecord {
        manifest: manifest.clone(),
        package_path: package_path.display().to_string(),
        source_path: source.display().to_string(),
        service_names: service_names.clone(),
        installed_at_epoch_ms: now_ms(),
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
    let dir = connectors_dir()?;
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut connectors = Vec::new();
    for entry in fs::read_dir(&dir).with_context(|| format!("failed to read {}", dir.display()))? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let id = entry.file_name().to_string_lossy().to_string();
        match load_install_record(&id) {
            Ok(record) => connectors.push(summary_from_record(record)),
            Err(err) => tracing::warn!("failed to load connector `{id}`: {err:#}"),
        }
    }
    connectors.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(connectors)
}

pub fn show_connector(connector_id: &str) -> Result<ConnectorInstallRecord> {
    load_install_record(connector_id)
}

pub fn start_connector(connector_id: &str, config_path: &Path) -> Result<ConnectorStartResult> {
    ensure_config_exists(config_path)?;
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
    ConnectorSummary {
        id: record.manifest.id,
        name: record.manifest.name,
        version: record.manifest.version,
        package_path: record.package_path,
        service_names: record.service_names,
        installed_at_epoch_ms: record.installed_at_epoch_ms,
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
    use tempfile::tempdir;

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
    fn install_connector_updates_agent_config() {
        let dir = tempdir().unwrap();
        std::env::set_var("BRIDGE_AGENT_CONNECTORS_DIR", dir.path().join("connectors"));
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
}

use anyhow::{bail, Context, Result};
use clap::Parser;
use directories::{BaseDirs, ProjectDirs};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};
use sysinfo::{Pid, Signal, System};

const ARTIFACT_VERSION: &str = "1.0.0";
const LEDGER_FILE: &str = "unified-app-id-migration-ledger.json";
const LOCK_FILE: &str = "unified-app-id-migration.lock";
const LEGACY_MANAGED_CLI_APP_ID: &str = "com.baijimu.cli";
const MANAGED_CLI_APP_ID: &str = "baijimu-cli";

#[derive(Debug, Parser)]
#[command(name = "bridge-agent-unified-app-id-migration")]
struct Args {
    #[arg(long)]
    config_dir: Option<PathBuf>,
    #[arg(long)]
    config: Option<PathBuf>,
    #[arg(long)]
    leave_host_running: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LegacyManifestIdentity {
    schema_version: String,
    id: String,
    version: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LegacyInstallRecord {
    manifest: LegacyManifestIdentity,
    market_app_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LegacyLocalAppConfig {
    connector_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LocalControlDiscovery {
    schema_version: u32,
    pid: u32,
    base_url: String,
    token: String,
    started_at_epoch_ms: u64,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct AgentConfigWithoutInstallations {
    platform: Box<RawValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    upload: Option<Box<RawValue>>,
    relay: Box<RawValue>,
    device: Box<RawValue>,
    runtime: Box<RawValue>,
    #[serde(default)]
    services: Vec<Box<RawValue>>,
    #[serde(default)]
    local_apps: Vec<Box<RawValue>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyAgentConfig {
    platform: Box<RawValue>,
    #[serde(default)]
    upload: Option<Box<RawValue>>,
    relay: Box<RawValue>,
    device: Box<RawValue>,
    runtime: Box<RawValue>,
    #[serde(default)]
    services: Vec<Box<RawValue>>,
    #[serde(default)]
    local_apps: Vec<LegacyLocalAppConfig>,
}

impl From<LegacyAgentConfig> for AgentConfigWithoutInstallations {
    fn from(value: LegacyAgentConfig) -> Self {
        Self {
            platform: value.platform,
            upload: value.upload,
            relay: value.relay,
            device: value.device,
            runtime: value.runtime,
            services: value.services,
            local_apps: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum MigrationPhase {
    Prepared,
    AppsStopped,
    DataMoved,
    PackagesArchived,
    ConfigWritten,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MigrationEntry {
    legacy_identity: String,
    app_id: String,
    version: String,
    phase: MigrationPhase,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MigrationLedger {
    artifact_version: String,
    legacy_apps_stopped: bool,
    entries: Vec<MigrationEntry>,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let config_dir = args.config_dir.unwrap_or(default_config_dir()?);
    let config_path = args
        .config
        .unwrap_or_else(|| config_dir.join("agent-config.json"));
    let managed_apps = default_managed_apps_dir()?;
    migrate(
        &config_dir,
        &config_path,
        args.leave_host_running,
        &managed_apps,
    )
}

fn default_config_dir() -> Result<PathBuf> {
    ProjectDirs::from("com", "baijimu", "bridge-agent")
        .map(|dirs| dirs.config_dir().to_path_buf())
        .context("failed to resolve Bridge Agent configuration directory")
}

fn migrate(
    config_dir: &Path,
    config_path: &Path,
    leave_host_running: bool,
    managed_apps: &Path,
) -> Result<()> {
    fs::create_dir_all(config_dir)
        .with_context(|| format!("failed to create {}", config_dir.display()))?;
    let lock_path = config_dir.join(LOCK_FILE);
    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)
        .with_context(|| format!("failed to open migration lock {}", lock_path.display()))?;
    lock.try_lock_exclusive()
        .context("another unified app ID migration is running")?;

    let ledger_path = config_dir.join(LEDGER_FILE);
    let mut ledger = if ledger_path.exists() {
        read_json::<MigrationLedger>(&ledger_path)?
    } else {
        let ledger = preflight(config_dir, config_path)?;
        write_json_atomically(&ledger_path, &ledger)?;
        ledger
    };
    if ledger.artifact_version != ARTIFACT_VERSION {
        bail!(
            "migration ledger version {} cannot be handled by artifact {}",
            ledger.artifact_version,
            ARTIFACT_VERSION
        );
    }

    let needs_stop = ledger
        .entries
        .iter()
        .any(|entry| entry.phase == MigrationPhase::Prepared);
    if needs_stop || (!ledger.legacy_apps_stopped && !ledger.entries.is_empty()) {
        let discovery =
            read_json::<LocalControlDiscovery>(&config_dir.join("local-app-control.json"))?;
        if discovery.schema_version != 1 {
            bail!(
                "unsupported local control schema {}",
                discovery.schema_version
            );
        }
        for index in 0..ledger.entries.len() {
            if ledger.entries[index].phase != MigrationPhase::Prepared {
                continue;
            }
            stop_legacy_app(&discovery, &ledger.entries[index].legacy_identity)?;
            ledger.entries[index].phase = MigrationPhase::AppsStopped;
            write_json_atomically(&ledger_path, &ledger)?;
        }
        if !leave_host_running {
            stop_legacy_bridge(&discovery)?;
        }
        ledger.legacy_apps_stopped = true;
        write_json_atomically(&ledger_path, &ledger)?;
    }
    for index in 0..ledger.entries.len() {
        if ledger.entries[index].phase == MigrationPhase::AppsStopped {
            move_app_data(config_dir, &ledger.entries[index])?;
            ledger.entries[index].phase = MigrationPhase::DataMoved;
            write_json_atomically(&ledger_path, &ledger)?;
        }
    }
    for index in 0..ledger.entries.len() {
        if ledger.entries[index].phase == MigrationPhase::DataMoved {
            archive_package(config_dir, &ledger.entries[index])?;
            ledger.entries[index].phase = MigrationPhase::PackagesArchived;
            write_json_atomically(&ledger_path, &ledger)?;
        }
    }
    migrate_managed_cli_app_id_at(managed_apps)?;
    if ledger
        .entries
        .iter()
        .any(|entry| entry.phase == MigrationPhase::PackagesArchived)
    {
        rewrite_config_without_installations(config_path, &ledger)?;
        for entry in &mut ledger.entries {
            if entry.phase == MigrationPhase::PackagesArchived {
                entry.phase = MigrationPhase::ConfigWritten;
            }
        }
        write_json_atomically(&ledger_path, &ledger)?;
    }
    if ledger
        .entries
        .iter()
        .any(|entry| entry.phase != MigrationPhase::ConfigWritten)
    {
        bail!("migration did not reach CONFIG_WRITTEN for every application");
    }
    fs::remove_file(&ledger_path).with_context(|| {
        format!(
            "failed to remove completed ledger {}",
            ledger_path.display()
        )
    })?;
    FileExt::unlock(&lock)?;
    fs::remove_file(&lock_path)
        .with_context(|| format!("failed to remove migration lock {}", lock_path.display()))?;
    Ok(())
}

fn default_managed_apps_dir() -> Result<PathBuf> {
    let base_dirs = BaseDirs::new().context("failed to resolve local data directory")?;
    #[cfg(windows)]
    let managed_apps = base_dirs.data_local_dir().join("Baijimu").join("apps");
    #[cfg(not(windows))]
    let managed_apps = base_dirs.data_local_dir().join("baijimu").join("apps");
    Ok(managed_apps)
}

fn migrate_managed_cli_app_id_at(managed_apps: &Path) -> Result<()> {
    move_once(
        &managed_apps.join(LEGACY_MANAGED_CLI_APP_ID),
        &managed_apps.join(MANAGED_CLI_APP_ID),
        "managed Baijimu CLI application",
    )
}

fn preflight(config_dir: &Path, config_path: &Path) -> Result<MigrationLedger> {
    let config = read_json::<LegacyAgentConfig>(config_path)?;
    let connectors_dir = config_dir.join("connectors");
    let mut entries = Vec::new();
    if connectors_dir.exists() {
        for item in fs::read_dir(&connectors_dir)
            .with_context(|| format!("failed to read {}", connectors_dir.display()))?
        {
            let item = item?;
            if !item.file_type()?.is_dir() {
                continue;
            }
            let record_path = item.path().join("install.json");
            if !record_path.is_file() {
                continue;
            }
            let record = read_json::<LegacyInstallRecord>(&record_path)?;
            if !matches!(
                record.manifest.schema_version.as_str(),
                "1.0" | "1.1" | "1.2" | "2.0"
            ) {
                bail!(
                    "legacy installation {} uses unsupported manifest schema {}",
                    record.manifest.id,
                    record.manifest.schema_version
                );
            }
            let app_id = record
                .market_app_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .with_context(|| {
                    format!(
                        "legacy installation {} is not associated with a registered market app; register and reinstall it before upgrading",
                        record.manifest.id
                    )
                })?;
            if entries
                .iter()
                .any(|entry: &MigrationEntry| entry.app_id == app_id)
            {
                bail!("multiple legacy installations resolve to appId {app_id}");
            }
            entries.push(MigrationEntry {
                legacy_identity: record.manifest.id,
                app_id: app_id.to_string(),
                version: record.manifest.version,
                phase: MigrationPhase::Prepared,
            });
        }
    }
    entries.sort_by(|left, right| left.app_id.cmp(&right.app_id));
    for app in &config.local_apps {
        if !entries
            .iter()
            .any(|entry| entry.legacy_identity == app.connector_id)
        {
            bail!(
                "configured legacy application {} has no registered installation record",
                app.connector_id
            );
        }
    }
    Ok(MigrationLedger {
        artifact_version: ARTIFACT_VERSION.to_string(),
        legacy_apps_stopped: false,
        entries,
    })
}

fn stop_legacy_app(discovery: &LocalControlDiscovery, legacy_identity: &str) -> Result<()> {
    let url = format!(
        "{}/local-apps/{}/stop",
        discovery.base_url.trim_end_matches('/'),
        legacy_identity
    );
    let response = reqwest::blocking::Client::new()
        .post(url)
        .bearer_auth(&discovery.token)
        .send()
        .with_context(|| format!("failed to stop legacy application {legacy_identity}"))?;
    if !response.status().is_success() {
        bail!(
            "legacy application {legacy_identity} stop request returned {}",
            response.status()
        );
    }
    Ok(())
}

fn stop_legacy_bridge(discovery: &LocalControlDiscovery) -> Result<()> {
    let _ = discovery.started_at_epoch_ms;
    let pid = Pid::from_u32(discovery.pid);
    let mut system = System::new_all();
    let Some(process) = system.process(pid) else {
        return Ok(());
    };
    let signalled = process.kill_with(Signal::Term).unwrap_or(false) || process.kill();
    if !signalled {
        bail!(
            "failed to terminate legacy Bridge Agent process {}",
            discovery.pid
        );
    }
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        thread::sleep(Duration::from_millis(200));
        system.refresh_all();
        if system.process(pid).is_none() {
            return Ok(());
        }
    }
    bail!(
        "legacy Bridge Agent process {} did not terminate within 15 seconds",
        discovery.pid
    )
}

fn move_app_data(config_dir: &Path, entry: &MigrationEntry) -> Result<()> {
    let source = config_dir
        .join("connector-data")
        .join(&entry.legacy_identity);
    let target = config_dir.join("app-data").join(&entry.app_id);
    move_once(&source, &target, "application data")
}

fn archive_package(config_dir: &Path, entry: &MigrationEntry) -> Result<()> {
    let source = config_dir.join("connectors").join(&entry.legacy_identity);
    let target = config_dir
        .join("migration-backups")
        .join("unified-app-id")
        .join(ARTIFACT_VERSION)
        .join("connectors")
        .join(&entry.legacy_identity);
    move_once(&source, &target, "legacy package")
}

fn move_once(source: &Path, target: &Path, label: &str) -> Result<()> {
    match (source.exists(), target.exists()) {
        (false, true) | (false, false) => Ok(()),
        (true, true) => bail!(
            "cannot migrate {label}: both source {} and target {} exist",
            source.display(),
            target.display()
        ),
        (true, false) => {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("failed to create {}", parent.display()))?;
            }
            fs::rename(source, target).with_context(|| {
                format!(
                    "failed to move {label} from {} to {}",
                    source.display(),
                    target.display()
                )
            })
        }
    }
}

fn rewrite_config_without_installations(
    config_path: &Path,
    ledger: &MigrationLedger,
) -> Result<()> {
    let config = read_json::<LegacyAgentConfig>(config_path)?;
    for app in &config.local_apps {
        if !ledger
            .entries
            .iter()
            .any(|entry| entry.legacy_identity == app.connector_id)
        {
            bail!(
                "configuration changed after preflight; unknown legacy application {}",
                app.connector_id
            );
        }
    }
    let config_backup = config_path
        .parent()
        .context("agent configuration path has no parent directory")?
        .join("migration-backups")
        .join("unified-app-id")
        .join(ARTIFACT_VERSION)
        .join("agent-config.json");
    if !config_backup.exists() {
        if let Some(parent) = config_backup.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        fs::copy(config_path, &config_backup).with_context(|| {
            format!(
                "failed to back up configuration from {} to {}",
                config_path.display(),
                config_backup.display()
            )
        })?;
    }
    write_json_atomically(config_path, &AgentConfigWithoutInstallations::from(config))
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T> {
    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    serde_json::from_slice(&bytes).with_context(|| format!("failed to parse {}", path.display()))
}

fn write_json_atomically(path: &Path, value: &impl Serialize) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let temporary = path.with_extension("migration.tmp");
    let bytes = serde_json::to_vec_pretty(value)?;
    fs::write(&temporary, bytes)
        .with_context(|| format!("failed to write {}", temporary.display()))?;
    fs::rename(&temporary, path).with_context(|| format!("failed to replace {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    use tempfile::tempdir;

    fn write_legacy_config(path: &Path, connector_ids: &[&str]) {
        let local_apps = connector_ids
            .iter()
            .map(|connector_id| json!({ "connectorId": connector_id, "enabled": true }))
            .collect::<Vec<_>>();
        fs::write(
            path,
            serde_json::to_vec_pretty(&json!({
                "platform": { "base_url": "https://api.example.test", "workspace_id": 42 },
                "upload": { "inline_limit_bytes": 262144 },
                "relay": { "agent_id": "device-1", "token": "secret-reference" },
                "device": { "name": "Test Device" },
                "runtime": { "default_timeout_secs": 30 },
                "services": [{ "name": "shell", "enabled": true }],
                "local_apps": local_apps
            }))
            .unwrap(),
        )
        .unwrap();
    }

    fn write_install_record(config_dir: &Path, legacy_identity: &str, market_app_id: Option<&str>) {
        let install_root = config_dir.join("connectors").join(legacy_identity);
        fs::create_dir_all(&install_root).unwrap();
        fs::write(
            install_root.join("install.json"),
            serde_json::to_vec_pretty(&json!({
                "manifest": {
                    "schemaVersion": "2.0",
                    "id": legacy_identity,
                    "version": "0.1.0"
                },
                "marketAppId": market_app_id
            }))
            .unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn preflight_uses_registered_market_id_as_the_only_target_app_id() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("agent-config.json");
        write_legacy_config(&config_path, &["com.example.legacy"]);
        write_install_record(dir.path(), "com.example.legacy", Some("server-app-01"));

        let ledger = preflight(dir.path(), &config_path).unwrap();

        assert_eq!(ledger.artifact_version, ARTIFACT_VERSION);
        assert_eq!(ledger.entries.len(), 1);
        assert_eq!(ledger.entries[0].legacy_identity, "com.example.legacy");
        assert_eq!(ledger.entries[0].app_id, "server-app-01");
        assert_eq!(ledger.entries[0].phase, MigrationPhase::Prepared);
    }

    #[test]
    fn preflight_rejects_unregistered_and_duplicate_target_app_ids() {
        let unregistered = tempdir().unwrap();
        let config_path = unregistered.path().join("agent-config.json");
        write_legacy_config(&config_path, &["com.example.unregistered"]);
        write_install_record(unregistered.path(), "com.example.unregistered", None);
        assert!(preflight(unregistered.path(), &config_path)
            .unwrap_err()
            .to_string()
            .contains("not associated with a registered market app"));

        let duplicate = tempdir().unwrap();
        let duplicate_config = duplicate.path().join("agent-config.json");
        write_legacy_config(&duplicate_config, &["legacy.one", "legacy.two"]);
        write_install_record(duplicate.path(), "legacy.one", Some("server-app-01"));
        write_install_record(duplicate.path(), "legacy.two", Some("server-app-01"));
        assert!(preflight(duplicate.path(), &duplicate_config)
            .unwrap_err()
            .to_string()
            .contains("multiple legacy installations resolve to appId"));
    }

    #[test]
    fn rewrite_clears_only_installations_and_preserves_other_config_documents() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("agent-config.json");
        write_legacy_config(&config_path, &["com.example.legacy"]);
        let ledger = MigrationLedger {
            artifact_version: ARTIFACT_VERSION.to_string(),
            legacy_apps_stopped: true,
            entries: vec![MigrationEntry {
                legacy_identity: "com.example.legacy".to_string(),
                app_id: "server-app-01".to_string(),
                version: "0.1.0".to_string(),
                phase: MigrationPhase::PackagesArchived,
            }],
        };

        rewrite_config_without_installations(&config_path, &ledger).unwrap();
        let rewritten: Value = read_json(&config_path).unwrap();

        assert_eq!(rewritten["local_apps"], json!([]));
        assert_eq!(rewritten["platform"]["workspace_id"], 42);
        assert_eq!(rewritten["relay"]["token"], "secret-reference");
        assert_eq!(rewritten["device"]["name"], "Test Device");
        assert_eq!(rewritten["services"][0]["name"], "shell");
        let backup = dir
            .path()
            .join("migration-backups/unified-app-id/1.0.0/agent-config.json");
        let original: Value = read_json(&backup).unwrap();
        assert_eq!(
            original["local_apps"][0]["connectorId"],
            "com.example.legacy"
        );
    }

    #[test]
    fn data_and_packages_move_once_and_conflicts_are_rejected() {
        let dir = tempdir().unwrap();
        let entry = MigrationEntry {
            legacy_identity: "com.example.legacy".to_string(),
            app_id: "server-app-01".to_string(),
            version: "0.1.0".to_string(),
            phase: MigrationPhase::AppsStopped,
        };
        let source_data = dir
            .path()
            .join("connector-data")
            .join(&entry.legacy_identity);
        fs::create_dir_all(&source_data).unwrap();
        fs::write(source_data.join("session.json"), b"signed-in-state").unwrap();
        let source_package = dir.path().join("connectors").join(&entry.legacy_identity);
        fs::create_dir_all(&source_package).unwrap();
        fs::write(source_package.join("install.json"), b"{}").unwrap();

        move_app_data(dir.path(), &entry).unwrap();
        archive_package(dir.path(), &entry).unwrap();

        assert_eq!(
            fs::read(
                dir.path()
                    .join("app-data")
                    .join(&entry.app_id)
                    .join("session.json")
            )
            .unwrap(),
            b"signed-in-state"
        );
        assert!(dir
            .path()
            .join("migration-backups/unified-app-id/1.0.0/connectors")
            .join(&entry.legacy_identity)
            .join("install.json")
            .is_file());

        fs::create_dir_all(&source_data).unwrap();
        assert!(move_app_data(dir.path(), &entry).is_err());
    }

    #[test]
    fn empty_installation_migration_completes_without_control_discovery() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("agent-config.json");
        write_legacy_config(&config_path, &[]);

        migrate(
            dir.path(),
            &config_path,
            false,
            &dir.path().join("managed-apps"),
        )
        .unwrap();

        assert!(!dir.path().join(LEDGER_FILE).exists());
        assert!(!dir.path().join(LOCK_FILE).exists());
    }

    #[test]
    fn managed_cli_directory_moves_to_the_registered_app_id() {
        let dir = tempdir().unwrap();
        let legacy = dir.path().join(LEGACY_MANAGED_CLI_APP_ID);
        fs::create_dir_all(legacy.join("versions/0.11.0")).unwrap();
        fs::write(legacy.join("state.json"), b"managed-state").unwrap();

        migrate_managed_cli_app_id_at(dir.path()).unwrap();
        migrate_managed_cli_app_id_at(dir.path()).unwrap();

        assert!(!legacy.exists());
        assert_eq!(
            fs::read(dir.path().join(MANAGED_CLI_APP_ID).join("state.json")).unwrap(),
            b"managed-state"
        );
    }
}

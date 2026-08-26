use anyhow::{bail, Context, Result};
use clap::Parser;
use directories::{BaseDirs, ProjectDirs};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};
use sysinfo::{Pid, Signal, System};

const ARTIFACT_VERSION: &str = "1.0.1";
const PREVIOUS_ARTIFACT_VERSION: &str = "1.0.0";
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
    #[arg(long, conflicts_with = "leave_host_running")]
    host_already_stopped: bool,
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
        args.host_already_stopped,
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
    host_already_stopped: bool,
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
    if !matches!(
        ledger.artifact_version.as_str(),
        ARTIFACT_VERSION | PREVIOUS_ARTIFACT_VERSION
    ) {
        bail!(
            "migration ledger version {} cannot be handled by artifact {}",
            ledger.artifact_version,
            ARTIFACT_VERSION
        );
    }

    let has_legacy_state = legacy_state_exists(config_dir, config_path)?;
    let needs_stop = ledger
        .entries
        .iter()
        .any(|entry| entry.phase == MigrationPhase::Prepared)
        || (!ledger.legacy_apps_stopped && has_legacy_state);
    if needs_stop {
        if host_already_stopped {
            verify_legacy_host_and_apps_stopped(config_dir, &ledger)?;
            for entry in &mut ledger.entries {
                if entry.phase == MigrationPhase::Prepared {
                    entry.phase = MigrationPhase::AppsStopped;
                }
            }
            write_json_atomically(&ledger_path, &ledger)?;
        } else {
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
    archive_unregistered_legacy_state(config_dir, config_path, &ledger)?;
    migrate_managed_cli_app_id_at(managed_apps)?;
    if legacy_config_has_local_apps(config_path)?
        || ledger
            .entries
            .iter()
            .any(|entry| entry.phase == MigrationPhase::PackagesArchived)
    {
        rewrite_config_without_installations(config_path)?;
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

fn verify_legacy_host_and_apps_stopped(config_dir: &Path, _ledger: &MigrationLedger) -> Result<()> {
    let mut system = System::new_all();
    system.refresh_all();
    let discovery_path = config_dir.join("local-app-control.json");
    if discovery_path.is_file() {
        let discovery = read_json::<LocalControlDiscovery>(&discovery_path)?;
        if discovery.schema_version != 1 {
            bail!(
                "unsupported local control schema {}",
                discovery.schema_version
            );
        }
        if system.process(Pid::from_u32(discovery.pid)).is_some() {
            bail!(
                "legacy Bridge Agent process {} is still running; close it before offline migration",
                discovery.pid
            );
        }
    }

    let packages_root = config_dir.join("connectors");
    for (pid, process) in system.processes() {
        let uses_legacy_package = process
            .exe()
            .is_some_and(|path| path.starts_with(&packages_root))
            || process
                .cwd()
                .is_some_and(|path| path.starts_with(&packages_root))
            || process
                .cmd()
                .iter()
                .any(|argument| Path::new(argument).starts_with(&packages_root));
        if uses_legacy_package {
            bail!(
                "legacy application process {pid} is still using {}; close it before offline migration",
                packages_root.display()
            );
        }
    }
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
    let source = managed_apps.join(LEGACY_MANAGED_CLI_APP_ID);
    let target = managed_apps.join(MANAGED_CLI_APP_ID);
    if source.exists() && target.exists() {
        return move_once(
            &source,
            &managed_apps
                .join("migration-backups")
                .join("unified-app-id")
                .join(ARTIFACT_VERSION)
                .join(LEGACY_MANAGED_CLI_APP_ID),
            "superseded managed Baijimu CLI application",
        );
    }
    move_once(&source, &target, "managed Baijimu CLI application")
}

fn preflight(config_dir: &Path, config_path: &Path) -> Result<MigrationLedger> {
    let _: LegacyAgentConfig = read_json(config_path)?;
    let connectors_dir = config_dir.join("connectors");
    let mut candidates = Vec::new();
    if connectors_dir.exists() {
        for item in fs::read_dir(&connectors_dir)
            .with_context(|| format!("failed to read {}", connectors_dir.display()))?
        {
            let item = item?;
            if !item.file_type()?.is_dir() {
                continue;
            }
            let directory_identity = item.file_name().to_string_lossy().into_owned();
            validate_path_segment(&directory_identity, "legacy application identity")?;
            let record_path = item.path().join("install.json");
            if !record_path.is_file() {
                continue;
            }
            let Ok(record) = read_json::<LegacyInstallRecord>(&record_path) else {
                continue;
            };
            if !matches!(
                record.manifest.schema_version.as_str(),
                "1.0" | "1.1" | "1.2" | "2.0"
            ) {
                continue;
            }
            if record.manifest.id != directory_identity {
                continue;
            }
            let Some(app_id) = record
                .market_app_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            else {
                continue;
            };
            validate_path_segment(&record.manifest.id, "legacy application identity")?;
            validate_path_segment(app_id, "registered application ID")?;
            candidates.push(MigrationEntry {
                legacy_identity: record.manifest.id,
                app_id: app_id.to_string(),
                version: record.manifest.version,
                phase: MigrationPhase::Prepared,
            });
        }
    }
    let mut app_id_counts = BTreeMap::<String, usize>::new();
    for candidate in &candidates {
        *app_id_counts.entry(candidate.app_id.clone()).or_default() += 1;
    }
    let mut entries = candidates
        .into_iter()
        .filter(|candidate| app_id_counts.get(&candidate.app_id) == Some(&1))
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| left.app_id.cmp(&right.app_id));
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
    if source.exists() && target.exists() {
        return move_once(
            &source,
            &config_dir
                .join("migration-backups")
                .join("unified-app-id")
                .join(ARTIFACT_VERSION)
                .join("superseded")
                .join("connector-data")
                .join(&entry.legacy_identity),
            "superseded legacy application data",
        );
    }
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

fn legacy_config_has_local_apps(config_path: &Path) -> Result<bool> {
    Ok(!read_json::<LegacyAgentConfig>(config_path)?
        .local_apps
        .is_empty())
}

fn directory_has_entries(path: &Path) -> Result<bool> {
    if !path.is_dir() {
        return Ok(false);
    }
    Ok(fs::read_dir(path)
        .with_context(|| format!("failed to read {}", path.display()))?
        .next()
        .transpose()?
        .is_some())
}

fn legacy_state_exists(config_dir: &Path, config_path: &Path) -> Result<bool> {
    Ok(legacy_config_has_local_apps(config_path)?
        || directory_has_entries(&config_dir.join("connectors"))?
        || directory_has_entries(&config_dir.join("connector-data"))?)
}

fn legacy_identity_set(config_dir: &Path, config_path: &Path) -> Result<BTreeSet<String>> {
    let mut identities = read_json::<LegacyAgentConfig>(config_path)?
        .local_apps
        .into_iter()
        .map(|app| app.connector_id)
        .collect::<BTreeSet<_>>();
    for root in [
        config_dir.join("connectors"),
        config_dir.join("connector-data"),
    ] {
        if !root.is_dir() {
            continue;
        }
        for item in
            fs::read_dir(&root).with_context(|| format!("failed to read {}", root.display()))?
        {
            let item = item?;
            if item.file_type()?.is_dir() {
                identities.insert(item.file_name().to_string_lossy().into_owned());
            }
        }
    }
    for identity in &identities {
        validate_path_segment(identity, "legacy application identity")?;
    }
    Ok(identities)
}

fn validate_path_segment(value: &str, label: &str) -> Result<()> {
    let path = Path::new(value);
    if value.is_empty()
        || value.trim() != value
        || path.is_absolute()
        || path.components().count() != 1
        || matches!(value, "." | "..")
    {
        bail!("invalid {label} {value:?}");
    }
    Ok(())
}

fn archive_unregistered_legacy_state(
    config_dir: &Path,
    config_path: &Path,
    ledger: &MigrationLedger,
) -> Result<()> {
    let registered = ledger
        .entries
        .iter()
        .map(|entry| entry.legacy_identity.as_str())
        .collect::<BTreeSet<_>>();
    let backup_root = config_dir
        .join("migration-backups")
        .join("unified-app-id")
        .join(ARTIFACT_VERSION)
        .join("unregistered");
    for identity in legacy_identity_set(config_dir, config_path)? {
        if registered.contains(identity.as_str()) {
            continue;
        }
        move_once(
            &config_dir.join("connectors").join(&identity),
            &backup_root.join("connectors").join(&identity),
            "unregistered legacy application package",
        )?;
        move_once(
            &config_dir.join("connector-data").join(&identity),
            &backup_root.join("connector-data").join(&identity),
            "unregistered legacy application data",
        )?;
    }
    Ok(())
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

fn rewrite_config_without_installations(config_path: &Path) -> Result<()> {
    let config = read_json::<LegacyAgentConfig>(config_path)?;
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
    fn preflight_excludes_unregistered_and_ambiguous_target_app_ids() {
        let unregistered = tempdir().unwrap();
        let config_path = unregistered.path().join("agent-config.json");
        write_legacy_config(&config_path, &["com.example.unregistered"]);
        write_install_record(unregistered.path(), "com.example.unregistered", None);
        assert!(preflight(unregistered.path(), &config_path)
            .unwrap()
            .entries
            .is_empty());

        let duplicate = tempdir().unwrap();
        let duplicate_config = duplicate.path().join("agent-config.json");
        write_legacy_config(&duplicate_config, &["legacy.one", "legacy.two"]);
        write_install_record(duplicate.path(), "legacy.one", Some("server-app-01"));
        write_install_record(duplicate.path(), "legacy.two", Some("server-app-01"));
        assert!(preflight(duplicate.path(), &duplicate_config)
            .unwrap()
            .entries
            .is_empty());
    }

    #[test]
    fn rewrite_clears_only_installations_and_preserves_other_config_documents() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("agent-config.json");
        write_legacy_config(&config_path, &["com.example.legacy"]);

        rewrite_config_without_installations(&config_path).unwrap();
        let rewritten: Value = read_json(&config_path).unwrap();

        assert_eq!(rewritten["local_apps"], json!([]));
        assert_eq!(rewritten["platform"]["workspace_id"], 42);
        assert_eq!(rewritten["relay"]["token"], "secret-reference");
        assert_eq!(rewritten["device"]["name"], "Test Device");
        assert_eq!(rewritten["services"][0]["name"], "shell");
        let backup = dir
            .path()
            .join("migration-backups/unified-app-id")
            .join(ARTIFACT_VERSION)
            .join("agent-config.json");
        let original: Value = read_json(&backup).unwrap();
        assert_eq!(
            original["local_apps"][0]["connectorId"],
            "com.example.legacy"
        );
    }

    #[test]
    fn data_and_packages_move_once_and_superseded_data_is_archived() {
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
            .join("migration-backups/unified-app-id")
            .join(ARTIFACT_VERSION)
            .join("connectors")
            .join(&entry.legacy_identity)
            .join("install.json")
            .is_file());

        fs::create_dir_all(&source_data).unwrap();
        fs::write(source_data.join("legacy-session.json"), b"older-state").unwrap();
        move_app_data(dir.path(), &entry).unwrap();
        assert_eq!(
            fs::read(
                dir.path()
                    .join("migration-backups/unified-app-id")
                    .join(ARTIFACT_VERSION)
                    .join("superseded/connector-data")
                    .join(&entry.legacy_identity)
                    .join("legacy-session.json")
            )
            .unwrap(),
            b"older-state"
        );
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
            false,
            &dir.path().join("managed-apps"),
        )
        .unwrap();

        assert!(!dir.path().join(LEDGER_FILE).exists());
        assert!(!dir.path().join(LOCK_FILE).exists());
    }

    #[test]
    fn offline_migration_repairs_legacy_config_after_installer_upgrade() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("agent-config.json");
        write_legacy_config(&config_path, &["com.example.legacy"]);
        write_install_record(dir.path(), "com.example.legacy", Some("registered-app-01"));
        let legacy_data = dir.path().join("connector-data").join("com.example.legacy");
        fs::create_dir_all(&legacy_data).unwrap();
        fs::write(legacy_data.join("session.json"), b"signed-in-state").unwrap();

        migrate(
            dir.path(),
            &config_path,
            false,
            true,
            &dir.path().join("managed-apps"),
        )
        .unwrap();

        let migrated: Value = read_json(&config_path).unwrap();
        assert_eq!(migrated["local_apps"], json!([]));
        assert!(!dir.path().join("connectors/com.example.legacy").exists());
        assert!(dir
            .path()
            .join("migration-backups/unified-app-id")
            .join(ARTIFACT_VERSION)
            .join("connectors/com.example.legacy/install.json")
            .is_file());
        assert_eq!(
            fs::read(dir.path().join("app-data/registered-app-01/session.json")).unwrap(),
            b"signed-in-state"
        );
    }

    #[test]
    fn offline_migration_archives_unregistered_derived_state_and_clears_config() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("agent-config.json");
        write_legacy_config(&config_path, &["com.baijimu.connector.codex"]);
        let package = dir.path().join("connectors/com.baijimu.connector.codex");
        let data = dir
            .path()
            .join("connector-data/com.baijimu.connector.codex");
        fs::create_dir_all(&package).unwrap();
        fs::create_dir_all(&data).unwrap();
        fs::write(package.join("connector.json"), b"legacy-package").unwrap();
        fs::write(data.join("session.json"), b"signed-in-state").unwrap();

        migrate(
            dir.path(),
            &config_path,
            false,
            true,
            &dir.path().join("managed-apps"),
        )
        .unwrap();

        let migrated: Value = read_json(&config_path).unwrap();
        assert_eq!(migrated["local_apps"], json!([]));
        let backup_root = dir
            .path()
            .join("migration-backups/unified-app-id")
            .join(ARTIFACT_VERSION);
        assert!(backup_root.join("agent-config.json").is_file());
        assert_eq!(
            fs::read(
                backup_root
                    .join("unregistered/connectors/com.baijimu.connector.codex/connector.json")
            )
            .unwrap(),
            b"legacy-package"
        );
        assert_eq!(
            fs::read(
                backup_root
                    .join("unregistered/connector-data/com.baijimu.connector.codex/session.json")
            )
            .unwrap(),
            b"signed-in-state"
        );
        assert!(!dir.path().join(LEDGER_FILE).exists());
        assert!(!dir.path().join(LOCK_FILE).exists());
    }

    #[test]
    fn offline_migration_clears_stale_config_without_local_installation_files() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("agent-config.json");
        write_legacy_config(&config_path, &["com.baijimu.connector.codex"]);

        migrate(
            dir.path(),
            &config_path,
            false,
            true,
            &dir.path().join("managed-apps"),
        )
        .unwrap();

        let migrated: Value = read_json(&config_path).unwrap();
        assert_eq!(migrated["local_apps"], json!([]));
        let original: Value = read_json(
            &dir.path()
                .join("migration-backups/unified-app-id")
                .join(ARTIFACT_VERSION)
                .join("agent-config.json"),
        )
        .unwrap();
        assert_eq!(
            original["local_apps"][0]["connectorId"],
            "com.baijimu.connector.codex"
        );
    }

    #[test]
    fn current_artifact_resumes_previous_version_ledger() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("agent-config.json");
        write_legacy_config(&config_path, &["com.example.legacy"]);
        write_json_atomically(
            &dir.path().join(LEDGER_FILE),
            &MigrationLedger {
                artifact_version: PREVIOUS_ARTIFACT_VERSION.to_string(),
                legacy_apps_stopped: true,
                entries: vec![MigrationEntry {
                    legacy_identity: "com.example.legacy".to_string(),
                    app_id: "registered-app-01".to_string(),
                    version: "1.0.0".to_string(),
                    phase: MigrationPhase::PackagesArchived,
                }],
            },
        )
        .unwrap();

        migrate(
            dir.path(),
            &config_path,
            false,
            true,
            &dir.path().join("managed-apps"),
        )
        .unwrap();

        let migrated: Value = read_json(&config_path).unwrap();
        assert_eq!(migrated["local_apps"], json!([]));
        assert!(!dir.path().join(LEDGER_FILE).exists());
    }

    #[test]
    fn offline_migration_refuses_to_mutate_while_the_legacy_host_is_running() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("local-app-control.json"),
            serde_json::to_vec_pretty(&json!({
                "schemaVersion": 1,
                "pid": std::process::id(),
                "baseUrl": "http://127.0.0.1:1",
                "token": "test-token",
                "startedAtEpochMs": 1
            }))
            .unwrap(),
        )
        .unwrap();
        let ledger = MigrationLedger {
            artifact_version: ARTIFACT_VERSION.to_string(),
            legacy_apps_stopped: false,
            entries: vec![MigrationEntry {
                legacy_identity: "com.example.legacy".to_string(),
                app_id: "registered-app-01".to_string(),
                version: "1.0.0".to_string(),
                phase: MigrationPhase::Prepared,
            }],
        };

        let error = verify_legacy_host_and_apps_stopped(dir.path(), &ledger).unwrap_err();
        assert!(error.to_string().contains("is still running"));
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

    #[test]
    fn managed_cli_conflict_preserves_current_target_and_archives_legacy_source() {
        let dir = tempdir().unwrap();
        let legacy = dir.path().join(LEGACY_MANAGED_CLI_APP_ID);
        let current = dir.path().join(MANAGED_CLI_APP_ID);
        fs::create_dir_all(&legacy).unwrap();
        fs::create_dir_all(&current).unwrap();
        fs::write(legacy.join("state.json"), b"legacy-state").unwrap();
        fs::write(current.join("state.json"), b"current-state").unwrap();

        migrate_managed_cli_app_id_at(dir.path()).unwrap();

        assert_eq!(
            fs::read(current.join("state.json")).unwrap(),
            b"current-state"
        );
        assert_eq!(
            fs::read(
                dir.path()
                    .join("migration-backups/unified-app-id")
                    .join(ARTIFACT_VERSION)
                    .join(LEGACY_MANAGED_CLI_APP_ID)
                    .join("state.json")
            )
            .unwrap(),
            b"legacy-state"
        );
    }
}

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

    stop_legacy_runtime_if_needed(
        config_dir,
        config_path,
        &ledger_path,
        &mut ledger,
        leave_host_running,
        host_already_stopped,
    )?;
    move_prepared_app_data(config_dir, &ledger_path, &mut ledger)?;
    archive_moved_packages(config_dir, &ledger_path, &mut ledger)?;
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

fn stop_legacy_runtime_if_needed(
    config_dir: &Path,
    config_path: &Path,
    ledger_path: &Path,
    ledger: &mut MigrationLedger,
    leave_host_running: bool,
    host_already_stopped: bool,
) -> Result<()> {
    let has_prepared_apps = ledger
        .entries
        .iter()
        .any(|entry| entry.phase == MigrationPhase::Prepared);
    if !has_prepared_apps
        && (ledger.legacy_apps_stopped || !legacy_state_exists(config_dir, config_path)?)
    {
        return Ok(());
    }
    if host_already_stopped {
        verify_legacy_host_and_apps_stopped(config_dir, ledger)?;
        for entry in &mut ledger.entries {
            if entry.phase == MigrationPhase::Prepared {
                entry.phase = MigrationPhase::AppsStopped;
            }
        }
        write_json_atomically(ledger_path, ledger)?;
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
            write_json_atomically(ledger_path, ledger)?;
        }
        if !leave_host_running {
            stop_legacy_bridge(&discovery)?;
        }
    }
    ledger.legacy_apps_stopped = true;
    write_json_atomically(ledger_path, ledger)
}

fn move_prepared_app_data(
    config_dir: &Path,
    ledger_path: &Path,
    ledger: &mut MigrationLedger,
) -> Result<()> {
    for index in 0..ledger.entries.len() {
        if ledger.entries[index].phase == MigrationPhase::AppsStopped {
            move_app_data(config_dir, &ledger.entries[index])?;
            ledger.entries[index].phase = MigrationPhase::DataMoved;
            write_json_atomically(ledger_path, ledger)?;
        }
    }
    Ok(())
}

fn archive_moved_packages(
    config_dir: &Path,
    ledger_path: &Path,
    ledger: &mut MigrationLedger,
) -> Result<()> {
    for index in 0..ledger.entries.len() {
        if ledger.entries[index].phase == MigrationPhase::DataMoved {
            archive_package(config_dir, &ledger.entries[index])?;
            ledger.entries[index].phase = MigrationPhase::PackagesArchived;
            write_json_atomically(ledger_path, ledger)?;
        }
    }
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

include!("main/file_migration.rs");
include!("main/process_control.rs");
include!("main/migration_state.rs");

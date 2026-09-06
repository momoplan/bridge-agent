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

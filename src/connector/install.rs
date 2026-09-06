#[cfg(test)]
pub fn install_connector_from_path(
    source: &Path,
    config_path: &Path,
    replace: bool,
) -> Result<ConnectorInstallResult> {
    install_connector_from_path_with_provenance(
        source,
        config_path,
        replace,
        ConnectorInstallProvenance::registered(
            &source.display().to_string(),
            "DRAFT",
            &"0".repeat(64),
        )?,
    )
}

#[cfg(test)]
pub fn install_connector_from_path_with_source_reference(
    source: &Path,
    config_path: &Path,
    replace: bool,
    source_reference: Option<&str>,
) -> Result<ConnectorInstallResult> {
    let source_reference = source_reference
        .map(str::to_string)
        .unwrap_or_else(|| source.display().to_string());
    install_connector_from_path_with_provenance(
        source,
        config_path,
        replace,
        ConnectorInstallProvenance::registered(&source_reference, "DRAFT", &"0".repeat(64))?,
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
    let original_config = config.clone();
    let source = source
        .canonicalize()
        .unwrap_or_else(|_| source.to_path_buf());
    let manifest = load_connector_manifest(&source)?;
    let registrations = load_connector_service_registrations(&source, &manifest)?;
    if registrations.is_empty() {
        bail!(
            "connector `{}` does not declare any services",
            manifest.app_id
        );
    }

    let mut services = registrations
        .iter()
        .cloned()
        .map(ServiceRegistration::into_connector_service_config)
        .collect::<Result<Vec<_>>>()?;

    let package_path = installed_connector_package_path(&manifest)?;
    if source == package_path || source.starts_with(&package_path) {
        bail!(
            "connector source {} cannot be inside its installed package {}",
            source.display(),
            package_path.display()
        );
    }
    if package_path.exists() && !replace {
        bail!(
            "connector package already exists at {}; pass replace to overwrite it",
            package_path.display()
        );
    }
    let previous_record = load_install_record(&manifest.app_id).ok();
    if package_path.exists() && previous_record.is_some() {
        stop_connector_for_package_change(&manifest.app_id, &config)?;
    }
    let replaced_package = if package_path.exists() {
        Some(prepare_connector_package_destination(
            &package_path,
            replace,
        )?)
    } else {
        None
    };
    let mut config_saved = false;
    let install_result = complete_connector_install(
        &source,
        &package_path,
        config_path,
        replace,
        &manifest,
        &registrations,
        &mut services,
        &mut config,
        previous_record.as_ref(),
        &provenance,
        &mut config_saved,
    );

    finalize_connector_install(
        install_result,
        config_path,
        &original_config,
        &package_path,
        replaced_package,
        config_saved,
        &manifest.app_id,
    )
}

#[allow(clippy::too_many_arguments)]
fn finalize_connector_install(
    install_result: Result<ConnectorInstallResult>,
    config_path: &Path,
    original_config: &AgentConfig,
    package_path: &Path,
    replaced_package: Option<PathBuf>,
    config_saved: bool,
    app_id: &str,
) -> Result<ConnectorInstallResult> {
    match install_result {
        Ok(result) => {
            if let Some(replaced_package) = replaced_package {
                if let Err(err) = fs::remove_dir_all(&replaced_package) {
                    tracing::warn!(
                        "installed connector {} but failed to remove replaced package {}: {err:#}",
                        app_id,
                        replaced_package.display()
                    );
                }
            }
            Ok(result)
        }
        Err(install_err) => {
            let mut rollback_failures = Vec::new();
            if config_saved {
                if let Err(err) = save_config(config_path, original_config) {
                    rollback_failures.push(format!("restore connector config: {err:#}"));
                }
            }
            if let Err(err) =
                restore_replaced_connector_package(package_path, replaced_package.as_deref())
            {
                rollback_failures.push(format!("restore connector package: {err:#}"));
            }
            if rollback_failures.is_empty() {
                Err(install_err)
            } else {
                bail!(
                    "{install_err:#}; connector replacement rollback also failed: {}",
                    rollback_failures.join("; ")
                )
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn complete_connector_install(
    source: &Path,
    package_path: &Path,
    config_path: &Path,
    replace: bool,
    manifest: &ConnectorManifest,
    registrations: &[ServiceRegistration],
    services: &mut [ServiceConfig],
    config: &mut AgentConfig,
    previous_record: Option<&ConnectorInstallRecord>,
    provenance: &ConnectorInstallProvenance,
    config_saved: &mut bool,
) -> Result<ConnectorInstallResult> {
    copy_connector_package(source, package_path)?;
    let package_checksum = connector_package_sha256(package_path)?;
    resolve_installed_start_commands(
        services,
        package_path,
        &config.runtime,
        manifest,
        ConnectorRuntimePreparation::Deferred,
    )?;
    cleanup_legacy_connector_autostarts_for_manifest(manifest);

    let local_app = local_app_from_services(manifest, services, registrations)?;
    let method_names = local_app.methods.iter().map(|method| method.name.clone()).collect();
    let event_names = local_app.events.iter().map(|event| event.name.clone()).collect();
    upsert_local_app(&mut config.local_apps, local_app, replace)?;
    save_config(config_path, config)?;
    *config_saved = true;

    let now = now_ms();
    save_install_record(&ConnectorInstallRecord {
        manifest: manifest.clone(),
        package_path: package_path.display().to_string(),
        source_path: source.display().to_string(),
        source_reference: provenance.source_reference.clone(),
        review_status: provenance.review_status.clone(),
        source_checksum: Some(provenance.source_checksum.clone()),
        package_checksum: Some(package_checksum),
        installed_at_epoch_ms: previous_record
            .map(|record| record.installed_at_epoch_ms)
            .unwrap_or(now),
        last_synced_at_epoch_ms: now,
    })?;

    Ok(ConnectorInstallResult {
        app_id: manifest.app_id.clone(),
        name: manifest.name.clone(),
        version: manifest.version.clone(),
        package_path: package_path.display().to_string(),
        method_names,
        event_names,
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ConnectorUninstallOptions {
    pub force: bool,
}

#[derive(Debug)]
pub struct ConnectorPackageStopError {
    message: String,
}

impl std::fmt::Display for ConnectorPackageStopError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ConnectorPackageStopError {}

pub fn is_connector_package_stop_error(error: &anyhow::Error) -> bool {
    error
        .chain()
        .any(|cause| cause.is::<ConnectorPackageStopError>())
}

pub fn uninstall_connector(app_id: &str, config_path: &Path) -> Result<ConnectorSummary> {
    uninstall_connector_with_options(app_id, config_path, ConnectorUninstallOptions::default())
}

pub fn uninstall_connector_with_options(
    app_id: &str,
    config_path: &Path,
    options: ConnectorUninstallOptions,
) -> Result<ConnectorSummary> {
    ensure_config_exists(config_path)?;
    let record = load_install_record(app_id)?;
    let summary = summary_from_record(record.clone());
    let mut config = load_config(config_path)?;
    match stop_connector_for_package_change(app_id, &config) {
        Ok(()) => release_connector_package_processes(Path::new(&record.package_path))?,
        Err(stop_error) if options.force => {
            if let Err(recovery_error) =
                force_release_connector_package_processes(Path::new(&record.package_path))
            {
                bail!(
                    "forced uninstall could not recover after graceful stop failed: {stop_error:#}; host process recovery failed: {recovery_error:#}"
                );
            }
        }
        Err(stop_error) => return Err(stop_error),
    }
    cleanup_legacy_connector_autostarts_for_manifest(&record.manifest);

    config
        .local_apps
        .retain(|app| app.app_id != record.manifest.app_id);

    let install_root = install_record_path(app_id)?
        .parent()
        .with_context(|| format!("connector `{app_id}` install root is missing"))?
        .to_path_buf();
    let uninstalling_root = if install_root.exists() {
        Some(quarantine_connector_install_root(&install_root)?)
    } else {
        None
    };
    if let Err(config_err) = save_config(config_path, &config) {
        if let Some(uninstalling_root) = uninstalling_root.as_deref() {
            if let Err(restore_err) = fs::rename(uninstalling_root, &install_root) {
                bail!(
                    "failed to update config while uninstalling connector `{app_id}`: {config_err:#}; failed to restore installation {}: {restore_err}",
                    install_root.display()
                );
            }
        }
        return Err(config_err);
    }
    if let Some(uninstalling_root) = uninstalling_root {
        if let Err(err) = fs::remove_dir_all(&uninstalling_root) {
            tracing::warn!(
                "uninstalled connector {app_id} but failed to remove quarantined installation {}: {err:#}",
                uninstalling_root.display()
            );
        }
    }

    Ok(summary)
}

pub fn list_connectors() -> Result<Vec<ConnectorSummary>> {
    let mut connectors = load_install_records()?
        .into_iter()
        .map(summary_from_record)
        .collect::<Vec<_>>();
    connectors.sort_by(|left, right| left.app_id.cmp(&right.app_id));
    Ok(connectors)
}

pub fn show_connector(app_id: &str) -> Result<ConnectorInstallRecord> {
    load_install_record(app_id)
}

pub fn sync_installed_connectors(config_path: &Path) -> Result<Vec<ConnectorSummary>> {
    let report = sync_installed_connectors_report(config_path)?;
    if !report.failures.is_empty() {
        bail!("{}", format_connector_sync_failures(&report.failures));
    }
    Ok(report.summaries)
}

pub fn sync_installed_connector(config_path: &Path, app_id: &str) -> Result<ConnectorSummary> {
    ensure_config_exists(config_path)?;
    let mut config = load_config(config_path)?;
    let record = load_install_record(app_id)?;
    let summary = sync_installed_connector_record(
        &mut config,
        record,
        now_ms(),
        ConnectorRuntimePreparation::Deferred,
    )?;
    save_config(config_path, &config)?;
    Ok(summary)
}

pub fn prepare_installed_connector_runtime(
    config_path: &Path,
    app_id: &str,
) -> Result<ConnectorSummary> {
    ensure_config_exists(config_path)?;
    let mut config = load_config(config_path)?;
    let record = load_install_record(app_id)?;
    let summary = sync_installed_connector_record(
        &mut config,
        record,
        now_ms(),
        ConnectorRuntimePreparation::Prepare,
    )?;
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
        let app_id = record.manifest.app_id.clone();
        let name = record.manifest.name.clone();
        match sync_installed_connector_record(
            &mut config,
            record,
            now,
            ConnectorRuntimePreparation::Deferred,
        ) {
            Ok(summary) => summaries.push(summary),
            Err(err) => {
                let error = format!("{err:#}");
                tracing::warn!(
                    app_id = %app_id,
                    name = %name,
                    error = %error,
                    "failed to sync installed connector"
                );
                failures.push(ConnectorSyncFailure {
                    app_id,
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
                failure.name, failure.app_id, failure.error
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
    runtime_preparation: ConnectorRuntimePreparation,
) -> Result<ConnectorSummary> {
    cleanup_legacy_connector_autostarts_for_manifest(&record.manifest);
    let package_path = PathBuf::from(&record.package_path);
    let app_id = record.manifest.app_id.clone();
    let registrations = load_connector_service_registrations(&package_path, &record.manifest)
        .with_context(|| {
            format!("failed to reload service registrations for connector `{app_id}`")
        })?;
    let mut services = registrations
        .iter()
        .cloned()
        .map(ServiceRegistration::into_connector_service_config)
        .collect::<Result<Vec<_>>>()?;
    resolve_installed_start_commands(
        &mut services,
        &package_path,
        &config.runtime,
        &record.manifest,
        runtime_preparation,
    )?;

    let local_app = local_app_from_services(&record.manifest, &services, &registrations)?;
    upsert_synced_local_app(&mut config.local_apps, local_app);

    record.last_synced_at_epoch_ms = now;
    save_install_record(&record)?;
    Ok(summary_from_record(record))
}

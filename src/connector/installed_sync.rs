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
    crate::config::initialize_config(config_path)?;
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
    crate::config::initialize_config(config_path)?;
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
    crate::config::initialize_config(config_path)?;
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
        let app_id = record.installation_id();
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
    if record.bundle_source.is_none() {
        cleanup_legacy_connector_autostarts_for_manifest(&record.manifest);
    }
    let package_path = PathBuf::from(&record.package_path);
    let app_id = record.installation_id();
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
        &app_id,
        runtime_preparation,
    )?;

    let mut local_app = local_app_from_services(&record.manifest, &services, &registrations)?;
    local_app.app_id = app_id;
    upsert_synced_local_app(&mut config.local_apps, local_app);

    record.last_synced_at_epoch_ms = now;
    save_install_record(&record)?;
    Ok(summary_from_record(record))
}

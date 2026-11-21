fn connector_manifest_path(source: &Path) -> PathBuf {
    if source.is_file() {
        source.to_path_buf()
    } else {
        source.join(CONNECTOR_MANIFEST_FILE)
    }
}

fn load_connector_service_registrations(
    _source: &Path,
    manifest: &ConnectorManifest,
) -> Result<Vec<ServiceRegistration>> {
    let runtime = manifest
        .runtime
        .as_ref()
        .context("connector schemaVersion 3.0.0 requires runtime")?;
    let command = runtime
        .command
        .as_deref()
        .map(str::trim)
        .filter(|command| !command.is_empty())
        .context("connector runtime.command cannot be empty")?;
    let start_command = Some(ServiceStartCommand::ShellCommand {
        command: std::iter::once(command.to_string())
            .chain(runtime.args.iter().cloned())
            .collect(),
        cwd: None,
        env: runtime.env.clone(),
        timeout_secs: None,
    });
    let stop_command = if runtime.stop_args.is_empty() {
        derive_stop_command_from_start(start_command.as_ref())
    } else {
        Some(ServiceStartCommand::ShellCommand {
            command: std::iter::once(command.to_string())
                .chain(runtime.stop_args.iter().cloned())
                .collect(),
            cwd: None,
            env: runtime.env.clone(),
            timeout_secs: None,
        })
    };
    Ok(vec![ServiceRegistration {
        name: manifest.app_id.clone(),
        description: manifest.description.clone(),
        enabled: true,
        transport: manifest
            .transport
            .clone()
            .context("connector schemaVersion 3.0.0 requires transport")?,
        health_check: runtime.health_check.clone(),
        start_command,
        stop_command,
        methods: manifest.methods.clone(),
        local_app_events: manifest.events.clone(),
        replace: true,
        managed_by: Some(manifest.app_id.clone()),
    }])
}

fn installed_connector_package_path(manifest: &ConnectorManifest) -> Result<PathBuf> {
    Ok(connectors_dir()?
        .join(sanitize_path_component(&manifest.app_id))
        .join("package"))
}

fn install_record_path(app_id: &str) -> Result<PathBuf> {
    Ok(connectors_dir()?
        .join(sanitize_path_component(app_id))
        .join(CONNECTOR_INSTALL_RECORD_FILE))
}

fn save_install_record(record: &ConnectorInstallRecord) -> Result<()> {
    let path = install_record_path(&record.manifest.app_id)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create connector dir {}", parent.display()))?;
    }
    let temporary_path = path.with_file_name(format!(
        ".{CONNECTOR_INSTALL_RECORD_FILE}.{}.tmp",
        Uuid::new_v4()
    ));
    let backup_path = path.with_file_name(format!(
        ".{CONNECTOR_INSTALL_RECORD_FILE}.{}.backup",
        Uuid::new_v4()
    ));
    let write_result = fs::write(
        &temporary_path,
        format!("{}\n", serde_json::to_string_pretty(record)?),
    )
    .with_context(|| {
        format!(
            "failed to write temporary connector install record {}",
            temporary_path.display()
        )
    });
    if let Err(err) = write_result {
        let _ = fs::remove_file(&temporary_path);
        return Err(err);
    }
    let had_previous = path.exists();
    if had_previous {
        if let Err(err) = fs::rename(&path, &backup_path) {
            let _ = fs::remove_file(&temporary_path);
            return Err(err).with_context(|| {
                format!(
                    "failed to preserve connector install record {}",
                    path.display()
                )
            });
        }
    }
    if let Err(err) = fs::rename(&temporary_path, &path) {
        let _ = fs::remove_file(&temporary_path);
        if had_previous {
            let _ = fs::rename(&backup_path, &path);
        }
        return Err(err).with_context(|| {
            format!(
                "failed to replace connector install record {}",
                path.display()
            )
        });
    }
    if had_previous {
        if let Err(err) = fs::remove_file(&backup_path) {
            tracing::warn!(
                "saved connector install record {} but failed to remove backup {}: {err:#}",
                path.display(),
                backup_path.display()
            );
        }
    }
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
        if id.starts_with('.') && id.contains(".uninstalling-") {
            continue;
        }
        match load_install_record(&id) {
            Ok(record) => records.push(record),
            Err(err) => tracing::warn!("failed to load connector `{id}`: {err:#}"),
        }
    }
    records.sort_by(|left, right| left.manifest.app_id.cmp(&right.manifest.app_id));
    Ok(records)
}

fn load_install_record(app_id: &str) -> Result<ConnectorInstallRecord> {
    read_install_record(app_id)
}

fn read_install_record<T: serde::de::DeserializeOwned>(app_id: &str) -> Result<T> {
    let path = install_record_path(app_id)?;
    let content = fs::read_to_string(&path)
        .with_context(|| format!("failed to read connector install record {}", path.display()))?;
    serde_json::from_str(&content).with_context(|| {
        format!(
            "failed to parse connector install record {}",
            path.display()
        )
    })
}

/// Attest the actual callable targets and events in the host registry against the
/// installed manifest. Hand-configured apps do not have an installation identity.
pub(crate) fn registered_source(app: &LocalAppConfig) -> Result<local_app_contract::SourceVersion> {
    let record = read_install_record::<ConnectorInstallRecord>(&app.app_id)?;
    check_registered_source(app, &record)
}

fn check_registered_source(app: &LocalAppConfig, record: &ConnectorInstallRecord) -> Result<local_app_contract::SourceVersion> {
    let source = crate::market_distribution::installed_source_version(record.install_source.as_ref())?;
    if record.manifest.app_id != app.app_id || record.manifest.version != app.version
        || source.version.to_string() != app.version {
        bail!("installed connector does not match the registered app and version");
    }
    let registrations = load_connector_service_registrations(Path::new(&record.package_path), &record.manifest)?;
    let services = registrations.iter().cloned().map(ServiceRegistration::into_connector_service_config)
        .collect::<Result<Vec<_>>>()?;
    let installed = local_app_from_services(&record.manifest, &services, &registrations)?;
    if serde_json::to_value(installed.methods)? != serde_json::to_value(&app.methods)?
        || serde_json::to_value(installed.events)? != serde_json::to_value(&app.events)? {
        bail!("registered targets differ from the installed connector");
    }
    Ok(source.clone())
}

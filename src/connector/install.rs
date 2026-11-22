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

// Replacement needs the installed identity even when the old manifest can no
// longer be executed by this host. The incoming package is fully validated.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct InstalledConnectorIdentity {
    manifest: InstalledAppId,
    bundle_source: Option<local_app_contract::bundle::LocatedAppVersion>,
    install_source: Option<local_app_contract::InstallSource>,
    installed_at_epoch_ms: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct InstalledAppId {
    app_id: String,
}

pub fn install_connector_from_path_with_provenance(
    source: &Path,
    config_path: &Path,
    replace: bool,
    provenance: ConnectorInstallProvenance,
) -> Result<ConnectorInstallResult> {
    crate::config::initialize_config(config_path)?;
    let mut config = load_config(config_path)?;
    let original_config = config.clone();
    let source = source
        .canonicalize()
        .unwrap_or_else(|_| source.to_path_buf());
    let manifest = load_connector_manifest(&source)?;
    provenance.validate_manifest(&manifest)?;
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

    let installation_id = provenance.installation_id(&manifest.app_id);
    if provenance.bundle_source.is_some() {
        let candidate = local_app_from_services(&manifest, &services, &registrations)?;
        let legacy_id = provenance
            .adopted_legacy
            .as_ref()
            .map(|source| source.application.app_id.as_str());
        validate_bundle_transport(&config.local_apps, &installation_id, legacy_id, &candidate)?;
    }
    let package_path = installed_connector_package_path(&installation_id)?;
    let previous_record = previous_connector_installation(&source, &package_path, &installation_id, &manifest.app_id, replace)?;
    validate_install_replacement(previous_record.as_ref(), &provenance)?;

    if package_path.exists() && previous_record.is_some() {
        stop_connector_for_package_change(&installation_id, &config)?;
    }
    let adoption = provenance
        .adopted_legacy
        .as_ref()
        .map(|legacy| {
            ConnectorAdoption::start(legacy, &installation_id, &original_config)
        })
        .transpose()?;
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
        &installation_id,
        &mut config_saved,
    );

    let result = finalize_connector_install(
        install_result,
        config_path,
        &original_config,
        &package_path,
        replaced_package,
        config_saved,
        &installation_id,
    );
    finish_connector_adoption(adoption, result, config_path, &original_config)
}

fn previous_connector_installation(
    source: &Path,
    package_path: &Path,
    installation_id: &str,
    app_id: &str,
    replace: bool,
) -> Result<Option<InstalledConnectorIdentity>> {
    if source == package_path || source.starts_with(package_path) {
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
    Ok(match read_install_record::<InstalledConnectorIdentity>(installation_id)
    {
        Ok(record) => {
            if record.manifest.app_id != app_id {
                bail!("stored connector installation identity does not match the package");
            }
            if !replace {
                bail!("connector install record already exists; pass replace to reinstall");
            }
            Some(record)
        }
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|cause| cause.kind() == std::io::ErrorKind::NotFound)
                && !package_path.try_exists()? =>
        {
            None
        }
        Err(error) => return Err(error.context("cannot establish installed connector provenance")),
    })
}

fn finish_connector_adoption(
    adoption: Option<ConnectorAdoption>,
    result: Result<ConnectorInstallResult>,
    config_path: &Path,
    original_config: &AgentConfig,
) -> Result<ConnectorInstallResult> {
    if let Some(adoption) = adoption {
        match result {
            Ok(install) => {
                adoption.finish(config_path, original_config)?;
                return Ok(install);
            }
            Err(error) => {
                adoption
                    .rollback()
                    .context(format!("adoption failed: {error:#}"))?;
                return Err(error);
            }
        }
    }
    result
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
    previous_record: Option<&InstalledConnectorIdentity>,
    provenance: &ConnectorInstallProvenance,
    installation_id: &str,
    config_saved: &mut bool,
) -> Result<ConnectorInstallResult> {
    copy_connector_package(source, package_path)?;
    resolve_installed_start_commands(
        services,
        package_path,
        &config.runtime,
        manifest,
        installation_id,
        ConnectorRuntimePreparation::Deferred,
    )?;
    if provenance.bundle_source.is_none() {
        cleanup_legacy_connector_autostarts_for_manifest(manifest);
    }

    let mut local_app = local_app_from_services(manifest, services, registrations)?;
    local_app.app_id = installation_id.to_owned();
    if let Some(legacy) = &provenance.adopted_legacy {
        let legacy_id = legacy.application.app_id.as_str();
        if let Some(previous) = config.local_apps.iter().find(|app| app.app_id == legacy_id) {
            local_app.enabled = previous.enabled;
        }
        config.local_apps.retain(|app| app.app_id != legacy_id);
    }
    let method_names = local_app
        .methods
        .iter()
        .map(|method| method.name.clone())
        .collect();
    let event_names = local_app
        .events
        .iter()
        .map(|event| event.name.clone())
        .collect();
    upsert_local_app(&mut config.local_apps, local_app, replace)?;
    save_config(config_path, config)?;
    *config_saved = true;

    let now = now_ms();
    save_install_record(&ConnectorInstallRecord {
        bundle_source: provenance.bundle_source.clone(),
        install_source: provenance.install_source.clone(),
        manifest: manifest.clone(),
        package_path: package_path.display().to_string(),
        source_path: source.display().to_string(),
        source_reference: provenance.source_reference.clone(),
        review_status: provenance.review_status.clone(),
        source_checksum: provenance.source_checksum.clone(),
        package_checksum: None,
        installed_at_epoch_ms: previous_record
            .map(|record| record.installed_at_epoch_ms)
            .unwrap_or(now),
        last_synced_at_epoch_ms: now,
    })?;
    if provenance.source_identity_migration
        && previous_record.is_some_and(|record| record.install_source.is_none())
    {
        if let Some(selection) = provenance.install_source.as_ref() {
            let source = match selection {
                local_app_contract::InstallSource::Market { source, .. }
                | local_app_contract::InstallSource::Environment { source } => source,
            };
            tracing::info!(
                app_id = %source.application.app_id.as_str(),
                environment_key = %source.application.environment_key.as_str(),
                "migrated legacy connector installation to an explicit source application identity"
            );
        }
    }

    Ok(ConnectorInstallResult {
        app_id: installation_id.to_owned(),
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
    crate::config::initialize_config(config_path)?;
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

    config.local_apps.retain(|app| app.app_id != app_id);

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

fn validate_install_replacement(
    previous_record: Option<&InstalledConnectorIdentity>,
    provenance: &ConnectorInstallProvenance,
) -> Result<()> {
    if let Some(previous) = previous_record {
        if previous
            .bundle_source
            .as_ref()
            .map(|source| &source.resource_locator)
            != provenance
                .bundle_source
                .as_ref()
                .map(|source| &source.resource_locator)
        {
            bail!("different source applications cannot replace the same installation");
        }
    }
    if previous_record.is_some_and(|record| record.install_source.is_some())
        && provenance.install_source.is_none()
    {
        bail!("a verified installation cannot be replaced by a source without provenance");
    }

    if let Some(selected) = provenance.install_source.as_ref() {
        if let Some(previous) = previous_record {
            let application = |identity: &local_app_contract::InstallSource| match identity {
                local_app_contract::InstallSource::Market { source, .. }
                | local_app_contract::InstallSource::Environment { source } => {
                    source.application.clone()
                }
            };
            match previous.install_source.as_ref() {
                Some(existing) if application(existing) != application(selected) => {
                    bail!("different source applications cannot replace the same installation");
                }
                Some(_) => {}
                None if provenance.source_identity_migration => {
                    if application(selected).app_id.as_str() != previous.manifest.app_id {
                        bail!("source identity migration does not match the installed application");
                    }
                }
                None => {
                    bail!(
                        "existing installation has no verified source identity; explicit source identity migration is required before upgrade"
                    );
                }
            }
        }
    }

    Ok(())
}

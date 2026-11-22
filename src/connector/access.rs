pub fn connectors_dir() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("BRIDGE_AGENT_LOCAL_APPS_DIR") {
        return Ok(PathBuf::from(path));
    }
    if let Some(dirs) = ProjectDirs::from("com", "baijimu", "bridge-agent") {
        return Ok(dirs.config_dir().join("local-apps"));
    }
    bail!("failed to resolve BridgeAgent config directory")
}

pub fn connector_data_dir(app_id: &str) -> Result<PathBuf> {
    let path_component = installation_storage_path(app_id)?;
    if let Some(path) = std::env::var_os("BRIDGE_AGENT_LOCAL_APP_DATA_DIR") {
        return Ok(PathBuf::from(path).join(&path_component));
    }
    if let Some(dirs) = ProjectDirs::from("com", "baijimu", "bridge-agent") {
        return Ok(dirs.config_dir().join("app-data").join(path_component));
    }
    bail!("failed to resolve BridgeAgent connector data directory")
}

pub fn connector_management_token_path(app_id: &str) -> Result<PathBuf> {
    Ok(connector_data_dir(app_id)?.join(CONNECTOR_MANAGEMENT_TOKEN_FILE))
}

pub fn connector_event_token_path(app_id: &str) -> Result<PathBuf> {
    Ok(connector_data_dir(app_id)?.join(CONNECTOR_EVENT_TOKEN_FILE))
}

pub fn connector_asset_upload_token_path(app_id: &str) -> Result<PathBuf> {
    Ok(connector_data_dir(app_id)?.join(CONNECTOR_ASSET_UPLOAD_TOKEN_FILE))
}

pub(crate) fn connector_declares_asset_upload(app_id: &str) -> Result<bool> {
    let record = load_install_record(app_id)?;
    Ok(connector_permission_is_active(
        &record.manifest,
        CONNECTOR_ASSET_UPLOAD_PERMISSION,
    ))
}

pub fn authorize_connector_asset_upload(
    config_path: &Path,
    app_id: &str,
    token: &str,
) -> Result<PathBuf> {
    let record = connector_with_token(app_id, token, connector_asset_upload_token_path)?;
    let installation_id = record.installation_id();
    if !connector_permission_is_active(&record.manifest, CONNECTOR_ASSET_UPLOAD_PERMISSION) {
        bail!("connector `{app_id}` does not declare asset upload permission");
    }
    let config = load_config(config_path)?;
    if !config
        .local_apps
        .iter()
        .any(|app| app.app_id == installation_id && app.enabled)
    {
        bail!("connector `{app_id}` is not enabled");
    }
    connector_data_dir(&installation_id)
}

fn connector_permission_is_active(manifest: &ConnectorManifest, permission_id: &str) -> bool {
    manifest.permissions.iter().any(|permission| {
        permission.id == permission_id
            && (permission.platforms.is_empty()
                || permission
                    .platforms
                    .iter()
                    .any(|platform| platform == std::env::consts::OS))
    })
}

pub fn authorize_connector_event(
    config_path: &Path,
    app_id: &str,
    event_name: &str,
    token: &str,
) -> Result<()> {
    authorized_event_source(config_path, app_id, event_name, token).map(|_| ())
}

pub(crate) fn authorized_event_source(
    config_path: &Path,
    app_id: &str,
    event_name: &str,
    token: &str,
) -> Result<(
    String,
    Option<relay::contracts::device_events::InstalledLocalAppSource>,
)> {
    let record = connector_with_token(app_id, token, connector_event_token_path)?;
    let installation_id = record.installation_id();
    let config = load_config(config_path)?;
    let event_name = event_name.trim();
    let app = config
        .local_apps
        .iter()
        .find(|app| {
            app.app_id == installation_id
                && app.enabled
                && app
                    .events
                    .iter()
                    .any(|event| event.enabled && event.name == event_name)
        })
        .with_context(|| {
            format!("local app event `{event_name}` is not declared by connector `{app_id}`")
        })?;
    let source = if record.bundle_source.is_some() {
        Some(check_registered_source(app, &record)?)
    } else {
        check_registered_source(app, &record).ok()
    };
    Ok((installation_id, source))
}

fn connector_with_token(
    app_id: &str,
    token: &str,
    token_path: fn(&str) -> Result<PathBuf>,
) -> Result<ConnectorInstallRecord> {
    let mut selected = None;
    for record in load_install_records()? {
        let id = record.installation_id();
        if id != app_id && record.manifest.app_id != app_id {
            continue;
        }
        let path = token_path(&id)?;
        let expected = match fs::read_to_string(path) {
            Ok(value) => value,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        if constant_time_text_eq(expected.trim(), token.trim()) {
            if selected.is_some() {
                bail!("ambiguous connector credential");
            }
            selected = Some(record);
        }
    }
    selected.context("invalid connector credential")
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
    if let Some(ui) = manifest.ui.as_ref() {
        resolve_connector_ui_entry(source, ui)?;
    }
    Ok(manifest)
}

pub fn resolve_connector_ui_entry(package_path: &Path, ui: &ConnectorUi) -> Result<PathBuf> {
    resolve_connector_ui_asset(package_path, ui, None)
}

pub fn resolve_connector_ui_asset(
    package_path: &Path,
    ui: &ConnectorUi,
    asset_path: Option<&str>,
) -> Result<PathBuf> {
    let package_root = package_path.canonicalize().with_context(|| {
        format!(
            "failed to resolve connector package {}",
            package_path.display()
        )
    })?;
    if !package_root.is_dir() {
        bail!("connector UI requires a package directory");
    }

    let entry_relative = validated_connector_ui_relative_path(&ui.entry, "entry")?;
    let entry = package_root.join(&entry_relative);
    let ui_root = entry
        .parent()
        .with_context(|| "connector UI entry must have a parent directory")?
        .canonicalize()
        .with_context(|| format!("failed to resolve connector UI root {}", entry.display()))?;
    if !ui_root.starts_with(&package_root) {
        bail!("connector UI root escapes the connector package");
    }

    let candidate = match asset_path {
        Some(path) if !path.trim().is_empty() => {
            ui_root.join(validated_connector_ui_relative_path(path, "asset path")?)
        }
        _ => entry,
    };
    let resolved = candidate.canonicalize().with_context(|| {
        format!(
            "failed to resolve connector UI asset {}",
            candidate.display()
        )
    })?;
    if !resolved.starts_with(&ui_root) || !resolved.is_file() {
        bail!("connector UI asset is outside the declared UI directory");
    }
    Ok(resolved)
}

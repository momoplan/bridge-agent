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
    validate_app_id(app_id)?;
    if let Some(path) = std::env::var_os("BRIDGE_AGENT_LOCAL_APP_DATA_DIR") {
        return Ok(PathBuf::from(path).join(app_id));
    }
    if let Some(dirs) = ProjectDirs::from("com", "baijimu", "bridge-agent") {
        return Ok(dirs.config_dir().join("app-data").join(app_id));
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
    let record = load_install_record(app_id)?;
    if !connector_permission_is_active(&record.manifest, CONNECTOR_ASSET_UPLOAD_PERMISSION) {
        bail!("connector `{app_id}` does not declare assets.upload permission");
    }
    let expected_token_path = connector_asset_upload_token_path(app_id)?;
    let expected_token = fs::read_to_string(&expected_token_path).with_context(|| {
        format!(
            "failed to read connector asset upload credential {}",
            expected_token_path.display()
        )
    })?;
    if !constant_time_text_eq(expected_token.trim(), token.trim()) {
        bail!("invalid connector asset upload credential");
    }
    let config = load_config(config_path)?;
    if !config
        .local_apps
        .iter()
        .any(|app| app.app_id == app_id && app.enabled)
    {
        bail!("connector `{app_id}` is not enabled");
    }
    connector_data_dir(app_id)
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
    load_install_record(app_id)?;
    let expected_token_path = connector_event_token_path(app_id)?;
    let expected_token = fs::read_to_string(&expected_token_path).with_context(|| {
        format!(
            "failed to read connector event credential {}",
            expected_token_path.display()
        )
    })?;
    if !constant_time_text_eq(expected_token.trim(), token.trim()) {
        bail!("invalid connector event credential");
    }

    let config = load_config(config_path)?;
    let event_name = event_name.trim();
    let declared = config
        .local_apps
        .iter()
        .find(|app| app.app_id == app_id && app.enabled)
        .is_some_and(|app| {
            app.events
                .iter()
                .any(|event| event.enabled && event.name == event_name)
        });
    if !declared {
        bail!("local app event `{event_name}` is not declared by connector `{app_id}`");
    }
    Ok(())
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

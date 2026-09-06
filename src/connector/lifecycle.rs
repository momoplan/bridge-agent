pub fn start_connector(app_id: &str, config_path: &Path) -> Result<ConnectorStartResult> {
    start_connector_with_env(app_id, config_path, &BTreeMap::new())
}

pub fn start_connector_with_env(
    app_id: &str,
    config_path: &Path,
    additional_env: &BTreeMap<String, String>,
) -> Result<ConnectorStartResult> {
    ensure_config_exists(config_path)?;
    prepare_installed_connector_runtime(config_path, app_id)?;
    let record = load_install_record(app_id)?;
    if record
        .manifest
        .runtime
        .as_ref()
        .is_some_and(|runtime| runtime.process_ownership == ConnectorProcessOwnership::Host)
    {
        bail!(
            "connector `{app_id}` declares runtime.processOwnership=host and must be started by the Bridge Agent desktop process supervisor"
        );
    }
    let config = load_config(config_path)?;
    let app = config.local_apps.iter().find(|app| app.app_id == app_id);
    let lifecycle = match app.and_then(|app| app.start_command.as_ref()) {
        Some(command) => run_start_command(app_id, command, additional_env)?,
        None => ConnectorLifecycleResult {
            app_id: app_id.to_string(),
            configured: false,
            exit_code: None,
            stdout: String::new(),
            stderr: "start command is not configured".to_string(),
        },
    };
    Ok(ConnectorStartResult {
        app_id: record.manifest.app_id,
        lifecycle,
    })
}

pub fn stop_connector(app_id: &str, config_path: &Path) -> Result<ConnectorStartResult> {
    ensure_config_exists(config_path)?;
    let record = load_install_record(app_id)?;
    let config = load_config(config_path)?;
    let app = config.local_apps.iter().find(|app| app.app_id == app_id);
    let lifecycle = match app.and_then(|app| app.stop_command.as_ref()) {
        Some(command) => run_start_command(app_id, command, &BTreeMap::new())?,
        None => ConnectorLifecycleResult {
            app_id: app_id.to_string(),
            configured: false,
            exit_code: None,
            stdout: String::new(),
            stderr: "stop command is not configured".to_string(),
        },
    };
    Ok(ConnectorStartResult {
        app_id: record.manifest.app_id,
        lifecycle,
    })
}

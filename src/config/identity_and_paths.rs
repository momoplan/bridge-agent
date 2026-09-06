pub fn ensure_browser_auth_agent_id(config: &mut AgentConfig) -> bool {
    if is_legacy_default_agent_id(&config.relay.agent_id) {
        config.relay.agent_id = generate_agent_id();
        return true;
    }
    false
}

fn generate_agent_id() -> String {
    format!("{GENERATED_AGENT_ID_PREFIX}{}", Uuid::new_v4().simple())
}

fn generate_registration_token() -> String {
    Uuid::new_v4().simple().to_string()
}

fn default_device_name() -> String {
    let username = first_env_value(&["BRIDGE_AGENT_DEVICE_USER", "USER", "USERNAME"]);
    let hostname = first_env_value(&[
        "BRIDGE_AGENT_DEVICE_HOST",
        "COMPUTERNAME",
        "HOSTNAME",
        "NAME",
    ]);
    format_default_device_name(username.as_deref(), hostname.as_deref())
}

fn first_env_value(names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| {
        env::var(name)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    })
}

fn format_default_device_name(username: Option<&str>, hostname: Option<&str>) -> String {
    match (
        username.map(str::trim).filter(|value| !value.is_empty()),
        hostname.map(str::trim).filter(|value| !value.is_empty()),
    ) {
        (Some(user), Some(host)) if !user.eq_ignore_ascii_case(host) => format!("{user}@{host}"),
        (Some(user), _) => user.to_string(),
        (_, Some(host)) => host.to_string(),
        _ => LEGACY_DEFAULT_DEVICE_NAME.to_string(),
    }
}

fn is_legacy_default_agent_id(agent_id: &str) -> bool {
    agent_id.trim() == LEGACY_DEFAULT_AGENT_ID
}

pub fn default_config_path() -> Result<PathBuf> {
    if let Some(path) = config_path_override_from_env() {
        return Ok(path);
    }

    #[cfg(windows)]
    if let Some(path) = windows_shared_config_path() {
        return Ok(path);
    }

    project_config_path()
}

fn default_config_file_name() -> &'static str {
    config_file_name_for_build(cfg!(debug_assertions))
}

fn config_file_name_for_build(debug_assertions: bool) -> &'static str {
    if debug_assertions {
        DEVELOPMENT_CONFIG_FILE_NAME
    } else {
        DEFAULT_CONFIG_FILE_NAME
    }
}

#[cfg(windows)]
pub fn windows_shared_config_path() -> Option<PathBuf> {
    let program_data = env::var_os("ProgramData")?;
    Some(
        PathBuf::from(program_data)
            .join("Baijimu")
            .join("BridgeAgent")
            .join(default_config_file_name()),
    )
}

#[cfg(not(windows))]
pub fn windows_shared_config_path() -> Option<PathBuf> {
    None
}

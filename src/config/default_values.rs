fn default_enabled() -> bool {
    true
}

fn default_timeout_secs() -> u64 {
    30
}

fn validate_optional_runtime_path(label: &str, value: Option<&str>) -> Result<()> {
    if value.is_some_and(|path| path.trim().is_empty()) {
        bail!("{label} cannot be empty when set");
    }
    Ok(())
}

fn default_max_timeout_secs() -> u64 {
    120
}

fn default_reconnect_secs() -> u64 {
    3
}

fn default_log_limit() -> usize {
    500
}

fn default_log_file_enabled() -> bool {
    true
}

fn default_log_file_max_bytes() -> u64 {
    5 * 1024 * 1024
}

fn default_log_file_max_files() -> usize {
    5
}

fn default_event_server_enabled() -> bool {
    true
}

fn default_event_server_bind() -> String {
    "127.0.0.1:18081".to_string()
}

fn default_service_registration_enabled() -> bool {
    true
}

fn default_http_method() -> String {
    "POST".to_string()
}

fn default_health_check_http_method() -> String {
    "GET".to_string()
}

fn default_platform_config() -> PlatformConfig {    PlatformConfig {
            environment_key: None,        base_url: DEFAULT_PLATFORM_BASE_URL.to_string(),
        workspace_id: None,
    }
}

fn normalize_default_platform_base_url(value: &str) -> Option<String> {
    let trimmed = value.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return None;
    }

    let Ok(url) = Url::parse(trimmed) else {
        return None;
    };
    let host = url.host_str()?;
    let normalized_host = host.trim_start_matches("www.");
    if normalized_host != "baijimu.com" && host != "api.baijimu.com" {
        return None;
    }
    if !matches!(url.scheme(), "https" | "http") {
        return None;
    }

    let path = url.path().trim_end_matches('/');
    match (host, path) {
        ("api.baijimu.com", "" | "/" | "/lowcode3") => Some(DEFAULT_PLATFORM_BASE_URL.to_string()),
        (_, "" | "/" | "/lowcode" | "/manager" | "/lowcode3") => {
            Some(DEFAULT_PLATFORM_BASE_URL.to_string())
        }
        _ => None,
    }
}

impl Default for UploadConfig {
    fn default() -> Self {
        Self {
            prepare_url: None,
            inline_limit_bytes: default_inline_limit_bytes(),
            timeout_secs: default_upload_timeout_secs(),
        }
    }
}

impl UploadConfig {
    pub fn prepare_url(&self, relay: &RelayConfig) -> Option<String> {
        if let Some(prepare_url) = self.prepare_url.as_deref() {
            let trimmed = prepare_url.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }

        default_prepare_url_from_relay(&relay.url)
    }
}

fn default_prepare_url_from_relay(relay_url: &str) -> Option<String> {
    let trimmed = relay_url.trim();
    if trimmed.is_empty() {
        return None;
    }

    let url = url::Url::parse(trimmed).ok()?;
    let scheme = match url.scheme() {
        "wss" => "https",
        "ws" => "http",
        "https" => "https",
        "http" => "http",
        _ => return None,
    };

    let host = url.host_str()?;
    let mut base = format!("{scheme}://{host}");
    if let Some(port) = url.port() {
        let default_port = matches!((scheme, port), ("https", 443) | ("http", 80));
        if !default_port {
            base.push(':');
            base.push_str(&port.to_string());
        }
    }

    Some(format!("{base}/api/bridge-agent/uploads/prepare"))
}

fn default_inline_limit_bytes() -> usize {
    DEFAULT_INLINE_LIMIT_BYTES
}

fn default_upload_timeout_secs() -> u64 {
    60
}

fn config_path_override_from_env() -> Option<PathBuf> {
    env::var_os("WS_BRIDGE_CONFIG").map(PathBuf::from)
}

fn project_config_path() -> Result<PathBuf> {
    let dirs = ProjectDirs::from("com", "baijimu", "bridge-agent")
        .context("failed to determine config directory")?;
    Ok(dirs.config_dir().join(default_config_file_name()))
}

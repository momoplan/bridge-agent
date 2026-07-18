use crate::protocol::{EventDefinition, MethodDefinition, ServiceDefinition};
use anyhow::{bail, Context, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::io::Write;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use url::Url;
use uuid::Uuid;

const DEFAULT_RELAY_URL: &str = "wss://relay.baijimu.com/ws/agent";
const LEGACY_DEFAULT_RELAY_URL: &str = "ws://127.0.0.1:8080/ws/agent";
const DEFAULT_PLATFORM_BASE_URL: &str = "https://baijimu.com/lowcode3";
const DEFAULT_CONFIG_FILE_NAME: &str = "agent-config.json";
const LEGACY_DEFAULT_AGENT_ID: &str = "devbox";
const LEGACY_DEFAULT_DEVICE_NAME: &str = "我的百积木";
const GENERATED_AGENT_ID_PREFIX: &str = "dev_";
const DEFAULT_INLINE_LIMIT_BYTES: usize = 256 * 1024;
const LEGACY_INLINE_LIMIT_BYTES: usize = 8 * 1024 * 1024;
const CONFIG_BACKUP_SUFFIX: &str = "bak";
const INVALID_CONFIG_MARKER: &str = "invalid";
const TEMP_CONFIG_MARKER: &str = "tmp";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    #[serde(default = "default_platform_config")]
    pub platform: PlatformConfig,
    #[serde(default)]
    pub upload: UploadConfig,
    pub relay: RelayConfig,
    pub device: DeviceConfig,
    pub runtime: RuntimeConfig,
    pub services: Vec<ServiceConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformConfig {
    pub base_url: String,
    #[serde(default)]
    pub workspace_id: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UploadConfig {
    #[serde(default)]
    pub prepare_url: Option<String>,
    #[serde(default = "default_inline_limit_bytes")]
    pub inline_limit_bytes: usize,
    #[serde(default = "default_upload_timeout_secs")]
    pub timeout_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelayConfig {
    pub url: String,
    pub agent_id: String,
    pub token: String,
    #[serde(default = "default_reconnect_secs")]
    pub reconnect_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceConfig {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeConfig {
    #[serde(default)]
    pub node_path: Option<String>,
    #[serde(default)]
    pub codex_binary_path: Option<String>,
    #[serde(default = "default_timeout_secs")]
    pub default_timeout_secs: u64,
    #[serde(default = "default_max_timeout_secs")]
    pub max_timeout_secs: u64,
    #[serde(default = "default_log_limit")]
    pub log_limit: usize,
    #[serde(default = "default_log_file_enabled")]
    pub log_file_enabled: bool,
    #[serde(default)]
    pub log_file_dir: Option<String>,
    #[serde(default = "default_log_file_max_bytes")]
    pub log_file_max_bytes: u64,
    #[serde(default = "default_log_file_max_files")]
    pub log_file_max_files: usize,
    #[serde(default = "default_event_server_enabled")]
    pub event_server_enabled: bool,
    #[serde(default = "default_event_server_bind")]
    pub event_server_bind: String,
    #[serde(default)]
    pub event_server_token: Option<String>,
    #[serde(default = "default_service_registration_enabled")]
    pub service_registration_enabled: bool,
    #[serde(default)]
    pub service_registration_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceConfig {
    pub name: String,
    pub description: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default, alias = "healthCheck")]
    pub health_check: Option<ServiceHealthCheck>,
    #[serde(default, alias = "startCommand")]
    pub start_command: Option<ServiceStartCommand>,
    #[serde(default, alias = "stopCommand")]
    pub stop_command: Option<ServiceStartCommand>,
    #[serde(default)]
    pub methods: Vec<MethodConfig>,
    #[serde(default)]
    pub events: Vec<EventConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MethodConfig {
    pub name: String,
    pub description: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default = "default_object_schema")]
    pub input_schema: Value,
    pub binding: MethodBinding,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventConfig {
    pub name: String,
    pub description: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default = "default_object_schema")]
    pub payload_schema: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServiceHealthCheck {
    Http {
        url: String,
        #[serde(default = "default_http_method")]
        http_method: String,
        #[serde(default)]
        headers: BTreeMap<String, String>,
        #[serde(default, alias = "timeoutSecs")]
        timeout_secs: Option<u64>,
        #[serde(default, alias = "expectStatus")]
        expect_status: Option<u16>,
        #[serde(default, alias = "bodyContains")]
        body_contains: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServiceStartCommand {
    ShellCommand {
        command: Vec<String>,
        #[serde(default)]
        cwd: Option<String>,
        #[serde(default)]
        env: BTreeMap<String, String>,
        #[serde(default, alias = "timeoutSecs")]
        timeout_secs: Option<u64>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MethodBinding {
    ShellCommand(ShellCommandBinding),
    Http(HttpBinding),
    ComputerUse(ComputerUseBinding),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShellCommandBinding {
    pub root_dir: String,
    #[serde(default)]
    pub allow_commands: Vec<String>,
    #[serde(default)]
    pub default_timeout_secs: Option<u64>,
    #[serde(default)]
    pub max_timeout_secs: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpBinding {
    pub url: String,
    #[serde(default = "default_http_method")]
    pub http_method: String,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub timeout_secs: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ComputerUseAction {
    Screenshot,
    Click,
    DoubleClick,
    Scroll,
    Type,
    Wait,
    Keypress,
    Drag,
    Move,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComputerUseBinding {
    pub action: ComputerUseAction,
    #[serde(default)]
    pub display_id: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ManifestPreview {
    pub device: DeviceConfig,
    pub services: Vec<ServiceDefinition>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BrowserAuthManifestPreview {
    pub device: DeviceConfig,
    pub services: Vec<BrowserAuthServiceDefinition>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BrowserAuthServiceDefinition {
    pub name: String,
    pub description: String,
    pub methods: Vec<BrowserAuthMethodDefinition>,
    #[serde(default)]
    pub events: Vec<BrowserAuthEventDefinition>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BrowserAuthMethodDefinition {
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct BrowserAuthEventDefinition {
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone)]
pub struct ConfigRecovery {
    pub archived_path: Option<PathBuf>,
    pub config: AgentConfig,
}

impl AgentConfig {
    pub fn example() -> Self {
        Self {
            platform: PlatformConfig {
                base_url: DEFAULT_PLATFORM_BASE_URL.to_string(),
                workspace_id: None,
            },
            upload: UploadConfig::default(),
            relay: RelayConfig {
                url: DEFAULT_RELAY_URL.to_string(),
                agent_id: generate_agent_id(),
                token: String::new(),
                reconnect_secs: default_reconnect_secs(),
            },
            device: DeviceConfig {
                name: default_device_name(),
                description: "Installed on the user's local machine.".to_string(),
                tags: vec!["desktop".to_string(), "local".to_string()],
            },
            runtime: RuntimeConfig {
                node_path: None,
                codex_binary_path: None,
                default_timeout_secs: default_timeout_secs(),
                max_timeout_secs: default_max_timeout_secs(),
                log_limit: default_log_limit(),
                log_file_enabled: default_log_file_enabled(),
                log_file_dir: None,
                log_file_max_bytes: default_log_file_max_bytes(),
                log_file_max_files: default_log_file_max_files(),
                event_server_enabled: default_event_server_enabled(),
                event_server_bind: default_event_server_bind(),
                event_server_token: None,
                service_registration_enabled: true,
                service_registration_token: Some(generate_registration_token()),
            },
            services: vec![default_computer_service(), default_shell_service()],
        }
    }

    pub fn normalize(&mut self) -> bool {
        let mut changed = ensure_default_computer_methods(self);
        changed |= ensure_default_shell_service(self);
        changed |= ensure_service_registration_defaults(self);
        changed |= ensure_default_platform_base_url(self);
        changed
    }

    pub fn validate(&self) -> Result<()> {
        if self.platform.base_url.trim().is_empty() {
            bail!("platform.base_url cannot be empty");
        }
        if let Some(prepare_url) = &self.upload.prepare_url {
            if prepare_url.trim().is_empty() {
                bail!("upload.prepare_url cannot be empty when set");
            }
        }
        if self.upload.inline_limit_bytes == 0 {
            bail!("upload.inline_limit_bytes must be greater than zero");
        }
        if self.upload.timeout_secs == 0 {
            bail!("upload.timeout_secs must be greater than zero");
        }
        if self.relay.url.trim().is_empty() {
            bail!("relay.url cannot be empty");
        }
        if self.relay.agent_id.trim().is_empty() {
            bail!("relay.agent_id cannot be empty");
        }
        if self.runtime.default_timeout_secs == 0 || self.runtime.max_timeout_secs == 0 {
            bail!("runtime timeouts must be greater than zero");
        }
        validate_optional_runtime_path("runtime.node_path", self.runtime.node_path.as_deref())?;
        validate_optional_runtime_path(
            "runtime.codex_binary_path",
            self.runtime.codex_binary_path.as_deref(),
        )?;
        if self.runtime.default_timeout_secs > self.runtime.max_timeout_secs {
            bail!("runtime.default_timeout_secs cannot exceed runtime.max_timeout_secs");
        }
        if self.runtime.log_limit == 0 {
            bail!("runtime.log_limit must be greater than zero");
        }
        if self.runtime.log_file_enabled {
            if self.runtime.log_file_max_bytes < 1024 {
                bail!("runtime.log_file_max_bytes must be at least 1024");
            }
            if self.runtime.log_file_max_files == 0 {
                bail!("runtime.log_file_max_files must be greater than zero");
            }
        }
        if self.runtime.event_server_enabled {
            let bind: SocketAddr = self
                .runtime
                .event_server_bind
                .parse()
                .with_context(|| "runtime.event_server_bind must be a socket address")?;
            if !bind.ip().is_loopback()
                && self
                    .runtime
                    .event_server_token
                    .as_deref()
                    .map(str::trim)
                    .unwrap_or_default()
                    .is_empty()
            {
                bail!(
                    "runtime.event_server_token is required when event_server_bind is not loopback"
                );
            }
        }
        if self.runtime.service_registration_enabled {
            let bind: SocketAddr = self
                .runtime
                .event_server_bind
                .parse()
                .with_context(|| "runtime.event_server_bind must be a socket address")?;
            if !bind.ip().is_loopback() {
                bail!("runtime.service_registration_enabled requires event_server_bind to be loopback");
            }
            if self
                .runtime
                .service_registration_token
                .as_deref()
                .map(str::trim)
                .unwrap_or_default()
                .is_empty()
            {
                bail!("runtime.service_registration_token is required when service registration is enabled");
            }
        }

        let mut service_names = BTreeSet::new();
        for service in &self.services {
            if service.name.trim().is_empty() {
                bail!("service name cannot be empty");
            }
            if !service_names.insert(service.name.as_str()) {
                bail!("duplicate service `{}`", service.name);
            }

            let mut method_names = BTreeSet::new();
            let mut event_names = BTreeSet::new();
            for method in &service.methods {
                if method.name.trim().is_empty() {
                    bail!("method name cannot be empty in service `{}`", service.name);
                }
                if !method_names.insert(method.name.as_str()) {
                    bail!(
                        "duplicate method `{}` in service `{}`",
                        method.name,
                        service.name
                    );
                }

                match &method.binding {
                    MethodBinding::ShellCommand(binding) => {
                        if binding.root_dir.trim().is_empty() {
                            bail!(
                                "shell binding root_dir cannot be empty for {}.{}",
                                service.name,
                                method.name
                            );
                        }
                        if binding.allow_commands.is_empty() {
                            bail!(
                                "shell binding allow_commands cannot be empty for {}.{}",
                                service.name,
                                method.name
                            );
                        }
                    }
                    MethodBinding::Http(binding) => {
                        if binding.url.trim().is_empty() {
                            bail!(
                                "http binding url cannot be empty for {}.{}",
                                service.name,
                                method.name
                            );
                        }
                        if binding.http_method.trim().is_empty() {
                            bail!(
                                "http binding method cannot be empty for {}.{}",
                                service.name,
                                method.name
                            );
                        }
                    }
                    MethodBinding::ComputerUse(_) => {}
                }
            }
            for event in &service.events {
                if event.name.trim().is_empty() {
                    bail!("event name cannot be empty in service `{}`", service.name);
                }
                if !event_names.insert(event.name.as_str()) {
                    bail!(
                        "duplicate event `{}` in service `{}`",
                        event.name,
                        service.name
                    );
                }
                if method_names.contains(event.name.as_str()) {
                    bail!(
                        "event `{}` conflicts with method of the same name in service `{}`",
                        event.name,
                        service.name
                    );
                }
            }
        }

        Ok(())
    }

    pub fn service_definitions(&self) -> Vec<ServiceDefinition> {
        self.services
            .iter()
            .filter(|service| service.enabled)
            .map(|service| ServiceDefinition {
                name: service.name.clone(),
                description: service.description.clone(),
                methods: service
                    .methods
                    .iter()
                    .filter(|method| method.enabled)
                    .map(|method| MethodDefinition {
                        name: method.name.clone(),
                        description: method.description.clone(),
                        input_schema: method.input_schema.clone(),
                    })
                    .collect(),
                events: service
                    .events
                    .iter()
                    .filter(|event| event.enabled)
                    .map(|event| EventDefinition {
                        name: event.name.clone(),
                        description: event.description.clone(),
                        payload_schema: event.payload_schema.clone(),
                    })
                    .collect(),
            })
            .filter(|service| !service.methods.is_empty() || !service.events.is_empty())
            .collect()
    }

    pub fn manifest_preview(&self) -> ManifestPreview {
        ManifestPreview {
            device: self.device.clone(),
            services: self.service_definitions(),
        }
    }

    pub fn browser_auth_manifest_preview(&self) -> BrowserAuthManifestPreview {
        BrowserAuthManifestPreview {
            device: self.device.clone(),
            services: self
                .services
                .iter()
                .filter(|service| service.enabled)
                .map(|service| BrowserAuthServiceDefinition {
                    name: service.name.clone(),
                    description: service.description.clone(),
                    methods: service
                        .methods
                        .iter()
                        .filter(|method| method.enabled)
                        .map(|method| BrowserAuthMethodDefinition {
                            name: method.name.clone(),
                            description: method.description.clone(),
                        })
                        .collect(),
                    events: service
                        .events
                        .iter()
                        .filter(|event| event.enabled)
                        .map(|event| BrowserAuthEventDefinition {
                            name: event.name.clone(),
                            description: event.description.clone(),
                        })
                        .collect(),
                })
                .filter(|service| !service.methods.is_empty() || !service.events.is_empty())
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceRegistration {
    pub name: String,
    pub description: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    pub transport: RegistrationTransport,
    #[serde(default, alias = "healthCheck")]
    pub health_check: Option<RegistrationHealthCheck>,
    #[serde(default, alias = "startCommand")]
    pub start_command: Option<ServiceStartCommand>,
    #[serde(default, alias = "stopCommand")]
    pub stop_command: Option<ServiceStartCommand>,
    #[serde(default)]
    pub methods: Vec<RegistrationMethod>,
    #[serde(default)]
    pub events: Vec<EventConfig>,
    #[serde(default)]
    pub replace: bool,
    #[serde(default)]
    pub managed_by: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RegistrationTransport {
    Http {
        #[serde(alias = "baseUrl")]
        base_url: String,
        #[serde(default)]
        headers: BTreeMap<String, String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RegistrationHealthCheck {
    Http {
        #[serde(default)]
        path: Option<String>,
        #[serde(default)]
        url: Option<String>,
        #[serde(default = "default_health_check_http_method", alias = "httpMethod")]
        http_method: String,
        #[serde(default)]
        headers: BTreeMap<String, String>,
        #[serde(default, alias = "timeoutSecs")]
        timeout_secs: Option<u64>,
        #[serde(default, alias = "expectStatus")]
        expect_status: Option<u16>,
        #[serde(default, alias = "bodyContains")]
        body_contains: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistrationMethod {
    pub name: String,
    pub description: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default = "default_object_schema")]
    pub input_schema: Value,
    #[serde(default, alias = "path")]
    pub path: String,
    #[serde(default = "default_http_method", alias = "httpMethod")]
    pub http_method: String,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default, alias = "timeoutSecs")]
    pub timeout_secs: Option<u64>,
}

impl ServiceRegistration {
    pub fn into_service_config(self) -> Result<ServiceConfig> {
        if self.name.trim().is_empty() {
            bail!("service name cannot be empty");
        }
        if self.methods.is_empty() && self.events.is_empty() {
            bail!("service registration must include at least one method or event");
        }

        let (methods, health_check) = match self.transport {
            RegistrationTransport::Http { base_url, headers } => {
                let base_url = normalize_registration_base_url(&base_url)?;
                let health_check = self
                    .health_check
                    .map(|health_check| health_check.into_service_health_check(&base_url, &headers))
                    .transpose()?;
                let methods = self
                    .methods
                    .into_iter()
                    .map(|method| method.into_http_method_config(&base_url, &headers))
                    .collect::<Result<Vec<_>>>()?;
                (methods, health_check)
            }
        };

        Ok(ServiceConfig {
            name: self.name.trim().to_string(),
            description: self.description.trim().to_string(),
            enabled: self.enabled,
            health_check,
            start_command: self.start_command,
            stop_command: self.stop_command,
            methods,
            events: self.events,
        })
    }
}

impl RegistrationHealthCheck {
    fn into_service_health_check(
        self,
        base_url: &Url,
        transport_headers: &BTreeMap<String, String>,
    ) -> Result<ServiceHealthCheck> {
        match self {
            RegistrationHealthCheck::Http {
                path,
                url,
                http_method,
                headers,
                timeout_secs,
                expect_status,
                body_contains,
            } => {
                let resolved_url = match url
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    Some(url) => normalize_registration_base_url(url)?.to_string(),
                    None => join_registration_url(base_url, path.as_deref().unwrap_or("/health"))?,
                };
                let mut resolved_headers = transport_headers.clone();
                resolved_headers.extend(headers);
                Ok(ServiceHealthCheck::Http {
                    url: resolved_url,
                    http_method: http_method.trim().to_uppercase(),
                    headers: resolved_headers,
                    timeout_secs,
                    expect_status,
                    body_contains,
                })
            }
        }
    }
}

impl RegistrationMethod {
    fn into_http_method_config(
        self,
        base_url: &Url,
        transport_headers: &BTreeMap<String, String>,
    ) -> Result<MethodConfig> {
        if self.name.trim().is_empty() {
            bail!("method name cannot be empty");
        }
        let mut headers = transport_headers.clone();
        headers.extend(self.headers);

        Ok(MethodConfig {
            name: self.name.trim().to_string(),
            description: self.description.trim().to_string(),
            enabled: self.enabled,
            input_schema: self.input_schema,
            binding: MethodBinding::Http(HttpBinding {
                url: join_registration_url(base_url, &self.path)?,
                http_method: self.http_method.trim().to_uppercase(),
                headers,
                timeout_secs: self.timeout_secs,
            }),
        })
    }
}

fn normalize_registration_base_url(base_url: &str) -> Result<Url> {
    let base_url = base_url.trim();
    if base_url.is_empty() {
        bail!("transport.baseUrl cannot be empty");
    }
    let url =
        Url::parse(base_url).with_context(|| format!("invalid transport.baseUrl `{base_url}`"))?;
    if url.scheme() != "http" && url.scheme() != "https" {
        bail!("transport.baseUrl must use http or https");
    }
    Ok(url)
}

fn join_registration_url(base_url: &Url, path: &str) -> Result<String> {
    let path = path.trim();
    if path.is_empty() {
        return Ok(base_url.as_str().trim_end_matches('/').to_string());
    }
    let path = path.trim_start_matches('/');
    Ok(base_url
        .join(path)
        .with_context(|| format!("invalid method path `{path}`"))?
        .to_string())
}

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
    if let Some(path) = windows_service_config_path() {
        return Ok(path);
    }

    project_config_path()
}

pub fn windows_service_config_path() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        let program_data = env::var_os("ProgramData")?;
        return Some(
            PathBuf::from(program_data)
                .join("Baijimu")
                .join("BridgeAgent")
                .join(DEFAULT_CONFIG_FILE_NAME),
        );
    }

    #[cfg(not(windows))]
    {
        None
    }
}

pub fn load_config(path: &Path) -> Result<AgentConfig> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("failed to read config {}", path.display()))?;
    let mut config: AgentConfig = serde_json::from_str(&content)
        .with_context(|| format!("failed to parse config {}", path.display()))?;
    let mut changed = config.normalize();
    changed |= migrate_legacy_defaults(&mut config);
    config.validate()?;
    if changed {
        save_config(path, &config)?;
    }
    Ok(config)
}

pub fn save_config(path: &Path, config: &AgentConfig) -> Result<()> {
    let mut config = config.clone();
    config.normalize();
    config.validate()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create config dir {}", parent.display()))?;
    }
    let content = serde_json::to_string_pretty(&config)?;
    write_config_atomically(path, format!("{content}\n").as_bytes())?;
    Ok(())
}

pub fn ensure_config_exists(path: &Path) -> Result<()> {
    if !path.exists() {
        save_config(path, &AgentConfig::example())?;
    }
    Ok(())
}

pub fn reset_invalid_config(path: &Path) -> Result<ConfigRecovery> {
    let archived_path = if path.exists() {
        Some(archive_existing_config(path)?)
    } else {
        None
    };
    let config = AgentConfig::example();
    save_config(path, &config)?;
    Ok(ConfigRecovery {
        archived_path,
        config,
    })
}

fn write_config_atomically(path: &Path, content: &[u8]) -> Result<()> {
    let temp_path = unique_sibling_path(path, TEMP_CONFIG_MARKER);
    let write_result = (|| -> Result<()> {
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp_path)
            .with_context(|| format!("failed to create temp config {}", temp_path.display()))?;
        file.write_all(content)
            .with_context(|| format!("failed to write temp config {}", temp_path.display()))?;
        file.sync_all()
            .with_context(|| format!("failed to flush temp config {}", temp_path.display()))?;
        Ok(())
    })();

    if let Err(err) = write_result {
        let _ = fs::remove_file(&temp_path);
        return Err(err);
    }

    if path.exists() {
        let backup_path = backup_config_path(path);
        fs::copy(path, &backup_path).with_context(|| {
            format!(
                "failed to backup config {} to {}",
                path.display(),
                backup_path.display()
            )
        })?;
    }

    if let Err(err) = replace_file(&temp_path, path) {
        let _ = fs::remove_file(&temp_path);
        return Err(err);
    }

    Ok(())
}

fn backup_config_path(path: &Path) -> PathBuf {
    sibling_path(path, CONFIG_BACKUP_SUFFIX)
}

fn archive_existing_config(path: &Path) -> Result<PathBuf> {
    let archived_path = unique_sibling_path(path, INVALID_CONFIG_MARKER);
    fs::rename(path, &archived_path).with_context(|| {
        format!(
            "failed to archive config {} to {}",
            path.display(),
            archived_path.display()
        )
    })?;
    Ok(archived_path)
}

fn unique_sibling_path(path: &Path, marker: &str) -> PathBuf {
    let timestamp = current_timestamp_millis();
    for attempt in 0..1000 {
        let suffix = if attempt == 0 {
            format!("{marker}-{timestamp}")
        } else {
            format!("{marker}-{timestamp}-{attempt}")
        };
        let candidate = sibling_path(path, &suffix);
        if !candidate.exists() {
            return candidate;
        }
    }
    sibling_path(
        path,
        &format!("{marker}-{timestamp}-{}", uuid::Uuid::new_v4().simple()),
    )
}

fn sibling_path(path: &Path, suffix: &str) -> PathBuf {
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_else(|| DEFAULT_CONFIG_FILE_NAME.into());
    let name = format!("{file_name}.{suffix}");
    path.with_file_name(name)
}

fn current_timestamp_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}

#[cfg(windows)]
fn replace_file(from: &Path, to: &Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    fn wide_path(path: &Path) -> Vec<u16> {
        path.as_os_str().encode_wide().chain(Some(0)).collect()
    }

    let from_wide = wide_path(from);
    let to_wide = wide_path(to);
    let replaced = unsafe {
        MoveFileExW(
            from_wide.as_ptr(),
            to_wide.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if replaced == 0 {
        return Err(std::io::Error::last_os_error()).with_context(|| {
            format!(
                "failed to replace config {} with {}",
                to.display(),
                from.display()
            )
        });
    }
    Ok(())
}

#[cfg(not(windows))]
fn replace_file(from: &Path, to: &Path) -> Result<()> {
    fs::rename(from, to).with_context(|| {
        format!(
            "failed to replace config {} with {}",
            to.display(),
            from.display()
        )
    })
}

fn migrate_legacy_defaults(config: &mut AgentConfig) -> bool {
    let mut changed = false;
    changed |= ensure_default_platform_base_url(config);
    if config.relay.url.trim() == LEGACY_DEFAULT_RELAY_URL {
        config.relay.url = DEFAULT_RELAY_URL.to_string();
        changed = true;
    }
    if config.upload.inline_limit_bytes == LEGACY_INLINE_LIMIT_BYTES {
        config.upload.inline_limit_bytes = DEFAULT_INLINE_LIMIT_BYTES;
        changed = true;
    }
    if config.device.name.trim() == LEGACY_DEFAULT_DEVICE_NAME {
        let device_name = default_device_name();
        if device_name != LEGACY_DEFAULT_DEVICE_NAME {
            config.device.name = device_name;
            changed = true;
        }
    }
    changed |= remove_legacy_default_local_java_service(config);
    changed
}

fn ensure_default_platform_base_url(config: &mut AgentConfig) -> bool {
    let Some(base_url) = normalize_default_platform_base_url(&config.platform.base_url) else {
        return false;
    };
    if base_url == config.platform.base_url {
        return false;
    }
    config.platform.base_url = base_url;
    true
}

fn remove_legacy_default_local_java_service(config: &mut AgentConfig) -> bool {
    let initial_len = config.services.len();
    config
        .services
        .retain(|service| !is_legacy_default_local_java_service(service));
    config.services.len() != initial_len
}

fn is_legacy_default_local_java_service(service: &ServiceConfig) -> bool {
    if service.name != "local-java-service"
        || service.description != "Example business service backed by a local HTTP endpoint."
        || service.enabled
        || service.health_check.is_some()
        || service.start_command.is_some()
        || service.stop_command.is_some()
        || service.methods.len() != 1
        || service.events.len() != 1
    {
        return false;
    }

    let method = &service.methods[0];
    let method_matches = method.name == "invokeApi"
        && method.description == "Forward invocation arguments to a local HTTP service."
        && method.enabled
        && method.input_schema == default_object_schema()
        && matches!(
            &method.binding,
            MethodBinding::Http(HttpBinding {
                url,
                http_method,
                headers,
                timeout_secs
            }) if url == "http://127.0.0.1:8081/api/invoke"
                && http_method == "POST"
                && headers.is_empty()
                && *timeout_secs == Some(20)
        );

    let event = &service.events[0];
    let event_matches = event.name == "jobFinished"
        && event.description == "Emitted when the local service completes an asynchronous job."
        && event.enabled
        && event.payload_schema == default_object_schema();

    method_matches && event_matches
}

pub fn manifest_preview_json(config: &AgentConfig) -> Result<String> {
    Ok(serde_json::to_string_pretty(&config.manifest_preview())?)
}

pub fn browser_auth_manifest_json(config: &AgentConfig) -> Result<String> {
    Ok(serde_json::to_string(
        &config.browser_auth_manifest_preview(),
    )?)
}

pub fn resolve_config_base_dir(path: &Path) -> PathBuf {
    path.parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn default_object_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": true
    })
}

pub fn shell_input_schema() -> Value {
    json!({
        "type": "object",
        "required": ["command"],
        "properties": {
            "command": {
                "description": "Command argv array for direct execution. On Windows, run shell built-ins or PATH lookup through cmd /C, for example [\"cmd\", \"/C\", \"where\", \"wechat-decrypt\"]. For multi-line PowerShell scripts, avoid putting the script body in -Command; pass [\"powershell\", \"-NoProfile\", \"-ExecutionPolicy\", \"Bypass\", \"-File\", \"-\"] and put the script body in stdin.",
                "type": "array",
                "items": {"type": "string"},
                "minItems": 1
            },
            "cwd": {"type": "string"},
            "env": {
                "type": "object",
                "additionalProperties": {"type": "string"}
            },
            "stdin": {
                "description": "Optional text to write to the process standard input. Use this for multi-line scripts instead of embedding long script bodies in command arguments.",
                "type": "string"
            }
        }
    })
}

pub fn shell_execution_id_schema() -> Value {
    json!({
        "type": "object",
        "required": ["executionId"],
        "properties": {
            "executionId": {"type": "string"}
        }
    })
}

pub fn computer_action_input_schema(action: &ComputerUseAction) -> Value {
    match action {
        ComputerUseAction::Screenshot => json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {}
        }),
        ComputerUseAction::Click | ComputerUseAction::DoubleClick | ComputerUseAction::Move => {
            json!({
                "type": "object",
                "required": ["x", "y"],
                "properties": {
                    "x": {"type": "number"},
                    "y": {"type": "number"},
                    "button": {
                        "type": "string",
                        "enum": ["left", "middle", "right"]
                    },
                    "keys": {
                        "type": "array",
                        "items": {"type": "string"}
                    }
                }
            })
        }
        ComputerUseAction::Scroll => json!({
            "type": "object",
            "required": ["x", "y"],
            "properties": {
                "x": {"type": "number"},
                "y": {"type": "number"},
                "scroll_x": {"type": "integer"},
                "scroll_y": {"type": "integer"},
                "scrollX": {"type": "integer"},
                "scrollY": {"type": "integer"},
                "keys": {
                    "type": "array",
                    "items": {"type": "string"}
                }
            }
        }),
        ComputerUseAction::Type => json!({
            "type": "object",
            "required": ["text"],
            "properties": {
                "text": {"type": "string"}
            }
        }),
        ComputerUseAction::Wait => json!({
            "type": "object",
            "properties": {
                "ms": {
                    "type": "integer",
                    "minimum": 0
                }
            }
        }),
        ComputerUseAction::Keypress => json!({
            "type": "object",
            "required": ["keys"],
            "properties": {
                "keys": {
                    "type": "array",
                    "items": {"type": "string"},
                    "minItems": 1
                }
            }
        }),
        ComputerUseAction::Drag => json!({
            "type": "object",
            "required": ["path"],
            "properties": {
                "path": {
                    "type": "array",
                    "minItems": 2,
                    "items": {
                        "type": "object",
                        "required": ["x", "y"],
                        "properties": {
                            "x": {"type": "number"},
                            "y": {"type": "number"}
                        }
                    }
                },
                "keys": {
                    "type": "array",
                    "items": {"type": "string"}
                }
            }
        }),
    }
}

fn computer_method(name: &str, description: &str, action: ComputerUseAction) -> MethodConfig {
    MethodConfig {
        name: name.to_string(),
        description: description.to_string(),
        enabled: true,
        input_schema: computer_action_input_schema(&action),
        binding: MethodBinding::ComputerUse(ComputerUseBinding {
            action,
            display_id: None,
        }),
    }
}

fn default_computer_service() -> ServiceConfig {
    ServiceConfig {
        name: "computer".to_string(),
        description: "Computer control operations exposed as business methods.".to_string(),
        enabled: true,
        health_check: None,
        start_command: None,
        stop_command: None,
        methods: vec![
            computer_method(
                "screenshot",
                "Capture the current desktop and return a PNG screenshot.",
                ComputerUseAction::Screenshot,
            ),
            computer_method(
                "click",
                "Click at a screen coordinate with an optional mouse button.",
                ComputerUseAction::Click,
            ),
            computer_method(
                "double_click",
                "Double-click at a screen coordinate.",
                ComputerUseAction::DoubleClick,
            ),
            computer_method(
                "scroll",
                "Scroll at a screen coordinate with horizontal and vertical deltas.",
                ComputerUseAction::Scroll,
            ),
            computer_method(
                "type",
                "Type text into the currently focused app.",
                ComputerUseAction::Type,
            ),
            computer_method(
                "keypress",
                "Press one key or a key chord such as Command+L.",
                ComputerUseAction::Keypress,
            ),
            computer_method(
                "drag",
                "Drag the pointer across a path of coordinates.",
                ComputerUseAction::Drag,
            ),
            computer_method(
                "move",
                "Move the pointer to a screen coordinate.",
                ComputerUseAction::Move,
            ),
            computer_method(
                "wait",
                "Pause briefly to let the desktop settle before the next screenshot.",
                ComputerUseAction::Wait,
            ),
        ],
        events: Vec::new(),
    }
}

fn default_shell_service() -> ServiceConfig {
    ServiceConfig {
        name: "shell".to_string(),
        description: "Run and query allowlisted shell commands on the local machine.".to_string(),
        enabled: true,
        health_check: None,
        start_command: None,
        stop_command: None,
        methods: default_shell_methods(),
        events: Vec::new(),
    }
}

fn default_shell_methods() -> Vec<MethodConfig> {
    vec![
        default_shell_exec_method("exec"),
        default_shell_start_execution_method(),
        default_shell_query_execution_method("queryExecution"),
        default_shell_cancel_execution_method(),
    ]
}

fn default_shell_exec_method(name: &str) -> MethodConfig {
    MethodConfig {
        name: name.to_string(),
        description:
            "Run one allowlisted command with optional cwd, env, and stdin. The command is tracked as an execution; quick commands return their result directly, while longer commands return status=RUNNING with executionId and recommendedService/recommendedMethod for polling. For multi-line PowerShell scripts, pass the script via stdin with powershell -File - instead of embedding it in -Command."
                .to_string(),
        enabled: true,
        input_schema: shell_input_schema(),
        binding: MethodBinding::ShellCommand(ShellCommandBinding {
            root_dir: ".".to_string(),
            allow_commands: default_shell_exec_allow_commands(),
            default_timeout_secs: Some(default_timeout_secs()),
            max_timeout_secs: Some(default_max_timeout_secs()),
        }),
    }
}

fn default_shell_start_execution_method() -> MethodConfig {
    MethodConfig {
        name: "startExecution".to_string(),
        description:
            "Start one allowlisted shell command and immediately return an executionId for status polling. Prefer this for installs, downloads, builds, service startup, or any command expected to run longer than 30 seconds. For multi-line PowerShell scripts, pass the script via stdin with powershell -File - instead of embedding it in -Command."
                .to_string(),
        enabled: true,
        input_schema: shell_input_schema(),
        binding: MethodBinding::ShellCommand(ShellCommandBinding {
            root_dir: ".".to_string(),
            allow_commands: default_shell_exec_allow_commands(),
            default_timeout_secs: Some(default_timeout_secs()),
            max_timeout_secs: Some(default_max_timeout_secs()),
        }),
    }
}

fn default_shell_query_execution_method(name: &str) -> MethodConfig {
    MethodConfig {
        name: name.to_string(),
        description:
            "Query status, output, exit code, timing, and errors for a shell executionId returned by exec or startExecution."
            .to_string(),
        enabled: true,
        input_schema: shell_execution_id_schema(),
        binding: MethodBinding::ShellCommand(ShellCommandBinding {
            root_dir: ".".to_string(),
            allow_commands: default_shell_exec_allow_commands(),
            default_timeout_secs: Some(default_timeout_secs()),
            max_timeout_secs: Some(default_max_timeout_secs()),
        }),
    }
}

fn default_shell_cancel_execution_method() -> MethodConfig {
    MethodConfig {
        name: "cancelExecution".to_string(),
        description: "Request cancellation for a running shell executionId.".to_string(),
        enabled: true,
        input_schema: shell_execution_id_schema(),
        binding: MethodBinding::ShellCommand(ShellCommandBinding {
            root_dir: ".".to_string(),
            allow_commands: default_shell_exec_allow_commands(),
            default_timeout_secs: Some(default_timeout_secs()),
            max_timeout_secs: Some(default_max_timeout_secs()),
        }),
    }
}

fn default_shell_exec_allow_commands() -> Vec<String> {
    default_shell_exec_core_commands()
        .into_iter()
        .chain(default_shell_exec_runtime_commands())
        .map(str::to_string)
        .collect()
}

fn default_shell_exec_core_commands() -> Vec<&'static str> {
    vec![
        "cmd",
        "powershell",
        "pwsh",
        "sh",
        "bash",
        "echo",
        "pwd",
        "ls",
        "git",
    ]
}

fn default_shell_exec_runtime_commands() -> Vec<&'static str> {
    vec![
        "node", "npm", "npx", "pnpm", "yarn", "python3", "python", "pip3", "pip", "uv",
    ]
}

fn ensure_default_computer_methods(config: &mut AgentConfig) -> bool {
    let default_service = default_computer_service();
    let default_names: BTreeSet<String> = default_service
        .methods
        .iter()
        .map(|method| method.name.clone())
        .collect();

    if let Some(service) = config
        .services
        .iter_mut()
        .find(|service| service.name == "computer")
    {
        let existing_names: BTreeSet<String> = service
            .methods
            .iter()
            .map(|method| method.name.clone())
            .collect();
        let mut changed = false;

        for method in default_service.methods {
            if !existing_names.contains(&method.name) {
                service.methods.push(method);
                changed = true;
            }
        }

        if service.description.trim().is_empty() {
            service.description = default_service.description;
            changed = true;
        }

        if !service
            .methods
            .iter()
            .any(|method| default_names.contains(&method.name))
        {
            service.enabled = true;
            changed = true;
        }

        return changed;
    }

    config.services.insert(0, default_service);
    true
}

fn ensure_default_shell_service(config: &mut AgentConfig) -> bool {
    if config
        .services
        .iter()
        .any(|service| service.name == "shell")
    {
        return ensure_shell_service_methods(config, "shell", default_shell_service());
    }

    let insert_index = config
        .services
        .iter()
        .position(|service| service.name == "computer")
        .map(|index| index + 1)
        .unwrap_or(0);
    config
        .services
        .insert(insert_index, default_shell_service());
    true
}

fn ensure_shell_service_methods(
    config: &mut AgentConfig,
    service_name: &str,
    default_service: ServiceConfig,
) -> bool {
    if let Some(service) = config
        .services
        .iter_mut()
        .find(|service| service.name == service_name)
    {
        let mut changed = false;

        if service.description.trim().is_empty() {
            service.description = default_service.description;
            changed = true;
        }

        let existing_names = service
            .methods
            .iter()
            .map(|method| method.name.clone())
            .collect::<BTreeSet<_>>();
        for default_method in default_service.methods {
            if !existing_names.contains(&default_method.name) {
                service.methods.push(default_method);
                changed = true;
            }
        }

        return changed;
    }

    false
}

fn ensure_service_registration_defaults(config: &mut AgentConfig) -> bool {
    if !config.runtime.service_registration_enabled {
        return false;
    }

    if let Ok(bind) = config.runtime.event_server_bind.parse::<SocketAddr>() {
        if !bind.ip().is_loopback() {
            config.runtime.service_registration_enabled = false;
            return true;
        }
    }

    if config
        .runtime
        .service_registration_token
        .as_deref()
        .map(str::trim)
        .unwrap_or_default()
        .is_empty()
    {
        config.runtime.service_registration_token = Some(generate_registration_token());
        return true;
    }

    false
}

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

fn default_platform_config() -> PlatformConfig {
    PlatformConfig {
        base_url: DEFAULT_PLATFORM_BASE_URL.to_string(),
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
    let Some(host) = url.host_str() else {
        return None;
    };
    let host = host.trim_start_matches("www.");
    if host != "baijimu.com" {
        return None;
    }
    if !matches!(url.scheme(), "https" | "http") {
        return None;
    }

    let path = url.path().trim_end_matches('/');
    match path {
        "" | "/" | "/lowcode" | "/manager" => Some(DEFAULT_PLATFORM_BASE_URL.to_string()),
        "/lowcode3" => Some(DEFAULT_PLATFORM_BASE_URL.to_string()),
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
    Ok(dirs.config_dir().join(DEFAULT_CONFIG_FILE_NAME))
}

#[cfg(test)]
mod tests {
    use super::{
        browser_auth_manifest_json, default_shell_exec_allow_commands,
        ensure_browser_auth_agent_id, format_default_device_name, load_config,
        manifest_preview_json, reset_invalid_config, save_config, AgentConfig, EventConfig,
        HttpBinding, MethodBinding, MethodConfig, ServiceConfig, ServiceHealthCheck,
        ServiceRegistration, ServiceStartCommand,
    };
    use serde_json::json;
    use std::collections::BTreeMap;
    use std::fs;
    use tempfile::tempdir;

    fn assert_generated_agent_id(agent_id: &str) {
        assert!(agent_id.starts_with("dev_"));
        assert_ne!(agent_id, "devbox");
    }

    #[test]
    fn example_config_is_valid() {
        AgentConfig::example().validate().unwrap();
    }

    #[test]
    fn default_device_name_uses_user_and_host() {
        assert_eq!(
            format_default_device_name(Some("alice"), Some("workstation-1")),
            "alice@workstation-1"
        );
        assert_eq!(
            format_default_device_name(Some("alice"), Some("alice")),
            "alice"
        );
        assert_eq!(format_default_device_name(Some("alice"), None), "alice");
        assert_eq!(
            format_default_device_name(None, Some("workstation-1")),
            "workstation-1"
        );
    }

    #[test]
    fn config_round_trip() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("agent-config.json");
        let config = AgentConfig::example();
        save_config(&path, &config).unwrap();
        let loaded = load_config(&path).unwrap();
        assert_eq!(loaded.relay.agent_id, config.relay.agent_id);
        assert_generated_agent_id(&loaded.relay.agent_id);
        assert_eq!(
            loaded.upload.prepare_url(&loaded.relay).as_deref(),
            Some("https://relay.baijimu.com/api/bridge-agent/uploads/prepare")
        );
        assert_eq!(loaded.services.len(), 2);
        assert!(loaded.services.iter().any(|service| service.name == "shell"
            && service.methods.iter().any(|method| method.name == "exec")));
        let shell_methods = loaded
            .services
            .iter()
            .find(|service| service.name == "shell")
            .unwrap()
            .methods
            .iter()
            .map(|method| method.name.as_str())
            .collect::<Vec<_>>();
        for method in [
            "exec",
            "startExecution",
            "queryExecution",
            "cancelExecution",
        ] {
            assert!(shell_methods.contains(&method));
        }
    }

    #[test]
    fn save_config_preserves_last_config_backup() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("agent-config.json");
        let mut config = AgentConfig::example();
        config.device.name = "first".to_string();
        save_config(&path, &config).unwrap();

        config.device.name = "second".to_string();
        save_config(&path, &config).unwrap();

        let backup_path = path.with_file_name("agent-config.json.bak");
        let backup = fs::read_to_string(backup_path).unwrap();
        assert!(backup.contains("\"name\": \"first\""));
        let current = load_config(&path).unwrap();
        assert_eq!(current.device.name, "second");
    }

    #[test]
    fn load_config_migrates_legacy_default_device_name_when_identity_is_available() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("agent-config.json");
        let mut config = AgentConfig::example();
        config.device.name = "我的百积木".to_string();
        save_config(&path, &config).unwrap();

        let loaded = load_config(&path).unwrap();

        if loaded.device.name != "我的百积木" {
            assert!(!loaded.device.name.trim().is_empty());
        }
    }

    #[test]
    fn load_config_preserves_custom_device_name() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("agent-config.json");
        let mut config = AgentConfig::example();
        config.device.name = "会议室开发机".to_string();
        save_config(&path, &config).unwrap();

        let loaded = load_config(&path).unwrap();

        assert_eq!(loaded.device.name, "会议室开发机");
    }

    #[test]
    fn reset_invalid_config_archives_bad_file_and_recreates_default() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("agent-config.json");
        fs::write(&path, "{").unwrap();

        let recovery = reset_invalid_config(&path).unwrap();

        let archived_path = recovery.archived_path.unwrap();
        assert!(archived_path.exists());
        assert_eq!(fs::read_to_string(archived_path).unwrap(), "{");
        assert_generated_agent_id(&recovery.config.relay.agent_id);
        let loaded = load_config(&path).unwrap();
        assert_eq!(loaded.services.len(), 2);
    }

    #[test]
    fn example_config_contains_only_builtin_services() {
        let config = AgentConfig::example();
        let service_names = config
            .services
            .iter()
            .map(|service| service.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(service_names, vec!["computer", "shell"]);
    }

    #[test]
    fn load_config_removes_legacy_disabled_local_java_example() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("agent-config.json");
        let mut config = AgentConfig::example();
        config
            .services
            .push(legacy_default_local_java_service(false));

        fs::write(&path, serde_json::to_string_pretty(&config).unwrap()).unwrap();
        let loaded = load_config(&path).unwrap();

        assert!(!loaded
            .services
            .iter()
            .any(|service| service.name == "local-java-service"));
        let migrated = fs::read_to_string(&path).unwrap();
        assert!(!migrated.contains("local-java-service"));
    }

    #[test]
    fn load_config_keeps_user_modified_local_java_service() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("agent-config.json");
        let mut config = AgentConfig::example();
        config
            .services
            .push(legacy_default_local_java_service(true));

        fs::write(&path, serde_json::to_string_pretty(&config).unwrap()).unwrap();
        let loaded = load_config(&path).unwrap();

        assert!(loaded
            .services
            .iter()
            .any(|service| service.name == "local-java-service"));
    }

    fn legacy_default_local_java_service(enabled: bool) -> ServiceConfig {
        ServiceConfig {
            name: "local-java-service".to_string(),
            description: "Example business service backed by a local HTTP endpoint.".to_string(),
            enabled,
            health_check: None,
            start_command: None,
            stop_command: None,
            methods: vec![MethodConfig {
                name: "invokeApi".to_string(),
                description: "Forward invocation arguments to a local HTTP service.".to_string(),
                enabled: true,
                input_schema: super::default_object_schema(),
                binding: MethodBinding::Http(HttpBinding {
                    url: "http://127.0.0.1:8081/api/invoke".to_string(),
                    http_method: "POST".to_string(),
                    headers: BTreeMap::new(),
                    timeout_secs: Some(20),
                }),
            }],
            events: vec![EventConfig {
                name: "jobFinished".to_string(),
                description: "Emitted when the local service completes an asynchronous job."
                    .to_string(),
                enabled: true,
                payload_schema: super::default_object_schema(),
            }],
        }
    }

    #[test]
    fn default_shell_exec_allowlist_includes_common_runtimes() {
        let allowlist = default_shell_exec_allow_commands();
        for command in ["node", "npm", "npx", "python3", "python", "pip3", "uv"] {
            assert!(allowlist.contains(&command.to_string()));
        }
    }

    #[test]
    fn upload_prepare_url_prefers_explicit_value() {
        let mut config = AgentConfig::example();
        config.upload.prepare_url = Some("https://uploads.example.com/prepare".to_string());
        assert_eq!(
            config.upload.prepare_url(&config.relay).as_deref(),
            Some("https://uploads.example.com/prepare")
        );
    }

    #[test]
    fn manifest_preview_contains_enabled_service_only() {
        let payload = manifest_preview_json(&AgentConfig::example()).unwrap();
        assert!(payload.contains("\"computer\""));
        assert!(payload.contains("\"shell\""));
        assert!(!payload.contains("\"local-java-service\""));
    }

    #[test]
    fn browser_auth_manifest_omits_schemas() {
        let mut config = AgentConfig::example();
        config.services.push(ServiceConfig {
            name: "eventful".to_string(),
            description: "Eventful service".to_string(),
            enabled: true,
            health_check: None,
            start_command: None,
            stop_command: None,
            methods: vec![MethodConfig {
                name: "doThing".to_string(),
                description: "Does a thing".to_string(),
                enabled: true,
                input_schema: json!({
                    "type": "object",
                    "required": ["large"],
                    "properties": {
                        "large": {
                            "type": "string",
                            "description": "schema-only text"
                        }
                    }
                }),
                binding: MethodBinding::Http(HttpBinding {
                    url: "http://127.0.0.1:18000/do-thing".to_string(),
                    http_method: "POST".to_string(),
                    headers: BTreeMap::new(),
                    timeout_secs: Some(20),
                }),
            }],
            events: vec![EventConfig {
                name: "thingDone".to_string(),
                description: "Thing completed".to_string(),
                enabled: true,
                payload_schema: json!({
                    "type": "object",
                    "properties": {
                        "result": {
                            "type": "string",
                            "description": "payload-only text"
                        }
                    }
                }),
            }],
        });

        let payload = browser_auth_manifest_json(&config).unwrap();
        let value: serde_json::Value = serde_json::from_str(&payload).unwrap();
        let service = value["services"]
            .as_array()
            .unwrap()
            .iter()
            .find(|service| service["name"] == "eventful")
            .unwrap();

        assert_eq!(service["methods"][0]["name"], "doThing");
        assert_eq!(service["events"][0]["name"], "thingDone");
        assert!(service["methods"][0].get("input_schema").is_none());
        assert!(service["events"][0].get("payload_schema").is_none());
        assert!(!payload.contains("schema-only text"));
        assert!(!payload.contains("payload-only text"));
    }

    #[test]
    fn shell_manifest_exposes_single_argv_command_schema() {
        let manifest = AgentConfig::example().manifest_preview();
        let shell_method = manifest
            .services
            .iter()
            .find(|service| service.name == "shell")
            .and_then(|service| service.methods.iter().find(|method| method.name == "exec"))
            .unwrap();
        let command_schema = &shell_method.input_schema["properties"]["command"];
        assert_eq!(command_schema["type"], "array");
        assert_eq!(command_schema["items"]["type"], "string");
        assert!(command_schema.get("anyOf").is_none());

        let query_execution_method = manifest
            .services
            .iter()
            .find(|service| service.name == "shell")
            .and_then(|service| {
                service
                    .methods
                    .iter()
                    .find(|method| method.name == "queryExecution")
            })
            .unwrap();
        assert_eq!(
            query_execution_method.input_schema["properties"]["executionId"]["type"],
            "string"
        );
    }

    #[test]
    fn event_only_service_is_exposed_in_manifest() {
        let mut config = AgentConfig::example();
        config.services.push(ServiceConfig {
            name: "asyncJob".to_string(),
            description: "Async job events.".to_string(),
            enabled: true,
            health_check: None,
            start_command: None,
            stop_command: None,
            methods: Vec::new(),
            events: vec![EventConfig {
                name: "finished".to_string(),
                description: "Job finished.".to_string(),
                enabled: true,
                payload_schema: json!({
                    "type": "object",
                    "properties": {
                        "jobId": { "type": "string" }
                    }
                }),
            }],
        });

        let manifest = config.manifest_preview();
        let service = manifest
            .services
            .iter()
            .find(|service| service.name == "asyncJob")
            .unwrap();
        assert!(service.methods.is_empty());
        assert_eq!(service.events[0].name, "finished");
    }

    #[test]
    fn public_service_registration_builds_http_service_config() {
        let registration: ServiceRegistration = serde_json::from_value(json!({
            "name": "reportTool",
            "description": "AI generated report service.",
            "transport": {
                "type": "http",
                "baseUrl": "http://127.0.0.1:39127/api/",
                "headers": {
                    "x-tool": "report"
                }
            },
            "healthCheck": {
                "type": "http",
                "path": "/health",
                "timeoutSecs": 2,
                "expectStatus": 200
            },
            "startCommand": {
                "type": "shell_command",
                "command": ["report-tool", "start"],
                "timeoutSecs": 10
            },
            "methods": [
                {
                    "name": "generate",
                    "description": "Generate a report.",
                    "path": "/invoke/generate",
                    "timeoutSecs": 60,
                    "input_schema": {
                        "type": "object",
                        "additionalProperties": true
                    }
                }
            ],
            "events": [
                {
                    "name": "finished",
                    "description": "Report generation finished."
                }
            ],
            "replace": true
        }))
        .unwrap();

        let replace = registration.replace;
        let service = registration.into_service_config().unwrap();
        assert!(replace);
        assert_eq!(service.name, "reportTool");
        assert_eq!(service.events[0].name, "finished");
        match service.health_check.as_ref().unwrap() {
            ServiceHealthCheck::Http {
                url,
                timeout_secs,
                expect_status,
                ..
            } => {
                assert_eq!(url, "http://127.0.0.1:39127/api/health");
                assert_eq!(*timeout_secs, Some(2));
                assert_eq!(*expect_status, Some(200));
            }
        }
        match service.start_command.as_ref().unwrap() {
            ServiceStartCommand::ShellCommand {
                command,
                timeout_secs,
                ..
            } => {
                assert_eq!(
                    command,
                    &vec!["report-tool".to_string(), "start".to_string()]
                );
                assert_eq!(*timeout_secs, Some(10));
            }
        }
        match &service.methods[0].binding {
            MethodBinding::Http(binding) => {
                assert_eq!(binding.url, "http://127.0.0.1:39127/api/invoke/generate");
                assert_eq!(binding.http_method, "POST");
                assert_eq!(binding.timeout_secs, Some(60));
                assert_eq!(binding.headers.get("x-tool").unwrap(), "report");
            }
            other => panic!("unexpected binding: {other:?}"),
        }
    }

    #[test]
    fn load_legacy_config_without_platform() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("agent-config.json");
        fs::write(
            &path,
            r#"{
  "upload": {
    "prepare_url": null,
    "inline_limit_bytes": 8388608,
    "timeout_secs": 60
  },
  "relay": {
    "url": "ws://127.0.0.1:8080/ws/agent",
    "agent_id": "devbox",
    "token": "",
    "reconnect_secs": 3
  },
  "device": {
    "name": "我的百积木",
    "description": "Installed on the user's local machine.",
    "tags": ["desktop", "local"]
  },
  "runtime": {
    "default_timeout_secs": 30,
    "max_timeout_secs": 120,
    "log_limit": 500
  },
  "services": []
}"#,
        )
        .unwrap();

        let loaded = load_config(&path).unwrap();
        assert_eq!(loaded.platform.base_url, "https://baijimu.com/lowcode3");
        assert_eq!(loaded.platform.workspace_id, None);
        assert_eq!(loaded.relay.url, "wss://relay.baijimu.com/ws/agent");
        assert_eq!(loaded.upload.inline_limit_bytes, 256 * 1024);
        assert_eq!(loaded.relay.agent_id, "devbox");
        assert!(loaded.runtime.service_registration_enabled);
        assert!(loaded
            .runtime
            .service_registration_token
            .as_deref()
            .is_some_and(|token| !token.trim().is_empty()));
        let migrated = fs::read_to_string(&path).unwrap();
        assert!(migrated.contains("service_registration_token"));
        assert_eq!(loaded.services[0].name, "computer");
        assert!(loaded.services[0]
            .methods
            .iter()
            .any(|method| method.name == "screenshot"));
        assert!(loaded
            .services
            .iter()
            .any(|service| service.name == "shell"));
    }

    #[test]
    fn normalizes_common_baijimu_platform_entrypoints() {
        for legacy_url in [
            "https://baijimu.com",
            "https://www.baijimu.com/",
            "https://baijimu.com/lowcode",
            "https://baijimu.com/manager",
            "https://www.baijimu.com/lowcode3/",
        ] {
            let dir = tempdir().unwrap();
            let path = dir.path().join("agent-config.json");
            let mut config = AgentConfig::example();
            config.platform.base_url = legacy_url.to_string();
            save_config(&path, &config).unwrap();

            let loaded = load_config(&path).unwrap();
            assert_eq!(
                loaded.platform.base_url, "https://baijimu.com/lowcode3",
                "legacy url {legacy_url} should normalize to the production API prefix"
            );
        }
    }

    #[test]
    fn leaves_custom_platform_base_url_unchanged() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("agent-config.json");
        let mut config = AgentConfig::example();
        config.platform.base_url = "https://dev.baijimu.test/lowcode3".to_string();
        save_config(&path, &config).unwrap();

        let loaded = load_config(&path).unwrap();
        assert_eq!(
            loaded.platform.base_url,
            "https://dev.baijimu.test/lowcode3"
        );
    }

    #[test]
    fn non_loopback_event_server_disables_service_registration_migration() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("agent-config.json");
        let mut config = AgentConfig::example();
        config.runtime.event_server_bind = "0.0.0.0:18081".to_string();
        config.runtime.event_server_token = Some("event-secret".to_string());
        config.runtime.service_registration_token = None;
        save_config(&path, &config).unwrap();

        let loaded = load_config(&path).unwrap();
        assert!(!loaded.runtime.service_registration_enabled);
    }

    #[test]
    fn load_legacy_computer_service_adds_missing_default_methods() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("agent-config.json");
        fs::write(
            &path,
            r#"{
  "platform": {
    "base_url": "https://baijimu.com/lowcode3",
    "workspace_id": null
  },
  "upload": {
    "prepare_url": null,
    "inline_limit_bytes": 8388608,
    "timeout_secs": 60
  },
  "relay": {
    "url": "wss://relay.baijimu.com/ws/agent",
    "agent_id": "devbox",
    "token": "",
    "reconnect_secs": 3
  },
  "device": {
    "name": "我的百积木",
    "description": "Installed on the user's local machine.",
    "tags": ["desktop", "local"]
  },
  "runtime": {
    "default_timeout_secs": 30,
    "max_timeout_secs": 120,
    "log_limit": 500
  },
  "services": [
    {
      "name": "computer",
      "description": "legacy",
      "enabled": true,
      "methods": [
        {
          "name": "exec",
          "description": "legacy shell",
          "enabled": true,
          "input_schema": {
            "type": "object"
          },
          "binding": {
            "type": "shell_command",
            "root_dir": ".",
            "allow_commands": ["echo"]
          }
        }
      ]
    }
  ]
}"#,
        )
        .unwrap();

        let loaded = load_config(&path).unwrap();
        let computer = loaded
            .services
            .iter()
            .find(|service| service.name == "computer")
            .unwrap();
        assert!(computer.methods.iter().any(|method| method.name == "exec"));
        assert!(computer
            .methods
            .iter()
            .any(|method| method.name == "screenshot"));
        assert!(computer.methods.iter().any(|method| method.name == "click"));
    }

    #[test]
    fn browser_auth_migrates_legacy_default_agent_id() {
        let mut config = AgentConfig::example();
        config.relay.agent_id = "devbox".to_string();

        let changed = ensure_browser_auth_agent_id(&mut config);

        assert!(changed);
        assert_generated_agent_id(&config.relay.agent_id);
    }

    #[test]
    fn browser_auth_keeps_existing_custom_agent_id() {
        let mut config = AgentConfig::example();
        config.relay.agent_id = "dev_my_custom_box".to_string();

        let changed = ensure_browser_auth_agent_id(&mut config);

        assert!(!changed);
        assert_eq!(config.relay.agent_id, "dev_my_custom_box");
    }
}

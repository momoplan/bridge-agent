use crate::protocol::{MethodDefinition, ServiceDefinition};
use anyhow::{bail, Context, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

const DEFAULT_RELAY_URL: &str = "ws://127.0.0.1:8080/ws/agent";
const DEFAULT_CONFIG_FILE_NAME: &str = "agent-config.json";

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
    #[serde(default = "default_timeout_secs")]
    pub default_timeout_secs: u64,
    #[serde(default = "default_max_timeout_secs")]
    pub max_timeout_secs: u64,
    #[serde(default = "default_log_limit")]
    pub log_limit: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceConfig {
    pub name: String,
    pub description: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    pub methods: Vec<MethodConfig>,
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

impl AgentConfig {
    pub fn example() -> Self {
        Self {
            platform: PlatformConfig {
                base_url: "https://baijimu.com/lowcode3".to_string(),
                workspace_id: None,
            },
            upload: UploadConfig::default(),
            relay: RelayConfig {
                url: DEFAULT_RELAY_URL.to_string(),
                agent_id: "devbox".to_string(),
                token: String::new(),
                reconnect_secs: default_reconnect_secs(),
            },
            device: DeviceConfig {
                name: "My Bridge Agent".to_string(),
                description: "Installed on the user's local machine.".to_string(),
                tags: vec!["desktop".to_string(), "local".to_string()],
            },
            runtime: RuntimeConfig {
                default_timeout_secs: default_timeout_secs(),
                max_timeout_secs: default_max_timeout_secs(),
                log_limit: default_log_limit(),
            },
            services: vec![
                default_computer_service(),
                ServiceConfig {
                    name: "local-java-service".to_string(),
                    description: "Example business service backed by a local HTTP endpoint."
                        .to_string(),
                    enabled: false,
                    methods: vec![MethodConfig {
                        name: "invokeApi".to_string(),
                        description: "Forward invocation arguments to a local HTTP service."
                            .to_string(),
                        enabled: true,
                        input_schema: default_object_schema(),
                        binding: MethodBinding::Http(HttpBinding {
                            url: "http://127.0.0.1:8081/api/invoke".to_string(),
                            http_method: "POST".to_string(),
                            headers: BTreeMap::new(),
                            timeout_secs: Some(20),
                        }),
                    }],
                },
            ],
        }
    }

    pub fn normalize(&mut self) -> bool {
        ensure_default_computer_methods(self)
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
        if self.runtime.default_timeout_secs > self.runtime.max_timeout_secs {
            bail!("runtime.default_timeout_secs cannot exceed runtime.max_timeout_secs");
        }
        if self.runtime.log_limit == 0 {
            bail!("runtime.log_limit must be greater than zero");
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
            })
            .filter(|service| !service.methods.is_empty())
            .collect()
    }

    pub fn manifest_preview(&self) -> ManifestPreview {
        ManifestPreview {
            device: self.device.clone(),
            services: self.service_definitions(),
        }
    }
}

pub fn default_config_path() -> Result<PathBuf> {
    if let Some(path) = config_path_override_from_env() {
        return Ok(path);
    }

    #[cfg(windows)]
    if let Some(path) = windows_service_config_path().filter(|path| path.exists()) {
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
    config.normalize();
    config.validate()?;
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
    fs::write(path, format!("{content}\n"))
        .with_context(|| format!("failed to write config {}", path.display()))?;
    Ok(())
}

pub fn ensure_config_exists(path: &Path) -> Result<()> {
    if !path.exists() {
        save_config(path, &AgentConfig::example())?;
    }
    Ok(())
}

pub fn manifest_preview_json(config: &AgentConfig) -> Result<String> {
    Ok(serde_json::to_string_pretty(&config.manifest_preview())?)
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
                "type": "array",
                "items": {"type": "string"},
                "minItems": 1
            },
            "cwd": {"type": "string"},
            "env": {
                "type": "object",
                "additionalProperties": {"type": "string"}
            }
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
    }
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

fn default_enabled() -> bool {
    true
}

fn default_timeout_secs() -> u64 {
    30
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

fn default_http_method() -> String {
    "POST".to_string()
}

fn default_platform_config() -> PlatformConfig {
    PlatformConfig {
        base_url: "https://baijimu.com/lowcode3".to_string(),
        workspace_id: None,
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
    8 * 1024 * 1024
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
    use super::{load_config, manifest_preview_json, save_config, AgentConfig};
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn example_config_is_valid() {
        AgentConfig::example().validate().unwrap();
    }

    #[test]
    fn config_round_trip() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("agent-config.json");
        let config = AgentConfig::example();
        save_config(&path, &config).unwrap();
        let loaded = load_config(&path).unwrap();
        assert_eq!(loaded.relay.agent_id, "devbox");
        assert_eq!(
            loaded.upload.prepare_url(&loaded.relay).as_deref(),
            Some("http://127.0.0.1:8080/api/bridge-agent/uploads/prepare")
        );
        assert_eq!(loaded.services.len(), 2);
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
        assert!(!payload.contains("\"local-java-service\""));
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
    "name": "My Bridge Agent",
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
        assert_eq!(loaded.upload.inline_limit_bytes, 8 * 1024 * 1024);
        assert_eq!(loaded.services[0].name, "computer");
        assert!(loaded.services[0]
            .methods
            .iter()
            .any(|method| method.name == "screenshot"));
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
    "url": "ws://127.0.0.1:8080/ws/agent",
    "agent_id": "devbox",
    "token": "",
    "reconnect_secs": 3
  },
  "device": {
    "name": "My Bridge Agent",
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
}

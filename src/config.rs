use crate::protocol::{MethodDefinition, ServiceDefinition};
use anyhow::{bail, Context, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

const DEFAULT_RELAY_URL: &str = "ws://127.0.0.1:8080/ws/agent";
const DEFAULT_CONFIG_FILE_NAME: &str = "agent-config.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    #[serde(default = "default_platform_config")]
    pub platform: PlatformConfig,
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
                ServiceConfig {
                    name: "computer".to_string(),
                    description: "Computer control operations exposed as business methods."
                        .to_string(),
                    enabled: true,
                    methods: vec![MethodConfig {
                        name: "exec".to_string(),
                        description: "Run one allowlisted command with optional cwd and env."
                            .to_string(),
                        enabled: true,
                        input_schema: shell_input_schema(),
                        binding: MethodBinding::ShellCommand(ShellCommandBinding {
                            root_dir: ".".to_string(),
                            allow_commands: vec![
                                "echo".to_string(),
                                "pwd".to_string(),
                                "ls".to_string(),
                                "git".to_string(),
                            ],
                            default_timeout_secs: Some(30),
                            max_timeout_secs: Some(120),
                        }),
                    }],
                },
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

    pub fn validate(&self) -> Result<()> {
        if self.platform.base_url.trim().is_empty() {
            bail!("platform.base_url cannot be empty");
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
    let dirs = ProjectDirs::from("com", "baijimu", "bridge-agent")
        .context("failed to determine config directory")?;
    Ok(dirs.config_dir().join(DEFAULT_CONFIG_FILE_NAME))
}

pub fn load_config(path: &Path) -> Result<AgentConfig> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("failed to read config {}", path.display()))?;
    let config: AgentConfig = serde_json::from_str(&content)
        .with_context(|| format!("failed to parse config {}", path.display()))?;
    config.validate()?;
    Ok(config)
}

pub fn save_config(path: &Path, config: &AgentConfig) -> Result<()> {
    config.validate()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create config dir {}", parent.display()))?;
    }
    let content = serde_json::to_string_pretty(config)?;
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
        assert_eq!(loaded.services.len(), 2);
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
    }
}

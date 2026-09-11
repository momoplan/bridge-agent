use crate::protocol::{
    EventDefinition, LocalAppDefinition, MethodDefinition, ResponseMode, ServiceDefinition,
};
use crate::secret_store::{delete_relay_token, load_relay_token, store_relay_token};
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
const DEFAULT_PLATFORM_BASE_URL: &str = "https://api.baijimu.com/lowcode3";
const DEFAULT_CONFIG_FILE_NAME: &str = "agent-config.json";
const DEVELOPMENT_CONFIG_FILE_NAME: &str = "agent-config.development.json";
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
    #[serde(default)]
    pub services: Vec<ServiceConfig>,
    #[serde(default)]
    pub local_apps: Vec<LocalAppConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformConfig {
    #[serde(default)]
    pub environment_key: Option<String>,
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
    #[serde(default)]
    pub token_issued_at_epoch_seconds: Option<String>,
    #[serde(default)]
    pub token_expires_at_epoch_seconds: Option<String>,
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
    pub python_path: Option<String>,
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalAppConfig {
    pub app_id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
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
    #[serde(default, rename = "responseMode", alias = "response_mode")]
    pub response_mode: ResponseMode,
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
    #[serde(rename = "localApps")]
    pub local_apps: Vec<LocalAppDefinition>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BrowserAuthManifestPreview {
    pub device: DeviceConfig,
    pub services: Vec<BrowserAuthServiceDefinition>,
    #[serde(rename = "localApps")]
    pub local_apps: Vec<BrowserAuthLocalAppDefinition>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserAuthLocalAppDefinition {
    pub app_id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub methods: Vec<BrowserAuthMethodDefinition>,
    #[serde(default)]
    pub events: Vec<BrowserAuthEventDefinition>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BrowserAuthServiceDefinition {
    pub name: String,
    pub description: String,
    pub methods: Vec<BrowserAuthMethodDefinition>,
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

include!("config/agent_config.rs");
include!("config/local_app_validation.rs");
include!("config/service_registration.rs");
include!("config/identity_and_paths.rs");
include!("config/persistence.rs");
include!("config/legacy_migrations.rs");
include!("config/manifest_and_defaults.rs");
include!("config/service_reconciliation.rs");
include!("config/default_values.rs");

#[cfg(test)]
mod tests {
    use super::{
        browser_auth_manifest_json, config_file_name_for_build, default_shell_exec_allow_commands,
        ensure_browser_auth_agent_id, format_default_device_name, load_config,
        manifest_preview_json, reset_invalid_config, save_config, AgentConfig, HttpBinding,
        MethodBinding, MethodConfig, ServiceConfig, ServiceHealthCheck, ServiceRegistration,
        ServiceStartCommand,
    };
    use crate::protocol::ResponseMode;
    use serde_json::json;
    use std::collections::BTreeMap;
    use std::fs;
    use tempfile::tempdir;

    include!("config/tests/core.rs");
    include!("config/tests/legacy.rs");
    include!("config/tests/manifest.rs");
    include!("config/tests/registration.rs");
    include!("config/tests/migrations.rs");
    include!("config/tests/identity.rs");
}

pub mod environment;

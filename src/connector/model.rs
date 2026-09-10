const CONNECTOR_MANIFEST_FILE: &str = "connector.json";
const CONNECTOR_INSTALL_RECORD_FILE: &str = "install.json";
const CONNECTOR_PYTHON_ENV_DIR: &str = ".bridge-agent-python";
const CONNECTOR_PYTHON_ENV_MARKER: &str = ".install-ok";
const LOCAL_APP_ID_ENV: &str = "BAIJIMU_LOCAL_APP_ID";
const LOCAL_APP_DATA_DIR_ENV: &str = "BAIJIMU_LOCAL_APP_DATA_DIR";
const LOCAL_APP_START_POLICY_ENV: &str = "BAIJIMU_LOCAL_APP_START_POLICY";
const CONNECTOR_MANAGEMENT_TOKEN_FILE: &str = "management-token";
const LOCAL_APP_TOKEN_FILE_ENV: &str = "BAIJIMU_LOCAL_APP_TOKEN_FILE";
const CONNECTOR_EVENT_TOKEN_FILE: &str = "event-publisher-token";
const CONNECTOR_ASSET_UPLOAD_TOKEN_FILE: &str = "asset-upload-token";
const LOCAL_APP_EVENT_TOKEN_FILE_ENV: &str = "BAIJIMU_LOCAL_APP_EVENT_TOKEN_FILE";
const LOCAL_APP_EVENT_ENDPOINT_ENV: &str = "BAIJIMU_LOCAL_APP_EVENT_ENDPOINT";
const LOCAL_APP_ASSET_UPLOAD_TOKEN_FILE_ENV: &str = "BAIJIMU_LOCAL_APP_ASSET_UPLOAD_TOKEN_FILE";
const LOCAL_APP_ASSET_UPLOAD_ENDPOINT_ENV: &str = "BAIJIMU_LOCAL_APP_ASSET_UPLOAD_ENDPOINT";
const DEFAULT_LOCAL_APP_EVENT_ENDPOINT: &str = "http://127.0.0.1:18081/v1/local-app-events";
const DEFAULT_LOCAL_APP_ASSET_UPLOAD_ENDPOINT: &str = "http://127.0.0.1:18081/v1/local-app-assets";
pub(crate) const CONNECTOR_ASSET_UPLOAD_PERMISSION: &str = "assets.upload";
const HOST_MANAGED_PROCESS_MINIMUM_VERSION: &str = "0.2.40";
const HOST_MANAGED_PROCESS_CAPABILITY: &str = "connector.process.host-managed.v1";
const MANAGED_TOOL_DEPENDENCIES_CAPABILITY: &str = "connector.managed-tool-dependencies.v1";
const CONNECTOR_ICON_CAPABILITY: &str = "connector.presentation.icon.v1";
const CONNECTOR_ICON_MEDIA_TYPE: &str = "image/png";
const CONNECTOR_ICON_EDGE_PX: u32 = 256;
const CONNECTOR_ICON_MAX_BYTES: usize = 128 * 1024;
const CONNECTOR_HOST_CAPABILITIES: &[&str] = &[
    "connector.setup.v1",
    "connector.asset-upload.v1",
    HOST_MANAGED_PROCESS_CAPABILITY,
    MANAGED_TOOL_DEPENDENCIES_CAPABILITY,
    CONNECTOR_ICON_CAPABILITY,
];
const LIFECYCLE_OUTPUT_MAX_BYTES: u64 = 1024 * 1024;
#[cfg(windows)]
const WINDOWS_CREATE_NO_WINDOW: u32 = 0x08000000;

fn configure_connector_command(command: &mut Command) {
    #[cfg(windows)]
    command.creation_flags(WINDOWS_CREATE_NO_WINDOW);
    #[cfg(not(windows))]
    let _ = command;
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PythonRuntimeStatus {
    pub configured_path: Option<String>,
    pub detected_path: Option<String>,
    pub version: Option<String>,
    pub available: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConnectorManifest {
    pub schema_version: String,
    pub app_id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub publisher: Option<ConnectorPublisher>,
    #[serde(default)]
    pub source: Option<ConnectorSource>,
    #[serde(default)]
    pub runtime: Option<ConnectorRuntime>,
    #[serde(default)]
    pub management: Option<ConnectorManagement>,
    #[serde(default)]
    pub setup: Option<ConnectorSetup>,
    #[serde(default)]
    pub host_requirements: Option<ConnectorHostRequirements>,
    #[serde(default)]
    pub managed_tool_dependencies: Vec<ConnectorManagedToolDependency>,
    #[serde(default)]
    pub icon: Option<ConnectorIcon>,
    #[serde(default)]
    pub ui: Option<ConnectorUi>,
    #[serde(default)]
    pub config_schema: Option<Value>,
    #[serde(default)]
    pub upgrade_review: Option<ConnectorUpgradeReview>,
    #[serde(default)]
    pub database: Option<ConnectorDatabaseContract>,
    #[serde(default)]
    pub remote_capabilities: Vec<ConnectorRemoteCapability>,
    #[serde(default)]
    pub permissions: Vec<ConnectorPermission>,
    #[serde(default)]
    pub legacy_autostart_labels: Vec<String>,
    #[serde(default)]
    pub transport: Option<RegistrationTransport>,
    #[serde(default)]
    pub methods: Vec<RegistrationMethod>,
    #[serde(default)]
    pub events: Vec<crate::config::EventConfig>,
    #[serde(default)]
    pub hooks: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConnectorIcon {
    pub media_type: String,
    pub data: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorManagedToolDependency {
    pub id: String,
    pub minimum_version: String,
    pub required_for: Vec<ConnectorManagedToolDependencyPhase>,
    pub executable_path_env: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ConnectorManagedToolDependencyPhase {
    Install,
    Start,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorUi {
    #[serde(rename = "type")]
    pub ui_type: String,
    pub entry: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub default_view: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectorPublisher {
    pub name: String,
    #[serde(default)]
    pub homepage: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectorSource {
    #[serde(rename = "type")]
    pub source_type: String,
    #[serde(default)]
    pub repo: Option<String>,
    #[serde(default)]
    pub revision: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorRuntime {
    #[serde(rename = "type")]
    pub runtime_type: String,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub health_check: Option<RegistrationHealthCheck>,
    #[serde(default)]
    pub stop_args: Vec<String>,
    #[serde(default = "default_connector_start_policy")]
    pub start_policy: String,
    #[serde(default)]
    pub process_ownership: ConnectorProcessOwnership,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectorProcessOwnership {
    #[default]
    Connector,
    Host,
}

fn default_connector_start_policy() -> String {
    "automatic".to_string()
}

pub fn local_app_starts_automatically(app: &LocalAppConfig) -> bool {
    if !app.enabled {
        return false;
    }
    let Some(ServiceStartCommand::ShellCommand { env, .. }) = app.start_command.as_ref() else {
        return false;
    };
    env.get(LOCAL_APP_START_POLICY_ENV)
        .map(String::as_str)
        .unwrap_or("automatic")
        == "automatic"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorPermission {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub platforms: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorUpgradeReview {
    pub configuration: String,
    pub interfaces: String,
    pub database: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorDatabaseContract {
    pub engine: String,
    pub schema_version: String,
    #[serde(default)]
    pub migrations: Vec<ConnectorDatabaseMigration>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorDatabaseMigration {
    pub id: String,
    pub from_version: String,
    pub to_version: String,
    pub description: String,
    #[serde(default)]
    pub changes: Vec<ConnectorDatabaseChange>,
    #[serde(default)]
    pub destructive: bool,
    #[serde(default = "default_database_rollback")]
    pub rollback: String,
    #[serde(default = "default_database_downtime")]
    pub downtime: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorDatabaseChange {
    pub operation: String,
    pub target: String,
    pub description: String,
    #[serde(default)]
    pub destructive: bool,
}

fn default_database_rollback() -> String {
    "not_declared".to_string()
}

fn default_database_downtime() -> String {
    "not_declared".to_string()
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorMethodContract {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    pub response_mode: String,
    pub path: String,
    pub http_method: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorEventContract {
    pub name: String,
    pub description: String,
    pub payload_schema: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorManagement {
    #[serde(rename = "type")]
    pub management_type: String,
    pub base_url: String,
    pub auth: ConnectorManagementAuth,
    #[serde(default)]
    pub operations: BTreeMap<String, ConnectorManagementOperation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectorManagementAuth {
    #[serde(rename = "type")]
    pub auth_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectorManagementOperation {
    pub method: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorSetup {
    pub operation: String,
    pub status_operation: String,
    #[serde(default = "default_connector_setup_timeout_secs")]
    pub timeout_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorHostRequirements {
    #[serde(default)]
    pub minimum_version: Option<String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
}

fn default_connector_setup_timeout_secs() -> u64 {
    1_800
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectorRemoteCapability {
    pub name: String,
    #[serde(default)]
    pub risk: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorInstallRecord {
    #[serde(default)]
    pub install_source: Option<local_app_contract::InstallSource>,
    pub manifest: ConnectorManifest,
    pub package_path: String,
    pub source_path: String,
    #[serde(default)]
    pub source_reference: Option<String>,
    pub review_status: String,
    #[serde(default)]
    pub source_checksum: Option<String>,
    #[serde(default)]
    pub package_checksum: Option<String>,
    pub installed_at_epoch_ms: u64,
    #[serde(default)]
    pub last_synced_at_epoch_ms: u64,
}

#[derive(Debug, Clone)]
pub struct ConnectorInstallProvenance {
    install_source: Option<local_app_contract::InstallSource>,
    source_reference: Option<String>,
    review_status: String,
    source_checksum: Option<String>,
}

impl ConnectorInstallProvenance {
    pub fn frozen_market(selection: local_app_contract::InstallSource, listing: &local_app_contract::MarketListing) -> Result<Self> {
        crate::market_distribution::resolve_exact(&selection, listing)?;
        Ok(Self {
            install_source: Some(selection),
            source_reference: None,
            review_status: "PUBLISHED".into(),
            source_checksum: None,
        })
    }

    pub fn validate_manifest(&self, manifest: &ConnectorManifest) -> Result<()> {
        if let Some(selection) = &self.install_source {
            let source = match selection {
                local_app_contract::InstallSource::Market { source, .. }
                | local_app_contract::InstallSource::Environment { source } => source,
            };
            if source.application.app_id.as_str() != manifest.app_id
                || source.version.to_string() != manifest.version {
                bail!("installed manifest differs from the selected frozen source version");
            }
        }
        Ok(())
    }

    pub fn registered(
        source_reference: &str,
        review_status: &str,
        source_checksum: &str,
    ) -> Result<Self> {
        let source_reference = normalized_optional_text(Some(source_reference))
            .context("registered connector source is required")?;
        let review_status = normalized_optional_text(Some(review_status))
            .context("registered connector review status is required")?;
        let source_checksum = normalize_sha256_checksum(source_checksum)?;
        Ok(Self {
            install_source: None,
            source_reference: Some(source_reference),
            review_status,
            source_checksum: Some(source_checksum),
        })
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorInstallResult {
    pub app_id: String,
    pub name: String,
    pub version: String,
    pub package_path: String,
    pub method_names: Vec<String>,
    pub event_names: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorSummary {
    pub install_source: Option<local_app_contract::InstallSource>,
    pub app_id: String,
    pub name: String,
    pub version: String,
    pub package_path: String,
    pub source_path: String,
    pub source_reference: Option<String>,
    pub review_status: String,
    pub source_checksum: Option<String>,
    pub package_checksum: Option<String>,
    pub icon_data_url: Option<String>,
    pub ui: Option<ConnectorUi>,
    pub permissions: Vec<ConnectorPermission>,
    pub start_policy: String,
    pub process_ownership: ConnectorProcessOwnership,
    pub config_schema: Option<Value>,
    pub database: Option<ConnectorDatabaseContract>,
    pub methods: Vec<ConnectorMethodContract>,
    pub events: Vec<ConnectorEventContract>,
    pub method_names: Vec<String>,
    pub event_names: Vec<String>,
    pub installed_at_epoch_ms: u64,
    pub last_synced_at_epoch_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorSyncFailure {
    pub app_id: String,
    pub name: String,
    pub error: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorSyncReport {
    pub summaries: Vec<ConnectorSummary>,
    pub failures: Vec<ConnectorSyncFailure>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorStartResult {
    pub app_id: String,
    pub lifecycle: ConnectorLifecycleResult,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorLifecycleResult {
    pub app_id: String,
    pub configured: bool,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

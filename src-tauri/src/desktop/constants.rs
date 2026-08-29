use super::*;

pub(super) const UPDATE_USER_AGENT: &str =
    concat!("bridge-agent-desktop/", env!("CARGO_PKG_VERSION"));
pub(super) const CONNECTOR_DOWNLOAD_USER_AGENT: &str = concat!(
    "Baijimu-Connector-Installer/",
    env!("CARGO_PKG_VERSION"),
    " Wget/1.21.4"
);
pub(super) const UPDATE_PROGRESS_EVENT: &str = "app-update-progress";
pub(super) const UNIFIED_APP_ID_MIGRATION_BINARY: &str = "bridge-agent-unified-app-id-migration";
pub(super) const UNIFIED_APP_ID_MIGRATION_LEDGER: &str = "unified-app-id-migration-ledger.json";
pub(super) const RUNTIME_SNAPSHOT_EVENT: &str = "runtime-snapshot-changed";
pub(super) const RUNTIME_LOG_EVENT: &str = "runtime-log-appended";
pub(super) const RUNTIME_LOGS_SNAPSHOT_EVENT: &str = "runtime-logs-snapshot";
pub(super) const MAIN_WINDOW_VISIBILITY_EVENT: &str = "main-window-visibility-changed";
pub(super) const STARTUP_HEALTH_EVENT: &str = "startup-health-changed";
pub(super) const HOST_CAPABILITY_CONNECTOR_SETUP_V1: &str = "connector.setup.v1";
pub(super) const HOST_CAPABILITY_CONNECTOR_PROCESS_HOST_MANAGED_V1: &str =
    "connector.process.host-managed.v1";
pub(super) const HOST_CAPABILITY_CONNECTOR_MANAGED_TOOL_DEPENDENCIES_V1: &str =
    "connector.managed-tool-dependencies.v1";
pub(super) const HOST_CAPABILITY_CONNECTOR_PRESENTATION_ICON_V1: &str =
    "connector.presentation.icon.v1";
pub(super) const LOCAL_APP_HOST_CAPABILITIES: &[&str] = &[
    HOST_CAPABILITY_CONNECTOR_SETUP_V1,
    HOST_CAPABILITY_CONNECTOR_PROCESS_HOST_MANAGED_V1,
    HOST_CAPABILITY_CONNECTOR_MANAGED_TOOL_DEPENDENCIES_V1,
    HOST_CAPABILITY_CONNECTOR_PRESENTATION_ICON_V1,
];
pub(super) const STARTUP_UPDATE_CHECK_TIMEOUT: Duration = Duration::from_secs(12);
pub(super) const STARTUP_UPDATE_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(3);
pub(super) const STARTUP_UPDATE_RETRY_DELAYS: &[Duration] = &[
    Duration::from_millis(250),
    Duration::from_millis(750),
    Duration::from_secs(2),
];
pub(super) const STARTUP_UPDATE_RECOVERY_DELAYS: &[Duration] = &[
    Duration::from_secs(10),
    Duration::from_secs(30),
    Duration::from_secs(60),
    Duration::from_secs(120),
];
pub(super) const UPDATE_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
pub(super) const CONNECTOR_MANIFEST_FILE: &str = "connector.json";
pub(super) const LOCAL_APP_UI_BRIDGE_ASSET: &str = "__baijimu_bridge.js";
pub(super) const LOCAL_APP_UI_MAX_MANAGEMENT_PAYLOAD_BYTES: usize = 1024 * 1024;
pub(super) const LOCAL_APP_UI_MAX_MANAGEMENT_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
pub(super) const LIFECYCLE_OUTPUT_MAX_BYTES: u64 = 1024 * 1024;
pub(super) const TRAY_ID: &str = "bridge-agent";
pub(super) const TRAY_MENU_SHOW: &str = "show";
pub(super) const TRAY_MENU_QUIT: &str = "quit";
pub(super) const QUIT_RUNNING_INSTANCE_ARG: &str = "--quit-running-instance";
pub(super) const AUTOSTART_BACKGROUND_ARG: &str = "--background-autostart";
pub(super) const STARTUP_LOG_FILE_NAME: &str = "bridge-agent-desktop-startup.log";
pub(super) const STARTUP_STATE_FILE_NAME: &str = "bridge-agent-desktop-startup-state.json";
pub(super) const INTERACTIVE_RESTART_MARKER_FILE_NAME: &str =
    "bridge-agent-desktop-interactive-restart";
pub(super) const LOCAL_APP_CONTROL_FILE_NAME: &str = "local-app-control.json";
pub(super) const LOCAL_APP_CONTROL_SCHEMA_VERSION: &str = "2.0.0";
pub(super) const SAFE_MODE_FAILURE_THRESHOLD: u32 = 2;
#[cfg(windows)]
pub(super) const WINDOWS_CREATE_NO_WINDOW: u32 = 0x08000000;

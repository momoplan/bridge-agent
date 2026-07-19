pub mod config;
pub mod connector;
mod event_server;
pub mod logging;
mod power;
mod process_identity;
pub mod protocol;
pub mod runtime;
pub mod services;

use anyhow::{anyhow, Result};

pub use config::{
    browser_auth_manifest_json, default_config_path, ensure_browser_auth_agent_id,
    ensure_config_exists, load_config, manifest_preview_json, reset_invalid_config, save_config,
    windows_service_config_path, AgentConfig, ComputerUseAction, ComputerUseBinding, DeviceConfig,
    EventConfig, HttpBinding, MethodBinding, MethodConfig, PlatformConfig, RegistrationHealthCheck,
    RegistrationMethod, RegistrationTransport, RelayConfig, RuntimeConfig, ServiceConfig,
    ServiceHealthCheck, ServiceRegistration, ServiceStartCommand, UploadConfig,
};
pub use connector::{
    connector_data_dir, connector_management_token_path, connectors_dir,
    format_connector_sync_failures, install_connector_from_path,
    install_connector_from_path_with_source_reference, list_connectors, load_connector_manifest,
    resolve_connector_ui_asset, resolve_connector_ui_entry, show_connector, start_connector,
    stop_connector, sync_installed_connector, sync_installed_connectors,
    sync_installed_connectors_report, uninstall_connector, ConnectorInstallRecord,
    ConnectorInstallResult, ConnectorManagement, ConnectorManagementOperation, ConnectorManifest,
    ConnectorStartResult, ConnectorSummary, ConnectorSyncFailure, ConnectorSyncReport, ConnectorUi,
};
pub use logging::{LogEntry, LogMetadata};
pub use runtime::{
    terminate_runtime_lock_owner, AgentRuntimeManager, RuntimeLockConflict, RuntimeProcessInfo,
    RuntimeSnapshot, RuntimeStatus,
};

pub fn install_rustls_crypto_provider() -> Result<()> {
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        rustls::crypto::ring::default_provider()
            .install_default()
            .map_err(|_| anyhow!("failed to install rustls ring provider"))?;
    }
    Ok(())
}

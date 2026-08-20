pub mod config;
pub mod connector;
mod event_server;
pub mod logging;
mod power;
mod process_identity;
pub mod protocol;
pub mod runtime;
mod secret_store;
pub mod services;
#[cfg(windows)]
mod windows_process;
mod windows_tcp;

use anyhow::{anyhow, Result};

pub use config::{
    browser_auth_manifest_json, clear_relay_credentials, default_config_path,
    ensure_browser_auth_agent_id, ensure_config_exists, load_config, manifest_preview_json,
    reset_invalid_config, save_config, windows_shared_config_path, AgentConfig, ComputerUseAction,
    ComputerUseBinding, DeviceConfig, EventConfig, HttpBinding, LocalAppConfig, MethodBinding,
    MethodConfig, PlatformConfig, RegistrationHealthCheck, RegistrationMethod,
    RegistrationTransport, RelayConfig, RuntimeConfig, ServiceConfig, ServiceHealthCheck,
    ServiceRegistration, ServiceStartCommand, UploadConfig,
};
pub use connector::inspect_python_runtime;
pub use connector::{
    connector_asset_upload_token_path, connector_data_dir, connector_icon_data_url,
    connector_management_token_path, connectors_dir, format_connector_sync_failures,
    install_connector_from_path, install_connector_from_path_with_provenance,
    install_connector_from_path_with_source_reference, is_connector_package_stop_error,
    list_connectors, load_connector_manifest, local_app_starts_automatically,
    prepare_installed_connector_runtime, resolve_connector_ui_asset, resolve_connector_ui_entry,
    show_connector, start_connector, start_connector_with_env, stop_connector,
    sync_installed_connector, sync_installed_connectors, sync_installed_connectors_report,
    uninstall_connector, uninstall_connector_with_options, ConnectorIcon,
    ConnectorInstallProvenance, ConnectorInstallRecord, ConnectorInstallResult,
    ConnectorLifecycleResult, ConnectorManagedToolDependency, ConnectorManagedToolDependencyPhase,
    ConnectorManagement, ConnectorManagementOperation, ConnectorManifest,
    ConnectorProcessOwnership, ConnectorSetup, ConnectorStartResult, ConnectorSummary,
    ConnectorSyncFailure, ConnectorSyncReport, ConnectorTrustLevel, ConnectorUi,
    ConnectorUninstallOptions, PythonRuntimeStatus,
};
pub use logging::{LogEntry, LogMetadata};
pub use runtime::{
    terminate_runtime_lock_owner, AgentRuntimeManager, RuntimeEvent, RuntimeLockConflict,
    RuntimeProcessInfo, RuntimeSnapshot, RuntimeStatus,
};

pub fn install_rustls_crypto_provider() -> Result<()> {
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        rustls::crypto::ring::default_provider()
            .install_default()
            .map_err(|_| anyhow!("failed to install rustls ring provider"))?;
    }
    Ok(())
}

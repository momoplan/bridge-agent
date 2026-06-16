pub mod config;
mod event_server;
pub mod logging;
pub mod protocol;
pub mod runtime;
pub mod services;

use anyhow::{anyhow, Result};

pub use config::{
    default_config_path, ensure_browser_auth_agent_id, ensure_config_exists, load_config,
    manifest_preview_json, reset_invalid_config, save_config, windows_service_config_path,
    AgentConfig, ComputerUseAction, ComputerUseBinding, DeviceConfig, EventConfig, HttpBinding,
    MethodBinding, MethodConfig, PlatformConfig, RegistrationHealthCheck, RegistrationMethod,
    RegistrationTransport, RelayConfig, RuntimeConfig, ServiceConfig, ServiceHealthCheck,
    ServiceRegistration, ServiceStartCommand, UploadConfig,
};
pub use runtime::{AgentRuntimeManager, LogEntry, RuntimeSnapshot, RuntimeStatus};

pub fn install_rustls_crypto_provider() -> Result<()> {
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        rustls::crypto::ring::default_provider()
            .install_default()
            .map_err(|_| anyhow!("failed to install rustls ring provider"))?;
    }
    Ok(())
}

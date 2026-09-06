use crate::config::{
    ensure_config_exists, load_config, save_config, AgentConfig, LocalAppConfig,
    RegistrationHealthCheck, RegistrationMethod, RegistrationTransport, RuntimeConfig,
    ServiceConfig, ServiceRegistration, ServiceStartCommand,
};
use crate::process_environment::{
    current_user_command_path, enrich_user_command_environment_with_path,
};
use crate::protocol::ResponseMode;
use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use directories::ProjectDirs;
use image::ImageFormat;
use pep440_rs::{Version, VersionSpecifiers};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::io::Read;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::process::Stdio;
use std::str::FromStr;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use uuid::Uuid;

// Responsibility-focused fragments share this module so the public API remains stable.
include!("connector/model.rs");
include!("connector/access.rs");
include!("connector/install.rs");
include!("connector/lifecycle.rs");
include!("connector/manifest_validation.rs");
include!("connector/service_registration.rs");
include!("connector/package_swap.rs");
include!("connector/runtime_commands.rs");
include!("connector/python_project.rs");
include!("connector/python_discovery.rs");
include!("connector/python_installation.rs");
include!("connector/local_app_config.rs");
include!("connector/command_execution.rs");
include!("connector/summary.rs");
include!("connector/tokens.rs");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AgentConfig;
    use image::{DynamicImage, Rgba, RgbaImage};
    use serde_json::json;
    use std::ffi::OsString;
    use std::io::Cursor;
    use std::sync::{Mutex, MutexGuard};
    use tempfile::tempdir;

    static CONNECTOR_ENV_LOCK: Mutex<()> = Mutex::new(());
    include!("connector/tests/core.rs");
    include!("connector/tests/manifest_contract.rs");
    include!("connector/tests/permissions_and_ui.rs");
    include!("connector/tests/install_and_uninstall.rs");
    include!("connector/tests/lifecycle_process.rs");
    include!("connector/tests/runtime_commands.rs");
    include!("connector/tests/python_runtime.rs");
}

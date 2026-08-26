use crate::{codex_skill, local_app, macos_installation, managed_tool, managed_tool_dependency};

mod app;
mod auth;
mod constants;
mod local_app_archive;
mod local_app_commands;
mod local_app_http;
mod local_app_install;
mod local_app_management;
mod local_app_market;
mod runtime_commands;
mod startup;
mod startup_health;
mod state;
mod update;
mod window;

pub(crate) use app::run;
use auth::*;
use constants::*;
use local_app_archive::*;
use local_app_commands::*;
use local_app_http::*;
use local_app_install::*;
use local_app_management::*;
use local_app_market::*;
use runtime_commands::*;
use startup::*;
use startup_health::*;
use state::*;
use update::*;
use window::*;

#[cfg(test)]
use local_app::{
    apply_managed_process_status, check_local_app, ensure_lifecycle_command_succeeded,
    format_byte_count, format_health_http_error, inactive_local_app_status,
    local_app_runtime_statuses_changed, registered_service_statuses_changed,
    RegisteredServiceState, HEALTH_ERROR_MESSAGE_MAX_CHARS,
};
use local_app::{
    attach_lifecycle_events, collect_local_app_runtime_statuses, connector_local_app_is_healthy,
    connector_local_app_status, start_connector_and_wait, start_connector_with_lifecycle,
    start_runtime_monitor_with_tauri, stop_connector_with_lifecycle, ConnectorHealthState,
    ConnectorLifecycleManager, ConnectorLifecycleOperation, ConnectorLifecycleSnapshot,
    ConnectorLifecycleState, ConnectorManagementNotReady, ConnectorOperationKind,
    ConnectorProcessManager, LocalAppInstallProgressReporter, LocalAppInstallTask,
    LocalAppInstallTaskManager, LocalAppInstallTaskOperation, LocalAppInstallTaskPhase,
    LocalAppRuntimeStatus, LocalAppsChangeNotifier, LocalAppsChangeOperation,
    RegisteredServiceMonitor, RegisteredServiceMonitorReceiver, RegisteredServiceStatus,
};

use crate::window_layout::{fit_main_window_to_work_area, WindowLayoutOutcome, WindowLayoutPolicy};
use anyhow::Context as _;
use axum::{
    body::Body,
    extract::{Path as AxumPath, Query as AxumQuery, State as AxumState},
    http::{header, HeaderMap, Response as HttpResponse, StatusCode},
    response::{IntoResponse, Response as AxumResponse},
    routing::{get, post},
    Json, Router,
};
use bridge_agent::config::resolve_config_base_dir;
use bridge_agent::connector::{
    ConnectorDatabaseContract, ConnectorEventContract, ConnectorMethodContract, ConnectorPermission,
};
use bridge_agent::logging::LogMetadata;
use bridge_agent::protocol::InvokeResult;
use bridge_agent::services::ServiceRegistry;
use bridge_agent::{
    browser_auth_manifest_json, clear_relay_credentials, connector_icon_data_url,
    connector_management_token_path, default_config_path, ensure_browser_auth_agent_id,
    ensure_config_exists, format_connector_sync_failures, inspect_python_runtime,
    install_connector_from_path_with_provenance, install_rustls_crypto_provider,
    is_connector_package_stop_error, list_connectors, load_config as load_agent_config,
    load_connector_manifest, manifest_preview_json,
    process_environment::enrich_user_command_environment, reset_invalid_config,
    resolve_connector_ui_asset, resolve_connector_ui_entry, save_config as save_agent_config,
    show_connector, sync_installed_connectors_report, terminate_runtime_lock_owner,
    uninstall_connector_with_options, AgentConfig, AgentRuntimeManager, ConnectorIcon,
    ConnectorInstallProvenance, ConnectorInstallRecord, ConnectorInstallResult, ConnectorManifest,
    ConnectorStartResult, ConnectorSummary, ConnectorSyncReport, ConnectorUninstallOptions,
    RuntimeEvent, RuntimeLockConflict, RuntimeSnapshot, RuntimeStatus, ServiceConfig,
    ServiceStartCommand,
};
#[cfg(test)]
use bridge_agent::{LocalAppConfig, ServiceHealthCheck};
use reqwest::Client;
use semver::Version;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
#[cfg(test)]
use std::collections::BTreeMap;
use std::collections::HashSet;
use std::ffi::OsStr;
use std::fs::{self, OpenOptions};
use std::io::{Cursor, Read, Write};
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex, RwLock,
};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Emitter, Manager, WindowEvent,
};
use tauri_plugin_autostart::ManagerExt as _;
use tauri_plugin_updater::UpdaterExt;
use tauri_plugin_window_state::StateFlags;
use tokio::process::Command as AsyncCommand;
use tokio::time::timeout;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(windows)]
use std::os::windows::process::CommandExt as _;

#[cfg(target_os = "macos")]
use core_foundation::base::TCFType;
#[cfg(target_os = "macos")]
use core_foundation::boolean::CFBoolean;
#[cfg(target_os = "macos")]
use core_foundation::dictionary::CFDictionary;
#[cfg(target_os = "macos")]
use core_foundation::string::CFString;

#[cfg(test)]
mod market_lifecycle_tests;
#[cfg(test)]
mod startup_update_tests;

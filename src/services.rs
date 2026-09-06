use crate::cmodel_response::normalize_cmodel_http_response;
use crate::config::{
    AgentConfig, ComputerUseBinding, HttpBinding, LocalAppConfig, MethodBinding, MethodConfig,
    ServiceConfig, ServiceHealthCheck, ServiceStartCommand, ShellCommandBinding,
};
#[cfg(any(target_os = "macos", windows))]
use crate::config::{ComputerUseAction, UploadConfig};
use crate::process_environment::enrich_user_command_environment;
use crate::protocol::{
    EventDefinition, InvokeError, InvokeResult, LocalAppDefinition, ResponseMode, ServiceDefinition,
};
use anyhow::{anyhow, bail, Context, Result};
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
#[cfg(target_os = "macos")]
use image::GenericImageView;
use reqwest::{Client, Method};
use serde::{de, Deserialize, Deserializer, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::ErrorKind;
#[cfg(windows)]
use std::os::windows::process::CommandExt as _;
use std::path::{Path, PathBuf};
#[cfg(windows)]
use std::process::Command as StdCommand;
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, Command};
use tokio::sync::{oneshot, Mutex};
use tokio::time::{sleep, Duration};
use tracing::{info, warn};
use uuid::Uuid;

#[cfg(windows)]
const WINDOWS_CREATE_NO_WINDOW: u32 = 0x08000000;
const RUNNING_SHELL_OUTPUT_TAIL_BYTES: usize = 64 * 1024;
const LOCAL_APP_START_POLICY_ENV: &str = "BAIJIMU_LOCAL_APP_START_POLICY";
const BAIJIMU_WORKSPACE_ID_HEADER: &str = "x-baijimu-workspace-id";

#[cfg(target_os = "macos")]
use core_graphics::event::{
    CGEvent, CGEventTapLocation, CGEventType, CGMouseButton, ScrollEventUnit,
};
#[cfg(target_os = "macos")]
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
#[cfg(target_os = "macos")]
use core_graphics::geometry::CGPoint;
#[cfg(windows)]
use windows_sys::Win32::Foundation::{GetLastError, LPARAM, RECT};
#[cfg(windows)]
use windows_sys::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject,
    EnumDisplayMonitors, GetDC, GetDIBits, GetMonitorInfoW, ReleaseDC, SelectObject, BITMAPINFO,
    BITMAPINFOHEADER, BI_RGB, CAPTUREBLT, DIB_RGB_COLORS, HBITMAP, HDC, HGDIOBJ, HMONITOR,
    MONITORINFOEXW, SRCCOPY,
};
#[cfg(windows)]
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYEVENTF_KEYUP,
    KEYEVENTF_UNICODE, MOUSEEVENTF_HWHEEL, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP,
    MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP,
    MOUSEEVENTF_WHEEL, MOUSEINPUT, VK_CONTROL, VK_DOWN, VK_END, VK_ESCAPE, VK_HOME, VK_LEFT,
    VK_LWIN, VK_MENU, VK_NEXT, VK_PRIOR, VK_RETURN, VK_RIGHT, VK_SHIFT, VK_SPACE, VK_TAB, VK_UP,
};
#[cfg(windows)]
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, SetCursorPos, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
    SM_YVIRTUALSCREEN, WHEEL_DELTA,
};

pub struct ServiceRegistry {
    services: BTreeMap<String, RuntimeService>,
    local_apps: BTreeMap<String, RuntimeLocalApp>,
}

struct RuntimeService {
    definition: ServiceDefinition,
    methods: BTreeMap<String, RuntimeMethod>,
}

struct RuntimeLocalApp {
    definition: LocalAppDefinition,
    runtime: RuntimeService,
    events: BTreeSet<String>,
}

enum RuntimeMethod {
    Shell(ShellMethod),
    Http(HttpMethod),
    Computer(ComputerMethod),
}

struct ShellMethod {
    service_name: String,
    method_name: String,
    root_dir: PathBuf,
    allow_commands: Vec<String>,
    default_timeout_secs: u64,
    max_timeout_secs: u64,
    executions: ShellExecutionStore,
}

struct HttpMethod {
    service_name: String,
    method_name: String,
    client: reqwest::Client,
    url: String,
    http_method: Method,
    headers: BTreeMap<String, String>,
    timeout_secs: u64,
    response_mode: ResponseMode,
}

#[cfg(any(target_os = "macos", windows))]
struct ComputerMethod {
    action: ComputerUseAction,
    display_id: Option<u32>,
    upload: UploadConfig,
    upload_prepare_url: Option<String>,
    agent_id: String,
    relay_token: String,
    workspace_id: Option<u64>,
    client: Client,
}

#[cfg(not(any(target_os = "macos", windows)))]
struct ComputerMethod;

struct ServiceOutcome {
    success: bool,
    data: Option<Value>,
    error: Option<InvokeError>,
}

#[derive(Debug, Deserialize)]
struct ShellExecArgs {
    #[serde(deserialize_with = "deserialize_shell_command")]
    command: ShellCommand,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(default)]
    stdin: Option<String>,
    #[serde(default, alias = "timeoutSeconds", alias = "timeout_secs")]
    timeout_secs: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ShellExecutionIdArgs {
    #[serde(alias = "execution_id")]
    execution_id: String,
}

#[derive(Debug, PartialEq, Eq)]
enum ShellCommand {
    Args(Vec<String>),
    Line(String),
}

impl ShellCommand {
    fn is_empty(&self) -> bool {
        match self {
            Self::Args(args) => args.is_empty(),
            Self::Line(line) => line.trim().is_empty(),
        }
    }

    fn into_args(self) -> Vec<String> {
        match self {
            Self::Args(args) => args,
            Self::Line(line) => platform_shell_command(line),
        }
    }
}

fn deserialize_shell_command<'de, D>(deserializer: D) -> std::result::Result<ShellCommand, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    match value {
        Value::Array(_) => serde_json::from_value(value)
            .map(ShellCommand::Args)
            .map_err(de::Error::custom),
        Value::String(line) => Ok(ShellCommand::Line(line)),
        _ => Err(de::Error::custom(
            "command must be either an argv array of strings or a command line string",
        )),
    }
}

fn platform_shell_command(line: String) -> Vec<String> {
    #[cfg(windows)]
    {
        vec!["cmd".to_string(), "/C".to_string(), line]
    }

    #[cfg(not(windows))]
    {
        vec!["sh".to_string(), "-lc".to_string(), line]
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ShellExecData {
    execution_id: String,
    status: ShellExecutionStatus,
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
    timed_out: bool,
    timeout_secs: Option<u64>,
    started_at_epoch_ms: u64,
    completed_at_epoch_ms: Option<u64>,
    duration_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    recommended_action: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    recommended_service: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    recommended_method: Option<&'static str>,
}

#[derive(Clone, Default)]
struct ShellExecutionStore {
    entries: Arc<Mutex<BTreeMap<String, ShellExecutionRecord>>>,
}

struct ShellExecutionRecord {
    execution_id: String,
    command: Vec<String>,
    cwd: String,
    status: ShellExecutionStatus,
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
    timed_out: bool,
    success: Option<bool>,
    error: Option<InvokeError>,
    timeout_secs: Option<u64>,
    started_at_epoch_ms: u64,
    completed_at_epoch_ms: Option<u64>,
    duration_ms: Option<u64>,
    cancel_requested: bool,
    cancel_tx: Option<oneshot::Sender<()>>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum ShellExecutionStatus {
    Running,
    Succeeded,
    Failed,
    TimedOut,
    Canceled,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ShellExecutionSnapshot {
    execution_id: String,
    command: Vec<String>,
    cwd: String,
    status: ShellExecutionStatus,
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
    timed_out: bool,
    success: Option<bool>,
    error: Option<InvokeError>,
    timeout_secs: Option<u64>,
    started_at_epoch_ms: u64,
    completed_at_epoch_ms: Option<u64>,
    duration_ms: Option<u64>,
    cancel_requested: bool,
}

#[derive(Debug)]
struct PreparedShellExec {
    command_args: Vec<String>,
    cwd: PathBuf,
    env: BTreeMap<String, String>,
    stdin: Option<String>,
    timeout_secs: Option<u64>,
    path_for_diagnostics: String,
}

struct CompletedShellExec {
    status: ShellExecutionStatus,
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
    timed_out: bool,
    success: bool,
    error: Option<InvokeError>,
    duration_ms: u64,
    completed_at_epoch_ms: u64,
}

#[derive(Clone)]
struct ShellLiveOutput {
    store: ShellExecutionStore,
    execution_id: String,
}

#[derive(Clone, Copy)]
enum ShellOutputStream {
    Stdout,
    Stderr,
}

#[cfg(any(target_os = "macos", windows))]
#[derive(Debug, Deserialize)]
struct ComputerPoint {
    x: f64,
    y: f64,
}

#[cfg(any(target_os = "macos", windows))]
#[derive(Debug, Deserialize)]
struct ComputerMouseArgs {
    x: f64,
    y: f64,
    #[serde(default = "default_mouse_button")]
    button: String,
    #[serde(default)]
    keys: Vec<String>,
}

#[cfg(any(target_os = "macos", windows))]
#[derive(Debug, Deserialize)]
struct ComputerScrollArgs {
    x: f64,
    y: f64,
    #[serde(default, alias = "scrollX")]
    scroll_x: i64,
    #[serde(default, alias = "scrollY")]
    scroll_y: i64,
    #[serde(default)]
    keys: Vec<String>,
}

#[cfg(any(target_os = "macos", windows))]
#[derive(Debug, Deserialize)]
struct ComputerTypeArgs {
    text: String,
}

#[cfg(any(target_os = "macos", windows))]
#[derive(Debug, Deserialize)]
struct ComputerKeypressArgs {
    keys: Vec<String>,
}

#[cfg(any(target_os = "macos", windows))]
#[derive(Debug, Deserialize)]
struct ComputerWaitArgs {
    #[serde(default = "default_wait_ms")]
    ms: u64,
}

#[cfg(any(target_os = "macos", windows))]
#[derive(Debug, Deserialize)]
struct ComputerDragArgs {
    path: Vec<ComputerPoint>,
    #[serde(default)]
    keys: Vec<String>,
}

#[cfg(any(test, target_os = "macos", windows))]
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PrepareUploadRequest {
    agent_id: String,
    content_type: String,
    file_name: String,
    size_bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    workspace_id: Option<u64>,
    purpose: String,
}

#[cfg(any(target_os = "macos", windows))]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PrepareUploadResponse {
    file_id: String,
    upload_url: String,
    #[serde(default)]
    method: Option<String>,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    #[serde(default)]
    object_key: Option<String>,
    #[serde(default)]
    download_url: Option<String>,
    #[serde(default)]
    expires_at: Option<String>,
}

include!("services/registry.rs");
include!("services/local_apps.rs");
include!("services/runtime_health.rs");
include!("services/shell_method.rs");
include!("services/shell_store.rs");
include!("services/shell_execution.rs");
include!("services/http.rs");
include!("services/runtime_build.rs");
include!("services/computer_windows_actions.rs");
include!("services/computer_macos_actions.rs");
include!("services/computer_macos_native.rs");
include!("services/computer_windows_native.rs");
include!("services/computer_macos_keys.rs");
include!("services/common.rs");

#[cfg(test)]
#[path = "services_cmodel_tests.rs"]
mod cmodel_tests;

#[cfg(test)]
mod tests {
    use super::{
        bgra_to_rgba, is_command_allowed, passthrough_http_outcome, resolve_cwd, sanitize_env,
        shell_exec_path, PrepareUploadRequest, ServiceRegistry, ShellCommand, ShellExecArgs,
        LOCAL_APP_START_POLICY_ENV,
    };
    use crate::config::{
        AgentConfig, EventConfig, HttpBinding, LocalAppConfig, MethodBinding, MethodConfig,
        ServiceConfig, ServiceHealthCheck, ServiceStartCommand,
    };
    use crate::protocol::ResponseMode;
    use axum::{
        http::HeaderMap,
        routing::{get, post},
        Json, Router,
    };
    use serde_json::{json, Value};
    use std::collections::BTreeMap;
    use std::fs;
    use tempfile::tempdir;
    use tokio::net::TcpListener;
    use tokio::time::{sleep, Duration, Instant};

    include!("services/tests/core.rs");
    include!("services/tests/shell.rs");
    include!("services/tests/health.rs");
    include!("services/tests/http.rs");
    include!("services/tests/helpers.rs");
}

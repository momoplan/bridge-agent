use crate::config::{
    AgentConfig, ComputerUseAction, ComputerUseBinding, HttpBinding, MethodBinding, MethodConfig,
    ServiceConfig, ShellCommandBinding, UploadConfig,
};
use crate::protocol::{InvokeError, InvokeResult, ServiceDefinition};
use anyhow::{anyhow, bail, Context, Result};
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
#[cfg(target_os = "macos")]
use image::GenericImageView;
use reqwest::{Client, Method};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
#[cfg(target_os = "macos")]
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tokio::process::Command;
use tokio::time::{sleep, timeout, Duration};

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
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CAPTUREBLT, CreateCompatibleBitmap,
    CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC, DeleteObject, EnumDisplayMonitors, GetDC,
    GetDIBits, GetMonitorInfoW, HBITMAP, HDC, HGDIOBJ, HMONITOR, MONITORINFOEXW, ReleaseDC,
    SRCCOPY, SelectObject,
};
#[cfg(windows)]
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYEVENTF_KEYUP,
    KEYEVENTF_UNICODE, MOUSEEVENTF_HWHEEL, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP,
    MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP,
    MOUSEEVENTF_WHEEL, MOUSEINPUT, SendInput, VK_CONTROL, VK_DOWN, VK_END, VK_ESCAPE, VK_HOME,
    VK_LEFT, VK_LWIN, VK_MENU, VK_NEXT, VK_PRIOR, VK_RETURN, VK_RIGHT, VK_SHIFT, VK_SPACE,
    VK_TAB, VK_UP,
};
#[cfg(windows)]
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
    SM_YVIRTUALSCREEN, SetCursorPos, WHEEL_DELTA,
};

pub struct ServiceRegistry {
    services: BTreeMap<String, RuntimeService>,
}

struct RuntimeService {
    definition: ServiceDefinition,
    methods: BTreeMap<String, RuntimeMethod>,
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
}

struct HttpMethod {
    service_name: String,
    method_name: String,
    client: reqwest::Client,
    url: String,
    http_method: Method,
    headers: BTreeMap<String, String>,
    timeout_secs: u64,
}

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

struct ServiceOutcome {
    success: bool,
    data: Option<Value>,
    error: Option<InvokeError>,
}

#[derive(Debug, Deserialize)]
struct ShellExecArgs {
    command: Vec<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
}

#[derive(Debug, Serialize)]
struct ShellExecData {
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
    timed_out: bool,
}

#[derive(Debug, Deserialize)]
struct ComputerPoint {
    x: f64,
    y: f64,
}

#[derive(Debug, Deserialize)]
struct ComputerMouseArgs {
    x: f64,
    y: f64,
    #[serde(default = "default_mouse_button")]
    button: String,
    #[serde(default)]
    keys: Vec<String>,
}

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

#[derive(Debug, Deserialize)]
struct ComputerTypeArgs {
    text: String,
}

#[derive(Debug, Deserialize)]
struct ComputerKeypressArgs {
    keys: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ComputerWaitArgs {
    #[serde(default = "default_wait_ms")]
    ms: u64,
}

#[derive(Debug, Deserialize)]
struct ComputerDragArgs {
    path: Vec<ComputerPoint>,
    #[serde(default)]
    keys: Vec<String>,
}

#[derive(Debug, Serialize)]
struct PrepareUploadRequest {
    agent_id: String,
    content_type: String,
    file_name: String,
    size_bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    workspace_id: Option<u64>,
    purpose: String,
}

#[derive(Debug, Deserialize)]
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

impl ServiceRegistry {
    pub fn from_config(config: &AgentConfig, config_base_dir: &Path) -> Result<Self> {
        let mut services = BTreeMap::new();

        for service in &config.services {
            if !service.enabled {
                continue;
            }
            let runtime_service = build_runtime_service(service, config, config_base_dir)?;
            if !runtime_service.methods.is_empty() {
                services.insert(service.name.clone(), runtime_service);
            }
        }

        Ok(Self { services })
    }

    pub fn definitions(&self) -> Vec<ServiceDefinition> {
        self.services
            .values()
            .map(|service| service.definition.clone())
            .collect()
    }

    pub async fn invoke(
        &self,
        request_id: String,
        service: &str,
        method: &str,
        arguments: Value,
        timeout_secs: Option<u64>,
    ) -> InvokeResult {
        let started = Instant::now();
        let response = match self.services.get(service) {
            Some(service_definition) => match service_definition.methods.get(method) {
                Some(runtime_method) => runtime_method.invoke(arguments, timeout_secs).await,
                None => Err(anyhow!("unknown method `{method}` on service `{service}`")),
            },
            None => Err(anyhow!("unknown service `{service}`")),
        };

        match response {
            Ok(outcome) => InvokeResult {
                request_id,
                success: outcome.success,
                data: outcome.data,
                error: outcome.error,
                duration_ms: started.elapsed().as_millis() as u64,
            },
            Err(err) => InvokeResult {
                request_id,
                success: false,
                data: None,
                error: Some(InvokeError {
                    code: "INVOKE_FAILED".to_string(),
                    message: err.to_string(),
                }),
                duration_ms: started.elapsed().as_millis() as u64,
            },
        }
    }
}

impl RuntimeMethod {
    async fn invoke(&self, arguments: Value, timeout_secs: Option<u64>) -> Result<ServiceOutcome> {
        match self {
            Self::Shell(method) => method.exec(arguments, timeout_secs).await,
            Self::Http(method) => method.invoke(arguments, timeout_secs).await,
            Self::Computer(method) => method.invoke(arguments).await,
        }
    }
}

impl ShellMethod {
    async fn exec(&self, arguments: Value, timeout_secs: Option<u64>) -> Result<ServiceOutcome> {
        let args: ShellExecArgs = serde_json::from_value(arguments).with_context(|| {
            format!(
                "invalid arguments for {}.{}",
                self.service_name, self.method_name
            )
        })?;

        if args.command.is_empty() {
            bail!(
                "{}.{} requires a non-empty command",
                self.service_name,
                self.method_name
            );
        }

        let executable = &args.command[0];
        if !is_command_allowed(executable, &self.allow_commands) {
            return Ok(ServiceOutcome {
                success: false,
                data: None,
                error: Some(InvokeError {
                    code: "COMMAND_NOT_ALLOWED".to_string(),
                    message: format!("command `{executable}` is not in allowlist"),
                }),
            });
        }

        let cwd = resolve_cwd(&self.root_dir, args.cwd.as_deref())?;
        let timeout_secs = timeout_secs
            .unwrap_or(self.default_timeout_secs)
            .min(self.max_timeout_secs);
        let env = sanitize_env(args.env);

        let mut command = Command::new(executable);
        command
            .args(args.command.iter().skip(1))
            .current_dir(&cwd)
            .env_clear()
            .envs(env)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let child = command
            .spawn()
            .with_context(|| format!("failed to spawn `{executable}` in {}", cwd.display()))?;

        match timeout(Duration::from_secs(timeout_secs), child.wait_with_output()).await {
            Ok(output) => {
                let output = output?;
                Ok(ServiceOutcome {
                    success: output.status.success(),
                    data: Some(serde_json::to_value(ShellExecData {
                        exit_code: output.status.code(),
                        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
                        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
                        timed_out: false,
                    })?),
                    error: None,
                })
            }
            Err(_) => Ok(ServiceOutcome {
                success: false,
                data: Some(serde_json::to_value(ShellExecData {
                    exit_code: None,
                    stdout: String::new(),
                    stderr: String::new(),
                    timed_out: true,
                })?),
                error: Some(InvokeError {
                    code: "TIMEOUT".to_string(),
                    message: format!("timed out after {timeout_secs}s"),
                }),
            }),
        }
    }
}

impl HttpMethod {
    async fn invoke(&self, arguments: Value, timeout_secs: Option<u64>) -> Result<ServiceOutcome> {
        let timeout_secs = timeout_secs.unwrap_or(self.timeout_secs);
        let mut request = self
            .client
            .request(self.http_method.clone(), &self.url)
            .timeout(Duration::from_secs(timeout_secs));

        for (key, value) in &self.headers {
            request = request.header(key, value);
        }

        if matches!(self.http_method, Method::GET | Method::DELETE) {
            let query = query_pairs_from_json(&arguments);
            request = request.query(&query);
        } else {
            request = request.json(&arguments);
        }

        let response = request.send().await.with_context(|| {
            format!(
                "failed to call local http binding for {}.{}",
                self.service_name, self.method_name
            )
        })?;

        let status = response.status();
        let headers = collect_headers(response.headers());
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .to_string();
        let bytes = response.bytes().await?;
        let body = decode_response_body(&bytes, &content_type);

        let error = if status.is_success() {
            None
        } else {
            Some(InvokeError {
                code: "HTTP_REQUEST_FAILED".to_string(),
                message: format!(
                    "local endpoint returned status {} for {}.{}",
                    status.as_u16(),
                    self.service_name,
                    self.method_name
                ),
            })
        };

        Ok(ServiceOutcome {
            success: status.is_success(),
            data: Some(json!({
                "status": status.as_u16(),
                "headers": headers,
                "body": body,
            })),
            error,
        })
    }
}

impl ComputerMethod {
    async fn invoke(&self, arguments: Value) -> Result<ServiceOutcome> {
        #[cfg(target_os = "macos")]
        {
            execute_macos_computer_action(self, arguments).await
        }

        #[cfg(windows)]
        {
            execute_windows_computer_action(self, arguments).await
        }

        #[cfg(not(any(target_os = "macos", windows)))]
        {
            let _ = arguments;
            Ok(ServiceOutcome {
                success: false,
                data: None,
                error: Some(InvokeError {
                    code: "UNSUPPORTED_PLATFORM".to_string(),
                    message: "computer_use currently only supports macOS in bridge-agent"
                        .to_string(),
                }),
            })
        }
    }
}

fn build_runtime_service(
    service: &ServiceConfig,
    config: &AgentConfig,
    config_base_dir: &Path,
) -> Result<RuntimeService> {
    let mut methods = BTreeMap::new();
    let mut method_definitions = Vec::new();

    for method in &service.methods {
        if !method.enabled {
            continue;
        }
        let runtime_method = build_runtime_method(service, method, config, config_base_dir)?;
        methods.insert(method.name.clone(), runtime_method);
        method_definitions.push(crate::protocol::MethodDefinition {
            name: method.name.clone(),
            description: method.description.clone(),
            input_schema: method.input_schema.clone(),
        });
    }

    Ok(RuntimeService {
        definition: ServiceDefinition {
            name: service.name.clone(),
            description: service.description.clone(),
            methods: method_definitions,
        },
        methods,
    })
}

fn build_runtime_method(
    service: &ServiceConfig,
    method: &MethodConfig,
    config: &AgentConfig,
    config_base_dir: &Path,
) -> Result<RuntimeMethod> {
    match &method.binding {
        MethodBinding::ShellCommand(binding) => Ok(RuntimeMethod::Shell(build_shell_method(
            service,
            method,
            binding,
            config,
            config_base_dir,
        )?)),
        MethodBinding::Http(binding) => Ok(RuntimeMethod::Http(build_http_method(
            service, method, binding, config,
        )?)),
        MethodBinding::ComputerUse(binding) => Ok(RuntimeMethod::Computer(build_computer_method(
            service, method, config, binding,
        )?)),
    }
}

fn build_shell_method(
    service: &ServiceConfig,
    method: &MethodConfig,
    binding: &ShellCommandBinding,
    config: &AgentConfig,
    config_base_dir: &Path,
) -> Result<ShellMethod> {
    let raw_root = PathBuf::from(&binding.root_dir);
    let joined_root = if raw_root.is_absolute() {
        raw_root
    } else {
        config_base_dir.join(raw_root)
    };
    let root_dir = joined_root.canonicalize().with_context(|| {
        format!(
            "failed to resolve root_dir for {}.{}: {}",
            service.name,
            method.name,
            joined_root.display()
        )
    })?;

    Ok(ShellMethod {
        service_name: service.name.clone(),
        method_name: method.name.clone(),
        root_dir,
        allow_commands: binding.allow_commands.clone(),
        default_timeout_secs: binding
            .default_timeout_secs
            .unwrap_or(config.runtime.default_timeout_secs),
        max_timeout_secs: binding
            .max_timeout_secs
            .unwrap_or(config.runtime.max_timeout_secs),
    })
}

fn build_http_method(
    service: &ServiceConfig,
    method: &MethodConfig,
    binding: &HttpBinding,
    config: &AgentConfig,
) -> Result<HttpMethod> {
    let http_method = binding
        .http_method
        .parse::<Method>()
        .with_context(|| format!("invalid HTTP method `{}`", binding.http_method))?;

    Ok(HttpMethod {
        service_name: service.name.clone(),
        method_name: method.name.clone(),
        client: reqwest::Client::new(),
        url: binding.url.clone(),
        http_method,
        headers: binding.headers.clone(),
        timeout_secs: binding
            .timeout_secs
            .unwrap_or(config.runtime.default_timeout_secs),
    })
}

fn build_computer_method(
    _service: &ServiceConfig,
    _method: &MethodConfig,
    config: &AgentConfig,
    binding: &ComputerUseBinding,
) -> Result<ComputerMethod> {
    Ok(ComputerMethod {
        action: binding.action.clone(),
        display_id: binding.display_id,
        upload: config.upload.clone(),
        upload_prepare_url: config.upload.prepare_url(&config.relay),
        agent_id: config.relay.agent_id.clone(),
        relay_token: config.relay.token.clone(),
        workspace_id: config.platform.workspace_id,
        client: Client::new(),
    })
}

fn default_mouse_button() -> String {
    "left".to_string()
}

fn default_wait_ms() -> u64 {
    500
}

#[cfg(windows)]
async fn execute_windows_computer_action(
    method: &ComputerMethod,
    arguments: Value,
) -> Result<ServiceOutcome> {
    match &method.action {
        ComputerUseAction::Screenshot => capture_windows_screenshot(method).await,
        ComputerUseAction::Click => {
            let args: ComputerMouseArgs = serde_json::from_value(arguments)?;
            perform_windows_click(&args, false).await
        }
        ComputerUseAction::DoubleClick => {
            let args: ComputerMouseArgs = serde_json::from_value(arguments)?;
            perform_windows_click(&args, true).await
        }
        ComputerUseAction::Scroll => {
            let args: ComputerScrollArgs = serde_json::from_value(arguments)?;
            perform_windows_scroll(&args).await
        }
        ComputerUseAction::Type => {
            let args: ComputerTypeArgs = serde_json::from_value(arguments)?;
            perform_windows_type(&args).await
        }
        ComputerUseAction::Wait => {
            let args: ComputerWaitArgs =
                serde_json::from_value(arguments).unwrap_or(ComputerWaitArgs {
                    ms: default_wait_ms(),
                });
            sleep(Duration::from_millis(args.ms)).await;
            Ok(success_outcome(json!({ "waited_ms": args.ms })))
        }
        ComputerUseAction::Keypress => {
            let args: ComputerKeypressArgs = serde_json::from_value(arguments)?;
            perform_windows_keypress(&args).await
        }
        ComputerUseAction::Drag => {
            let args: ComputerDragArgs = serde_json::from_value(arguments)?;
            perform_windows_drag(&args).await
        }
        ComputerUseAction::Move => {
            let args: ComputerMouseArgs = serde_json::from_value(arguments)?;
            perform_windows_move(&args).await
        }
    }
}

#[cfg(windows)]
async fn capture_windows_screenshot(method: &ComputerMethod) -> Result<ServiceOutcome> {
    let capture = capture_windows_monitor_png(method.display_id)?;
    if capture.bytes.len() > method.upload.inline_limit_bytes {
        return upload_screenshot(method, capture.bytes, capture.width, capture.height).await;
    }

    Ok(success_outcome(json!({
        "result_type": "inline_image",
        "mime_type": "image/png",
        "width": capture.width,
        "height": capture.height,
        "display_id": capture.display_id,
        "size_bytes": capture.bytes.len(),
        "image_base64": BASE64_STANDARD.encode(capture.bytes),
    })))
}

#[cfg(windows)]
async fn perform_windows_click(
    args: &ComputerMouseArgs,
    double_click: bool,
) -> Result<ServiceOutcome> {
    with_windows_modifiers(&args.keys, || {
        set_windows_cursor_position(args.x, args.y)?;
        let (down, up) = windows_mouse_button_flags(&args.button)?;
        send_windows_mouse_input(0, 0, down, 0)?;
        send_windows_mouse_input(0, 0, up, 0)?;
        if double_click {
            std::thread::sleep(Duration::from_millis(80));
            send_windows_mouse_input(0, 0, down, 0)?;
            send_windows_mouse_input(0, 0, up, 0)?;
        }
        Ok(())
    })?;

    Ok(success_outcome(json!({
        "action": if double_click { "double_click" } else { "click" },
        "x": args.x,
        "y": args.y,
        "button": args.button,
    })))
}

#[cfg(windows)]
async fn perform_windows_move(args: &ComputerMouseArgs) -> Result<ServiceOutcome> {
    with_windows_modifiers(&args.keys, || set_windows_cursor_position(args.x, args.y))?;
    Ok(success_outcome(json!({
        "action": "move",
        "x": args.x,
        "y": args.y,
    })))
}

#[cfg(windows)]
async fn perform_windows_scroll(args: &ComputerScrollArgs) -> Result<ServiceOutcome> {
    with_windows_modifiers(&args.keys, || {
        set_windows_cursor_position(args.x, args.y)?;
        if args.scroll_y != 0 {
            let delta = scale_windows_wheel_delta(args.scroll_y)?;
            send_windows_mouse_input(0, 0, MOUSEEVENTF_WHEEL, delta)?;
        }
        if args.scroll_x != 0 {
            let delta = scale_windows_wheel_delta(args.scroll_x)?;
            send_windows_mouse_input(0, 0, MOUSEEVENTF_HWHEEL, delta)?;
        }
        Ok(())
    })?;

    Ok(success_outcome(json!({
        "action": "scroll",
        "x": args.x,
        "y": args.y,
        "scroll_x": args.scroll_x,
        "scroll_y": args.scroll_y,
    })))
}

#[cfg(windows)]
async fn perform_windows_type(args: &ComputerTypeArgs) -> Result<ServiceOutcome> {
    send_windows_unicode_text(&args.text)?;
    Ok(success_outcome(json!({
        "action": "type",
        "length": args.text.chars().count(),
    })))
}

#[cfg(windows)]
async fn perform_windows_keypress(args: &ComputerKeypressArgs) -> Result<ServiceOutcome> {
    if args.keys.is_empty() {
        bail!("keypress requires at least one key");
    }
    send_windows_key_chord(&args.keys)?;
    Ok(success_outcome(json!({
        "action": "keypress",
        "keys": args.keys,
    })))
}

#[cfg(windows)]
async fn perform_windows_drag(args: &ComputerDragArgs) -> Result<ServiceOutcome> {
    if args.path.len() < 2 {
        bail!("drag requires at least two path points");
    }

    with_windows_modifiers(&args.keys, || {
        let start = &args.path[0];
        set_windows_cursor_position(start.x, start.y)?;
        send_windows_mouse_input(0, 0, MOUSEEVENTF_LEFTDOWN, 0)?;
        for point in args.path.iter().skip(1) {
            set_windows_cursor_position(point.x, point.y)?;
            std::thread::sleep(Duration::from_millis(16));
        }
        send_windows_mouse_input(0, 0, MOUSEEVENTF_LEFTUP, 0)?;
        Ok(())
    })?;

    Ok(success_outcome(json!({
        "action": "drag",
        "points": args.path.len(),
    })))
}

#[cfg(target_os = "macos")]
async fn execute_macos_computer_action(
    method: &ComputerMethod,
    arguments: Value,
) -> Result<ServiceOutcome> {
    match &method.action {
        ComputerUseAction::Screenshot => capture_macos_screenshot(method).await,
        ComputerUseAction::Click => {
            let args: ComputerMouseArgs = serde_json::from_value(arguments)?;
            perform_macos_click(&args, false).await
        }
        ComputerUseAction::DoubleClick => {
            let args: ComputerMouseArgs = serde_json::from_value(arguments)?;
            perform_macos_click(&args, true).await
        }
        ComputerUseAction::Scroll => {
            let args: ComputerScrollArgs = serde_json::from_value(arguments)?;
            perform_macos_scroll(&args).await
        }
        ComputerUseAction::Type => {
            let args: ComputerTypeArgs = serde_json::from_value(arguments)?;
            perform_macos_type(&args).await
        }
        ComputerUseAction::Wait => {
            let args: ComputerWaitArgs =
                serde_json::from_value(arguments).unwrap_or(ComputerWaitArgs {
                    ms: default_wait_ms(),
                });
            sleep(Duration::from_millis(args.ms)).await;
            Ok(success_outcome(json!({ "waited_ms": args.ms })))
        }
        ComputerUseAction::Keypress => {
            let args: ComputerKeypressArgs = serde_json::from_value(arguments)?;
            perform_macos_keypress(&args).await
        }
        ComputerUseAction::Drag => {
            let args: ComputerDragArgs = serde_json::from_value(arguments)?;
            perform_macos_drag(&args).await
        }
        ComputerUseAction::Move => {
            let args: ComputerMouseArgs = serde_json::from_value(arguments)?;
            perform_macos_move(&args).await
        }
    }
}

#[cfg(target_os = "macos")]
async fn capture_macos_screenshot(method: &ComputerMethod) -> Result<ServiceOutcome> {
    let path = std::env::temp_dir().join(format!(
        "bridge-agent-screenshot-{}-{}.png",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));

    let mut command = Command::new("/usr/sbin/screencapture");
    command.arg("-x").arg("-t").arg("png");
    if let Some(display_id) = method.display_id {
        command.arg("-D").arg(display_id.to_string());
    }
    command.arg(&path);

    let output = command
        .output()
        .await
        .context("failed to run screencapture")?;
    if !output.status.success() {
        bail!(
            "screencapture failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    let bytes = fs::read(&path).with_context(|| format!("failed to read {}", path.display()))?;
    let image = image::load_from_memory(&bytes).context("failed to decode screenshot")?;
    let (width, height) = image.dimensions();
    let _ = fs::remove_file(&path);

    if bytes.len() > method.upload.inline_limit_bytes {
        return upload_screenshot(method, bytes, width, height).await;
    }

    Ok(success_outcome(json!({
        "result_type": "inline_image",
        "mime_type": "image/png",
        "width": width,
        "height": height,
        "display_id": method.display_id,
        "size_bytes": bytes.len(),
        "image_base64": BASE64_STANDARD.encode(bytes),
    })))
}

async fn upload_screenshot(
    method: &ComputerMethod,
    bytes: Vec<u8>,
    width: u32,
    height: u32,
) -> Result<ServiceOutcome> {
    let Some(prepare_url) = method.upload_prepare_url.as_deref() else {
        return Ok(ServiceOutcome {
            success: false,
            data: Some(json!({
                "result_type": "too_large",
                "mime_type": "image/png",
                "width": width,
                "height": height,
                "display_id": method.display_id,
                "size_bytes": bytes.len(),
                "inline_limit_bytes": method.upload.inline_limit_bytes,
            })),
            error: Some(InvokeError {
                code: "PAYLOAD_TOO_LARGE".to_string(),
                message: format!(
                    "screenshot is {} bytes, exceeds inline limit {} bytes, and upload.prepare_url is not configured",
                    bytes.len(),
                    method.upload.inline_limit_bytes
                ),
            }),
        });
    };

    let file_name = format!(
        "bridge-agent-screenshot-{}.png",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    );
    let prepare = method
        .client
        .post(prepare_url)
        .timeout(Duration::from_secs(method.upload.timeout_secs))
        .bearer_auth(&method.relay_token)
        .json(&PrepareUploadRequest {
            agent_id: method.agent_id.clone(),
            content_type: "image/png".to_string(),
            file_name,
            size_bytes: bytes.len() as u64,
            workspace_id: method.workspace_id,
            purpose: "computer_screenshot".to_string(),
        })
        .send()
        .await
        .context("failed to request screenshot upload slot")?;

    if !prepare.status().is_success() {
        let status = prepare.status();
        let body = prepare.text().await.unwrap_or_default();
        bail!("prepare upload failed with status {}: {}", status, body);
    }

    let slot: PrepareUploadResponse = prepare
        .json()
        .await
        .context("failed to decode prepare upload response")?;
    let upload_method = slot.method.as_deref().unwrap_or("PUT").parse::<Method>()?;
    let mut upload = method
        .client
        .request(upload_method, &slot.upload_url)
        .timeout(Duration::from_secs(method.upload.timeout_secs))
        .body(bytes.clone());
    for (key, value) in &slot.headers {
        upload = upload.header(key, value);
    }
    if !slot.headers.keys().any(|key| key.eq_ignore_ascii_case("content-type")) {
        upload = upload.header(reqwest::header::CONTENT_TYPE, "image/png");
    }

    let upload_response = upload
        .send()
        .await
        .context("failed to upload screenshot asset")?;
    if !upload_response.status().is_success() {
        let status = upload_response.status();
        let body = upload_response.text().await.unwrap_or_default();
        bail!("screenshot upload failed with status {}: {}", status, body);
    }

    Ok(success_outcome(json!({
        "result_type": "asset_ref",
        "asset_id": slot.file_id,
        "object_key": slot.object_key,
        "download_url": slot.download_url,
        "expires_at": slot.expires_at,
        "mime_type": "image/png",
        "width": width,
        "height": height,
        "display_id": method.display_id,
        "size_bytes": bytes.len(),
    })))
}

#[cfg(target_os = "macos")]
async fn perform_macos_click(
    args: &ComputerMouseArgs,
    double_click: bool,
) -> Result<ServiceOutcome> {
    with_macos_modifiers(&args.keys, || {
        post_mouse_move(args.x, args.y)?;
        let button = parse_mouse_button(&args.button)?;
        post_mouse_click(button, args.x, args.y)?;
        if double_click {
            std::thread::sleep(Duration::from_millis(80));
            post_mouse_click(button, args.x, args.y)?;
        }
        Ok(())
    })?;

    Ok(success_outcome(json!({
        "action": if double_click { "double_click" } else { "click" },
        "x": args.x,
        "y": args.y,
        "button": args.button,
    })))
}

#[cfg(target_os = "macos")]
async fn perform_macos_move(args: &ComputerMouseArgs) -> Result<ServiceOutcome> {
    with_macos_modifiers(&args.keys, || post_mouse_move(args.x, args.y))?;
    Ok(success_outcome(json!({
        "action": "move",
        "x": args.x,
        "y": args.y,
    })))
}

#[cfg(target_os = "macos")]
async fn perform_macos_scroll(args: &ComputerScrollArgs) -> Result<ServiceOutcome> {
    with_macos_modifiers(&args.keys, || {
        post_mouse_move(args.x, args.y)?;
        post_scroll(args.scroll_x, args.scroll_y)
    })?;

    Ok(success_outcome(json!({
        "action": "scroll",
        "x": args.x,
        "y": args.y,
        "scroll_x": args.scroll_x,
        "scroll_y": args.scroll_y,
    })))
}

#[cfg(target_os = "macos")]
async fn perform_macos_type(args: &ComputerTypeArgs) -> Result<ServiceOutcome> {
    let script = format!(
        "tell application \"System Events\" to keystroke {}",
        apple_script_string(&args.text)
    );
    run_osascript(&script).await?;
    Ok(success_outcome(json!({
        "action": "type",
        "length": args.text.chars().count(),
    })))
}

#[cfg(target_os = "macos")]
async fn perform_macos_keypress(args: &ComputerKeypressArgs) -> Result<ServiceOutcome> {
    if args.keys.is_empty() {
        bail!("keypress requires at least one key");
    }
    post_key_chord(&args.keys)?;
    Ok(success_outcome(json!({
        "action": "keypress",
        "keys": args.keys,
    })))
}

#[cfg(target_os = "macos")]
async fn perform_macos_drag(args: &ComputerDragArgs) -> Result<ServiceOutcome> {
    if args.path.len() < 2 {
        bail!("drag requires at least two path points");
    }

    with_macos_modifiers(&args.keys, || {
        let start = &args.path[0];
        post_mouse_move(start.x, start.y)?;
        post_drag_event(CGEventType::LeftMouseDown, start.x, start.y)?;
        for point in args.path.iter().skip(1) {
            post_drag_event(CGEventType::LeftMouseDragged, point.x, point.y)?;
            std::thread::sleep(Duration::from_millis(16));
        }
        let end = args.path.last().expect("drag path has at least 2 points");
        post_drag_event(CGEventType::LeftMouseUp, end.x, end.y)?;
        Ok(())
    })?;

    Ok(success_outcome(json!({
        "action": "drag",
        "points": args.path.len(),
    })))
}

fn success_outcome(data: Value) -> ServiceOutcome {
    ServiceOutcome {
        success: true,
        data: Some(data),
        error: None,
    }
}

#[cfg(target_os = "macos")]
fn with_macos_modifiers<F>(keys: &[String], action: F) -> Result<()>
where
    F: FnOnce() -> Result<()>,
{
    let modifiers: Vec<MacKey> = keys
        .iter()
        .map(|key| parse_modifier_key(key))
        .collect::<Result<Vec<_>>>()?;
    for modifier in &modifiers {
        post_key_event(*modifier, true)?;
    }
    let result = action();
    for modifier in modifiers.into_iter().rev() {
        let _ = post_key_event(modifier, false);
    }
    result
}

#[cfg(target_os = "macos")]
fn post_key_chord(keys: &[String]) -> Result<()> {
    let mut modifiers = Vec::new();
    let mut regular_keys = Vec::new();

    for key in keys {
        let parsed = parse_key(key)?;
        if parsed.is_modifier() {
            modifiers.push(parsed);
        } else {
            regular_keys.push(parsed);
        }
    }

    for key in &modifiers {
        post_key_event(*key, true)?;
    }
    if regular_keys.is_empty() {
        for key in modifiers.iter().rev() {
            post_key_event(*key, false)?;
        }
        return Ok(());
    }
    for key in &regular_keys {
        post_key_event(*key, true)?;
        post_key_event(*key, false)?;
    }
    for key in modifiers.iter().rev() {
        post_key_event(*key, false)?;
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn post_mouse_move(x: f64, y: f64) -> Result<()> {
    post_mouse_event(CGEventType::MouseMoved, CGMouseButton::Left, x, y)
}

#[cfg(target_os = "macos")]
fn post_mouse_click(button: CGMouseButton, x: f64, y: f64) -> Result<()> {
    let (down, up) = mouse_click_event_types(button);
    post_mouse_event(down, button, x, y)?;
    post_mouse_event(up, button, x, y)
}

#[cfg(target_os = "macos")]
fn post_drag_event(event_type: CGEventType, x: f64, y: f64) -> Result<()> {
    post_mouse_event(event_type, CGMouseButton::Left, x, y)
}

#[cfg(target_os = "macos")]
fn post_scroll(scroll_x: i64, scroll_y: i64) -> Result<()> {
    let source = event_source()?;
    let event = CGEvent::new_scroll_event(
        source,
        ScrollEventUnit::PIXEL,
        2,
        scroll_y as i32,
        scroll_x as i32,
        0,
    )
    .map_err(|_| anyhow!("failed to create scroll event"))?;
    event.post(CGEventTapLocation::HID);
    Ok(())
}

#[cfg(target_os = "macos")]
fn post_mouse_event(event_type: CGEventType, button: CGMouseButton, x: f64, y: f64) -> Result<()> {
    let source = event_source()?;
    let point = CGPoint::new(x, y);
    let event = CGEvent::new_mouse_event(source, event_type, point, button)
        .map_err(|_| anyhow!("failed to create mouse event"))?;
    event.post(CGEventTapLocation::HID);
    Ok(())
}

#[cfg(target_os = "macos")]
fn post_key_event(key: MacKey, key_down: bool) -> Result<()> {
    let source = event_source()?;
    let event = CGEvent::new_keyboard_event(source, key.code(), key_down)
        .map_err(|_| anyhow!("failed to create keyboard event"))?;
    event.post(CGEventTapLocation::HID);
    Ok(())
}

#[cfg(target_os = "macos")]
fn event_source() -> Result<CGEventSource> {
    CGEventSource::new(CGEventSourceStateID::CombinedSessionState)
        .map_err(|_| anyhow!("failed to create event source"))
}

#[cfg(target_os = "macos")]
fn mouse_click_event_types(button: CGMouseButton) -> (CGEventType, CGEventType) {
    match button {
        CGMouseButton::Left => (CGEventType::LeftMouseDown, CGEventType::LeftMouseUp),
        CGMouseButton::Right => (CGEventType::RightMouseDown, CGEventType::RightMouseUp),
        _ => (CGEventType::OtherMouseDown, CGEventType::OtherMouseUp),
    }
}

#[cfg(target_os = "macos")]
fn parse_mouse_button(value: &str) -> Result<CGMouseButton> {
    match value.trim().to_ascii_lowercase().as_str() {
        "" | "left" => Ok(CGMouseButton::Left),
        "right" => Ok(CGMouseButton::Right),
        "middle" => Ok(CGMouseButton::Center),
        other => bail!("unsupported mouse button `{other}`"),
    }
}

#[cfg(target_os = "macos")]
async fn run_osascript(script: &str) -> Result<()> {
    let output = Command::new("/usr/bin/osascript")
        .arg("-e")
        .arg(script)
        .output()
        .await
        .context("failed to run osascript")?;
    if !output.status.success() {
        bail!("{}", String::from_utf8_lossy(&output.stderr).trim());
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn apple_script_string(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len() + 2);
    escaped.push('"');
    for ch in value.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\" & linefeed & \""),
            '\r' => {}
            _ => escaped.push(ch),
        }
    }
    escaped.push('"');
    escaped
}

#[cfg(windows)]
#[derive(Clone, Copy)]
struct WindowsKey {
    virtual_key: u16,
}

#[cfg(windows)]
impl WindowsKey {
    fn is_modifier(self) -> bool {
        matches!(
            self.virtual_key,
            VK_SHIFT | VK_CONTROL | VK_MENU | VK_LWIN
        )
    }
}

#[cfg(windows)]
struct WindowsMonitorCapture {
    bytes: Vec<u8>,
    width: u32,
    height: u32,
    display_id: Option<u32>,
}

#[cfg(windows)]
#[derive(Clone, Copy)]
struct WindowsMonitorBounds {
    left: i32,
    top: i32,
    width: i32,
    height: i32,
}

#[cfg(windows)]
fn with_windows_modifiers<F>(keys: &[String], action: F) -> Result<()>
where
    F: FnOnce() -> Result<()>,
{
    let modifiers: Vec<WindowsKey> = keys
        .iter()
        .map(|key| parse_windows_modifier_key(key))
        .collect::<Result<Vec<_>>>()?;
    for key in &modifiers {
        send_windows_virtual_key(key.virtual_key, true)?;
    }
    let result = action();
    for key in modifiers.into_iter().rev() {
        let _ = send_windows_virtual_key(key.virtual_key, false);
    }
    result
}

#[cfg(windows)]
fn send_windows_key_chord(keys: &[String]) -> Result<()> {
    let mut modifiers = Vec::new();
    let mut regular_keys = Vec::new();

    for key in keys {
        let parsed = parse_windows_key(key)?;
        if parsed.is_modifier() {
            modifiers.push(parsed);
        } else {
            regular_keys.push(parsed);
        }
    }

    for key in &modifiers {
        send_windows_virtual_key(key.virtual_key, true)?;
    }
    if regular_keys.is_empty() {
        for key in modifiers.iter().rev() {
            send_windows_virtual_key(key.virtual_key, false)?;
        }
        return Ok(());
    }
    for key in &regular_keys {
        send_windows_virtual_key(key.virtual_key, true)?;
        send_windows_virtual_key(key.virtual_key, false)?;
    }
    for key in modifiers.iter().rev() {
        send_windows_virtual_key(key.virtual_key, false)?;
    }
    Ok(())
}

#[cfg(windows)]
fn send_windows_unicode_text(text: &str) -> Result<()> {
    for unit in text.encode_utf16() {
        let down = INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: 0,
                    wScan: unit,
                    dwFlags: KEYEVENTF_UNICODE,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        };
        let up = INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: 0,
                    wScan: unit,
                    dwFlags: KEYEVENTF_UNICODE | KEYEVENTF_KEYUP,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        };
        send_windows_inputs(&[down, up])?;
    }
    Ok(())
}

#[cfg(windows)]
fn send_windows_virtual_key(virtual_key: u16, key_down: bool) -> Result<()> {
    let input = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: virtual_key,
                wScan: 0,
                dwFlags: if key_down { 0 } else { KEYEVENTF_KEYUP },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    send_windows_inputs(&[input])
}

#[cfg(windows)]
fn send_windows_mouse_input(dx: i32, dy: i32, flags: u32, mouse_data: u32) -> Result<()> {
    let input = INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,
                mouseData: mouse_data,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    send_windows_inputs(&[input])
}

#[cfg(windows)]
fn send_windows_inputs(inputs: &[INPUT]) -> Result<()> {
    let sent = unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            std::mem::size_of::<INPUT>() as i32,
        )
    };
    if sent != inputs.len() as u32 {
        let error = unsafe { GetLastError() };
        bail!("SendInput failed with error {}", error);
    }
    Ok(())
}

#[cfg(windows)]
fn set_windows_cursor_position(x: f64, y: f64) -> Result<()> {
    let x = round_f64_to_i32(x, "x")?;
    let y = round_f64_to_i32(y, "y")?;
    if unsafe { SetCursorPos(x, y) } == 0 {
        let error = unsafe { GetLastError() };
        bail!("SetCursorPos failed with error {}", error);
    }
    Ok(())
}

#[cfg(windows)]
fn windows_mouse_button_flags(button: &str) -> Result<(u32, u32)> {
    match button.trim().to_ascii_lowercase().as_str() {
        "" | "left" => Ok((MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP)),
        "right" => Ok((MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP)),
        "middle" => Ok((MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP)),
        other => bail!("unsupported mouse button `{other}`"),
    }
}

#[cfg(windows)]
fn scale_windows_wheel_delta(amount: i64) -> Result<u32> {
    let delta = amount
        .checked_mul(WHEEL_DELTA as i64)
        .ok_or_else(|| anyhow!("scroll delta is too large"))?;
    let delta = i32::try_from(delta).map_err(|_| anyhow!("scroll delta is too large"))?;
    Ok(delta as u32)
}

#[cfg(windows)]
fn parse_windows_modifier_key(value: &str) -> Result<WindowsKey> {
    match parse_windows_key(value)? {
        key if key.is_modifier() => Ok(key),
        _ => bail!("mouse action modifiers only support Shift / Ctrl / Alt / Win"),
    }
}

#[cfg(windows)]
fn parse_windows_key(value: &str) -> Result<WindowsKey> {
    let virtual_key = match value.trim().to_ascii_uppercase().as_str() {
        "SHIFT" => VK_SHIFT,
        "CTRL" | "CONTROL" => VK_CONTROL,
        "ALT" | "OPTION" => VK_MENU,
        "META" | "COMMAND" | "CMD" | "WIN" | "WINDOWS" => VK_LWIN,
        "ENTER" | "RETURN" => VK_RETURN,
        "TAB" => VK_TAB,
        "SPACE" => VK_SPACE,
        "ESC" | "ESCAPE" => VK_ESCAPE,
        "UP" | "ARROWUP" => VK_UP,
        "DOWN" | "ARROWDOWN" => VK_DOWN,
        "LEFT" | "ARROWLEFT" => VK_LEFT,
        "RIGHT" | "ARROWRIGHT" => VK_RIGHT,
        "HOME" => VK_HOME,
        "END" => VK_END,
        "PAGEUP" => VK_PRIOR,
        "PAGEDOWN" => VK_NEXT,
        "A" => 0x41,
        "B" => 0x42,
        "C" => 0x43,
        "D" => 0x44,
        "E" => 0x45,
        "F" => 0x46,
        "G" => 0x47,
        "H" => 0x48,
        "I" => 0x49,
        "J" => 0x4A,
        "K" => 0x4B,
        "L" => 0x4C,
        "M" => 0x4D,
        "N" => 0x4E,
        "O" => 0x4F,
        "P" => 0x50,
        "Q" => 0x51,
        "R" => 0x52,
        "S" => 0x53,
        "T" => 0x54,
        "U" => 0x55,
        "V" => 0x56,
        "W" => 0x57,
        "X" => 0x58,
        "Y" => 0x59,
        "Z" => 0x5A,
        "0" => 0x30,
        "1" => 0x31,
        "2" => 0x32,
        "3" => 0x33,
        "4" => 0x34,
        "5" => 0x35,
        "6" => 0x36,
        "7" => 0x37,
        "8" => 0x38,
        "9" => 0x39,
        other => bail!("unsupported key `{other}`"),
    };
    Ok(WindowsKey { virtual_key })
}

#[cfg(windows)]
fn round_f64_to_i32(value: f64, label: &str) -> Result<i32> {
    if !value.is_finite() {
        bail!("{label} must be finite");
    }
    let rounded = value.round();
    if rounded < i32::MIN as f64 || rounded > i32::MAX as f64 {
        bail!("{label} is out of range");
    }
    Ok(rounded as i32)
}

#[cfg(windows)]
fn capture_windows_monitor_png(display_id: Option<u32>) -> Result<WindowsMonitorCapture> {
    let bounds = windows_monitor_bounds(display_id)?;
    let width_u32 = u32::try_from(bounds.width).map_err(|_| anyhow!("invalid monitor width"))?;
    let height_u32 =
        u32::try_from(bounds.height).map_err(|_| anyhow!("invalid monitor height"))?;

    let screen_dc = unsafe { GetDC(std::ptr::null_mut()) };
    if screen_dc.is_null() {
        bail!("GetDC failed");
    }
    let screen_dc_guard = ReleaseDcGuard { hdc: screen_dc };

    let memory_dc = unsafe { CreateCompatibleDC(screen_dc) };
    if memory_dc.is_null() {
        bail!("CreateCompatibleDC failed");
    }
    let memory_dc_guard = DeleteDcGuard { hdc: memory_dc };

    let bitmap = unsafe { CreateCompatibleBitmap(screen_dc, bounds.width, bounds.height) };
    if bitmap.is_null() {
        bail!("CreateCompatibleBitmap failed");
    }
    let bitmap_guard = DeleteObjectGuard {
        handle: bitmap as HGDIOBJ,
    };

    let previous = unsafe { SelectObject(memory_dc, bitmap as HGDIOBJ) };
    if previous.is_null() {
        bail!("SelectObject failed");
    }
    let selection_guard = SelectObjectGuard {
        hdc: memory_dc,
        previous,
    };

    if unsafe {
        BitBlt(
            memory_dc,
            0,
            0,
            bounds.width,
            bounds.height,
            screen_dc,
            bounds.left,
            bounds.top,
            SRCCOPY | CAPTUREBLT,
        )
    } == 0
    {
        let error = unsafe { GetLastError() };
        bail!("BitBlt failed with error {}", error);
    }

    let mut bitmap_info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: bounds.width,
            biHeight: -bounds.height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            biSizeImage: (width_u32 * height_u32 * 4),
            biXPelsPerMeter: 0,
            biYPelsPerMeter: 0,
            biClrUsed: 0,
            biClrImportant: 0,
        },
        ..Default::default()
    };
    let mut bgra = vec![0u8; (width_u32 as usize) * (height_u32 as usize) * 4];
    let rows = unsafe {
        GetDIBits(
            memory_dc,
            bitmap as HBITMAP,
            0,
            height_u32,
            bgra.as_mut_ptr().cast(),
            &mut bitmap_info,
            DIB_RGB_COLORS,
        )
    };
    if rows == 0 {
        let error = unsafe { GetLastError() };
        bail!("GetDIBits failed with error {}", error);
    }

    let _ = selection_guard;
    let _ = bitmap_guard;
    let _ = memory_dc_guard;
    let _ = screen_dc_guard;

    let mut rgba = vec![0u8; bgra.len()];
    for (src, dst) in bgra.chunks_exact(4).zip(rgba.chunks_exact_mut(4)) {
        dst[0] = src[2];
        dst[1] = src[1];
        dst[2] = src[0];
        dst[3] = 255;
    }

    let image = image::RgbaImage::from_raw(width_u32, height_u32, rgba)
        .ok_or_else(|| anyhow!("failed to build RGBA image"))?;
    let mut bytes = Vec::new();
    image::DynamicImage::ImageRgba8(image)
        .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
        .context("failed to encode screenshot")?;

    Ok(WindowsMonitorCapture {
        bytes,
        width: width_u32,
        height: height_u32,
        display_id,
    })
}

#[cfg(windows)]
fn windows_monitor_bounds(display_id: Option<u32>) -> Result<WindowsMonitorBounds> {
    if let Some(display_id) = display_id {
        let monitors = list_windows_monitors()?;
        let bounds = monitors
            .get(display_id as usize)
            .copied()
            .ok_or_else(|| anyhow!("display_id {} does not exist", display_id))?;
        return Ok(bounds);
    }

    let left = unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) };
    let top = unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) };
    let width = unsafe { GetSystemMetrics(SM_CXVIRTUALSCREEN) };
    let height = unsafe { GetSystemMetrics(SM_CYVIRTUALSCREEN) };
    if width <= 0 || height <= 0 {
        bail!("virtual screen metrics are invalid");
    }
    Ok(WindowsMonitorBounds {
        left,
        top,
        width,
        height,
    })
}

#[cfg(windows)]
fn list_windows_monitors() -> Result<Vec<WindowsMonitorBounds>> {
    let mut monitors = Vec::new();
    let ok = unsafe {
        EnumDisplayMonitors(
            std::ptr::null_mut(),
            std::ptr::null(),
            Some(collect_windows_monitor),
            (&mut monitors as *mut Vec<WindowsMonitorBounds>) as LPARAM,
        )
    };
    if ok == 0 {
        let error = unsafe { GetLastError() };
        bail!("EnumDisplayMonitors failed with error {}", error);
    }
    if monitors.is_empty() {
        bail!("no monitors found");
    }
    Ok(monitors)
}

#[cfg(windows)]
unsafe extern "system" fn collect_windows_monitor(
    monitor: HMONITOR,
    _hdc: HDC,
    _rect: *mut RECT,
    data: LPARAM,
) -> i32 {
    let monitors = &mut *(data as *mut Vec<WindowsMonitorBounds>);
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
    if GetMonitorInfoW(monitor, &mut info as *mut _ as *mut _) == 0 {
        return 1;
    }
    let rect = info.monitorInfo.rcMonitor;
    monitors.push(WindowsMonitorBounds {
        left: rect.left,
        top: rect.top,
        width: rect.right - rect.left,
        height: rect.bottom - rect.top,
    });
    1
}

#[cfg(windows)]
struct ReleaseDcGuard {
    hdc: HDC,
}

#[cfg(windows)]
impl Drop for ReleaseDcGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = ReleaseDC(std::ptr::null_mut(), self.hdc);
        }
    }
}

#[cfg(windows)]
struct DeleteDcGuard {
    hdc: HDC,
}

#[cfg(windows)]
impl Drop for DeleteDcGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = DeleteDC(self.hdc);
        }
    }
}

#[cfg(windows)]
struct DeleteObjectGuard {
    handle: HGDIOBJ,
}

#[cfg(windows)]
impl Drop for DeleteObjectGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = DeleteObject(self.handle);
        }
    }
}

#[cfg(windows)]
struct SelectObjectGuard {
    hdc: HDC,
    previous: HGDIOBJ,
}

#[cfg(windows)]
impl Drop for SelectObjectGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = SelectObject(self.hdc, self.previous);
        }
    }
}

#[cfg(target_os = "macos")]
fn parse_modifier_key(value: &str) -> Result<MacKey> {
    match parse_key(value)? {
        key if key.is_modifier() => Ok(key),
        _ => bail!("mouse action modifiers only support Shift / Ctrl / Alt / Command"),
    }
}

#[cfg(target_os = "macos")]
fn parse_key(value: &str) -> Result<MacKey> {
    let normalized = value.trim().to_ascii_uppercase();
    match normalized.as_str() {
        "SHIFT" => Ok(MacKey::Shift),
        "CTRL" | "CONTROL" => Ok(MacKey::Control),
        "ALT" | "OPTION" => Ok(MacKey::Option),
        "META" | "COMMAND" | "CMD" => Ok(MacKey::Command),
        "ENTER" | "RETURN" => Ok(MacKey::Return),
        "TAB" => Ok(MacKey::Tab),
        "SPACE" => Ok(MacKey::Space),
        "ESC" | "ESCAPE" => Ok(MacKey::Escape),
        "BACKSPACE" | "DELETE" => Ok(MacKey::Delete),
        "UP" | "ARROWUP" => Ok(MacKey::Up),
        "DOWN" | "ARROWDOWN" => Ok(MacKey::Down),
        "LEFT" | "ARROWLEFT" => Ok(MacKey::Left),
        "RIGHT" | "ARROWRIGHT" => Ok(MacKey::Right),
        "HOME" => Ok(MacKey::Home),
        "END" => Ok(MacKey::End),
        "PAGEUP" => Ok(MacKey::PageUp),
        "PAGEDOWN" => Ok(MacKey::PageDown),
        "A" => Ok(MacKey::A),
        "B" => Ok(MacKey::B),
        "C" => Ok(MacKey::C),
        "D" => Ok(MacKey::D),
        "E" => Ok(MacKey::E),
        "F" => Ok(MacKey::F),
        "G" => Ok(MacKey::G),
        "H" => Ok(MacKey::H),
        "I" => Ok(MacKey::I),
        "J" => Ok(MacKey::J),
        "K" => Ok(MacKey::K),
        "L" => Ok(MacKey::L),
        "M" => Ok(MacKey::M),
        "N" => Ok(MacKey::N),
        "O" => Ok(MacKey::O),
        "P" => Ok(MacKey::P),
        "Q" => Ok(MacKey::Q),
        "R" => Ok(MacKey::R),
        "S" => Ok(MacKey::S),
        "T" => Ok(MacKey::T),
        "U" => Ok(MacKey::U),
        "V" => Ok(MacKey::V),
        "W" => Ok(MacKey::W),
        "X" => Ok(MacKey::X),
        "Y" => Ok(MacKey::Y),
        "Z" => Ok(MacKey::Z),
        "0" => Ok(MacKey::Digit0),
        "1" => Ok(MacKey::Digit1),
        "2" => Ok(MacKey::Digit2),
        "3" => Ok(MacKey::Digit3),
        "4" => Ok(MacKey::Digit4),
        "5" => Ok(MacKey::Digit5),
        "6" => Ok(MacKey::Digit6),
        "7" => Ok(MacKey::Digit7),
        "8" => Ok(MacKey::Digit8),
        "9" => Ok(MacKey::Digit9),
        other => bail!("unsupported key `{other}`"),
    }
}

#[cfg(target_os = "macos")]
#[derive(Clone, Copy)]
enum MacKey {
    Shift,
    Control,
    Option,
    Command,
    Return,
    Tab,
    Space,
    Escape,
    Delete,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    A,
    B,
    C,
    D,
    E,
    F,
    G,
    H,
    I,
    J,
    K,
    L,
    M,
    N,
    O,
    P,
    Q,
    R,
    S,
    T,
    U,
    V,
    W,
    X,
    Y,
    Z,
    Digit0,
    Digit1,
    Digit2,
    Digit3,
    Digit4,
    Digit5,
    Digit6,
    Digit7,
    Digit8,
    Digit9,
}

#[cfg(target_os = "macos")]
impl MacKey {
    fn is_modifier(self) -> bool {
        matches!(
            self,
            Self::Shift | Self::Control | Self::Option | Self::Command
        )
    }

    fn code(self) -> u16 {
        match self {
            Self::A => 0,
            Self::S => 1,
            Self::D => 2,
            Self::F => 3,
            Self::H => 4,
            Self::G => 5,
            Self::Z => 6,
            Self::X => 7,
            Self::C => 8,
            Self::V => 9,
            Self::B => 11,
            Self::Q => 12,
            Self::W => 13,
            Self::E => 14,
            Self::R => 15,
            Self::Y => 16,
            Self::T => 17,
            Self::Digit1 => 18,
            Self::Digit2 => 19,
            Self::Digit3 => 20,
            Self::Digit4 => 21,
            Self::Digit6 => 22,
            Self::Digit5 => 23,
            Self::Digit9 => 25,
            Self::Digit7 => 26,
            Self::Digit8 => 28,
            Self::Digit0 => 29,
            Self::O => 31,
            Self::U => 32,
            Self::I => 34,
            Self::P => 35,
            Self::L => 37,
            Self::J => 38,
            Self::K => 40,
            Self::N => 45,
            Self::M => 46,
            Self::Tab => 48,
            Self::Space => 49,
            Self::Delete => 51,
            Self::Return => 36,
            Self::Escape => 53,
            Self::Command => 55,
            Self::Shift => 56,
            Self::Option => 58,
            Self::Control => 59,
            Self::PageUp => 116,
            Self::PageDown => 121,
            Self::End => 119,
            Self::Home => 115,
            Self::Left => 123,
            Self::Right => 124,
            Self::Down => 125,
            Self::Up => 126,
        }
    }
}

fn collect_headers(headers: &reqwest::header::HeaderMap) -> BTreeMap<String, String> {
    headers
        .iter()
        .filter_map(|(key, value)| {
            value
                .to_str()
                .ok()
                .map(|value| (key.as_str().to_string(), value.to_string()))
        })
        .collect()
}

fn decode_response_body(bytes: &[u8], content_type: &str) -> Value {
    if content_type.contains("application/json") {
        serde_json::from_slice(bytes)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(bytes).trim().to_string()))
    } else {
        Value::String(String::from_utf8_lossy(bytes).trim().to_string())
    }
}

fn query_pairs_from_json(value: &Value) -> Vec<(String, String)> {
    match value {
        Value::Object(map) => map
            .iter()
            .map(|(key, value)| (key.clone(), scalar_to_query_string(value)))
            .collect(),
        _ => Vec::new(),
    }
}

fn scalar_to_query_string(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => value.clone(),
        Value::Array(_) | Value::Object(_) => serde_json::to_string(value).unwrap_or_default(),
    }
}

pub fn sanitize_env(env: BTreeMap<String, String>) -> BTreeMap<String, String> {
    let mut base = BTreeMap::new();
    for key in ["PATH", "HOME", "LANG", "LC_ALL"] {
        if let Ok(value) = std::env::var(key) {
            base.insert(key.to_string(), value);
        }
    }

    for (key, value) in env.into_iter().filter(|(key, _)| {
        key.chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
    }) {
        base.insert(key, value);
    }

    base
}

pub fn is_command_allowed(command: &str, allowlist: &[String]) -> bool {
    if allowlist.iter().any(|allowed| allowed.trim() == "*") {
        return true;
    }
    let name = Path::new(command)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(command);
    allowlist
        .iter()
        .any(|allowed| allowed == command || allowed == name)
}

pub fn resolve_cwd(root_dir: &Path, requested: Option<&str>) -> Result<PathBuf> {
    let candidate = match requested {
        Some(raw) => {
            let path = PathBuf::from(raw);
            if path.is_absolute() {
                path
            } else {
                root_dir.join(path)
            }
        }
        None => root_dir.to_path_buf(),
    };

    let canonical = candidate
        .canonicalize()
        .with_context(|| format!("failed to resolve cwd {}", candidate.display()))?;
    if !canonical.starts_with(root_dir) {
        bail!(
            "cwd {} escapes root dir {}",
            canonical.display(),
            root_dir.display()
        );
    }
    Ok(canonical)
}

#[cfg(test)]
mod tests {
    use super::{is_command_allowed, resolve_cwd, sanitize_env, ServiceRegistry};
    use crate::config::AgentConfig;
    use serde_json::json;
    use std::collections::BTreeMap;
    use std::fs;

    #[test]
    fn allowlist_accepts_basename() {
        assert!(is_command_allowed("/usr/bin/git", &[String::from("git")]));
        assert!(!is_command_allowed("bash", &[String::from("git")]));
    }

    #[test]
    fn allowlist_accepts_wildcard() {
        assert!(is_command_allowed("bash", &[String::from("*")]));
    }

    #[test]
    fn sanitize_env_keeps_safe_keys_only() {
        let mut env = BTreeMap::new();
        env.insert("FOO_BAR".to_string(), "1".to_string());
        env.insert("bad-key".to_string(), "2".to_string());
        let sanitized = sanitize_env(env);
        assert_eq!(sanitized.get("FOO_BAR"), Some(&"1".to_string()));
        assert!(!sanitized.contains_key("bad-key"));
    }

    #[test]
    fn resolve_cwd_rejects_escape() {
        let base = std::env::temp_dir().join(format!("bridge-agent-test-{}", std::process::id()));
        let nested = base.join("nested");
        fs::create_dir_all(&nested).unwrap();
        let root = base.canonicalize().unwrap();
        let escaped = resolve_cwd(&root, Some("../"));
        assert!(escaped.is_err());
        fs::remove_dir_all(&base).unwrap();
    }

    #[tokio::test]
    async fn registry_exposes_enabled_service_definitions() {
        let current_dir = std::env::current_dir().unwrap();
        let registry = ServiceRegistry::from_config(&AgentConfig::example(), &current_dir).unwrap();
        let definitions = registry.definitions();
        assert_eq!(definitions.len(), 1);
        assert_eq!(definitions[0].name, "computer");
        assert_eq!(definitions[0].methods[0].name, "screenshot");
    }

    #[tokio::test]
    async fn unknown_service_returns_error() {
        let current_dir = std::env::current_dir().unwrap();
        let registry = ServiceRegistry::from_config(&AgentConfig::example(), &current_dir).unwrap();
        let result = registry
            .invoke("req-1".to_string(), "git", "status", json!({}), None)
            .await;
        assert!(!result.success);
        assert_eq!(result.error.unwrap().code, "INVOKE_FAILED");
    }
}

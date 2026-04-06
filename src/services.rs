use crate::config::{
    AgentConfig, ComputerUseAction, ComputerUseBinding, HttpBinding, MethodBinding, MethodConfig,
    ServiceConfig, ShellCommandBinding,
};
use crate::protocol::{InvokeError, InvokeResult, ServiceDefinition};
use anyhow::{anyhow, bail, Context, Result};
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use image::GenericImageView;
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
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
            execute_macos_computer_action(&self.action, self.display_id, arguments).await
        }

        #[cfg(not(target_os = "macos"))]
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
            service, method, binding,
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
    binding: &ComputerUseBinding,
) -> Result<ComputerMethod> {
    Ok(ComputerMethod {
        action: binding.action.clone(),
        display_id: binding.display_id,
    })
}

fn default_mouse_button() -> String {
    "left".to_string()
}

fn default_wait_ms() -> u64 {
    500
}

#[cfg(target_os = "macos")]
async fn execute_macos_computer_action(
    action: &ComputerUseAction,
    display_id: Option<u32>,
    arguments: Value,
) -> Result<ServiceOutcome> {
    match action {
        ComputerUseAction::Screenshot => capture_macos_screenshot(display_id).await,
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
async fn capture_macos_screenshot(display_id: Option<u32>) -> Result<ServiceOutcome> {
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
    if let Some(display_id) = display_id {
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

    Ok(success_outcome(json!({
        "mime_type": "image/png",
        "width": width,
        "height": height,
        "display_id": display_id,
        "image_base64": BASE64_STANDARD.encode(bytes),
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

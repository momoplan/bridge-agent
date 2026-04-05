use crate::config::{
    AgentConfig, HttpBinding, MethodBinding, MethodConfig, ServiceConfig, ShellCommandBinding,
};
use crate::protocol::{InvokeError, InvokeResult, ServiceDefinition};
use anyhow::{anyhow, bail, Context, Result};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Instant;
use tokio::process::Command;
use tokio::time::{timeout, Duration};

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
        assert_eq!(definitions[0].methods[0].name, "exec");
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

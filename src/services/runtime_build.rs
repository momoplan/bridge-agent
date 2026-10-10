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
    shell_executions: ShellExecutionStore,
) -> Result<RuntimeService> {
    let mut methods = BTreeMap::new();
    let mut method_definitions = Vec::new();

    for method in &service.methods {
        if !method.enabled {
            continue;
        }
        let runtime_method = build_runtime_method(
            service,
            method,
            config,
            config_base_dir,
            shell_executions.clone(),
        )?;
        methods.insert(method.name.clone(), runtime_method);
        method_definitions.push(crate::protocol::MethodDefinition {
            name: method.name.clone(),
            description: method.description.clone(),
            input_schema: method.input_schema.clone(),
            response_mode: method.response_mode,
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
    shell_executions: ShellExecutionStore,
) -> Result<RuntimeMethod> {
    match &method.binding {
        MethodBinding::ShellCommand(binding) => Ok(RuntimeMethod::Shell(build_shell_method(
            service,
            method,
            binding,
            config,
            config_base_dir,
            shell_executions,
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
    shell_executions: ShellExecutionStore,
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
        executions: shell_executions,
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
        response_mode: method.response_mode,
    })
}

#[cfg(any(target_os = "macos", windows))]
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

#[cfg(not(any(target_os = "macos", windows)))]
fn build_computer_method(
    _service: &ServiceConfig,
    _method: &MethodConfig,
    _config: &AgentConfig,
    _binding: &ComputerUseBinding,
) -> Result<ComputerMethod> {
    Ok(ComputerMethod)
}

#[cfg(any(target_os = "macos", windows))]
fn default_mouse_button() -> String {
    "left".to_string()
}

#[cfg(any(target_os = "macos", windows))]
fn default_wait_ms() -> u64 {
    500
}

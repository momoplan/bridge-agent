/// Materializes the in-memory service used for every local app HTTP request.
///
/// The returned configuration contains the app's private bearer credential and must never be
/// persisted or logged. Keeping this conversion in one place ensures capability invocation,
/// health monitoring, and lifecycle verification all use the same app-scoped authorization.
pub fn local_app_runtime_service(app: &LocalAppConfig) -> Result<ServiceConfig> {
    let mut service = ServiceConfig {
        name: app.app_id.clone(),
        description: app.description.clone(),
        enabled: app.enabled,
        health_check: app.health_check.clone(),
        start_command: app.start_command.clone(),
        stop_command: app.stop_command.clone(),
        methods: app.methods.clone(),
    };
    let token = read_local_app_token(app)?;
    inject_local_app_bearer_token(&mut service, &token);
    Ok(service)
}

/// Live output stream of a local app, authorized with the same private credential.
#[derive(Debug, Clone)]
pub struct LocalAppStreamTarget {
    pub url: String,
    pub headers: BTreeMap<String, String>,
}

pub fn local_app_stream_target(app: &LocalAppConfig) -> Result<Option<LocalAppStreamTarget>> {
    let Some(stream) = app.notification_stream.as_ref() else {
        return Ok(None);
    };
    let mut headers = stream.headers.clone();
    headers.insert(
        "Authorization".to_string(),
        format!("Bearer {}", read_local_app_token(app)?),
    );
    Ok(Some(LocalAppStreamTarget {
        url: stream.url.clone(),
        headers,
    }))
}

fn read_local_app_token(app: &LocalAppConfig) -> Result<String> {
    let token_path = local_app_token_path(app)?;
    let token = fs::read_to_string(&token_path).with_context(|| {
        format!(
            "failed to read private runtime token for local app `{}` from {}",
            app.app_id,
            token_path.display()
        )
    })?;
    let token = token.trim();
    if token.len() < 32 {
        bail!(
            "private runtime token for local app `{}` is invalid: {}",
            app.app_id,
            token_path.display()
        );
    }
    Ok(token.to_string())
}

fn local_app_token_path(app: &LocalAppConfig) -> Result<PathBuf> {
    for command in [&app.start_command, &app.stop_command] {
        if let Some(ServiceStartCommand::ShellCommand { env, .. }) = command {
            if let Some(path) = env
                .get("BAIJIMU_LOCAL_APP_TOKEN_FILE")
                .map(String::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                return Ok(PathBuf::from(path));
            }
        }
    }
    crate::connector::connector_management_token_path(&app.app_id)
}

fn inject_local_app_bearer_token(service: &mut ServiceConfig, token: &str) {
    let authorization = format!("Bearer {token}");
    if let Some(ServiceHealthCheck::Http { headers, .. }) = service.health_check.as_mut() {
        headers.insert("Authorization".to_string(), authorization.clone());
    }
    for method in &mut service.methods {
        if let MethodBinding::Http(binding) = &mut method.binding {
            binding
                .headers
                .insert("Authorization".to_string(), authorization.clone());
        }
    }
}

fn local_app_events(app: &LocalAppConfig) -> BTreeSet<String> {
    app.events
        .iter()
        .filter(|event| event.enabled)
        .map(|event| event.name.clone())
        .collect()
}

fn local_app_definition(app: &LocalAppConfig) -> LocalAppDefinition {
    LocalAppDefinition {
        app_id: app.app_id.clone(),
        name: app.name.clone(),
        version: app.version.clone(),
        description: app.description.clone(),
        methods: app
            .methods
            .iter()
            .filter(|method| method.enabled)
            .map(|method| crate::protocol::MethodDefinition {
                name: method.name.clone(),
                description: method.description.clone(),
                input_schema: method.input_schema.clone(),
                response_mode: method.response_mode,
            })
            .collect(),
        events: app
            .events
            .iter()
            .filter(|event| event.enabled)
            .map(|event| EventDefinition {
                name: event.name.clone(),
                description: event.description.clone(),
                payload_schema: event.payload_schema.clone(),
            })
            .collect(),
    }
}

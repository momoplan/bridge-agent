impl RuntimeMethod {
    async fn invoke(&self, arguments: Value, timeout_secs: Option<u64>) -> Result<ServiceOutcome> {
        match self {
            Self::Shell(method) => method.invoke(arguments, timeout_secs).await,
            Self::Http(method) => method.invoke(arguments, timeout_secs).await,
            Self::Computer(method) => method.invoke(arguments).await,
        }
    }

    async fn invoke_local_app(
        &self,
        arguments: Value,
        timeout_secs: Option<u64>,
        workspace_id: Option<u64>,
    ) -> Result<ServiceOutcome> {
        match self {
            Self::Http(method) => {
                method
                    .invoke_with_workspace(arguments, timeout_secs, workspace_id)
                    .await
            }
            Self::Shell(method) => method.invoke(arguments, timeout_secs).await,
            Self::Computer(method) => method.invoke(arguments).await,
        }
    }
}

async fn ensure_registered_service_ready(service: &ServiceConfig, client: &Client) -> bool {
    let Some(health_check) = service.health_check.as_ref() else {
        return true;
    };

    if registered_service_is_healthy(service, client).await {
        return true;
    }

    if service_start_is_manual(service.start_command.as_ref()) {
        info!(
            service = %service.name,
            "registered service requires an explicit user start; skipping automatic start"
        );
        return false;
    }
    let Some(start_command) = service.start_command.as_ref() else {
        return false;
    };
    if !start_registered_service(service, start_command).await {
        return false;
    }
    wait_for_registered_service_health(service, health_check, client).await
}

fn service_start_is_manual(start_command: Option<&ServiceStartCommand>) -> bool {
    matches!(
        start_command,
        Some(ServiceStartCommand::ShellCommand { env, .. })
            if env.get(LOCAL_APP_START_POLICY_ENV).map(String::as_str) == Some("manual")
    )
}

async fn start_registered_service(
    service: &ServiceConfig,
    start_command: &ServiceStartCommand,
) -> bool {
    match run_registered_service_start_command(service, start_command).await {
        Ok(completed) if completed.success => {
            info!(
                service = %service.name,
                duration_ms = completed.duration_ms,
                "registered service start command completed"
            );
            true
        }
        Ok(completed) => {
            warn!(
                service = %service.name,
                exit_code = ?completed.exit_code,
                timed_out = completed.timed_out,
                stderr = %completed.stderr.trim(),
                "registered service start command failed"
            );
            false
        }
        Err(err) => {
            warn!(service = %service.name, error = %err, "registered service start command failed");
            false
        }
    }
}

async fn wait_for_registered_service_health(
    service: &ServiceConfig,
    health_check: &ServiceHealthCheck,
    client: &Client,
) -> bool {
    for _ in 0..20 {
        if check_registered_service_health(service, health_check, client)
            .await
            .is_ok()
        {
            return true;
        }
        sleep(Duration::from_millis(500)).await;
    }

    false
}

async fn registered_service_is_healthy(service: &ServiceConfig, client: &Client) -> bool {
    let Some(health_check) = service.health_check.as_ref() else {
        return true;
    };
    match check_registered_service_health(service, health_check, client).await {
        Ok(()) => true,
        Err(err) => {
            info!(service = %service.name, error = %err, "registered service health check failed");
            false
        }
    }
}

async fn check_registered_service_health(
    service: &ServiceConfig,
    health_check: &ServiceHealthCheck,
    client: &Client,
) -> Result<()> {
    match health_check {
        ServiceHealthCheck::Http {
            url,
            http_method,
            headers,
            timeout_secs,
            expect_status,
            body_contains,
        } => {
            let method = http_method
                .parse::<Method>()
                .with_context(|| format!("invalid health check method for `{}`", service.name))?;
            let mut request = client
                .request(method, url)
                .timeout(Duration::from_secs(timeout_secs.unwrap_or(3).max(1)));
            for (key, value) in headers {
                request = request.header(key, value);
            }

            let response = request
                .send()
                .await
                .with_context(|| format!("health check request failed for `{}`", service.name))?;
            let expected_status = expect_status.unwrap_or(200);
            let status = response.status();
            if status.as_u16() != expected_status {
                bail!(
                    "health check for `{}` returned HTTP {}, expected {}",
                    service.name,
                    status.as_u16(),
                    expected_status
                );
            }

            if let Some(expected_text) = body_contains
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                let body = response
                    .text()
                    .await
                    .with_context(|| format!("read health check body for `{}`", service.name))?;
                if !body.contains(expected_text) {
                    bail!(
                        "health check body for `{}` did not contain expected text",
                        service.name
                    );
                }
            }

            Ok(())
        }
    }
}

async fn run_registered_service_start_command(
    service: &ServiceConfig,
    start_command: &ServiceStartCommand,
) -> Result<CompletedShellExec> {
    match start_command {
        ServiceStartCommand::ShellCommand {
            command,
            cwd,
            env,
            timeout_secs,
        } => {
            if command.is_empty() || command[0].trim().is_empty() {
                bail!("service `{}` start command is empty", service.name);
            }

            let cwd = match cwd
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                Some(cwd) => PathBuf::from(cwd),
                None => std::env::current_dir().context("resolve current directory")?,
            };
            let mut command_env = env.clone();
            enrich_user_command_environment(command.first().map(String::as_str), &mut command_env);
            let path_for_diagnostics = command_env.get("PATH").cloned().unwrap_or_default();
            let prepared = PreparedShellExec {
                command_args: command.clone(),
                cwd,
                env: command_env,
                stdin: None,
                timeout_secs: Some(timeout_secs.unwrap_or(15).max(1)),
                path_for_diagnostics,
            };
            Ok(run_shell_command(prepared, None, None).await)
        }
    }
}

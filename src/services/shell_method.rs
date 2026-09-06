impl ShellMethod {
    async fn invoke(&self, arguments: Value, timeout_secs: Option<u64>) -> Result<ServiceOutcome> {
        match self.method_name.as_str() {
            "startExecution" => self.start_execution(arguments, timeout_secs).await,
            "queryExecution" => self.get_execution(arguments).await,
            "cancelExecution" => self.cancel_execution(arguments).await,
            _ => self.exec(arguments, timeout_secs).await,
        }
    }

    async fn exec(&self, arguments: Value, timeout_secs: Option<u64>) -> Result<ServiceOutcome> {
        let prepared = match self.prepare_execution(arguments, timeout_secs, true, true)? {
            Ok(prepared) => prepared,
            Err(outcome) => return Ok(outcome),
        };
        let snapshot = self.executions.start(prepared).await;
        let snapshot = self
            .executions
            .wait_for_terminal_snapshot(&snapshot.execution_id, shell_exec_sync_wait_duration())
            .await
            .unwrap_or(snapshot);
        let running = matches!(snapshot.status, ShellExecutionStatus::Running);
        let recommended_method = running.then_some("queryExecution");

        Ok(ServiceOutcome {
            success: running || snapshot.success.unwrap_or(false),
            data: Some(serde_json::to_value(ShellExecData::from_snapshot(
                &snapshot,
                recommended_method,
                running.then(|| self.service_name.clone()),
                recommended_method,
            ))?),
            error: if running { None } else { snapshot.error },
        })
    }

    async fn start_execution(
        &self,
        arguments: Value,
        timeout_secs: Option<u64>,
    ) -> Result<ServiceOutcome> {
        let prepared = match self.prepare_execution(arguments, timeout_secs, false, false)? {
            Ok(prepared) => prepared,
            Err(outcome) => return Ok(outcome),
        };
        let snapshot = self.executions.start(prepared).await;

        Ok(ServiceOutcome {
            success: true,
            data: Some(serde_json::to_value(snapshot).context("serialize execution snapshot")?),
            error: None,
        })
    }

    async fn get_execution(&self, arguments: Value) -> Result<ServiceOutcome> {
        let args: ShellExecutionIdArgs = serde_json::from_value(arguments).map_err(|err| {
            anyhow!(
                "invalid arguments for {}.{}: {err}",
                self.service_name,
                self.method_name
            )
        })?;
        let execution_id = args.execution_id.trim();
        if execution_id.is_empty() {
            bail!(
                "{}.{} requires executionId",
                self.service_name,
                self.method_name
            );
        }

        match self.executions.get(execution_id).await {
            Some(snapshot) => Ok(ServiceOutcome {
                success: true,
                data: Some(serde_json::to_value(snapshot).context("serialize execution snapshot")?),
                error: None,
            }),
            None => Ok(ServiceOutcome {
                success: false,
                data: None,
                error: Some(InvokeError {
                    code: "EXECUTION_NOT_FOUND".to_string(),
                    message: format!("execution `{execution_id}` was not found"),
                }),
            }),
        }
    }

    async fn cancel_execution(&self, arguments: Value) -> Result<ServiceOutcome> {
        let args: ShellExecutionIdArgs = serde_json::from_value(arguments).map_err(|err| {
            anyhow!(
                "invalid arguments for {}.{}: {err}",
                self.service_name,
                self.method_name
            )
        })?;
        let execution_id = args.execution_id.trim();
        if execution_id.is_empty() {
            bail!(
                "{}.{} requires executionId",
                self.service_name,
                self.method_name
            );
        }

        match self.executions.cancel(execution_id).await {
            Some(snapshot) => Ok(ServiceOutcome {
                success: true,
                data: Some(serde_json::to_value(snapshot).context("serialize execution snapshot")?),
                error: None,
            }),
            None => Ok(ServiceOutcome {
                success: false,
                data: None,
                error: Some(InvokeError {
                    code: "EXECUTION_NOT_FOUND".to_string(),
                    message: format!("execution `{execution_id}` was not found"),
                }),
            }),
        }
    }

    fn prepare_execution(
        &self,
        arguments: Value,
        request_timeout_secs: Option<u64>,
        use_request_timeout: bool,
        use_default_timeout: bool,
    ) -> Result<std::result::Result<PreparedShellExec, ServiceOutcome>> {
        let args: ShellExecArgs = serde_json::from_value(arguments).map_err(|err| {
            anyhow!(
                "invalid arguments for {}.{}: {err}",
                self.service_name,
                self.method_name
            )
        })?;
        if args.command.is_empty() {
            bail!(
                "{}.{} requires a non-empty command",
                self.service_name,
                self.method_name
            );
        }

        let command_args = args.command.into_args();
        let executable = &command_args[0];
        if !is_command_allowed(executable, &self.allow_commands) {
            return Ok(Err(ServiceOutcome {
                success: false,
                data: None,
                error: Some(InvokeError {
                    code: "COMMAND_NOT_ALLOWED".to_string(),
                    message: format!("command `{executable}` is not in allowlist"),
                }),
            }));
        }

        let cwd = resolve_cwd(&self.root_dir, args.cwd.as_deref())?;
        let requested_timeout_secs = if use_request_timeout {
            request_timeout_secs.or(args.timeout_secs)
        } else {
            args.timeout_secs
        };
        let timeout_secs = requested_timeout_secs
            .filter(|value| *value > 0)
            .map(|value| value.min(self.max_timeout_secs))
            .or_else(|| use_default_timeout.then_some(self.default_timeout_secs));
        let env = sanitize_env(args.env);
        let path_for_diagnostics = env.get("PATH").cloned().unwrap_or_default();

        Ok(Ok(PreparedShellExec {
            command_args,
            cwd,
            env,
            stdin: args.stdin,
            timeout_secs,
            path_for_diagnostics,
        }))
    }
}

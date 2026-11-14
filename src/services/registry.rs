impl ServiceRegistry {
    /// Installed, explicitly enabled capabilities. Process readiness belongs to
    /// the lifecycle monitor and never removes declarations from this registry.
    pub fn from_config(config: &AgentConfig, config_base_dir: &Path) -> Result<Self> {
        let mut services = BTreeMap::new();
        let mut local_apps = BTreeMap::new();
        let shell_executions = ShellExecutionStore::default();

        for service in &config.services {
            if !service.enabled {
                continue;
            }
            let runtime_service =
                build_runtime_service(service, config, config_base_dir, shell_executions.clone())?;
            if !runtime_service.methods.is_empty() {
                services.insert(service.name.clone(), runtime_service);
            }
        }

        for app in &config.local_apps {
            if !app.enabled {
                continue;
            }
            let service = local_app_runtime_service(app)?;
            let runtime =
                build_runtime_service(&service, config, config_base_dir, shell_executions.clone())?;
            let events = local_app_events(app);
            let stream = local_app_stream_target(app)?;
            if !runtime.methods.is_empty() || !events.is_empty() || stream.is_some() {
                local_apps.insert(
                    app.app_id.clone(),
                    RuntimeLocalApp {
                        definition: local_app_definition(app),
                        runtime,
                        events,
                        stream,
                    },
                );
            }
        }

        Ok(Self {
            services: Arc::new(services),
            local_apps: Arc::new(local_apps),
        })
    }

    pub fn definitions(&self) -> Vec<ServiceDefinition> {
        self.services
            .values()
            .map(|service| service.definition.clone())
            .collect()
    }

    pub fn local_app_definitions(&self) -> Vec<LocalAppDefinition> {
        self.local_apps
            .values()
            .map(|app| app.definition.clone())
            .collect()
    }

    pub fn local_app_stream(&self, app_id: &str) -> Option<LocalAppStreamTarget> {
        self.local_apps.get(app_id)?.stream.clone()
    }

    pub fn has_local_app_event(&self, app_id: &str, event: &str) -> bool {
        self.local_apps
            .get(app_id)
            .map(|app| app.events.contains(event))
            .unwrap_or(false)
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
                    public_details: None,
                    code: "INVOKE_FAILED".to_string(),
                    message: err.to_string(),
                }),
                duration_ms: started.elapsed().as_millis() as u64,
            },
        }
    }

    pub async fn invoke_local_app(
        &self,
        request_id: String,
        workspace_id: Option<u64>,
        app_id: &str,
        method: &str,
        arguments: Value,
        timeout_secs: Option<u64>,
    ) -> InvokeResult {
        let started = Instant::now();
        let response = match self.local_apps.get(app_id) {
            Some(app) => match app.runtime.methods.get(method) {
                Some(runtime_method) => {
                    runtime_method
                        .invoke_local_app(arguments, timeout_secs, workspace_id)
                        .await
                }
                None => Err(anyhow!("unknown method `{method}` on local app `{app_id}`")),
            },
            None => Err(anyhow!("unknown local app `{app_id}`")),
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
                    public_details: None,
                    code: "LOCAL_APP_INVOKE_FAILED".to_string(),
                    message: err.to_string(),
                }),
                duration_ms: started.elapsed().as_millis() as u64,
            },
        }
    }
}

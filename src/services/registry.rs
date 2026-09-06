impl ServiceRegistry {
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
            if !runtime.methods.is_empty() || !events.is_empty() {
                local_apps.insert(
                    app.app_id.clone(),
                    RuntimeLocalApp {
                        definition: local_app_definition(app),
                        runtime,
                        events,
                    },
                );
            }
        }

        Ok(Self {
            services,
            local_apps,
        })
    }

    /// Builds the capabilities that are safe to advertise without waiting on any
    /// external process or network endpoint. Health-checked services are added by
    /// a later readiness refresh after the relay connection has started.
    pub fn from_config_initial(config: &AgentConfig, config_base_dir: &Path) -> Result<Self> {
        let mut services = BTreeMap::new();
        let mut local_apps = BTreeMap::new();
        let shell_executions = ShellExecutionStore::default();

        for service in &config.services {
            if !service.enabled || service.health_check.is_some() {
                continue;
            }
            let runtime_service =
                build_runtime_service(service, config, config_base_dir, shell_executions.clone())?;
            if !runtime_service.methods.is_empty() {
                services.insert(service.name.clone(), runtime_service);
            }
        }

        for app in &config.local_apps {
            if !app.enabled || app.health_check.is_some() {
                continue;
            }
            let service = local_app_runtime_service(app)?;
            let runtime =
                build_runtime_service(&service, config, config_base_dir, shell_executions.clone())?;
            let events = local_app_events(app);
            if !runtime.methods.is_empty() || !events.is_empty() {
                local_apps.insert(
                    app.app_id.clone(),
                    RuntimeLocalApp {
                        definition: local_app_definition(app),
                        runtime,
                        events,
                    },
                );
            }
        }

        Ok(Self {
            services,
            local_apps,
        })
    }

    pub async fn from_config_checked(config: &AgentConfig, config_base_dir: &Path) -> Result<Self> {
        let mut services = BTreeMap::new();
        let mut local_apps = BTreeMap::new();
        let shell_executions = ShellExecutionStore::default();
        let health_client = Client::builder()
            .timeout(Duration::from_secs(3))
            .build()
            .context("build registered service health client")?;

        for service in &config.services {
            if !service.enabled {
                continue;
            }
            if !ensure_registered_service_ready(service, &health_client).await {
                warn!(
                    service = %service.name,
                    "registered service is not healthy; omitting from runtime capabilities"
                );
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
            // Connector lifecycle belongs to the desktop process supervisor. The
            // registry only observes readiness and must never execute a host-owned
            // foreground command as if it were a one-shot service start command.
            if !registered_service_is_healthy(&service, &health_client).await {
                warn!(
                    app_id = %app.app_id,
                    "local app is not healthy; omitting from runtime capabilities"
                );
                continue;
            }
            let runtime =
                build_runtime_service(&service, config, config_base_dir, shell_executions.clone())?;
            let events = local_app_events(app);
            if !runtime.methods.is_empty() || !events.is_empty() {
                local_apps.insert(
                    app.app_id.clone(),
                    RuntimeLocalApp {
                        definition: local_app_definition(app),
                        runtime,
                        events,
                    },
                );
            }
        }

        Ok(Self {
            services,
            local_apps,
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
                    code: "LOCAL_APP_INVOKE_FAILED".to_string(),
                    message: err.to_string(),
                }),
                duration_ms: started.elapsed().as_millis() as u64,
            },
        }
    }
}

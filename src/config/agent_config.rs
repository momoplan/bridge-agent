impl AgentConfig {
    pub fn example() -> Self {
        Self {
            platform: PlatformConfig {
            environment_key: None,                base_url: environment::official_api_base(),
                workspace_id: None,
            },
            upload: UploadConfig::default(),
            relay: RelayConfig {
                url: DEFAULT_RELAY_URL.to_string(),
                agent_id: generate_agent_id(),
                token: String::new(),
                token_issued_at_epoch_seconds: None,
                token_expires_at_epoch_seconds: None,
                reconnect_secs: default_reconnect_secs(),
            },
            device: DeviceConfig {
                name: default_device_name(),
                description: "Installed on the user's local machine.".to_string(),
                tags: vec!["desktop".to_string(), "local".to_string()],
            },
            runtime: RuntimeConfig {
                node_path: None,
                python_path: None,
                default_timeout_secs: default_timeout_secs(),
                max_timeout_secs: default_max_timeout_secs(),
                log_limit: default_log_limit(),
                log_file_enabled: default_log_file_enabled(),
                log_file_dir: None,
                log_file_max_bytes: default_log_file_max_bytes(),
                log_file_max_files: default_log_file_max_files(),
                event_server_enabled: default_event_server_enabled(),
                event_server_bind: default_event_server_bind(),
                service_registration_enabled: true,
                service_registration_token: Some(generate_registration_token()),
            },
            services: vec![default_computer_service(), default_shell_service()],
            local_apps: Vec::new(),
        }
    }

    pub fn normalize(&mut self) -> bool {
        let mut changed = ensure_default_computer_methods(self);
        changed |= ensure_default_shell_service(self);
        changed |= ensure_service_registration_defaults(self);
        changed |= ensure_default_platform_base_url(self);
        changed |= remove_legacy_codex_binary_overrides(self);
        changed
    }

    pub fn validate(&self) -> Result<()> {
        if self.platform.base_url.trim().is_empty() {
            bail!("platform.base_url cannot be empty");
        }
        if let Some(prepare_url) = &self.upload.prepare_url {
            if prepare_url.trim().is_empty() {
                bail!("upload.prepare_url cannot be empty when set");
            }
        }
        if self.upload.inline_limit_bytes == 0 {
            bail!("upload.inline_limit_bytes must be greater than zero");
        }
        if self.upload.timeout_secs == 0 {
            bail!("upload.timeout_secs must be greater than zero");
        }
        if self.relay.url.trim().is_empty() {
            bail!("relay.url cannot be empty");
        }
        if self.relay.agent_id.trim().is_empty() {
            bail!("relay.agent_id cannot be empty");
        }
        if self.runtime.default_timeout_secs == 0 || self.runtime.max_timeout_secs == 0 {
            bail!("runtime timeouts must be greater than zero");
        }
        validate_optional_runtime_path("runtime.node_path", self.runtime.node_path.as_deref())?;
        validate_optional_runtime_path("runtime.python_path", self.runtime.python_path.as_deref())?;
        if self.runtime.default_timeout_secs > self.runtime.max_timeout_secs {
            bail!("runtime.default_timeout_secs cannot exceed runtime.max_timeout_secs");
        }
        if self.runtime.log_limit == 0 {
            bail!("runtime.log_limit must be greater than zero");
        }
        if self.runtime.log_file_enabled {
            if self.runtime.log_file_max_bytes < 1024 {
                bail!("runtime.log_file_max_bytes must be at least 1024");
            }
            if self.runtime.log_file_max_files == 0 {
                bail!("runtime.log_file_max_files must be greater than zero");
            }
        }
        if self.runtime.event_server_enabled {
            self.runtime
                .event_server_bind
                .parse::<SocketAddr>()
                .with_context(|| "runtime.event_server_bind must be a socket address")?;
        }
        if self.runtime.service_registration_enabled {
            let bind: SocketAddr = self
                .runtime
                .event_server_bind
                .parse()
                .with_context(|| "runtime.event_server_bind must be a socket address")?;
            if !bind.ip().is_loopback() {
                bail!("runtime.service_registration_enabled requires event_server_bind to be loopback");
            }
            if self
                .runtime
                .service_registration_token
                .as_deref()
                .map(str::trim)
                .unwrap_or_default()
                .is_empty()
            {
                bail!("runtime.service_registration_token is required when service registration is enabled");
            }
        }

        validate_services(&self.services)?;
        validate_local_apps(&self.local_apps)?;

        Ok(())
    }

    pub fn service_definitions(&self) -> Vec<ServiceDefinition> {
        self.services
            .iter()
            .filter(|service| service.enabled)
            .map(|service| ServiceDefinition {
                name: service.name.clone(),
                description: service.description.clone(),
                methods: service
                    .methods
                    .iter()
                    .filter(|method| method.enabled)
                    .map(|method| MethodDefinition {
                        name: method.name.clone(),
                        description: method.description.clone(),
                        input_schema: method.input_schema.clone(),
                        response_mode: method.response_mode,
                    })
                    .collect(),
            })
            .filter(|service| !service.methods.is_empty())
            .collect()
    }

    pub fn local_app_definitions(&self) -> Vec<LocalAppDefinition> {
        self.local_apps
            .iter()
            .filter(|app| app.enabled)
            .map(|app| LocalAppDefinition {
                app_id: app.app_id.clone(),
                name: app.name.clone(),
                version: app.version.clone(),
                description: app.description.clone(),
                methods: app
                    .methods
                    .iter()
                    .filter(|method| method.enabled)
                    .map(|method| MethodDefinition {
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
            })
            .filter(|app| !app.methods.is_empty() || !app.events.is_empty())
            .collect()
    }

    pub fn manifest_preview(&self) -> ManifestPreview {
        ManifestPreview {
            device: self.device.clone(),
            services: self.service_definitions(),
            local_apps: self.local_app_definitions(),
        }
    }

    pub fn browser_auth_manifest_preview(&self) -> BrowserAuthManifestPreview {
        BrowserAuthManifestPreview {
            device: self.device.clone(),
            services: self
                .services
                .iter()
                .filter(|service| service.enabled)
                .map(|service| BrowserAuthServiceDefinition {
                    name: service.name.clone(),
                    description: service.description.clone(),
                    methods: service
                        .methods
                        .iter()
                        .filter(|method| method.enabled)
                        .map(|method| BrowserAuthMethodDefinition {
                            name: method.name.clone(),
                            description: method.description.clone(),
                        })
                        .collect(),
                })
                .filter(|service| !service.methods.is_empty())
                .collect(),
            local_apps: self
                .local_apps
                .iter()
                .filter(|app| app.enabled)
                .map(|app| BrowserAuthLocalAppDefinition {
                    app_id: app.app_id.clone(),
                    name: app.name.clone(),
                    version: app.version.clone(),
                    description: app.description.clone(),
                    methods: app
                        .methods
                        .iter()
                        .filter(|method| method.enabled)
                        .map(|method| BrowserAuthMethodDefinition {
                            name: method.name.clone(),
                            description: method.description.clone(),
                        })
                        .collect(),
                    events: app
                        .events
                        .iter()
                        .filter(|event| event.enabled)
                        .map(|event| BrowserAuthEventDefinition {
                            name: event.name.clone(),
                            description: event.description.clone(),
                        })
                        .collect(),
                })
                .filter(|app| !app.methods.is_empty() || !app.events.is_empty())
                .collect(),
        }
    }
}

fn validate_services(services: &[ServiceConfig]) -> Result<()> {
    let mut service_names = BTreeSet::new();
    for service in services {
        if service.name.trim().is_empty() {
            bail!("service name cannot be empty");
        }
        if !service_names.insert(service.name.as_str()) {
            bail!("duplicate service `{}`", service.name);
        }
        validate_service_methods(service)?;
    }
    Ok(())
}

fn validate_service_methods(service: &ServiceConfig) -> Result<()> {
    let mut method_names = BTreeSet::new();
    for method in &service.methods {
        if method.name.trim().is_empty() {
            bail!("method name cannot be empty in service `{}`", service.name);
        }
        if !method_names.insert(method.name.as_str()) {
            bail!("duplicate method `{}` in service `{}`", method.name, service.name);
        }
        match &method.binding {
            MethodBinding::ShellCommand(binding) if binding.root_dir.trim().is_empty() => bail!(
                "shell binding root_dir cannot be empty for {}.{}",
                service.name,
                method.name
            ),
            MethodBinding::ShellCommand(binding) if binding.allow_commands.is_empty() => bail!(
                "shell binding allow_commands cannot be empty for {}.{}",
                service.name,
                method.name
            ),
            MethodBinding::Http(binding) if binding.url.trim().is_empty() => bail!(
                "http binding url cannot be empty for {}.{}",
                service.name,
                method.name
            ),
            MethodBinding::Http(binding) if binding.http_method.trim().is_empty() => bail!(
                "http binding method cannot be empty for {}.{}",
                service.name,
                method.name
            ),
            _ => {}
        }
    }
    Ok(())
}

fn validate_local_apps(local_apps: &[LocalAppConfig]) -> Result<()> {
    let mut app_ids = BTreeSet::new();
    for app in local_apps {
        if app.app_id.trim().is_empty() {
            bail!("local app appId cannot be empty");
        }
        if !app_ids.insert(app.app_id.as_str()) {
            bail!("duplicate local app appId `{}`", app.app_id);
        }
        if app.name.trim().is_empty() || app.version.trim().is_empty() {
            bail!("local app name and version cannot be empty for `{}`", app.app_id);
        }
        if app.methods.is_empty() && app.events.is_empty() {
            bail!("local app `{}` must declare at least one method or event", app.app_id);
        }
        validate_local_app_capabilities(app)?;
    }
    Ok(())
}

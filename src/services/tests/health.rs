    #[tokio::test]
    async fn checked_registry_omits_unhealthy_registered_service() {
        let current_dir = std::env::current_dir().unwrap();
        let mut config = AgentConfig::example();
        config.services.push(ServiceConfig {
            name: "wechatLocal".to_string(),
            description: "Local WeChat collector.".to_string(),
            enabled: true,
            health_check: Some(ServiceHealthCheck::Http {
                url: "http://127.0.0.1:1/health".to_string(),
                http_method: "GET".to_string(),
                headers: BTreeMap::new(),
                timeout_secs: Some(1),
                expect_status: Some(200),
                body_contains: None,
            }),
            start_command: None,
            stop_command: None,
            methods: vec![MethodConfig {
                name: "getRecentSessions".to_string(),
                description: "Recent sessions.".to_string(),
                enabled: true,
                input_schema: json!({"type": "object"}),
                response_mode: ResponseMode::Cmodel,
                binding: MethodBinding::Http(HttpBinding {
                    url: "http://127.0.0.1:1/invoke/getRecentSessions".to_string(),
                    http_method: "POST".to_string(),
                    headers: BTreeMap::new(),
                    timeout_secs: Some(1),
                }),
            }],
        });

        let registry = ServiceRegistry::from_config_checked(&config, &current_dir)
            .await
            .unwrap();
        assert!(!registry
            .definitions()
            .iter()
            .any(|service| service.name == "wechatLocal"));
    }

    #[tokio::test]
    async fn checked_registry_never_auto_starts_manual_connector_service() {
        let current_dir = std::env::current_dir().unwrap();
        let mut config = AgentConfig::example();
        config.services.push(ServiceConfig {
            name: "permissionGatedService".to_string(),
            description: "Requires explicit user permission.".to_string(),
            enabled: true,
            health_check: Some(ServiceHealthCheck::Http {
                url: "http://127.0.0.1:1/health".to_string(),
                http_method: "GET".to_string(),
                headers: BTreeMap::new(),
                timeout_secs: Some(1),
                expect_status: Some(200),
                body_contains: None,
            }),
            start_command: Some(ServiceStartCommand::ShellCommand {
                command: vec!["this-command-must-never-run".to_string()],
                cwd: None,
                env: BTreeMap::from([(
                    LOCAL_APP_START_POLICY_ENV.to_string(),
                    "manual".to_string(),
                )]),
                timeout_secs: Some(1),
            }),
            stop_command: None,
            methods: Vec::new(),
        });

        let registry = ServiceRegistry::from_config_checked(&config, &current_dir)
            .await
            .unwrap();
        assert!(registry
            .definitions()
            .iter()
            .all(|service| service.name != "permissionGatedService"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn checked_registry_observes_local_app_without_executing_its_start_command() {
        let current_dir = std::env::current_dir().unwrap();
        let marker_dir = tempdir().unwrap();
        let marker = marker_dir.path().join("local-app-started");
        let token_path = marker_dir.path().join("management-token");
        fs::write(&token_path, "bjm_app_test_host_owned_private_runtime_token").unwrap();
        let mut config = AgentConfig::example();
        config.local_apps.push(LocalAppConfig {
            app_id: "com.baijimu.connector.host-owned".to_string(),
            name: "Host-owned Connector".to_string(),
            version: "1.0.0".to_string(),
            description: "Must be started by the desktop process supervisor.".to_string(),
            enabled: true,
            health_check: Some(ServiceHealthCheck::Http {
                url: "http://127.0.0.1:1/health".to_string(),
                http_method: "GET".to_string(),
                headers: BTreeMap::new(),
                timeout_secs: Some(1),
                expect_status: Some(200),
                body_contains: None,
            }),
            start_command: Some(ServiceStartCommand::ShellCommand {
                command: vec![
                    "/bin/sh".to_string(),
                    "-c".to_string(),
                    format!("touch '{}'", marker.display()),
                ],
                cwd: None,
                env: BTreeMap::from([
                    (
                        LOCAL_APP_START_POLICY_ENV.to_string(),
                        "automatic".to_string(),
                    ),
                    (
                        "BAIJIMU_LOCAL_APP_TOKEN_FILE".to_string(),
                        token_path.display().to_string(),
                    ),
                ]),
                timeout_secs: Some(1),
            }),
            stop_command: None,
            methods: Vec::new(),
            events: vec![EventConfig {
                name: "ready".to_string(),
                description: "Ready.".to_string(),
                enabled: true,
                payload_schema: json!({"type": "object"}),
            }],
        });

        let registry = ServiceRegistry::from_config_checked(&config, &current_dir)
            .await
            .unwrap();
        assert!(!marker.exists());
        assert!(registry.local_app_definitions().is_empty());
    }

    #[test]
    fn initial_registry_omits_health_checked_capabilities() {
        let current_dir = std::env::current_dir().unwrap();
        let mut config = AgentConfig::example();
        config.local_apps.push(LocalAppConfig {
            app_id: "com.baijimu.connector.pending".to_string(),
            name: "Pending Connector".to_string(),
            version: "1.0.0".to_string(),
            description: "Waits for its supervised process.".to_string(),
            enabled: true,
            health_check: Some(ServiceHealthCheck::Http {
                url: "http://127.0.0.1:1/health".to_string(),
                http_method: "GET".to_string(),
                headers: BTreeMap::new(),
                timeout_secs: Some(1),
                expect_status: Some(200),
                body_contains: None,
            }),
            start_command: None,
            stop_command: None,
            methods: Vec::new(),
            events: vec![EventConfig {
                name: "ready".to_string(),
                description: "Ready.".to_string(),
                enabled: true,
                payload_schema: json!({"type": "object"}),
            }],
        });

        let registry = ServiceRegistry::from_config_initial(&config, &current_dir).unwrap();
        let definitions = registry.definitions();
        assert!(definitions.iter().any(|service| service.name == "shell"));
        assert!(registry.local_app_definitions().is_empty());
    }

    #[tokio::test]
    async fn checked_registry_keeps_healthy_registered_service() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new().route("/health", get(|| async { "ok" })),
            )
            .await
            .unwrap();
        });

        let current_dir = std::env::current_dir().unwrap();
        let mut config = AgentConfig::example();
        config.services.push(ServiceConfig {
            name: "healthyTool".to_string(),
            description: "Healthy HTTP tool.".to_string(),
            enabled: true,
            health_check: Some(ServiceHealthCheck::Http {
                url: format!("http://{addr}/health"),
                http_method: "GET".to_string(),
                headers: BTreeMap::new(),
                timeout_secs: Some(1),
                expect_status: Some(200),
                body_contains: Some("ok".to_string()),
            }),
            start_command: None,
            stop_command: None,
            methods: vec![MethodConfig {
                name: "status".to_string(),
                description: "Read status.".to_string(),
                enabled: true,
                input_schema: json!({"type": "object"}),
                response_mode: ResponseMode::Cmodel,
                binding: MethodBinding::Http(HttpBinding {
                    url: format!("http://{addr}/status"),
                    http_method: "POST".to_string(),
                    headers: BTreeMap::new(),
                    timeout_secs: Some(1),
                }),
            }],
        });

        let registry = ServiceRegistry::from_config_checked(&config, &current_dir)
            .await
            .unwrap();
        assert!(registry
            .definitions()
            .iter()
            .any(|service| service.name == "healthyTool"));
        server.abort();
    }

fn installed_app(token: &std::path::Path, endpoint: &str) -> LocalAppConfig {
    LocalAppConfig {
        app_id: "installed-app".into(),
        name: "Installed app".into(),
        version: "1.0.0".into(),
        description: "Installed capabilities are independent of process health.".into(),
        enabled: true,
        health_check: Some(ServiceHealthCheck::Http {
            url: format!("{endpoint}/health"),
            http_method: "GET".into(),
            headers: BTreeMap::new(),
            timeout_secs: Some(1),
            expect_status: Some(200),
            body_contains: None,
        }),
        start_command: Some(ServiceStartCommand::ShellCommand {
            command: vec!["capability-discovery-must-not-start-this-process".into()],
            cwd: None,
            env: BTreeMap::from([(
                "BAIJIMU_LOCAL_APP_TOKEN_FILE".into(),
                token.display().to_string(),
            )]),
            timeout_secs: Some(1),
        }),
        stop_command: None,
        methods: vec![MethodConfig {
            name: "status".into(),
            description: "Execute the actual method without a health preflight.".into(),
            enabled: true,
            input_schema: json!({"type":"object"}),
            response_mode: ResponseMode::Cmodel,
            binding: MethodBinding::Http(HttpBinding {
                url: format!("{endpoint}/invoke"),
                http_method: "POST".into(),
                headers: BTreeMap::new(),
                timeout_secs: Some(1),
            }),
        }],
        events: vec![EventConfig {
            name: "messageReceived".into(),
            description: "An event accepted into the device queue.".into(),
            enabled: true,
            payload_schema: json!({"type":"object"}),
        }],
        notification_stream: None,
    }
}

#[tokio::test]
async fn installed_capabilities_survive_failed_health_without_probing_or_starting() {
    use std::sync::{atomic::{AtomicUsize, Ordering}, Arc};
    let health_requests = Arc::new(AtomicUsize::new(0));
    let observed = health_requests.clone();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, Router::new()
            .route("/health", get(move || {
                observed.fetch_add(1, Ordering::SeqCst);
                async { axum::http::StatusCode::SERVICE_UNAVAILABLE }
            }))
            .route("/invoke", post(|| async { Json(json!({"ok":true,"data":{"executed":true}})) })))
            .await.unwrap();
    });
    let dir = tempdir().unwrap();
    let token = dir.path().join("token");
    fs::write(&token, "bjm_app_installed_capabilities_test_private_token").unwrap();
    let mut config = AgentConfig::example();
    config.local_apps = vec![installed_app(&token, &endpoint)];
    let registry = ServiceRegistry::from_config(&config, dir.path()).unwrap();
    assert_eq!(registry.local_app_definitions()[0].methods[0].name, "status");
    assert!(registry.has_local_app_event("installed-app", "messageReceived"));
    let result = registry.invoke_local_app("request".into(), Some(1), "installed-app", "status", json!({}), Some(1)).await;
    assert!(result.success, "{result:?}");
    assert_eq!(health_requests.load(Ordering::SeqCst), 0);
    server.abort();
}

#[test]
fn installed_declarations_follow_configuration_changes_and_uninstall() {
    let dir = tempdir().unwrap();
    let token = dir.path().join("token");
    fs::write(&token, "bjm_app_installed_capabilities_test_private_token").unwrap();
    let mut config = AgentConfig::example();
    config.local_apps = vec![installed_app(&token, "http://127.0.0.1:1")];
    let initial = ServiceRegistry::from_config(&config, dir.path()).unwrap();
    assert!(initial.has_local_app_event("installed-app", "messageReceived"));
    config.local_apps[0].version = "1.0.1".into();
    let refreshed = ServiceRegistry::from_config(&config, dir.path()).unwrap();
    assert_eq!(refreshed.local_app_definitions()[0].version, "1.0.1");
    assert!(refreshed.has_local_app_event("installed-app", "messageReceived"));
    config.local_apps.clear();
    let uninstalled = ServiceRegistry::from_config(&config, dir.path()).unwrap();
    assert!(uninstalled.local_app_definitions().is_empty());
    assert!(!uninstalled.has_local_app_event("installed-app", "messageReceived"));
}

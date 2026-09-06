#[test]
fn market_manifest_accepts_multiline_legacy_release_notes() {
    let manifest = serde_json::json!({
        "changelog": "- 新增能力\n* 修复问题\n\n"
    });
    assert_eq!(
        market_release_notes(&manifest),
        vec!["新增能力".to_string(), "修复问题".to_string()]
    );
}

#[test]
fn market_host_compatibility_checks_version_and_capabilities() {
    let setup = serde_json::json!({
        "hostRequirements": {
            "minimumVersion": env!("CARGO_PKG_VERSION"),
            "capabilities": ["connector.setup.v1"]
        }
    });
    assert!(market_host_compatibility(&setup, None).compatible);

    let future = serde_json::json!({
        "hostRequirements": {"minimumVersion": "99.0.0"}
    });
    let incompatible = market_host_compatibility(&future, None);
    assert!(!incompatible.compatible);
    assert!(incompatible.message.unwrap().contains("请先升级客户端"));

    let missing = serde_json::json!({
        "hostRequirements": {"capabilities": ["connector.unknown.v1"]}
    });
    assert!(!market_host_compatibility(&missing, None).compatible);
}

#[test]
fn config_for_ui_redacts_relay_credentials_and_reports_status() {
    let mut config = AgentConfig::example();
    config.relay.token = "relay-secret".to_string();

    let value = config_for_ui(&config).unwrap();

    assert_eq!(value["relay"]["token"], "");
    assert_eq!(value["credential_status"]["relay_token_configured"], true);
    assert!(!value.to_string().contains("relay-secret"));
}

fn registered_status(
    status: RegisteredServiceState,
    checked_at_ms: u64,
) -> RegisteredServiceStatus {
    RegisteredServiceStatus {
        service: "local-app".to_string(),
        status,
        detail: None,
        checked_at_ms,
        health_check_configured: true,
        start_command_configured: true,
        stop_command_configured: true,
    }
}

#[test]
fn registered_service_monitor_emits_only_meaningful_changes() {
    let previous = vec![registered_status(RegisteredServiceState::Healthy, 100)];
    let refreshed = vec![registered_status(RegisteredServiceState::Healthy, 200)];
    let unhealthy = vec![registered_status(RegisteredServiceState::Unhealthy, 300)];

    assert!(!registered_service_statuses_changed(&previous, &refreshed));
    assert!(registered_service_statuses_changed(&previous, &unhealthy));
}

fn local_app_status(
    status: RegisteredServiceState,
    process_running: Option<bool>,
) -> LocalAppRuntimeStatus {
    LocalAppRuntimeStatus {
        app_id: "com.baijimu.connector.test".to_string(),
        status,
        detail: None,
        checked_at_ms: 100,
        health_check_configured: false,
        start_command_configured: true,
        stop_command_configured: true,
        process_managed: process_running.is_some(),
        process_running,
    }
}

#[test]
fn inactive_connector_status_is_derived_without_a_health_probe() {
    let app = LocalAppConfig {
        app_id: "com.baijimu.connector.inactive".to_string(),
        name: "Inactive Connector".to_string(),
        version: "1.0.0".to_string(),
        description: String::new(),
        enabled: true,
        health_check: Some(ServiceHealthCheck::Http {
            url: "http://127.0.0.1:9/health".to_string(),
            http_method: "GET".to_string(),
            headers: BTreeMap::new(),
            timeout_secs: Some(60),
            expect_status: Some(200),
            body_contains: None,
        }),
        start_command: None,
        stop_command: None,
        methods: Vec::new(),
        events: Vec::new(),
    };

    let status = inactive_local_app_status(app, None);

    assert_eq!(status.app_id, "com.baijimu.connector.inactive");
    assert_eq!(status.status, RegisteredServiceState::Unhealthy);
    assert_eq!(
        status.detail.as_deref(),
        Some("应用尚未由 Bridge Agent 启动")
    );
    assert!(status.health_check_configured);
    assert!(!status.process_managed);
    assert_eq!(status.process_running, None);
}

#[tokio::test]
async fn local_app_health_probe_uses_the_app_scoped_bearer_token() {
    async fn authorized_health(
        AxumState(expected_token): AxumState<String>,
        headers: HeaderMap,
    ) -> StatusCode {
        let expected = format!("Bearer {expected_token}");
        if headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            == Some(expected.as_str())
        {
            StatusCode::OK
        } else {
            StatusCode::UNAUTHORIZED
        }
    }

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let token = "bjm_app_desktop_health_private_runtime_token";
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/health", get(authorized_health))
                .with_state(token.to_string()),
        )
        .await
        .unwrap();
    });
    let directory = tempfile::tempdir().unwrap();
    let token_path = directory.path().join("management-token");
    fs::write(&token_path, token).unwrap();
    let app = LocalAppConfig {
        app_id: "com.baijimu.connector.authenticated-health".to_string(),
        name: "Authenticated health Connector".to_string(),
        version: "1.0.0".to_string(),
        description: String::new(),
        enabled: true,
        health_check: Some(ServiceHealthCheck::Http {
            url: format!("http://{address}/health"),
            http_method: "GET".to_string(),
            headers: BTreeMap::new(),
            timeout_secs: Some(1),
            expect_status: Some(200),
            body_contains: None,
        }),
        start_command: Some(ServiceStartCommand::ShellCommand {
            command: vec!["unused-local-app-start".to_string()],
            cwd: None,
            env: BTreeMap::from([(
                "BAIJIMU_LOCAL_APP_TOKEN_FILE".to_string(),
                token_path.display().to_string(),
            )]),
            timeout_secs: Some(1),
        }),
        stop_command: None,
        methods: Vec::new(),
        events: Vec::new(),
    };

    let client = Client::builder().build().unwrap();
    let status = check_local_app(&client, app, Some(true)).await;

    assert_eq!(status.status, RegisteredServiceState::Healthy);
    assert_eq!(status.detail.as_deref(), Some("health HTTP 200"));
    server.abort();
}

#[tokio::test]
async fn local_app_health_probe_reports_an_invalid_private_token_without_sending_a_request() {
    let directory = tempfile::tempdir().unwrap();
    let token_path = directory.path().join("management-token");
    fs::write(&token_path, "short-secret").unwrap();
    let app = LocalAppConfig {
        app_id: "com.baijimu.connector.invalid-token".to_string(),
        name: "Invalid token Connector".to_string(),
        version: "1.0.0".to_string(),
        description: String::new(),
        enabled: true,
        health_check: Some(ServiceHealthCheck::Http {
            url: "http://127.0.0.1:9/health".to_string(),
            http_method: "GET".to_string(),
            headers: BTreeMap::new(),
            timeout_secs: Some(1),
            expect_status: Some(200),
            body_contains: None,
        }),
        start_command: Some(ServiceStartCommand::ShellCommand {
            command: vec!["unused-local-app-start".to_string()],
            cwd: None,
            env: BTreeMap::from([(
                "BAIJIMU_LOCAL_APP_TOKEN_FILE".to_string(),
                token_path.display().to_string(),
            )]),
            timeout_secs: Some(1),
        }),
        stop_command: None,
        methods: Vec::new(),
        events: Vec::new(),
    };

    let client = Client::builder().build().unwrap();
    let status = check_local_app(&client, app, Some(true)).await;

    assert_eq!(status.status, RegisteredServiceState::Unknown);
    let detail = status.detail.unwrap();
    assert!(detail.contains("应用本机凭证"));
    assert!(detail.contains("invalid"));
    assert!(!detail.contains("short-secret"));
}

#[test]
fn host_managed_process_state_is_authoritative_without_health_check() {
    let mut running = registered_status(RegisteredServiceState::NotConfigured, 100);
    running.health_check_configured = false;
    apply_managed_process_status(&mut running, Some(true));
    assert_eq!(running.status, RegisteredServiceState::Healthy);
    assert_eq!(running.detail.as_deref(), Some("宿主管理进程正在运行"));

    let mut stopped = registered_status(RegisteredServiceState::NotConfigured, 100);
    stopped.health_check_configured = false;
    apply_managed_process_status(&mut stopped, Some(false));
    assert_eq!(stopped.status, RegisteredServiceState::Unhealthy);
    assert_eq!(stopped.detail.as_deref(), Some("宿主管理进程未运行"));
}

#[test]
fn health_check_remains_authoritative_when_configured() {
    let mut unhealthy = registered_status(RegisteredServiceState::Unhealthy, 100);
    unhealthy.detail = Some("health HTTP 503".to_string());
    apply_managed_process_status(&mut unhealthy, Some(true));
    assert_eq!(unhealthy.status, RegisteredServiceState::Unhealthy);
    assert_eq!(unhealthy.detail.as_deref(), Some("health HTTP 503"));
}

#[test]
fn health_error_detail_preserves_connector_readiness_root_cause() {
    let body = serde_json::to_vec(&serde_json::json!({
        "ok": false,
        "status": {
            "startup": {
                "status": "failed",
                "error": "同步用户级 CODEX_HOME 失败：Windows 环境广播超时"
            }
        },
        "error": {
            "code": "connector_initialization_failed",
            "message": "同步用户级 CODEX_HOME 失败：Windows 环境广播超时"
        }
    }))
    .unwrap();

    assert_eq!(
        format_health_http_error(503, 200, &body),
        "health HTTP 503，期望 200：同步用户级 CODEX_HOME 失败：Windows 环境广播超时"
    );
}

#[test]
fn health_error_detail_ignores_unstructured_or_secret_fields() {
    let body = serde_json::to_vec(&serde_json::json!({
        "token": "must-not-be-rendered",
        "details": "arbitrary connector response"
    }))
    .unwrap();

    let detail = format_health_http_error(503, 200, &body);
    assert_eq!(detail, "health HTTP 503，期望 200");
    assert!(!detail.contains("must-not-be-rendered"));
}

#[test]
fn health_error_detail_is_bounded_for_local_connector_responses() {
    let body = serde_json::to_vec(&serde_json::json!({
        "error": {"message": "x".repeat(HEALTH_ERROR_MESSAGE_MAX_CHARS + 100)}
    }))
    .unwrap();

    let detail = format_health_http_error(503, 200, &body);
    let rendered = detail.split_once('：').unwrap().1;
    assert_eq!(rendered.chars().count(), HEALTH_ERROR_MESSAGE_MAX_CHARS);
}

#[test]
fn local_app_monitor_detects_process_lifecycle_changes() {
    let stopped = vec![local_app_status(
        RegisteredServiceState::Unhealthy,
        Some(false),
    )];
    let running = vec![local_app_status(
        RegisteredServiceState::Healthy,
        Some(true),
    )];
    assert!(local_app_runtime_statuses_changed(&stopped, &running));
    assert!(!local_app_runtime_statuses_changed(&running, &running));
}

#[test]
fn local_app_change_notifications_have_monotonic_revisions_and_context() {
    let notifier = LocalAppsChangeNotifier::default();

    let installed = notifier.notify(
        LocalAppsChangeOperation::Install,
        "com.baijimu.connector.test",
    );
    let upgraded = notifier.notify(
        LocalAppsChangeOperation::Upgrade,
        "com.baijimu.connector.test",
    );
    let synced = notifier.notify(LocalAppsChangeOperation::Sync, "com.baijimu.connector.test");

    assert_eq!(installed.revision, 1);
    assert_eq!(installed.operation, LocalAppsChangeOperation::Install);
    assert_eq!(installed.app_id, "com.baijimu.connector.test");
    assert_eq!(upgraded.revision, 2);
    assert_eq!(upgraded.operation, LocalAppsChangeOperation::Upgrade);
    assert_eq!(synced.revision, 3);
    assert_eq!(synced.operation, LocalAppsChangeOperation::Sync);
}

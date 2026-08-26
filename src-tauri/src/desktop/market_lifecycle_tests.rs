use super::*;

#[test]
fn registered_app_version_identity_requires_exact_semver() {
    let identity =
        RegisteredAppVersionIdentity::parse("app-1".to_string(), "3.0.1-beta.2+macos".to_string())
            .unwrap();
    assert_eq!(identity.app_id, "app-1");
    assert_eq!(identity.version.to_string(), "3.0.1-beta.2+macos");

    for invalid in ["3", "3.0", "v3.0.1", "3.00.1", " 3.0.1"] {
        assert!(
            RegisteredAppVersionIdentity::parse("app-1".to_string(), invalid.to_string()).is_err()
        );
    }
    assert!(
        RegisteredAppVersionIdentity::parse(" app-1".to_string(), "3.0.1".to_string()).is_err()
    );
}

#[test]
fn registered_install_url_preserves_base_path_and_encodes_identity() {
    let identity = RegisteredAppVersionIdentity::parse(
        "app/with/slash".to_string(),
        "3.0.1+macos".to_string(),
    )
    .unwrap();
    let url = registered_install_url("https://api.example.com/lowcode3/", &identity).unwrap();
    assert_eq!(
        url.as_str(),
        "https://api.example.com/lowcode3/api/local-app-registry/apps/app%2Fwith%2Fslash/versions/3.0.1+macos"
    );
}

#[test]
fn unreviewed_registered_version_requires_explicit_acceptance() {
    let registered = RegisteredInstallSource {
        identity: RegisteredAppVersionIdentity::parse("app-1".to_string(), "3.0.1".to_string())
            .unwrap(),
        review_status: "DRAFT".to_string(),
        name: "测试应用".to_string(),
        publisher: "测试发布者".to_string(),
        source: "https://example.invalid/app.zip".to_string(),
        checksum: "0".repeat(64),
    };
    let error = ensure_registered_install_is_accepted(&registered, false).unwrap_err();
    assert!(error.contains("尚未经过市场公开审核"));
    assert!(error.contains("app-1@3.0.1"));
    assert!(ensure_registered_install_is_accepted(&registered, true).is_ok());
}

#[test]
fn local_app_install_contract_accepts_identity_and_rejects_source_url() {
    let request: LocalAppControlInstallRequest = serde_json::from_value(serde_json::json!({
        "appId": "app-1",
        "version": "3.0.1",
        "replace": true,
        "start": true,
        "acceptUnreviewed": true
    }))
    .unwrap();
    assert_eq!(request.app_id, "app-1");
    assert_eq!(request.version, "3.0.1");

    assert!(
        serde_json::from_value::<LocalAppControlInstallRequest>(serde_json::json!({
            "source": "https://example.invalid/app.git#v3.0.1",
            "replace": true,
            "start": true,
            "acceptUnreviewed": true
        }))
        .is_err()
    );
}

#[test]
fn boxed_command_error_payloads_preserve_the_ipc_contract() {
    let conflict = RuntimeLockConflict {
        pid: 42,
        agent_id: "agent-test".to_string(),
        config_path: "/tmp/config.toml".to_string(),
        lock_path: "/tmp/runtime.lock".to_string(),
        process: bridge_agent::RuntimeProcessInfo {
            pid: 42,
            parent_pid: Some(1),
            name: Some("bridge-agent".to_string()),
            executable_path: Some("/tmp/bridge-agent".to_string()),
            command_line: None,
            running: true,
        },
    };
    let runtime_error = serde_json::to_value(CommandError::from(anyhow::Error::new(conflict)))
        .expect("runtime command error should serialize");
    assert_eq!(runtime_error["code"], "runtime_already_running");
    assert_eq!(runtime_error["conflict"]["pid"], 42);
    assert_eq!(runtime_error["conflict"]["process"]["running"], true);

    let lifecycle_error = ConnectorLifecycleManager::default()
        .try_management_permit("connector.test")
        .err()
        .expect("absent connector must reject management requests");
    let management_error =
        serde_json::to_value(ConnectorManagementCommandError::from(lifecycle_error))
            .expect("management command error should serialize");
    assert_eq!(management_error["code"], "connector_not_ready");
    assert_eq!(management_error["lifecycle"]["appId"], "connector.test");

    assert!(std::mem::size_of::<CommandError>() <= 64);
    assert!(std::mem::size_of::<ConnectorManagementCommandError>() <= 64);
}

#[test]
fn normalized_platform_uses_the_rust_target_os_contract() {
    assert_eq!(normalized_platform(), std::env::consts::OS);
}

fn market_connector(checksum: Option<&str>) -> MarketConnectorApp {
    MarketConnectorApp {
        app_id: "com.baijimu.connector.test".to_string(),
        application_type: "connector".to_string(),
        name: "Test Connector".to_string(),
        description: String::new(),
        source: "https://downloads.example.test/connector.zip".to_string(),
        repo: "https://github.com/example/connector".to_string(),
        revision: "0123456789abcdef".to_string(),
        checksum: checksum.map(str::to_string),
        archive_path: None,
        risk: String::new(),
        risk_level: "medium".to_string(),
        capability: String::new(),
        version: "1.0.0".to_string(),
        published_at: None,
        icon_data_url: None,
        release_notes: Vec::new(),
        configuration_declaration: "undeclared".to_string(),
        interface_declaration: "undeclared".to_string(),
        database_declaration: "undeclared".to_string(),
        config_schema: None,
        database: None,
        methods: Vec::new(),
        events: Vec::new(),
        method_names: Vec::new(),
        event_names: Vec::new(),
        permissions: Vec::new(),
        compatible: true,
        compatibility_message: None,
        minimum_host_version: None,
        required_host_capabilities: Vec::new(),
        missing_host_capabilities: Vec::new(),
    }
}

#[test]
fn local_app_install_tasks_track_progress_and_reject_duplicate_active_installs() {
    let manager = LocalAppInstallTaskManager::default();
    let task = manager
        .create(
            LocalAppInstallTaskOperation::Upgrade,
            Some("com.baijimu.connector.codex".to_string()),
            "Codex".to_string(),
            Some("1.2.1".to_string()),
        )
        .unwrap();
    assert_eq!(task.phase, LocalAppInstallTaskPhase::Queued);
    assert_eq!(task.operation, LocalAppInstallTaskOperation::Upgrade);
    assert_eq!(task.message, "等待开始升级");
    assert!(manager
        .create(
            LocalAppInstallTaskOperation::Upgrade,
            Some("com.baijimu.connector.codex".to_string()),
            "Codex".to_string(),
            Some("1.2.1".to_string()),
        )
        .unwrap_err()
        .contains("已在升级中"));

    let reporter = LocalAppInstallProgressReporter {
        manager: manager.clone(),
        task_id: task.task_id.clone(),
        on_event: None,
    };
    reporter.report(
        LocalAppInstallTaskPhase::Resolving,
        Some(5),
        "fallback message",
    );
    let resolving = manager.list().pop().unwrap();
    assert_eq!(resolving.message, "正在解析升级来源");
    reporter.download(50, Some(100));
    let downloading = manager.list().pop().unwrap();
    assert_eq!(downloading.phase, LocalAppInstallTaskPhase::Downloading);
    assert_eq!(downloading.progress_percent, Some(32));
    assert_eq!(downloading.downloaded_bytes, Some(50));
    assert_eq!(downloading.total_bytes, Some(100));

    manager.update(&task.task_id, |task| {
        task.phase = LocalAppInstallTaskPhase::Succeeded;
        task.progress_percent = Some(100);
    });
    assert!(manager
        .create(
            LocalAppInstallTaskOperation::Upgrade,
            Some("com.baijimu.connector.codex".to_string()),
            "Codex".to_string(),
            Some("1.2.1".to_string()),
        )
        .is_ok());
}

#[test]
fn local_app_install_progress_formats_download_sizes() {
    assert_eq!(format_byte_count(512), "512 B");
    assert_eq!(format_byte_count(1536), "1.5 KB");
    assert_eq!(format_byte_count(3 * 1024 * 1024), "3.0 MB");
}

#[test]
fn connector_uninstall_errors_preserve_force_eligibility_for_the_frontend() {
    let stop_failed = serde_json::to_value(ConnectorUninstallCommandError::StopFailed {
        message: "stop failed".to_string(),
    })
    .unwrap();
    let uninstall_failed = serde_json::to_value(ConnectorUninstallCommandError::Failed {
        message: "directory locked".to_string(),
    })
    .unwrap();

    assert_eq!(
        stop_failed["code"],
        serde_json::json!("connector_uninstall_stop_failed")
    );
    assert_eq!(
        uninstall_failed["code"],
        serde_json::json!("connector_uninstall_failed")
    );
}

#[test]
fn registered_desktop_commands_exactly_match_composition_root_acl() {
    let backend = include_str!("app.rs");
    let permissions = include_str!("../../permissions/main.toml");
    let handler_section = backend
        .split_once("tauri::generate_handler![")
        .and_then(|(_, rest)| rest.split_once("])"))
        .map(|(section, _)| section)
        .expect("desktop backend must register a Tauri command handler");
    let allow_section = permissions
        .split_once("commands.allow = [")
        .and_then(|(_, rest)| rest.split_once(']'))
        .map(|(section, _)| section)
        .expect("main ACL must define commands.allow");
    let registered = handler_section
        .lines()
        .map(str::trim)
        .map(|line| line.trim_end_matches(','))
        .filter(|line| !line.is_empty())
        .collect::<std::collections::BTreeSet<_>>();
    let allowed = allow_section
        .lines()
        .map(str::trim)
        .map(|line| line.trim_end_matches(',').trim_matches('"'))
        .filter(|line| !line.is_empty())
        .collect::<std::collections::BTreeSet<_>>();
    let missing = registered.difference(&allowed).copied().collect::<Vec<_>>();
    let stale = allowed.difference(&registered).copied().collect::<Vec<_>>();
    assert!(
        missing.is_empty() && stale.is_empty(),
        "desktop command ACL drift: missing=[{}], stale=[{}]",
        missing.join(", "),
        stale.join(", ")
    );
}

#[test]
fn market_connector_trust_requires_checksum_and_matching_identity() {
    let valid = market_connector(Some(&"a".repeat(64)));
    assert_eq!(
        required_market_checksum(&valid).unwrap(),
        format!("sha256:{}", "a".repeat(64))
    );
    assert!(validate_market_app_identity(&valid, "com.baijimu.connector.test").is_ok());

    assert!(required_market_checksum(&market_connector(None)).is_err());
    assert!(required_market_checksum(&market_connector(Some("invalid"))).is_err());
    assert!(validate_market_app_identity(&valid, "com.example.other").is_err());

    let mut insecure = valid;
    insecure.source = "http://downloads.example.test/connector.zip".to_string();
    assert!(validate_market_app_identity(&insecure, "com.baijimu.connector.test").is_err());
}

#[test]
fn market_manifest_exposes_release_notes_and_update_shape() {
    let manifest = serde_json::json!({
        "releaseNotes": ["新增文件发送", "修复重连", ""],
        "configSchema": {"type": "object", "required": ["token"], "properties": {"token": {"type": "string"}}},
        "upgradeReview": {"configuration": "declared", "interfaces": "declared", "database": "declared"},
        "methods": [{"name": "message.send", "path": "/send", "httpMethod": "POST", "input_schema": {"type": "object"}}, {"name": "file.send"}],
        "events": [{"name": "message.received", "payload_schema": {"type": "object"}}],
        "database": {
            "engine": "sqlite",
            "schemaVersion": "2",
            "migrations": [{
                "id": "002-add-status",
                "fromVersion": "1",
                "toVersion": "2",
                "description": "新增状态字段",
                "changes": [{"operation": "add_column", "target": "messages.status", "description": "新增状态", "destructive": false}],
                "destructive": false,
                "rollback": "automatic",
                "downtime": "none"
            }]
        },
        "permissions": [{
            "id": "filesystem",
            "title": "文件读取",
            "description": "选择文件后读取内容",
            "platforms": ["macos"]
        }]
    });

    assert_eq!(
        market_release_notes(&manifest),
        vec!["新增文件发送".to_string(), "修复重连".to_string()]
    );
    let methods = market_manifest_method_contracts(&manifest);
    assert_eq!(
        methods
            .iter()
            .map(|method| method.name.as_str())
            .collect::<Vec<_>>(),
        vec!["message.send", "file.send"]
    );
    assert_eq!(methods[0].path, "/send");
    assert_eq!(
        market_manifest_event_contracts(&manifest)[0].name,
        "message.received"
    );
    let database = market_manifest_database(&manifest).unwrap();
    assert_eq!(database.schema_version, "2");
    assert_eq!(database.migrations[0].changes[0].target, "messages.status");
    assert_eq!(
        market_contract_declaration(&manifest, "database", true, "connector"),
        "declared"
    );
    assert_eq!(
        market_contract_declaration(&serde_json::json!({}), "database", false, "managed_tool"),
        "not_applicable"
    );
    assert_eq!(market_manifest_permissions(&manifest)[0].id, "filesystem");
}

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

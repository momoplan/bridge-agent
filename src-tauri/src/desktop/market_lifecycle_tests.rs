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
fn market_presentation_preserves_source_without_using_manifest_download_urls() {
    let manifest: Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/market-managed-tool-3.0.0.json"
    ))
    .unwrap();
    let listing: local_app_contract::MarketListing = serde_json::from_value(serde_json::json!({
        "contractVersion":"2.0.0", "presentation":{"name":"Reviewed CLI title","description":"Reviewed description","publisher":null,"capability":"Run platform commands","risk":{"level":"medium","description":"Runs local commands"},"icon":null},
        "marketKey":"test-market", "listingId":"00000000-0000-0000-0000-000000000001",
        "frozenVersion": {"contractVersion":"1.0.0", "source": {
            "application":{"environmentKey":"author-a", "appId":"test-app"}, "version":"1.0.0"
        }, "content":{"applicationType":"managed_tool", "sourceRevision":"commit-1", "manifest":manifest,
            "artifacts":[{"artifactId":"00000000-0000-0000-0000-000000000002", "platform":"linux", "architecture":"x86_64", "fileName":"tool.zip", "sizeBytes":18}]
        }}
    })).unwrap();
    let app = market_listing_presentation(listing).unwrap();
    assert_eq!(app.name, "Reviewed CLI title");
    assert_eq!(app.description, "Reviewed description");
    assert_eq!(app.capability, "Run platform commands");
    assert_eq!(app.risk, "Runs local commands");
    assert!(app.source.is_empty());
    assert!(app.checksum.is_none());
    let local_app_contract::InstallSource::Market { source, .. } = app.install_source.unwrap()
    else {
        panic!("market source missing")
    };
    assert_eq!(source.application.environment_key.as_str(), "author-a");
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

include!("market_lifecycle_tests/runtime.rs");

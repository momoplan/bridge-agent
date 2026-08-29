use super::*;
use bridge_agent::ConnectorLifecycleResult;

fn update_release_response(
    force_update: Option<bool>,
    minimum_supported_version: Option<&str>,
) -> UpdateReleaseResponse {
    UpdateReleaseResponse {
        tag_name: Some("bridge-agent-v0.1.72".to_string()),
        version: Some("0.1.72".to_string()),
        release_url: None,
        release_name: None,
        published_at: None,
        update_available: None,
        force_update,
        minimum_supported_version: minimum_supported_version.map(str::to_string),
        force_update_message: None,
        assets: Vec::new(),
    }
}

#[test]
fn force_update_required_should_follow_minimum_supported_version() {
    let release = update_release_response(None, Some("0.1.72"));

    assert!(release_force_update_required(
        &release,
        &Version::parse("0.1.71").unwrap()
    ));
    assert!(!release_force_update_required(
        &release,
        &Version::parse("0.1.72").unwrap()
    ));
}

#[test]
fn force_update_flag_should_override_version_comparison() {
    let release = update_release_response(Some(true), Some("0.1.70"));

    assert!(release_force_update_required(
        &release,
        &Version::parse("0.1.72").unwrap()
    ));
}

fn app_update_status(force_update_required: bool) -> AppUpdateStatus {
    AppUpdateStatus {
        current_version: "0.6.5".to_string(),
        latest_version: Some("0.6.6".to_string()),
        update_available: true,
        force_update_required,
        minimum_supported_version: Some("0.6.6".to_string()),
        force_update_message: None,
        release_url: None,
        release_name: None,
        published_at: None,
        current_target: "windows-x86_64".to_string(),
        auto_download_available: true,
        asset_name: Some("Baijimu_0.6.6_x64_zh-CN.msi".to_string()),
    }
}

#[test]
fn startup_update_gate_blocks_business_components_for_a_required_update() {
    assert_eq!(
        startup_update_decision(&app_update_status(true)),
        StartupUpdateDecision::RequireUpdate
    );
}

#[test]
fn startup_update_gate_allows_optional_updates_without_breaking_offline_startup() {
    assert_eq!(
        startup_update_decision(&app_update_status(false)),
        StartupUpdateDecision::Continue
    );
}

#[tokio::test]
async fn startup_update_check_retries_temporary_failures_and_recovers() {
    let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let attempts_for_check = Arc::clone(&attempts);
    let status = run_update_check_with_retry(
        move || {
            let attempt = attempts_for_check.fetch_add(1, Ordering::SeqCst) + 1;
            async move {
                if attempt < 3 {
                    Err(UpdateCheckFailure::temporarily_unavailable(format!(
                        "temporary failure {attempt}"
                    )))
                } else {
                    Ok(app_update_status(false))
                }
            }
        },
        Duration::from_secs(1),
        &[Duration::ZERO, Duration::ZERO],
        None,
    )
    .await
    .expect("third update check should recover");

    assert_eq!(attempts.load(Ordering::SeqCst), 3);
    assert!(!status.force_update_required);
}

#[tokio::test]
async fn startup_update_check_does_not_retry_configuration_failures() {
    let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let attempts_for_check = Arc::clone(&attempts);
    let failure = run_update_check_with_retry(
        move || {
            attempts_for_check.fetch_add(1, Ordering::SeqCst);
            async {
                Err(UpdateCheckFailure::configuration(
                    "missing packaged update endpoint",
                ))
            }
        },
        Duration::from_secs(1),
        &[Duration::ZERO, Duration::ZERO],
        None,
    )
    .await
    .expect_err("configuration failure should be terminal");

    assert_eq!(attempts.load(Ordering::SeqCst), 1);
    assert_eq!(failure.kind, UpdateCheckFailureKind::Configuration);
}

#[test]
fn updater_health_distinguishes_temporary_unavailability_and_recovers() {
    let directory = tempfile::tempdir().unwrap();
    let config_path = directory.path().join("agent-config.json");
    let health = StartupHealthManager::new(
        &config_path,
        StartupDiagnostics::for_config_path(&config_path),
    );
    let failure = UpdateCheckFailure::temporarily_unavailable("proxy route is not ready");

    apply_updater_failure_health(&health, &failure, true);
    let unavailable = health
        .snapshot()
        .components
        .into_iter()
        .find(|component| component.id == "updater")
        .unwrap();
    assert_eq!(unavailable.status, "unavailable");
    assert!(unavailable.detail.unwrap().contains("后台自动重试"));

    apply_updater_health_status(&health, &app_update_status(false));
    let recovered = health
        .snapshot()
        .components
        .into_iter()
        .find(|component| component.id == "updater")
        .unwrap();
    assert_eq!(recovered.status, "ready");
}

#[test]
fn updater_asset_selection_requires_a_signature() {
    if matches!(std::env::consts::OS, "windows" | "linux") && std::env::consts::ARCH != "x86_64" {
        return;
    }
    let suffix = match std::env::consts::OS {
        "macos" => ".app.tar.gz",
        "windows" => ".msi",
        "linux" => ".AppImage",
        _ => return,
    };
    let mut release = update_release_response(None, None);
    release.assets = vec![
        UpdateReleaseAsset {
            name: format!("unsigned{suffix}"),
            signature: None,
        },
        UpdateReleaseAsset {
            name: format!("signed{suffix}"),
            signature: Some("minisign-signature".to_string()),
        },
    ];

    let selected = select_tauri_updater_asset(&release).expect("signed updater asset");
    assert_eq!(selected.name, format!("signed{suffix}"));
}

#[test]
fn shared_cli_auth_path_should_live_under_home_config() {
    let path = shared_cli_auth_path();

    assert!(path.ends_with(Path::new(".config").join("baijimu").join("auth.json")));
    assert!(path.is_absolute() || std::env::var_os("HOME").is_none());
}

#[test]
fn shared_cli_auth_sets_authorized_workspace_as_current_and_preserves_other_credentials() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("auth.json");
    fs::write(
        &path,
        serde_json::to_vec_pretty(&serde_json::json!({
            "currentEnvironment": "prod",
            "currentWorkspaceId": 1201,
            "environments": {
                "prod": {"baseUrl": "https://baijimu.com"}
            },
            "machineCredentials": [{
                "workspaceId": 1201,
                "clientId": "old-device",
                "token": "lc_pat_old",
                "tokenType": "workspace_user_api_key",
                "issuedAtEpochSeconds": 1
            }]
        }))
        .unwrap(),
    )
    .unwrap();
    let config = AgentConfig::example();
    let authorized = AuthorizedPayload {
        workspace_id: 1082,
        device_id: "wenya".to_string(),
        relay_ws_url: "wss://relay.example.test".to_string(),
        agent_token: "agent-token".to_string(),
        issued_at_epoch_seconds: Some(1_786_205_925),
        expires_at_epoch_seconds: Some(i64::MAX as u64),
        local_client_token: Some("lc_pat_workspace_1082".to_string()),
        local_client_token_type: Some("workspace_user_api_key".to_string()),
        local_client_key_id: Some("key-1082".to_string()),
        local_client_user_id: Some(433),
        local_client_scopes: vec![
            "baijimu:agent-cli".to_string(),
            "partner:api".to_string(),
            "workspace:1082".to_string(),
        ],
        local_client_issued_at: Some("2026-07-29 10:00:00".to_string()),
        local_client_expires_at: Some("2026-10-27 10:00:00".to_string()),
    };

    let mut authorized_config = config.clone();
    apply_authorized_device_credentials(&mut authorized_config, &authorized);
    assert_eq!(authorized_config.platform.workspace_id, Some(1082));
    assert_eq!(authorized_config.relay.agent_id, "wenya");
    assert_eq!(authorized_config.relay.token, "agent-token");
    assert_eq!(
        authorized_config.relay.token_expires_at_epoch_seconds,
        Some((i64::MAX as u64).to_string())
    );

    write_shared_cli_auth_at(&path, &config, &authorized).unwrap();

    let document: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(document["currentWorkspaceId"], 1082);
    assert_eq!(document["schemaVersion"], 2);
    assert!(document.get("machineCredentials").is_none());
    assert_eq!(document["credentials"].as_array().unwrap().len(), 2);
    assert_eq!(document["credentials"][0]["workspaceIds"][0], 1201);
    assert_eq!(document["credentials"][0]["tokenType"], "pat");
    assert_eq!(document["credentials"][1]["workspaceIds"][0], 1082);
    assert_eq!(document["credentials"][1]["userId"], 433);
    assert_eq!(document["credentials"][1]["source"], "bridge-agent");
    assert_eq!(
        document["credentials"][1]["expiresAt"],
        "2026-10-27 10:00:00"
    );
    assert!(document["credentials"][1]["issuedAtEpochSeconds"]
        .as_u64()
        .is_some());
    #[cfg(unix)]
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[test]
fn market_git_source_converts_to_github_archive() {
    let archive = connector_archive_download_url(
        "https://github.com/momoplan/wechat-bridge-collector.git",
        Some("v0.2.3"),
        false,
    )
    .unwrap();
    assert_eq!(
        archive.as_deref(),
        Some("https://github.com/momoplan/wechat-bridge-collector/archive/v0.2.3.zip")
    );
}

#[test]
fn custom_git_source_keeps_git_clone_path() {
    let archive = connector_archive_download_url(
        "https://github.com/momoplan/wechat-bridge-collector.git",
        Some("v0.2.3"),
        true,
    )
    .unwrap();
    assert!(archive.is_none());
}

#[test]
fn archive_source_downloads_directly() {
    let archive = connector_archive_download_url(
        "https://download.baijimu.com/connectors/wechat.zip",
        None,
        false,
    )
    .unwrap();
    assert_eq!(
        archive.as_deref(),
        Some("https://download.baijimu.com/connectors/wechat.zip")
    );
}

#[test]
fn connector_archive_checksum_is_required_to_match_exact_bytes() {
    let checksum = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
    assert!(verify_connector_archive_checksum(b"hello", Some(checksum)).is_ok());
    assert!(verify_connector_archive_checksum(b"changed", Some(checksum)).is_err());
    assert!(verify_connector_archive_checksum(b"hello", Some("invalid")).is_err());
}

#[test]
fn connector_upgrade_requires_every_lifecycle_command_to_succeed() {
    let success = ConnectorStartResult {
        app_id: "com.baijimu.connector.test".to_string(),
        lifecycle: ConnectorLifecycleResult {
            app_id: "com.baijimu.connector.test".to_string(),
            configured: true,
            exit_code: Some(0),
            stdout: "started".to_string(),
            stderr: String::new(),
        },
    };
    assert!(ensure_lifecycle_command_succeeded("启动新版应用", &success).is_ok());

    let failure = ConnectorStartResult {
        app_id: "com.baijimu.connector.test".to_string(),
        lifecycle: ConnectorLifecycleResult {
            app_id: "com.baijimu.connector.test".to_string(),
            configured: false,
            exit_code: None,
            stdout: String::new(),
            stderr: "stop command is not configured".to_string(),
        },
    };
    let error = ensure_lifecycle_command_succeeded("停止旧版应用", &failure).unwrap_err();
    assert!(error.contains("命令未配置"));
    assert!(error.contains("com.baijimu.connector.test"));
}

#[test]
fn local_app_ui_bridge_is_injected_before_head_closes() {
    let html =
        b"<!doctype html><html><head><title>Settings</title></head><body></body></html>".to_vec();

    let injected = String::from_utf8(inject_local_app_ui_bridge(html).unwrap()).unwrap();

    let bridge_index = injected.find(LOCAL_APP_UI_BRIDGE_ASSET).unwrap();
    let head_end_index = injected.to_ascii_lowercase().find("</head>").unwrap();
    assert!(bridge_index < head_end_index);
    assert_eq!(injected.matches(LOCAL_APP_UI_BRIDGE_ASSET).count(), 1);
}

#[test]
fn local_app_ui_bridge_reannounces_ready_after_host_hello() {
    assert!(LOCAL_APP_UI_BRIDGE_SCRIPT.contains("baijimu:local-app:hello"));
    assert!(LOCAL_APP_UI_BRIDGE_SCRIPT.contains("announceReady();"));
    assert!(
        LOCAL_APP_UI_BRIDGE_SCRIPT.contains("window.addEventListener(\"pageshow\", announceReady)")
    );
    assert!(LOCAL_APP_UI_BRIDGE_SCRIPT.contains("message.type === HELLO_TYPE"));
}

#[test]
fn macos_bundle_allows_loopback_assets_inside_webview() {
    let info_plist = include_str!("../../Info.plist");
    assert!(info_plist.contains("NSAppTransportSecurity"));
    assert!(info_plist.contains("NSAllowsArbitraryLoadsInWebContent"));
    assert!(info_plist.contains("<true/>"));
}

#[test]
fn local_app_ui_response_disables_direct_network_access() {
    let response = local_app_ui_response(
        StatusCode::OK,
        "text/html; charset=utf-8",
        b"<html></html>".to_vec(),
    );

    let csp = response
        .headers()
        .get(header::CONTENT_SECURITY_POLICY)
        .unwrap()
        .to_str()
        .unwrap();
    assert!(csp.contains("connect-src 'none'"));
    assert!(csp.contains("frame-ancestors tauri://localhost http://tauri.localhost"));
    assert_eq!(
        response
            .headers()
            .get(header::X_CONTENT_TYPE_OPTIONS)
            .unwrap(),
        "nosniff"
    );
}

#[test]
fn local_app_ui_hosts_are_isolated_per_connector() {
    let token = "0123456789abcdef0123456789abcdef";
    let first = local_app_ui_host(token, "com.baijimu.connector.first");
    let second = local_app_ui_host(token, "com.baijimu.connector.second");
    assert_ne!(first, second);
    assert!(first.ends_with(".localhost"));

    let mut headers = HeaderMap::new();
    headers.insert(header::HOST, format!("{first}:32123").parse().unwrap());
    assert!(local_app_ui_request_host_matches(
        &headers,
        token,
        "com.baijimu.connector.first"
    ));
    assert!(!local_app_ui_request_host_matches(
        &headers,
        token,
        "com.baijimu.connector.second"
    ));
}

#[test]
fn local_app_control_discovery_is_private_and_loopback_only() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join(LOCAL_APP_CONTROL_FILE_NAME);
    let token = "0123456789abcdef0123456789abcdef";

    write_local_app_control_discovery(&path, 39100, token).unwrap();

    let document: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(document["schemaVersion"], LOCAL_APP_CONTROL_SCHEMA_VERSION);
    assert_eq!(document["baseUrl"], "http://127.0.0.1:39100/api/v1");
    assert_eq!(document["token"], token);
    #[cfg(unix)]
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[test]
fn repeated_incomplete_startups_enable_safe_mode() {
    let directory = tempfile::tempdir().unwrap();
    let config_path = directory.path().join("agent-config.json");
    let state_path = directory.path().join(STARTUP_STATE_FILE_NAME);
    write_startup_state(
        &state_path,
        &PersistentStartupState {
            pending: true,
            consecutive_failures: SAFE_MODE_FAILURE_THRESHOLD - 1,
            version: Some(env!("CARGO_PKG_VERSION").to_string()),
            started_at_ms: Some(now_ms()),
            ready_at_ms: None,
        },
    )
    .unwrap();

    let health = StartupHealthManager::new(
        &config_path,
        StartupDiagnostics::for_config_path(&config_path),
    );
    health.begin_primary(false, None);

    assert!(health.safe_mode());
    assert_eq!(
        health.snapshot().consecutive_failures,
        SAFE_MODE_FAILURE_THRESHOLD
    );
}

#[test]
fn secondary_instance_construction_does_not_mutate_startup_state() {
    let directory = tempfile::tempdir().unwrap();
    let config_path = directory.path().join("agent-config.json");
    let state_path = directory.path().join(STARTUP_STATE_FILE_NAME);
    let previous = PersistentStartupState {
        pending: true,
        consecutive_failures: 1,
        version: Some("0.2.9".to_string()),
        started_at_ms: Some(123),
        ready_at_ms: None,
    };
    write_startup_state(&state_path, &previous).unwrap();
    let before = fs::read(&state_path).unwrap();

    let health = StartupHealthManager::new(
        &config_path,
        StartupDiagnostics::for_config_path(&config_path),
    );

    assert!(!health.safe_mode());
    assert_eq!(health.snapshot().consecutive_failures, 0);
    assert_eq!(fs::read(&state_path).unwrap(), before);
}

#[test]
fn desktop_is_the_release_autostart_owner_on_every_supported_platform() {
    assert_eq!(
        desktop_autostart_policy(false),
        DesktopAutostartPolicy::EnableForDesktop
    );
    assert_eq!(
        desktop_autostart_policy(true),
        DesktopAutostartPolicy::SkipDevelopmentBuild
    );
}

#[test]
fn desktop_launch_mode_distinguishes_background_autostart_from_user_launches() {
    assert_eq!(
        DesktopLaunchMode::from_args(["bridge-agent-desktop", AUTOSTART_BACKGROUND_ARG], false,),
        DesktopLaunchMode::BackgroundAutostart
    );
    assert_eq!(
        DesktopLaunchMode::from_args(["bridge-agent-desktop"], false),
        DesktopLaunchMode::Interactive
    );
    assert_eq!(
        DesktopLaunchMode::from_args(["bridge-agent-desktop", AUTOSTART_BACKGROUND_ARG], true,),
        DesktopLaunchMode::Interactive
    );
    assert!(!DesktopLaunchMode::BackgroundAutostart.should_show_main_window());
    assert!(DesktopLaunchMode::Interactive.should_show_main_window());
}

#[test]
fn background_secondary_launch_does_not_request_an_interactive_window() {
    assert!(background_autostart_requested([
        "bridge-agent-desktop",
        AUTOSTART_BACKGROUND_ARG,
    ]));
    assert!(!background_autostart_requested([
        "bridge-agent-desktop",
        "--safe-mode",
    ]));
}

#[test]
fn desktop_window_state_never_restores_visibility_or_focus() {
    let flags = desktop_window_state_flags();
    assert!(!flags.contains(StateFlags::VISIBLE));
    assert!(flags.contains(StateFlags::SIZE));
    assert!(flags.contains(StateFlags::POSITION));

    let config: Value = serde_json::from_str(include_str!("../../tauri.conf.json")).unwrap();
    assert_eq!(config["app"]["windows"][0]["visible"], false);
    assert!(config["app"]["windows"][0].get("minWidth").is_none());
    assert!(config["app"]["windows"][0].get("minHeight").is_none());
}

#[test]
fn macos_reopen_only_restores_a_missing_visible_window() {
    assert!(should_restore_main_window_on_macos_reopen(false));
    assert!(!should_restore_main_window_on_macos_reopen(true));
}

#[test]
fn interactive_restart_marker_is_consumed_once() {
    let directory = tempfile::tempdir().unwrap();
    let config_path = directory.path().join("agent-config.json");

    assert!(!consume_interactive_restart_request(&config_path).unwrap());
    request_interactive_restart(&config_path).unwrap();
    assert!(consume_interactive_restart_request(&config_path).unwrap());
    assert!(!consume_interactive_restart_request(&config_path).unwrap());
}

#[test]
fn startup_detects_legacy_connector_identity_before_strict_config_loading() {
    let directory = tempfile::tempdir().unwrap();
    let config_path = directory.path().join("agent-config.json");
    fs::write(
        &config_path,
        serde_json::to_vec_pretty(&serde_json::json!({
            "local_apps": [{
                "connectorId": "com.baijimu.connector.codex",
                "name": "Codex",
                "version": "1.5.5"
            }]
        }))
        .unwrap(),
    )
    .unwrap();

    assert!(legacy_config_requires_unified_app_id_migration(&config_path).unwrap());
}

#[test]
fn startup_skips_migration_for_current_app_identity_config() {
    let directory = tempfile::tempdir().unwrap();
    let config_path = directory.path().join("agent-config.json");
    fs::write(
        &config_path,
        serde_json::to_vec_pretty(&serde_json::json!({
            "local_apps": [{
                "appId": "codex",
                "name": "Codex",
                "version": "1.5.5"
            }]
        }))
        .unwrap(),
    )
    .unwrap();

    assert!(!legacy_config_requires_unified_app_id_migration(&config_path).unwrap());
}

#[test]
fn startup_resumes_an_interrupted_unified_app_id_migration() {
    let directory = tempfile::tempdir().unwrap();
    let config_path = directory.path().join("agent-config.json");
    fs::write(&config_path, br#"{"local_apps":[]}"#).unwrap();
    fs::write(
        directory.path().join(UNIFIED_APP_ID_MIGRATION_LEDGER),
        b"{}",
    )
    .unwrap();

    assert!(legacy_config_requires_unified_app_id_migration(&config_path).unwrap());
}

#[test]
fn auto_start_rebuilds_installed_connectors_before_loading_runtime_config() {
    let directory = tempfile::tempdir().unwrap();
    let config_path = directory.path().join("agent-config.json");
    let config = AgentConfig::example();
    save_agent_config(&config_path, &config).unwrap();
    assert!(load_agent_config(&config_path)
        .unwrap()
        .local_apps
        .is_empty());

    let (prepared, report) = prepare_config_for_auto_start_with(&config_path, |path| {
        let mut synchronized = load_agent_config(path)?;
        synchronized.local_apps.push(LocalAppConfig {
            app_id: "com.baijimu.connector.persisted".to_string(),
            name: "Persisted Connector".to_string(),
            version: "1.0.0".to_string(),
            description: "Installed before the host upgrade".to_string(),
            enabled: true,
            health_check: None,
            start_command: None,
            stop_command: None,
            methods: Vec::new(),
            events: vec![bridge_agent::EventConfig {
                name: "changed".to_string(),
                description: "Connector state changed".to_string(),
                enabled: true,
                payload_schema: serde_json::json!({
                    "type": "object",
                    "additionalProperties": true
                }),
            }],
        });
        save_agent_config(path, &synchronized)?;
        Ok(ConnectorSyncReport {
            summaries: Vec::new(),
            failures: Vec::new(),
        })
    })
    .unwrap();

    assert!(report.failures.is_empty());
    assert_eq!(prepared.local_apps.len(), 1);
    assert_eq!(
        prepared.local_apps[0].app_id,
        "com.baijimu.connector.persisted"
    );
}

#[test]
fn quit_running_instance_flag_is_explicit() {
    assert!(quit_running_instance_requested(&[
        "bridge-agent-desktop.exe".to_string(),
        QUIT_RUNNING_INSTANCE_ARG.to_string(),
    ]));
    assert!(!quit_running_instance_requested(&[
        "bridge-agent-desktop.exe".to_string(),
        "--safe-mode".to_string(),
    ]));
}

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
fn shared_cli_auth_preserves_cli_selection_and_other_credentials() {
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
    let mut authorized = AuthorizedPayload {
        environment_key: "baijimu".into(),
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
    assert_eq!(document["currentWorkspaceId"], 1201);
    assert_eq!(document["currentEnvironment"], "prod");
    assert_eq!(
        document["environments"]["baijimu"]["environmentKey"],
        "baijimu"
    );
    assert_eq!(document["credentials"][1]["environmentKey"], "baijimu");
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

    // A new environment may issue the same credential and device IDs.
    let mut private_config = config.clone();
    private_config.platform.base_url = "https://private.example.test/lowcode3".into();
    authorized.environment_key = "private-a".into();
    write_shared_cli_auth_at(&path, &private_config, &authorized).unwrap();
    write_shared_cli_auth_at(&path, &private_config, &authorized).unwrap();
    let before = fs::read(&path).unwrap();
    let document: Value = serde_json::from_slice(&before).unwrap();
    assert_eq!(document["credentials"].as_array().unwrap().len(), 3);
    assert_eq!(document["currentEnvironment"], "prod");
    assert_eq!(document["currentWorkspaceId"], 1201);
    private_config.platform.base_url = "https://different.example.test/lowcode3".into();
    assert!(write_shared_cli_auth_at(&path, &private_config, &authorized).is_err());
    assert_eq!(fs::read(&path).unwrap(), before);
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

include!("startup_update_tests/connectors.rs");

#[tokio::test]
async fn incomplete_identity_keeps_control_available_and_rejects_event_handoff() {
    for missing in ["environment", "workspace", "credential"] {
        let dir = tempdir().unwrap();
        let path = dir.path().join("agent-config.json");
        let relay = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let local = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = local.local_addr().unwrap();
        drop(local);
        let mut config = AgentConfig::example();
        config.platform.environment_key = Some("authorized-test".into());
        config.platform.workspace_id = Some(42);
        config.relay.token = "authorized-test-token".into();
        config.relay.url = format!("ws://{}/ws/agent", relay.local_addr().unwrap());
        config.runtime.event_server_bind = address.to_string();
        config.runtime.log_file_dir = Some(dir.path().join("logs").display().to_string());
        config.services.clear();
        match missing {
            "environment" => config.platform.environment_key = None,
            "workspace" => config.platform.workspace_id = None,
            "credential" => config.relay.token.clear(),
            _ => unreachable!(),
        }
        crate::config::save_config(&path, &config).unwrap();
        let manager = AgentRuntimeManager::new();
        manager.start(config, &path).await.unwrap();
        wait_for_runtime_status(&manager, RuntimeStatus::AuthorizationRequired, 3).await.unwrap();
        let client = reqwest::Client::new();
        wait_for_http_success(&client, &format!("http://{address}/healthz"), 3).await.unwrap();
        let response = client.post(format!("http://{address}/v1/local-app-events"))
            .json(&json!({"appId":"test-app","eventId":"stable-id","event":"test.event","payload":{}}))
            .send().await.unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE, "{missing}");
        assert!(response.text().await.unwrap().contains("device authorization required"));
        assert!(!crate::event_delivery::database_path(&path).unwrap().exists());
        assert!(tokio::time::timeout(std::time::Duration::from_millis(50), relay.accept()).await.is_err());
        manager.stop().await.unwrap();
    }
}

#[tokio::test]
async fn authorization_restart_opens_scoped_queue_and_connects_relay() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("agent-config.json");
    let relay = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut config = AgentConfig::example();
    config.relay.url = format!("ws://{}/ws/agent", relay.local_addr().unwrap());
    config.runtime.event_server_bind = "127.0.0.1:0".into();
    config.runtime.log_file_dir = Some(dir.path().join("logs").display().to_string());
    config.services.clear();
    crate::config::save_config(&path, &config).unwrap();
    let manager = AgentRuntimeManager::new();
    manager.start_from_path(&path).await.unwrap();
    wait_for_runtime_status(&manager, RuntimeStatus::AuthorizationRequired, 3).await.unwrap();
    assert!(!crate::event_delivery::database_path(&path).unwrap().exists());
    manager.stop().await.unwrap();

    config.platform.environment_key = Some("authorized-test".into());
    config.platform.workspace_id = Some(42);
    config.relay.token = "authorized-test-token".into();
    crate::config::save_config(&path, &config).unwrap();
    manager.start_from_path(&path).await.unwrap();
    let (stream, _) = tokio::time::timeout(std::time::Duration::from_secs(3), relay.accept()).await.unwrap().unwrap();
    let mut socket = accept_async(stream).await.unwrap();
    let message = tokio::time::timeout(std::time::Duration::from_secs(3), socket.next()).await.unwrap().unwrap().unwrap();
    assert!(matches!(message, Message::Text(_)));
    assert!(crate::event_delivery::database_path(&path).unwrap().exists());
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn authorized_queue_storage_failure_is_not_treated_as_missing_authorization() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("agent-config.json");
    let mut config = AgentConfig::example();
    config.platform.environment_key = Some("authorized-test".into());
    config.platform.workspace_id = Some(42);
    config.relay.token = "authorized-test-token".into();
    config.relay.url = "ws://127.0.0.1:9/ws/agent".into();
    config.runtime.log_file_dir = Some(dir.path().join("logs").display().to_string());
    config.services.clear();
    fs::write(dir.path().join("event-delivery"), "not a directory").unwrap();
    let manager = AgentRuntimeManager::new();
    assert!(manager.start(config, &path).await.is_err());
    assert_eq!(manager.snapshot().await.status, RuntimeStatus::Stopped);
}

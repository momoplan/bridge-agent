    #[tokio::test]
    async fn runtime_manager_serializes_concurrent_starts() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("agent-config.json");
        let mut config = AgentConfig::example();
        config.platform.environment_key = Some("test".into());
        config.relay.token = "test-authorized-device".into();
        config.platform.workspace_id = Some(1);
        config.relay.url = "ws://127.0.0.1:9/ws/agent".to_string();
        config.relay.agent_id = "dev_concurrent_start".to_string();
        config.runtime.log_file_dir = Some(dir.path().join("logs").display().to_string());
        config.runtime.event_server_enabled = false;
        config.runtime.service_registration_enabled = false;
        config.services.clear();

        let manager = AgentRuntimeManager::new();
        let first_manager = manager.clone();
        let second_manager = manager.clone();
        let first_config = config.clone();
        let second_config = config;
        let first_path = config_path.clone();
        let second_path = config_path.clone();

        let (first, second) = tokio::join!(
            async move { first_manager.start(first_config, &first_path).await },
            async move { second_manager.start(second_config, &second_path).await }
        );

        assert!(first.is_ok(), "first start failed: {first:?}");
        assert!(second.is_ok(), "second start failed: {second:?}");
        manager.stop().await.unwrap();
    }

    #[tokio::test]
    async fn first_relay_registration_and_refresh_include_unhealthy_installed_apps() {
        let mut relay = FakeRelay::start();
        let health_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let health_addr = health_listener.local_addr().unwrap();
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("agent-config.json");
        let token_path = dir.path().join("app-token");
        fs::write(&token_path, "bjm_app_startup_capabilities_test_private_token").unwrap();
        let mut config = AgentConfig::example();
        config.platform.environment_key = Some("test".into());
        config.platform.workspace_id = Some(1);
        config.relay.token = "test-authorized-device".into();
        config.relay.url = relay.url.clone();
        config.relay.agent_id = "dev_installed_capabilities".into();
        config.runtime.log_file_dir = Some(dir.path().join("logs").display().to_string());
        config.runtime.event_server_enabled = false;
        config.runtime.service_registration_enabled = false;
        config.local_apps = vec![serde_json::from_value(json!({
            "appId":"installed-app","name":"Installed app","version":"1.0.0",
            "description":"Not ready yet","enabled":true,"methods":[],
            "healthCheck":{"type":"http","url":format!("http://{health_addr}/health"),
                "http_method":"GET","headers":{},"timeoutSecs":60},
            "startCommand":{"type":"shell_command","command":["must-not-start"],
                "env":{"BAIJIMU_LOCAL_APP_TOKEN_FILE":token_path.display().to_string()}},
            "events":[{"name":"messageReceived","description":"Messages","enabled":true,"payload_schema":{}}]
        })).unwrap()];
        crate::config::save_config(&config_path, &config).unwrap();
        let manager = AgentRuntimeManager::new();
        manager.start(config.clone(), &config_path).await.unwrap();
        let mut session = relay.accept().await;
        let first = session.receive().await;
        assert_eq!(first["local_apps"][0]["appId"], "installed-app");
        assert_eq!(first["local_apps"][0]["events"][0]["name"], "messageReceived");
        manager.apply_capabilities_from_path(&config_path).await.unwrap();
        let refreshed = session.receive().await;
        assert_eq!(refreshed["local_apps"], first["local_apps"]);
        config.local_apps.clear();
        crate::config::save_config(&config_path, &config).unwrap();
        manager.apply_capabilities_from_path(&config_path).await.unwrap();
        assert_eq!(session.receive().await["local_apps"], json!([]));
        assert!(tokio::time::timeout(std::time::Duration::from_millis(50), health_listener.accept()).await.is_err());
        manager.stop().await.unwrap();
    }

    #[tokio::test]
    async fn runtime_manager_reuses_active_start_for_duplicate_start() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("agent-config.json");
        let mut config = AgentConfig::example();
        config.platform.environment_key = Some("test".into());
        config.relay.token = "test-authorized-device".into();
        config.platform.workspace_id = Some(1);
        config.relay.url = "ws://127.0.0.1:9/ws/agent".to_string();
        config.relay.agent_id = "dev_duplicate_start".to_string();
        config.runtime.log_file_dir = Some(dir.path().join("logs").display().to_string());
        config.runtime.event_server_enabled = false;
        config.runtime.service_registration_enabled = false;
        config.services.clear();

        let manager = AgentRuntimeManager::new();
        manager.start(config.clone(), &config_path).await.unwrap();
        let lock_path = runtime_lock_path(&config_path);
        let before = read_runtime_lock(&lock_path).unwrap();

        tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;

        manager.start(config, &config_path).await.unwrap();
        let after = read_runtime_lock(&lock_path).unwrap();

        assert_eq!(before.pid, after.pid);
        assert_eq!(before.started_at_ms, after.started_at_ms);
        manager.stop().await.unwrap();
    }

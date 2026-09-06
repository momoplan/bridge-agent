    #[tokio::test]
    async fn runtime_manager_serializes_concurrent_starts() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("agent-config.json");
        let mut config = AgentConfig::example();
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
    async fn runtime_connects_without_waiting_for_local_service_readiness() {
        let relay_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let relay_addr = relay_listener.local_addr().unwrap();
        let health_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let health_addr = health_listener.local_addr().unwrap();
        let health_server = tokio::spawn(async move {
            let Ok((socket, _)) = health_listener.accept().await else {
                return;
            };
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            drop(socket);
        });

        let dir = tempdir().unwrap();
        let config_path = dir.path().join("agent-config.json");
        let mut config = AgentConfig::example();
        config.relay.url = format!("ws://{relay_addr}/ws/agent");
        config.relay.agent_id = "dev_starting_during_service_preparation".to_string();
        config.runtime.log_file_dir = Some(dir.path().join("logs").display().to_string());
        config.runtime.event_server_enabled = false;
        config.runtime.service_registration_enabled = false;
        config.services = vec![ServiceConfig {
            name: "slowHealthCheck".to_string(),
            description: "Keeps runtime preparation pending for the test.".to_string(),
            enabled: true,
            health_check: Some(ServiceHealthCheck::Http {
                url: format!("http://{health_addr}/health"),
                http_method: "GET".to_string(),
                headers: BTreeMap::new(),
                timeout_secs: Some(1),
                expect_status: Some(200),
                body_contains: None,
            }),
            start_command: None,
            stop_command: None,
            methods: Vec::new(),
        }];

        let manager = AgentRuntimeManager::new();
        let started_at = std::time::Instant::now();
        let starting = tokio::time::timeout(
            std::time::Duration::from_millis(500),
            manager.start(config, &config_path),
        )
        .await
        .expect("runtime start waited for local service readiness")
        .expect("runtime start failed");
        assert!(started_at.elapsed() < std::time::Duration::from_millis(500));
        assert_eq!(
            starting.agent_id.as_deref(),
            Some("dev_starting_during_service_preparation")
        );
        assert_ne!(manager.snapshot().await.status, RuntimeStatus::Stopped);

        let (relay_socket, _) = tokio::time::timeout(
            std::time::Duration::from_millis(500),
            relay_listener.accept(),
        )
        .await
        .expect("relay connection waited for local service readiness")
        .expect("relay listener failed");
        let _relay_stream = tokio::time::timeout(
            std::time::Duration::from_millis(500),
            accept_async(relay_socket),
        )
        .await
        .expect("relay WebSocket handshake waited for local service readiness")
        .expect("relay WebSocket handshake failed");
        manager.stop().await.unwrap();
        health_server.abort();
    }

    #[tokio::test]
    async fn runtime_manager_reuses_active_start_for_duplicate_start() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("agent-config.json");
        let mut config = AgentConfig::example();
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

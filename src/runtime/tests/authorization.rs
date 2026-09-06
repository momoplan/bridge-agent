    #[test]
    fn relay_http_unauthorized_and_forbidden_require_reauthorization() {
        let unauthorized = WebSocketError::Http(Box::new(
            tokio_tungstenite::tungstenite::http::Response::builder()
                .status(StatusCode::UNAUTHORIZED)
                .body(None)
                .unwrap(),
        ));
        let forbidden = WebSocketError::Http(Box::new(
            tokio_tungstenite::tungstenite::http::Response::builder()
                .status(StatusCode::FORBIDDEN)
                .body(None)
                .unwrap(),
        ));
        let unavailable = WebSocketError::Http(Box::new(
            tokio_tungstenite::tungstenite::http::Response::builder()
                .status(StatusCode::SERVICE_UNAVAILABLE)
                .body(None)
                .unwrap(),
        ));

        assert!(is_relay_authorization_error(&unauthorized));
        assert!(is_relay_authorization_error(&forbidden));
        assert!(!is_relay_authorization_error(&unavailable));
    }

    #[tokio::test]
    async fn relay_unauthorized_pauses_reconnect_until_reauthorization() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let relay_addr = listener.local_addr().unwrap();
        let connection_count = Arc::new(AtomicUsize::new(0));
        let server_count = Arc::clone(&connection_count);
        let server = tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                server_count.fetch_add(1, Ordering::SeqCst);
                let mut request = [0_u8; 2048];
                let _ = socket.read(&mut request).await;
                let _ = socket
                    .write_all(
                        b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    )
                    .await;
            }
        });

        let dir = tempdir().unwrap();
        let config_path = dir.path().join("agent-config.json");
        let mut config = AgentConfig::example();
        config.relay.url = format!("ws://{relay_addr}/ws/agent");
        config.relay.agent_id = "dev_reauthorization_required".to_string();
        config.relay.token = "rejected-token".to_string();
        config.relay.reconnect_secs = 1;
        config.runtime.log_file_dir = Some(dir.path().join("logs").display().to_string());
        config.runtime.event_server_enabled = false;
        config.runtime.service_registration_enabled = false;
        config.services.clear();

        let manager = AgentRuntimeManager::new();
        manager.start(config, &config_path).await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            loop {
                if manager.snapshot().await.status == RuntimeStatus::AuthorizationRequired {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            }
        })
        .await
        .expect("runtime did not enter authorization-required state");

        tokio::time::sleep(std::time::Duration::from_millis(1_200)).await;
        assert_eq!(connection_count.load(Ordering::SeqCst), 1);
        assert_eq!(
            manager.snapshot().await.last_error.as_deref(),
            Some(RELAY_AUTHORIZATION_REQUIRED_MESSAGE)
        );

        manager.stop().await.unwrap();
        server.abort();
    }

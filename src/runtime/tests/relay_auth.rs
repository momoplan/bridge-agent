    #[tokio::test]
    async fn relay_handshake_sends_bearer_token() {
        assert_relay_handshake(true).await;
        assert_relay_handshake(false).await;
    }

    async fn assert_relay_handshake(include_legacy_query_token: bool) {
        let token = "handshake-secret-token";
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let relay_addr = listener.local_addr().unwrap();
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("agent-config.json");
        let mut config = AgentConfig::example();
        config.platform.environment_key = Some("test".into());
        config.platform.workspace_id = Some(1);
        config.relay.token = token.into();
        config.relay.url = format!("ws://{relay_addr}/ws/agent");
        config.relay.agent_id = format!("dev_handshake_{include_legacy_query_token}");
        config.relay.include_legacy_query_token = include_legacy_query_token;
        config.runtime.log_file_dir = Some(dir.path().join("logs").display().to_string());
        config.runtime.event_server_enabled = false;
        config.runtime.service_registration_enabled = false;
        config.services.clear();

        let manager = AgentRuntimeManager::new();
        manager.start(config, &config_path).await.unwrap();
        let (mut stream, _) = tokio::time::timeout(std::time::Duration::from_secs(3), listener.accept())
            .await
            .expect("relay handshake was not attempted")
            .unwrap();
        let mut collected = Vec::new();
        let mut buffer = [0_u8; 1024];
        loop {
            let read = tokio::time::timeout(
                std::time::Duration::from_secs(2),
                stream.read(&mut buffer),
            )
            .await
            .expect("timed out reading the websocket handshake")
            .unwrap();
            if read == 0 {
                break;
            }
            collected.extend_from_slice(&buffer[..read]);
            if collected.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
        }
        drop(stream);
        manager.stop().await.unwrap();

        let text = String::from_utf8_lossy(&collected);
        let request_line = text.lines().next().unwrap_or_default();
        assert!(
            text.to_ascii_lowercase()
                .contains(&format!("authorization: bearer {token}")),
            "{text}"
        );
        if include_legacy_query_token {
            assert!(
                request_line.contains(&format!("token={token}")),
                "{request_line}"
            );
        } else {
            assert!(!request_line.contains("token="), "{request_line}");
        }
    }

    #[test]
    fn malformed_relay_preview_redacts_arguments_and_truncates() {
        let secret = "super-secret-argument-token";
        let raw = json!({
            "type": "invoke_request",
            "request_id": "req-bad",
            "arguments": {"command": [secret], "stdin": "super-secret-stdin"},
            "note": "n".repeat(800)
        })
        .to_string();
        let preview = redact_relay_message_preview(&raw);
        assert!(preview.contains("<redacted>"));
        assert!(!preview.contains(secret));
        assert!(!preview.contains("super-secret-stdin"));
        assert!(preview.len() <= 420);

        let unstructured = format!("not-json {}", "x".repeat(1_000));
        let preview = redact_relay_message_preview(&unstructured);
        assert!(preview.len() < unstructured.len());
        assert!(preview.starts_with("not-json"));
    }

    #[test]
    fn invalid_request_reply_keeps_request_id_and_drops_payload() {
        let raw = r#"{"type":"invoke_request","request_id":"req-9","arguments":{"token":"secret-value"}}"#;
        assert!(decode_relay_message(raw).is_err());
        let reply = invalid_request_reply(raw).unwrap();
        let value = serde_json::to_value(&reply).unwrap();
        assert_eq!(value["type"], "invoke_result");
        assert_eq!(value["request_id"], "req-9");
        assert_eq!(value["success"], false);
        assert_eq!(value["error"]["code"], "INVALID_REQUEST");
        assert!(!value.to_string().contains("secret-value"));

        let local = r#"{"type":"local_app_invoke_request","requestId":"req-app","arguments":{"token":"secret-value"}}"#;
        let reply = invalid_request_reply(local).unwrap();
        let value = serde_json::to_value(&reply).unwrap();
        assert_eq!(value["type"], "local_app_invoke_result");
        assert_eq!(value["request_id"], "req-app");
        assert!(invalid_request_reply("not json").is_none());
        assert!(invalid_request_reply(r#"{"type":"registered_ack"}"#).is_none());
    }

    #[tokio::test]
    async fn malformed_relay_frame_is_skipped_without_dropping_the_connection() {
        let secret = "super-secret-argument-token";
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let relay_addr = listener.local_addr().unwrap();
        let (reply_tx, reply_rx) = oneshot::channel();
        let relay = tokio::spawn(async move {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let Ok(mut socket) = accept_async(stream).await else {
                return;
            };
            if socket.next().await.is_none() {
                let _ = reply_tx.send(Err("missing capabilities".to_string()));
                return;
            }
            let bad = json!({
                "type": "invoke_request",
                "request_id": "req-bad",
                "arguments": {"command": [secret], "stdin": "super-secret-stdin"}
            });
            if socket
                .send(Message::Text(bad.to_string().into()))
                .await
                .is_err()
            {
                let _ = reply_tx.send(Err("failed to send malformed frame".to_string()));
                return;
            }
            let reply = match tokio::time::timeout(std::time::Duration::from_secs(3), socket.next())
                .await
            {
                Ok(Some(Ok(Message::Text(text)))) => text.to_string(),
                other => {
                    let _ = reply_tx.send(Err(format!("connection did not stay up: {other:?}")));
                    return;
                }
            };
            let _ = reply_tx.send(Ok(reply));
            let ack = json!({
                "type": "registered_ack",
                "agent_id": "dev_malformed_frame",
                "workspace_id": 1,
                "connection_id": "connection-1",
                "registered_at_epoch_seconds": 1,
                "heartbeat_timeout_secs": 75
            });
            let _ = socket.send(Message::Text(ack.to_string().into())).await;
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        });

        let dir = tempdir().unwrap();
        let config_path = dir.path().join("agent-config.json");
        let mut config = AgentConfig::example();
        config.platform.environment_key = Some("test".into());
        config.platform.workspace_id = Some(1);
        config.relay.token = "test-authorized-device".into();
        config.relay.url = format!("ws://{relay_addr}/ws/agent");
        config.relay.agent_id = "dev_malformed_frame".into();
        config.runtime.log_file_dir = Some(dir.path().join("logs").display().to_string());
        config.runtime.event_server_enabled = false;
        config.runtime.service_registration_enabled = false;
        config.services.clear();

        let manager = AgentRuntimeManager::new();
        manager.start(config, &config_path).await.unwrap();
        let reply = tokio::time::timeout(std::time::Duration::from_secs(5), reply_rx)
            .await
            .expect("relay task did not report the client reply")
            .expect("relay task dropped the reply")
            .expect("malformed frame dropped the websocket");
        let reply: Value = serde_json::from_str(&reply).unwrap();
        assert_eq!(reply["type"], "invoke_result");
        assert_eq!(reply["request_id"], "req-bad");
        assert_eq!(reply["error"]["code"], "INVALID_REQUEST");
        assert!(!reply.to_string().contains(secret));
        assert!(!reply.to_string().contains("super-secret-stdin"));

        wait_for_runtime_status(&manager, RuntimeStatus::Online, 3)
            .await
            .expect("connection did not stay up long enough to register");
        let logs = manager.logs(200).await;
        assert!(logs
            .iter()
            .any(|entry| entry.message.contains("ignored malformed relay message")));
        assert!(logs.iter().all(|entry| {
            !entry.message.contains(secret) && !entry.message.contains("super-secret-stdin")
        }));
        assert!(manager.snapshot().await.last_error.is_none());
        manager.stop().await.unwrap();
        relay.abort();
    }

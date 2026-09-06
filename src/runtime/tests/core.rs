    #[test]
    fn build_url_injects_token() {
        let url = build_agent_url("wss://relay.baijimu.com/ws/agent", "devbox", "secret").unwrap();
        assert_eq!(
            url.as_str(),
            "wss://relay.baijimu.com/ws/agent/devbox?token=secret"
        );
    }

    #[tokio::test]
    async fn runtime_subscriber_receives_incremental_log_events() {
        let manager = AgentRuntimeManager::new();
        let mut events = manager.subscribe();

        manager
            .push_desktop_log("info", "event delivery test", LogMetadata::category("test"))
            .await;

        let event = tokio::time::timeout(std::time::Duration::from_secs(1), events.recv())
            .await
            .expect("runtime event timed out")
            .expect("runtime event channel closed");
        match event {
            RuntimeEvent::LogAppended(entry) => {
                assert_eq!(entry.level, "info");
                assert_eq!(entry.message, "event delivery test");
                assert_eq!(entry.metadata.category.as_deref(), Some("test"));
            }
            RuntimeEvent::SnapshotChanged(_) => panic!("expected a log event"),
        }
    }

    #[tokio::test]
    async fn clearing_logs_returns_a_sequence_barrier() {
        let manager = AgentRuntimeManager::new();
        manager
            .push_desktop_log("info", "before clear", LogMetadata::category("test"))
            .await;

        let cleared_through = manager.clear_logs().await;
        assert!(manager.logs(200).await.is_empty());

        manager
            .push_desktop_log("info", "after clear", LogMetadata::category("test"))
            .await;
        let logs = manager.logs(200).await;
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].message, "after clear");
        assert!(logs[0].sequence > cleared_through);
    }

    #[tokio::test]
    async fn relay_stays_online_while_event_server_rebinds() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("agent-config.json");
        let occupied_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let event_addr = occupied_listener.local_addr().unwrap();
        let relay_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let relay_addr = relay_listener.local_addr().unwrap();

        let relay_task = tokio::spawn(async move {
            let (stream, _) = relay_listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            let capabilities =
                tokio::time::timeout(std::time::Duration::from_secs(3), socket.next())
                    .await
                    .expect("relay did not receive capabilities")
                    .expect("relay websocket closed before capabilities")
                    .unwrap();
            let Message::Text(capabilities) = capabilities else {
                panic!("expected text capabilities message");
            };
            let capabilities: Value = serde_json::from_str(capabilities.as_str()).unwrap();
            assert_eq!(capabilities["type"], "capabilities");

            socket
                .send(Message::Text(
                    json!({
                        "type": "registered_ack",
                        "agent_id": "dev_event_server_recovery",
                        "workspace_id": 1,
                        "connection_id": "connection-1",
                        "registered_at_epoch_seconds": 1,
                        "heartbeat_timeout_secs": 75
                    })
                    .to_string()
                    .into(),
                ))
                .await
                .unwrap();

            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        });

        let mut config = AgentConfig::example();
        config.relay.url = format!("ws://{relay_addr}/ws/agent");
        config.relay.agent_id = "dev_event_server_recovery".to_string();
        config.relay.reconnect_secs = 1;
        config.runtime.log_file_dir = Some(dir.path().join("logs").display().to_string());
        config.runtime.event_server_bind = event_addr.to_string();
        config.runtime.event_server_enabled = true;
        config.runtime.service_registration_enabled = false;

        let manager = AgentRuntimeManager::new();
        manager.start(config, &config_path).await.unwrap();

        wait_for_runtime_status(&manager, RuntimeStatus::Online, 3)
            .await
            .expect("relay did not become online while event port was occupied");
        wait_for_event_server_outcome(&manager, "bind_failed", 3)
            .await
            .expect("event server bind failure was not reported");

        drop(occupied_listener);
        let client = reqwest::Client::new();
        wait_for_http_success(&client, &format!("http://{event_addr}/healthz"), 10)
        .await
        .expect("local event server did not recover after the port was released");

        let logs = manager.logs(200).await;
        assert!(logs.iter().any(|entry| {
            entry.metadata.category.as_deref() == Some("event_server")
                && entry.metadata.outcome.as_deref() == Some("recovered")
        }));
        manager.stop().await.unwrap();
        relay_task.abort();
    }

    async fn wait_for_runtime_status(
        manager: &AgentRuntimeManager,
        expected: RuntimeStatus,
        timeout_secs: u64,
    ) -> Result<(), tokio::time::error::Elapsed> {
        tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), async {
            while manager.snapshot().await.status != expected {
                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            }
        })
        .await
    }

    async fn wait_for_event_server_outcome(
        manager: &AgentRuntimeManager,
        outcome: &str,
        timeout_secs: u64,
    ) -> Result<(), tokio::time::error::Elapsed> {
        tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), async {
            loop {
                if manager.logs(200).await.iter().any(|entry| {
                    entry.metadata.category.as_deref() == Some("event_server")
                        && entry.metadata.outcome.as_deref() == Some(outcome)
                }) {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            }
        })
        .await
    }

    async fn wait_for_http_success(
        client: &reqwest::Client,
        url: &str,
        timeout_secs: u64,
    ) -> Result<(), tokio::time::error::Elapsed> {
        tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), async {
            loop {
                if client
                    .get(url)
                    .send()
                    .await
                    .is_ok_and(|response| response.status().is_success())
                {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        })
        .await
    }

    #[test]
    fn relay_decoder_ignores_unknown_message_type() {
        let message = r#"{"type":"server_notice","message":"hello"}"#;

        assert!(decode_relay_message(message).unwrap().is_none());
    }

    #[test]
    fn relay_decoder_accepts_registered_ack() {
        let message = r#"{"type":"registered_ack","agent_id":"dev_1","workspace_id":1327,"connection_id":"conn_1","registered_at_epoch_seconds":1783680377,"heartbeat_timeout_secs":75}"#;

        match decode_relay_message(message).unwrap().unwrap() {
            AgentMessage::RegisteredAck(ack) => {
                assert_eq!(ack.agent_id, "dev_1");
                assert_eq!(ack.workspace_id, 1327);
                assert_eq!(ack.connection_id, "conn_1");
            }
            other => panic!("expected registered_ack, got {other:?}"),
        }
    }

    #[test]
    fn relay_decoder_accepts_device_event_ack() {
        let message = r#"{"type":"event_ack","eventId":"evt-1","appId":"camera","duplicate":true,"matchedSubscriptionCount":2}"#;

        match decode_relay_message(message).unwrap().unwrap() {
            AgentMessage::EventAck(ack) => {
                assert_eq!(ack.event_id, "evt-1");
                assert_eq!(ack.app_id, "camera");
                assert!(ack.duplicate);
                assert_eq!(ack.matched_subscription_count, 2);
            }
            other => panic!("expected event_ack, got {other:?}"),
        }
    }

    #[test]
    fn pending_event_waiters_are_scoped_by_connector_and_expired_waiters_can_retry() {
        let mut pending = PendingEventWaiters::new();
        let (first_tx, first_rx) = oneshot::channel();
        assert!(register_event_waiter(
            &mut pending,
            ("camera".to_owned(), "evt-1".to_owned()),
            first_tx,
        ));

        let (coalesced_tx, coalesced_rx) = oneshot::channel();
        assert!(!register_event_waiter(
            &mut pending,
            ("camera".to_owned(), "evt-1".to_owned()),
            coalesced_tx,
        ));

        drop(first_rx);
        drop(coalesced_rx);
        let key = ("camera".to_owned(), "evt-1".to_owned());
        let (retry_tx, _retry_rx) = oneshot::channel();
        assert!(register_event_waiter(&mut pending, key, retry_tx));

        let (other_connector_tx, _other_connector_rx) = oneshot::channel();
        assert!(register_event_waiter(
            &mut pending,
            ("microphone".to_owned(), "evt-1".to_owned()),
            other_connector_tx,
        ));

        prune_closed_event_waiters(&mut pending);
        assert_eq!(pending.len(), 2);
    }

    #[test]
    fn runtime_removes_retired_event_storage_without_touching_other_state() {
        let base = tempfile::tempdir().unwrap();
        let retired = base.path().join(RETIRED_EVENT_OUTBOX_DIR);
        fs::create_dir_all(&retired).unwrap();
        fs::write(retired.join("event.json"), b"sensitive payload").unwrap();
        let retained = base.path().join("config.toml");
        fs::write(&retained, b"config").unwrap();

        remove_retired_event_storage(base.path()).unwrap();

        assert!(!retired.exists());
        assert_eq!(fs::read(retained).unwrap(), b"config");
    }

    #[test]
    fn relay_decoder_accepts_local_app_invoke_request() {
        let message = r#"{"type":"local_app_invoke_request","requestId":"req-1","workspaceId":642,"appId":"camera","method":"capture","arguments":{}}"#;

        match decode_relay_message(message).unwrap().unwrap() {
            AgentMessage::LocalAppInvokeRequest(request) => {
                assert_eq!(request.app_id, "camera");
                assert_eq!(request.method, "capture");
                assert_eq!(request.workspace_id, Some(642));
            }
            other => panic!("expected local_app_invoke_request, got {other:?}"),
        }
    }

    #[test]
    fn relay_decoder_rejects_invalid_known_message() {
        let message = r#"{"type":"invoke_request","request_id":"req_1"}"#;

        assert!(decode_relay_message(message).is_err());
    }

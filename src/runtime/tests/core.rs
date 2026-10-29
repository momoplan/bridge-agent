    #[test]
    fn relay_endpoint_is_the_origin_of_the_configured_relay_url() {
        for (configured, endpoint) in [
            ("wss://relay.baijimu.com/ws/agent", "https://relay.baijimu.com/"),
            ("ws://127.0.0.1:8081/ws/agent?x=1#y", "http://127.0.0.1:8081/"),
            ("https://relay.example.test:8443/", "https://relay.example.test:8443/"),
        ] {
            assert_eq!(relay_endpoint(configured).unwrap().as_str(), endpoint);
        }
        for invalid in ["ftp://relay/ws", "wss://user:pass@relay/ws", "not a url"] {
            assert!(relay_endpoint(invalid).is_err(), "{invalid}");
        }
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
        let mut relay = FakeRelay::start();
        let relay_url = relay.url.clone();
        let relay_task = tokio::spawn(async move {
            let mut session = relay.accept().await;
            assert_eq!(
                session.metadata.get("authorization").unwrap(),
                "Bearer test-authorized-device"
            );
            assert_eq!(
                session.metadata.get("x-baijimu-device-id").unwrap(),
                "dev_event_server_recovery"
            );
            assert_eq!(session.receive().await["type"], "capabilities");
            session
                .send(json!({
                    "type": "registered_ack",
                    "agent_id": "dev_event_server_recovery",
                    "workspace_id": 1,
                    "connection_id": "connection-1",
                    "registered_at_epoch_seconds": 1,
                    "heartbeat_timeout_secs": 75
                }))
                .await;
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            drop((session, relay));
        });

        let mut config = AgentConfig::example();
        config.platform.environment_key = Some("test".into());
        config.relay.token = "test-authorized-device".into();
        config.platform.workspace_id = Some(1);
        config.relay.url = relay_url;
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
    fn relay_decoder_ignores_unknown_frame_kind_and_reads_heartbeats() {
        assert!(matches!(
            decode_relay_frame(v1::RelayFrame { frame: None }).unwrap(),
            RelayInbound::Unsupported
        ));
        let heartbeat = v1::RelayFrame {
            frame: Some(RelayFrameKind::Heartbeat(v1::Heartbeat {})),
        };
        assert!(matches!(
            decode_relay_frame(heartbeat).unwrap(),
            RelayInbound::Heartbeat
        ));
    }

    fn decoded(message: Value) -> AgentMessage {
        let message: AgentMessage = serde_json::from_value(message).unwrap();
        match decode_relay_frame(relay_frame(message)).unwrap() {
            RelayInbound::Message(message) => message,
            _ => panic!("expected a relay message"),
        }
    }

    #[test]
    fn relay_decoder_accepts_registered_ack() {
        match decoded(json!({"type":"registered_ack","agent_id":"dev_1","workspace_id":1327,
            "connection_id":"conn_1","registered_at_epoch_seconds":1783680377,"heartbeat_timeout_secs":75}))
        {
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
        match decoded(json!({"type":"event_ack","eventId":"evt-1","appId":"camera",
            "receipts":[{"consumerId":"worker","receipt":{"status":"processed","duplicate":true}}]}))
        {
            AgentMessage::EventAck(ack) => {
                assert_eq!(ack.event_id, "evt-1");
                assert_eq!(ack.app_id, "camera");
                assert_eq!(ack.receipts.len(), 1);
                assert_eq!(ack.receipts[0].consumer_id, "worker");
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
        match decoded(json!({"type":"local_app_invoke_request","requestId":"req-1",
            "workspaceId":642,"appId":"camera","method":"capture","arguments":{}}))
        {
            AgentMessage::LocalAppInvokeRequest(request) => {
                assert_eq!(request.app_id, "camera");
                assert_eq!(request.method, "capture");
                assert_eq!(request.workspace_id, Some(642));
                assert_eq!(request.arguments, json!({}));
            }
            other => panic!("expected local_app_invoke_request, got {other:?}"),
        }
    }

    #[test]
    fn relay_decoder_treats_omitted_arguments_as_null_and_rejects_invalid_json() {
        let mut request = v1::InvokeRequest {
            request_id: "req_1".into(),
            service: "svc".into(),
            method: "call".into(),
            arguments_json: Vec::new(),
            timeout_secs: None,
        };
        let frame = |request: v1::InvokeRequest| v1::RelayFrame {
            frame: Some(RelayFrameKind::InvokeRequest(request)),
        };
        match decode_relay_frame(frame(request.clone())).unwrap() {
            RelayInbound::Message(AgentMessage::InvokeRequest(decoded)) => {
                assert_eq!(decoded.arguments, Value::Null)
            }
            _ => panic!("expected invoke_request"),
        }
        request.arguments_json = b"{not json".to_vec();
        assert!(decode_relay_frame(frame(request)).is_err());
    }

    #[test]
    fn device_frames_round_trip_through_relay_decoding() {
        let messages = [
            json!({"type":"capabilities","agent_id":"dev","protocol_version":"4.0.0",
                "protocol_features":["registered_ack"],
                "services":[{"name":"svc","description":"d","methods":[{"name":"m",
                    "description":"","input_schema":{"type":"object"},"responseMode":"plain"}]}],
                "local_apps":[{"appId":"app","name":"App","version":"1.0.0","description":"",
                    "methods":[],"events":[{"name":"changed","description":"","payload_schema":{}}]}]}),
            json!({"type":"invoke_result","request_id":"r","success":false,"data":null,
                "error":{"public_details":{"field":"x"},"code":"BAD","message":"m"},"duration_ms":3}),
            json!({"type":"local_app_invoke_result","request_id":"r","success":true,
                "data":{"ok":true},"error":null,"duration_ms":1}),
            json!({"type":"error","request_id":null,"message":"bad"}),
        ];
        for message in messages {
            let typed: AgentMessage = serde_json::from_value(message.clone()).unwrap();
            let frame = encode_device_frame(typed).unwrap().frame.unwrap();
            let decoded = serde_json::to_value(device_message(frame).unwrap()).unwrap();
            assert_eq!(decoded, message);
        }
        let relay_only: AgentMessage =
            serde_json::from_value(json!({"type":"stream_detach","attachId":"a"})).unwrap();
        assert!(encode_device_frame(relay_only).is_err());
    }

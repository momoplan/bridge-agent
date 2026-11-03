    fn update_required_config(relay: &FakeRelay, dir: &std::path::Path) -> AgentConfig {
        let mut config = AgentConfig::example();
        config.platform.environment_key = Some("test".into());
        config.platform.workspace_id = Some(1);
        config.relay.url = relay.url.clone();
        config.relay.agent_id = "dev_update_required".to_string();
        config.relay.token = "saved-token".to_string();
        config.relay.reconnect_secs = 1;
        config.runtime.log_file_dir = Some(dir.join("logs").display().to_string());
        config.runtime.event_server_enabled = false;
        config.runtime.service_registration_enabled = false;
        config.services.clear();
        config
    }

    #[tokio::test]
    async fn capabilities_report_the_client_version_and_platform() {
        let mut relay = FakeRelay::start();
        let dir = tempdir().unwrap();
        let manager = AgentRuntimeManager::new();
        manager
            .start(update_required_config(&relay, dir.path()), &dir.path().join("agent-config.json"))
            .await
            .unwrap();
        let mut session = relay.accept().await;
        let DeviceFrameKind::Capabilities(capabilities) = session.next_frame().await else {
            panic!("capabilities must be the first device frame");
        };
        assert_eq!(capabilities.client_version.as_deref(), Some(env!("CARGO_PKG_VERSION")));
        assert_eq!(capabilities.client_platform.as_deref(), Some(std::env::consts::OS));
        assert_eq!(capabilities.client_arch.as_deref(), Some(std::env::consts::ARCH));
        assert_eq!(
            capabilities.client_os_version,
            crate::current_os_version().unwrap(),
            "the update check and Relay see the same OS version"
        );
        manager.stop().await.unwrap();
    }

    #[tokio::test]
    async fn relay_update_required_stops_reconnecting_until_stopped() {
        let mut relay = FakeRelay::start();
        let dir = tempdir().unwrap();
        let manager = AgentRuntimeManager::new();
        manager
            .start(update_required_config(&relay, dir.path()), &dir.path().join("agent-config.json"))
            .await
            .unwrap();
        let mut session = relay.accept().await;
        assert!(matches!(session.next_frame().await, DeviceFrameKind::Capabilities(_)));
        session
            .send_frame(v1::RelayFrame {
                frame: Some(RelayFrameKind::UpdateRequired(v1::UpdateRequired {
                    platform: std::env::consts::OS.into(),
                    minimum_supported_version: "9.0.0".into(),
                    message: "请升级到最新版本".into(),
                })),
            })
            .await;
        session.close();

        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            while manager.snapshot().await.status != RuntimeStatus::UpdateRequired {
                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            }
        })
        .await
        .expect("runtime did not enter update-required state");
        // A regular disconnect would reconnect after `reconnect_secs`.
        tokio::time::sleep(std::time::Duration::from_millis(1_200)).await;
        assert_eq!(relay.attempts(), 1);
        let detail = manager.snapshot().await.last_error.unwrap();
        assert!(detail.contains("请升级到最新版本"), "{detail}");
        assert!(detail.contains("9.0.0"), "{detail}");

        manager.stop().await.unwrap();
        assert_eq!(manager.snapshot().await.status, RuntimeStatus::Stopped);
        let error = manager
            .start(update_required_config(&relay, dir.path()), &dir.path().join("agent-config.json"))
            .await
            .expect_err("stopping must not permit an unsupported client to start again");
        assert!(error.to_string().contains("必须先升级"));
        assert_eq!(relay.attempts(), 1);
    }

    #[tokio::test]
    async fn required_update_before_start_blocks_start_without_opening_a_connection() {
        let relay = FakeRelay::start();
        let dir = tempdir().unwrap();
        let manager = AgentRuntimeManager::new();
        manager.stop_for_required_update("需要官方更新".into()).await.unwrap();
        let error = manager.start(update_required_config(&relay, dir.path()), &dir.path().join("agent-config.json"))
            .await.expect_err("background startup must respect a concurrent required update");
        assert!(error.to_string().contains("需要官方更新"));
        assert_eq!(relay.attempts(), 0);
    }

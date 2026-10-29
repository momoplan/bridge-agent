    #[test]
    fn relay_unauthenticated_and_permission_denied_require_reauthorization() {
        assert!(is_relay_authorization_error(&tonic::Status::unauthenticated("Unauthorized")));
        assert!(is_relay_authorization_error(&tonic::Status::permission_denied("Forbidden")));
        // A token-store outage must not make the device discard its credential.
        assert!(!is_relay_authorization_error(&tonic::Status::unavailable("Service Unavailable")));
        assert!(!is_relay_authorization_error(&tonic::Status::failed_precondition("Bad Request")));
    }

    #[tokio::test]
    async fn relay_unauthorized_pauses_reconnect_until_reauthorization() {
        let relay = FakeRelay::rejecting(tonic::Code::Unauthenticated);
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("agent-config.json");
        let mut config = AgentConfig::example();
        config.platform.environment_key = Some("test".into());
        config.platform.workspace_id = Some(1);
        config.relay.url = relay.url.clone();
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
        assert_eq!(relay.attempts(), 1);
        assert_eq!(
            manager.snapshot().await.last_error.as_deref(),
            Some(RELAY_AUTHORIZATION_REQUIRED_MESSAGE)
        );

        manager.stop().await.unwrap();
    }

    #[tokio::test]
    async fn relay_unavailable_backs_off_and_retries_with_the_saved_credential() {
        let relay = FakeRelay::rejecting(tonic::Code::Unavailable);
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("agent-config.json");
        let mut config = AgentConfig::example();
        config.platform.environment_key = Some("test".into());
        config.platform.workspace_id = Some(1);
        config.relay.url = relay.url.clone();
        config.relay.agent_id = "dev_relay_unavailable".to_string();
        config.relay.token = "saved-token".to_string();
        config.relay.reconnect_secs = 1;
        config.runtime.log_file_dir = Some(dir.path().join("logs").display().to_string());
        config.runtime.event_server_enabled = false;
        config.runtime.service_registration_enabled = false;
        config.services.clear();

        let manager = AgentRuntimeManager::new();
        manager.start(config, &config_path).await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while relay.attempts() < 2 {
                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            }
        })
        .await
        .expect("runtime did not retry after relay was unavailable");
        assert_ne!(manager.snapshot().await.status, RuntimeStatus::AuthorizationRequired);
        manager.stop().await.unwrap();
    }

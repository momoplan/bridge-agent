    #[test]
    fn reconnect_backoff_grows_with_jitter_and_resets_after_success() {
        assert_eq!(reconnect_delay_ms(1, 0, 0), 500);
        assert_eq!(reconnect_delay_ms(1, 0, 999), 999);
        assert_eq!(reconnect_delay_ms(0, 0, 0), 500);
        assert_eq!(reconnect_delay_ms(3, 0, 0), 1_500);
        assert_eq!(reconnect_delay_ms(3, 1, 0), 3_000);
        assert_eq!(reconnect_delay_ms(3, 2, 0), 6_000);
        assert_eq!(reconnect_delay_ms(1, 6, 0), 30_000);
        assert_eq!(reconnect_delay_ms(1, 6, 999), 59_970);
        assert_eq!(reconnect_delay_ms(120, 4, 0), 60_000);

        let backoff = ReconnectBackoff::new();
        let first = backoff.next_delay(3);
        let second = backoff.next_delay(3);
        assert!(second >= first);
        backoff.reset();
        let reset = backoff.next_delay(3);
        let reset_ms = reset.as_millis();
        assert!(
            (1_500..=3_000).contains(&reset_ms),
            "successful registration must restore the base delay, got {reset_ms}ms"
        );
    }

    #[tokio::test]
    async fn reconnect_backoff_is_not_cut_short_by_local_audit_logs() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let relay_addr = listener.local_addr().unwrap();
        let attempts = Arc::new(AtomicUsize::new(0));
        let attempts_task = Arc::clone(&attempts);
        let relay = tokio::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok((stream, _)) => {
                        attempts_task.fetch_add(1, Ordering::SeqCst);
                        drop(stream);
                    }
                    Err(_) => break,
                }
            }
        });

        let event_probe = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let event_addr = event_probe.local_addr().unwrap();
        drop(event_probe);

        let dir = tempdir().unwrap();
        let config_path = dir.path().join("agent-config.json");
        let mut config = AgentConfig::example();
        config.platform.environment_key = Some("test".into());
        config.platform.workspace_id = Some(1);
        config.relay.token = "test-authorized-device".into();
        config.relay.url = format!("ws://{relay_addr}/ws/agent");
        config.relay.agent_id = "dev_reconnect_backoff".into();
        config.relay.reconnect_secs = 20;
        config.runtime.log_file_dir = Some(dir.path().join("logs").display().to_string());
        config.runtime.event_server_enabled = true;
        config.runtime.event_server_bind = event_addr.to_string();
        config.runtime.service_registration_enabled = false;
        config.services.clear();

        let manager = AgentRuntimeManager::new();
        manager.start(config, &config_path).await.unwrap();
        wait_for_runtime_status(&manager, RuntimeStatus::Backoff, 5)
            .await
            .expect("runtime did not enter reconnect backoff");
        assert_eq!(
            attempts.load(Ordering::SeqCst),
            1,
            "the first failed connect must be the only attempt before backoff"
        );

        let client = reqwest::Client::new();
        let healthz = format!("http://{event_addr}/healthz");
        wait_for_http_success(&client, &healthz, 5)
            .await
            .expect("local event server did not accept health checks during backoff");
        let started = tokio::time::Instant::now();
        while started.elapsed() < tokio::time::Duration::from_millis(1_200) {
            let _ = client.get(&healthz).send().await;
            tokio::time::sleep(tokio::time::Duration::from_millis(40)).await;
        }

        assert_eq!(
            attempts.load(Ordering::SeqCst),
            1,
            "local audit logs must not end the reconnect wait early"
        );
        assert_eq!(manager.snapshot().await.status, RuntimeStatus::Backoff);
        manager.stop().await.unwrap();
        relay.abort();
    }

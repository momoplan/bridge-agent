    #[tokio::test]
    async fn service_registration_api_writes_config_and_schedules_capability_update() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("agent-config.json");
        let mut config = AgentConfig::example();
        config.runtime.event_server_bind = "127.0.0.1:0".to_string();
        config.runtime.service_registration_enabled = true;
        config.runtime.service_registration_token = Some("secret".to_string());
        save_config(&config_path, &config).unwrap();

        let registry = Arc::new(RwLock::new(
            ServiceRegistry::from_config(&config, dir.path()).unwrap(),
        ));
        let (event_tx, _event_rx) = mpsc::channel(1);
        let (apply_tx, mut apply_rx) = mpsc::unbounded_channel();
        let (audit_tx, _audit_rx) = mpsc::unbounded_channel();
        let server = LocalEventServer::bind(
            &config,
            config_path.clone(),
            registry,
            event_tx,
            apply_tx,
            audit_tx,
        )
        .await
        .unwrap()
        .unwrap();
        let addr = server.bind_addr();
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let task = tokio::spawn(server.serve(shutdown_rx));

        let client = reqwest::Client::new();
        let retired_event_response = client
            .post(format!("http://{addr}/v1/services"))
            .bearer_auth("secret")
            .json(&json!({
                "name": "retiredEventTool",
                "description": "Must not restore retired Service events.",
                "transport": {
                    "type": "http",
                    "baseUrl": "http://127.0.0.1:39127"
                },
                "methods": [{
                    "name": "generate",
                    "description": "Generate a report.",
                    "path": "/invoke/generate"
                }],
                "events": [{
                    "name": "finished",
                    "description": "Retired Service event."
                }]
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(retired_event_response.status().as_u16(), 400);
        assert!(retired_event_response
            .text()
            .await
            .unwrap()
            .contains("Service custom events are retired"));

        let response = client
            .post(format!("http://{addr}/v1/services"))
            .bearer_auth("secret")
            .json(&json!({
                "name": "reportTool",
                "description": "AI generated report service.",
                "transport": {
                    "type": "http",
                    "baseUrl": "http://127.0.0.1:39127"
                },
                "methods": [
                    {
                        "name": "generate",
                        "description": "Generate a report.",
                        "path": "/invoke/generate"
                    }
                ],
                "replace": true
            }))
            .send()
            .await
            .unwrap();

        assert_eq!(response.status().as_u16(), 201);
        let updated = crate::config::load_config(&config_path).unwrap();
        assert!(updated
            .services
            .iter()
            .any(|service| service.name == "reportTool"));
        let update = apply_rx.recv().await.unwrap();
        assert!(update
            .services
            .iter()
            .any(|service| service.name == "reportTool"));

        shutdown_tx.send(true).unwrap();
        task.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn local_app_http_binding_forwards_trusted_workspace_context() {
        async fn capture_workspace(headers: HeaderMap) -> Json<Value> {
            Json(json!({
                "ok": true,
                "data": {
                    "workspaceId": headers
                        .get("x-baijimu-workspace-id")
                        .and_then(|value| value.to_str().ok()),
                    "authorization": headers
                        .get("authorization")
                        .and_then(|value| value.to_str().ok())
                }
            }))
        }

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new().route("/invoke/status", post(capture_workspace)),
            )
            .await
            .unwrap();
        });
        let current_dir = std::env::current_dir().unwrap();
        let token_dir = tempdir().unwrap();
        let token_path = token_dir.path().join("management-token");
        let token = "bjm_app_test_workspace_private_runtime_token";
        fs::write(&token_path, token).unwrap();
        let mut config = AgentConfig::example();
        config.local_apps.push(LocalAppConfig {
            app_id: "com.baijimu.connector.workspace-aware".to_string(),
            name: "Workspace-aware app".to_string(),
            version: "1.0.0".to_string(),
            description: "Captures trusted workspace context.".to_string(),
            enabled: true,
            health_check: None,
            start_command: Some(ServiceStartCommand::ShellCommand {
                command: vec!["unused-local-app-start".to_string()],
                cwd: None,
                env: BTreeMap::from([(
                    "BAIJIMU_LOCAL_APP_TOKEN_FILE".to_string(),
                    token_path.display().to_string(),
                )]),
                timeout_secs: Some(1),
            }),
            stop_command: None,
            methods: vec![MethodConfig {
                name: "status".to_string(),
                description: "Status".to_string(),
                enabled: true,
                input_schema: json!({"type": "object"}),
                response_mode: ResponseMode::Cmodel,
                binding: MethodBinding::Http(HttpBinding {
                    url: format!("http://{addr}/invoke/status"),
                    http_method: "POST".to_string(),
                    headers: BTreeMap::new(),
                    timeout_secs: Some(2),
                }),
            }],
            events: Vec::new(),
        });
        assert!(!serde_json::to_string(&config).unwrap().contains(token));
        let registry = ServiceRegistry::from_config(&config, &current_dir).unwrap();
        assert!(!serde_json::to_string(&config).unwrap().contains(token));
        let result = registry
            .invoke_local_app(
                "req-workspace".to_string(),
                Some(642),
                "com.baijimu.connector.workspace-aware",
                "status",
                json!({}),
                None,
            )
            .await;

        assert!(result.success);
        let data = result.data.unwrap();
        assert_eq!(data["workspaceId"], "642");
        assert_eq!(data["authorization"], format!("Bearer {token}"));
        server.abort();
    }

    #[tokio::test]
    async fn http_binding_unwraps_local_connector_success_envelope() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new().route(
                    "/invoke",
                    post(|| async {
                        Json(json!({
                            "success": true,
                            "data": {"messages": [{"text": "hello"}]},
                            "error": null
                        }))
                    }),
                ),
            )
            .await
            .unwrap();
        });

        let current_dir = std::env::current_dir().unwrap();
        let mut config = AgentConfig::example();
        config
            .services
            .push(http_test_service(&format!("http://{addr}/invoke")));
        let registry = ServiceRegistry::from_config(&config, &current_dir).unwrap();

        let result = registry
            .invoke(
                "req-http".to_string(),
                "localTool",
                "fetch",
                json!({}),
                None,
            )
            .await;

        assert!(result.success);
        assert_eq!(result.data.unwrap()["messages"][0]["text"], "hello");
        assert!(result.error.is_none());
        server.abort();
    }

    #[tokio::test]
    async fn http_binding_unwraps_local_connector_failure_envelope() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new().route(
                    "/invoke",
                    post(|| async {
                        Json(json!({
                            "success": false,
                            "data": null,
                            "error": {
                                "code": "LOCAL_FAILED",
                                "message": "collector failed"
                            }
                        }))
                    }),
                ),
            )
            .await
            .unwrap();
        });

        let current_dir = std::env::current_dir().unwrap();
        let mut config = AgentConfig::example();
        config
            .services
            .push(http_test_service(&format!("http://{addr}/invoke")));
        let registry = ServiceRegistry::from_config(&config, &current_dir).unwrap();

        let result = registry
            .invoke(
                "req-http".to_string(),
                "localTool",
                "fetch",
                json!({}),
                None,
            )
            .await;

        assert!(!result.success);
        let error = result.error.unwrap();
        assert_eq!(error.code, "LOCAL_FAILED");
        assert_eq!(error.message, "collector failed");
        server.abort();
    }

    #[tokio::test]
    async fn http_binding_unwraps_ok_local_connector_success_envelope() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new().route(
                    "/invoke",
                    post(|| async {
                        Json(json!({
                            "ok": true,
                            "data": {"result": {"data": [{"id": "thread-1"}]}}
                        }))
                    }),
                ),
            )
            .await
            .unwrap();
        });

        let current_dir = std::env::current_dir().unwrap();
        let mut config = AgentConfig::example();
        config
            .services
            .push(http_test_service(&format!("http://{addr}/invoke")));
        let registry = ServiceRegistry::from_config(&config, &current_dir).unwrap();

        let result = registry
            .invoke(
                "req-http-ok".to_string(),
                "localTool",
                "fetch",
                json!({}),
                None,
            )
            .await;

        assert!(result.success);
        assert_eq!(result.data.unwrap()["result"]["data"][0]["id"], "thread-1");
        assert!(result.error.is_none());
        server.abort();
    }

    #[tokio::test]
    async fn http_binding_preserves_ok_local_connector_error_details() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new().route(
                    "/invoke",
                    post(|| async {
                        (
                            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                            Json(json!({
                                "ok": false,
                                "error": {
                                    "code": "CODEX_APP_SERVER_FAILED",
                                    "message": "thread/list rejected the request"
                                }
                            })),
                        )
                    }),
                ),
            )
            .await
            .unwrap();
        });

        let current_dir = std::env::current_dir().unwrap();
        let mut config = AgentConfig::example();
        config
            .services
            .push(http_test_service(&format!("http://{addr}/invoke")));
        let registry = ServiceRegistry::from_config(&config, &current_dir).unwrap();

        let result = registry
            .invoke(
                "req-http-ok-error".to_string(),
                "localTool",
                "fetch",
                json!({}),
                None,
            )
            .await;

        assert!(!result.success);
        let error = result.error.unwrap();
        assert_eq!(error.code, "CODEX_APP_SERVER_FAILED");
        assert_eq!(error.message, "thread/list rejected the request");
        server.abort();
    }

    #[tokio::test]
    async fn http_binding_unwraps_cmodel_response() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new().route(
                    "/invoke",
                    post(|| async {
                        Json(json!({
                            "errorCode": "0",
                            "value": "成功",
                            "data": {"ok": true}
                        }))
                    }),
                ),
            )
            .await
            .unwrap();
        });

        let current_dir = std::env::current_dir().unwrap();
        let mut config = AgentConfig::example();
        config
            .services
            .push(http_test_service(&format!("http://{addr}/invoke")));
        let registry = ServiceRegistry::from_config(&config, &current_dir).unwrap();

        let result = registry
            .invoke(
                "req-http".to_string(),
                "localTool",
                "fetch",
                json!({}),
                None,
            )
            .await;

        assert!(result.success);
        assert_eq!(result.data.unwrap()["ok"], true);
        assert!(result.error.is_none());
        server.abort();
    }

    #[tokio::test]
    async fn http_binding_plain_mode_returns_json_without_cmodel_unwrapping() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new().route(
                    "/invoke",
                    post(|| async {
                        Json(json!({
                            "errorCode": "0",
                            "value": "OK",
                            "data": {"ok": true}
                        }))
                    }),
                ),
            )
            .await
            .unwrap();
        });

        let current_dir = std::env::current_dir().unwrap();
        let mut config = AgentConfig::example();
        config.services.push(http_test_service_with_mode(
            &format!("http://{addr}/invoke"),
            ResponseMode::Plain,
        ));
        let registry = ServiceRegistry::from_config(&config, &current_dir).unwrap();
        let result = registry
            .invoke(
                "req-http-plain".to_string(),
                "localTool",
                "fetch",
                json!({}),
                None,
            )
            .await;

        assert!(result.success);
        assert_eq!(result.data.unwrap()["errorCode"], "0");
        server.abort();
    }

    #[test]
    fn passthrough_outcome_preserves_utf8_and_binary_response_bytes() {
        let utf8 = passthrough_http_outcome(
            202,
            BTreeMap::from([("content-type".to_string(), "text/event-stream".to_string())]),
            b"data: first\n\ndata: [DONE]\n\n",
        );
        assert!(utf8.success);
        let utf8 = utf8.data.unwrap();
        assert_eq!(utf8["status"], 202);
        assert_eq!(utf8["bodyEncoding"], "utf8");
        assert_eq!(utf8["body"], "data: first\n\ndata: [DONE]\n\n");

        let binary = passthrough_http_outcome(200, BTreeMap::new(), &[0, 159, 146, 150]);
        let binary = binary.data.unwrap();
        assert_eq!(binary["bodyEncoding"], "base64");
        assert_eq!(binary["body"], "AJ+Slg==");
    }

    #[tokio::test]
    async fn unknown_service_returns_error() {
        let current_dir = std::env::current_dir().unwrap();
        let registry = ServiceRegistry::from_config(&AgentConfig::example(), &current_dir).unwrap();
        let result = registry
            .invoke("req-1".to_string(), "git", "status", json!({}), None)
            .await;
        assert!(!result.success);
        assert_eq!(result.error.unwrap().code, "INVOKE_FAILED");
    }

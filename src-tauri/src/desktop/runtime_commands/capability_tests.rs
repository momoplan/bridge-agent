use super::*;
use bridge_agent::config::{HttpBinding, MethodBinding, MethodConfig, ResponseMode};
use serde_json::json;

#[tokio::test]
async fn saved_local_app_test_forwards_authorized_workspace_and_unchanged_arguments() {
    async fn capture(headers: HeaderMap, Json(arguments): Json<Value>) -> Json<Value> {
        Json(json!({"ok": true, "data": {
            "workspace": headers.get("x-baijimu-workspace-id").unwrap().to_str().unwrap(),
            "arguments": arguments
        }}))
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, Router::new().route("/invoke", post(capture)))
            .await
            .unwrap();
    });
    let directory = tempfile::tempdir().unwrap();
    let config_path = directory.path().join("config.json");
    let mut config = AgentConfig::example();
    config.platform.workspace_id = Some(73);
    config.relay.token = "test-device-credential".into();
    config.local_apps.push(LocalAppConfig {
        app_id: "workspace-test".into(),
        name: "Workspace test".into(),
        version: "1.0.0".into(),
        description: String::new(),
        enabled: true,
        health_check: None,
        start_command: None,
        stop_command: None,
        methods: vec![MethodConfig {
            name: "request".into(),
            description: String::new(),
            enabled: true,
            input_schema: json!({"type": "object"}),
            response_mode: ResponseMode::Cmodel,
            binding: MethodBinding::Http(HttpBinding {
                url: format!("http://{address}/invoke"),
                http_method: "POST".into(),
                headers: BTreeMap::new(),
                timeout_secs: Some(2),
            }),
        }],
        events: Vec::new(),
    });
    save_agent_config(&config_path, &config).unwrap();
    // Business fields resembling identity must not become trusted request context.
    let arguments = json!({"workspaceId": 999, "method": "account/read",
        "params": {"refreshToken": false, "nested": [null, 42, "text"]}});
    for workspace in [73, 84] {
        config.platform.workspace_id = Some(workspace);
        save_agent_config(&config_path, &config).unwrap();
        let result = invoke_saved_local_app_capability(
            &config_path, "workspace-test", "request", arguments.clone(), Some(2),
        ).await.unwrap();
        assert!(result.success, "{result:?}");
        let data = result.data.unwrap();
        assert_eq!(data["workspace"], workspace.to_string());
        assert_eq!(data["arguments"], arguments);
    }
    server.abort();
}

#[tokio::test]
async fn saved_local_app_test_rejects_missing_authorization_before_dispatch() {
    for (workspace, token) in [(None, "credential"), (Some(73), ""), (Some(0), "credential")] {
        let directory = tempfile::tempdir().unwrap();
        let config_path = directory.path().join("config.json");
        let mut config = AgentConfig::example();
        config.platform.workspace_id = workspace;
        config.relay.token = token.into();
        save_agent_config(&config_path, &config).unwrap();
        let error = invoke_saved_local_app_capability(
            &config_path, "not-installed", "request", json!({"workspaceId": 73}), None,
        ).await.unwrap_err();
        assert!(error.contains("授权"), "{error}");
    }
}

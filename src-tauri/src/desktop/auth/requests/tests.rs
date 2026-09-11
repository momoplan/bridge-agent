use super::*;
use axum::{extract::Json, routing::post, Router};

#[tokio::test]
async fn migrated_config_starts_and_polls_against_owner_routes() {
    async fn start_auth(Json(body): Json<Value>) -> Json<Value> {
        assert_eq!(body["deviceId"], "upgrade-device");
        assert_eq!(body["workspaceId"], 77);
        assert!(body.get("serviceManifest").is_some());
        Json(serde_json::json!({
            "deviceCode":"test-code", "userCode":"ABCD",
            "verificationUri":"https://example.test/activate",
            "verificationUriComplete":"https://example.test/activate?user_code=ABCD",
            "expiresIn":600, "interval":3
        }))
    }
    async fn poll_auth(Json(body): Json<Value>) -> Json<Value> {
        assert_eq!(body["deviceCode"], "test-code");
        Json(serde_json::json!({"status":"pending", "message":"等待用户授权"}))
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route(
                    "/tenant/device-service/api/external-workspace-device-auth/start",
                    post(start_auth),
                )
                .route(
                    "/tenant/device-service/api/external-workspace-device-auth/poll",
                    post(poll_auth),
                ),
        )
        .await
        .unwrap();
    });
    let mut config = AgentConfig::example();
    config.platform.base_url = format!("http://{address}/tenant/lowcode3");
    config.platform.workspace_id = Some(77);
    config.relay.agent_id = "upgrade-device".into();
    config.normalize();
    let started = start(&config).await.unwrap();
    let polled = poll(&config, &started.device_code).await.unwrap();
    assert_eq!(polled.status, "pending");
    server.abort();
}

#[test]
fn endpoint_uses_current_device_contract_for_old_and_new_config() {
    for base in [
        "https://api.baijimu.com",
        "https://api.baijimu.com/lowcode3",
    ] {
        for operation in ["start", "poll"] {
            assert_eq!(endpoint(base, operation).unwrap().as_str(), format!(
                "https://api.baijimu.com/device-service/api/external-workspace-device-auth/{operation}"));
        }
    }
    for base in [
        "file:///tmp/config",
        "https://user:secret@example.test",
        "https://example.test?token=secret",
        "https://example.test#fragment",
    ] {
        assert!(endpoint(base, "start").is_err());
    }
}

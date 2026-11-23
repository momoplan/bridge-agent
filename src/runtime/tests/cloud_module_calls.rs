#[tokio::test]
async fn cloud_module_call_requires_matching_registration_and_returns_over_the_registered_stream() {
    use crate::connector::tests::{connector_test_env, install_bound_call_fixture};
    let installation = tempdir().unwrap();
    let _env = connector_test_env(installation.path().join("connectors"));
    let mut harness = InvocationHarness::with_config(install_bound_call_fixture).await;
    let source = harness
        .capabilities
        .local_apps
        .iter()
        .find(|app| app.app_id == "test-app")
        .unwrap()
        .installed_source
        .clone()
        .unwrap();
    let request = || {
        serde_json::from_value(json!({
            "requestId": "upload-one", "moduleKey": "storage", "method": "prepareFileUpload",
            "arguments": {"messageId": "fixture"}
        }))
        .unwrap()
    };

    for workspace in [2, 1] {
        harness.socket.send(json!({
            "type": "registered_ack", "agent_id": "dev_test", "workspace_id": workspace,
            "connection_id": "upstream", "registered_at_epoch_seconds": 1, "heartbeat_timeout_secs": 75
        })).await;
        harness.call("after-registration", false, false).await;
        assert_eq!(harness.receive().await["request_id"], "after-registration");
        if workspace == 2 {
            let result = harness
                .cloud_calls
                .call(source.clone().into(), request())
                .await;
            assert!(result
                .err()
                .unwrap()
                .to_string()
                .contains("no registered Relay connection"));
        }
    }
    let (shutdown, server, pending) = begin_authenticated_module_post(&harness).await;
    let DeviceFrameKind::CloudModuleCall(call) = harness.socket.next_frame().await else {
        panic!("expected device-initiated module call");
    };
    assert_eq!(call.installed_source, Some(source.clone()));
    assert_eq!(call.request_id, "upload-one");
    assert_eq!(call.module_key, "storage");
    assert_eq!(call.method, "prepareFileUpload");
    assert_eq!(
        serde_json::from_slice::<Value>(&call.arguments_json).unwrap(),
        json!({"messageId": "fixture"})
    );
    send_cloud_call_receipts(&mut harness.socket, &source).await;
    let result = tokio::time::timeout(std::time::Duration::from_secs(2), pending)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(result.status(), reqwest::StatusCode::OK);
    let result = result.json::<Value>().await.unwrap();
    shutdown.send(true).unwrap();
    server.await.unwrap().unwrap();
    assert_eq!(result["requestId"], "upload-one");
    assert_eq!(result["success"], true);
    assert_eq!(result["data"]["fileId"], "object-one");
}

async fn send_cloud_call_receipts(socket: &mut RelaySession, source: &v1::LocalAppSource) {
    // An unrelated incomplete receipt must not interrupt this call or replace the connection.
    for invalid in [
        v1::CloudModuleReply {
            installed_source: Some(source.clone()),
            result: None,
        },
        v1::CloudModuleReply {
            installed_source: None,
            result: Some(v1::InvokeResult::default()),
        },
    ] {
        socket
            .send_frame(v1::RelayFrame {
                frame: Some(RelayFrameKind::CloudModuleReply(invalid)),
            })
            .await;
    }
    for (id, body) in [
        ("other-request", b"invalid".as_slice()),
        ("upload-one", br#"{"fileId":"object-one"}"#.as_slice()),
    ] {
        socket
            .send_frame(v1::RelayFrame {
                frame: Some(RelayFrameKind::CloudModuleReply(v1::CloudModuleReply {
                    installed_source: Some(source.clone()),
                    result: Some(v1::InvokeResult {
                        request_id: id.into(),
                        success: true,
                        data_json: Some(body.to_vec()),
                        ..Default::default()
                    }),
                })),
            })
            .await;
    }
}

async fn begin_authenticated_module_post(
    harness: &InvocationHarness,
) -> (
    watch::Sender<bool>,
    tokio::task::JoinHandle<anyhow::Result<()>>,
    tokio::task::JoinHandle<reqwest::Result<reqwest::Response>>,
) {
    let config_path = harness._dir.path().join("config.json");
    let mut config = crate::config::load_config(&config_path).unwrap();
    config.runtime.event_server_bind = "127.0.0.1:0".into();
    config.runtime.event_server_enabled = false;
    config.runtime.service_registration_enabled = false;
    let (audit, _) = mpsc::unbounded_channel();
    let server = crate::event_server::LocalEventServer::bind(
        &config,
        config_path,
        Arc::new(RwLock::new(harness.registry.clone())),
        None,
        harness.apply.clone(),
        audit,
        harness.cloud_calls.clone(),
    )
    .await
    .unwrap()
    .unwrap();
    let endpoint = format!(
        "http://{}{}",
        server.bind_addr(),
        crate::cloud_calls::LOCAL_PATH
    );
    let (shutdown, receiver) = watch::channel(false);
    let server = tokio::spawn(server.serve(receiver));
    let client = reqwest::Client::new();
    let body = json!({"requestId":"upload-one","moduleKey":"storage",
                      "method":"prepareFileUpload","arguments":{"messageId":"fixture"}});
    let rejected = client
        .post(&endpoint)
        .bearer_auth("incorrect")
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(rejected.status(), reqwest::StatusCode::FORBIDDEN);
    let token =
        fs::read_to_string(crate::connector::connector_event_token_path("test-app").unwrap())
            .unwrap();
    let mut override_target = body.clone();
    override_target["runtimeId"] = json!("wrong");
    let rejected = client
        .post(&endpoint)
        .bearer_auth(token.trim())
        .json(&override_target)
        .send()
        .await
        .unwrap();
    assert_eq!(rejected.status(), reqwest::StatusCode::UNPROCESSABLE_ENTITY);
    let pending = tokio::spawn(
        client
            .post(&endpoint)
            .bearer_auth(token.trim())
            .json(&body)
            .send(),
    );
    (shutdown, server, pending)
}

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
    let calls = harness.cloud_calls.clone();
    let installed = source.clone().into();
    let pending = tokio::spawn(async move { calls.call(installed, request()).await });
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
        harness
            .socket
            .send_frame(v1::RelayFrame {
                frame: Some(RelayFrameKind::CloudModuleReply(invalid)),
            })
            .await;
    }
    for (id, body) in [
        ("other-request", b"invalid".as_slice()),
        ("upload-one", br#"{"fileId":"object-one"}"#.as_slice()),
    ] {
        harness
            .socket
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
    let result = tokio::time::timeout(std::time::Duration::from_secs(2), pending)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let result = serde_json::to_value(result).unwrap();
    assert_eq!(result["requestId"], "upload-one");
    assert_eq!(result["success"], true);
    assert_eq!(result["data"]["fileId"], "object-one");
}

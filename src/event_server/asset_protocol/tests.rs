use super::*;

#[test]
fn relay_origin_selects_asset_protocol_without_credentials_or_ws_path() {
    assert_eq!(
        base_url("wss://relay.example.test:8443/ws/agent?secret=hidden").unwrap(),
        "https://relay.example.test:8443/api/bridge-agent/local-app-assets/"
    );
    assert!(base_url("wss://user:secret@relay.example.test/ws").is_none());
}

#[tokio::test]
async fn preserves_owner_failure_and_never_prints_proxy_or_signed_url_bodies() {
    let error = AssetUploadError::decode::<serde_json::Value>(StatusCode::OK,
        br#"{"contractVersion":"1.0.0","errorCode":"WORKSPACE_FILE_SERVICE_CONFLICT","data":{"message":"quota exceeded","retryable":false}}"#, "prepare").unwrap_err();
    assert_eq!(error.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(error.error_code, "WORKSPACE_FILE_SERVICE_CONFLICT");
    assert_eq!(error.retryable, Some(false));
    let response = error.into_response();
    let body = axum::body::to_bytes(response.into_body(), 8192)
        .await
        .unwrap();
    let value: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(value["message"], "quota exceeded");
    assert_eq!(value["stage"], "prepare");
    let error = AssetUploadError::decode::<Value>(
        StatusCode::BAD_GATEWAY,
        b"proxy failed https://storage.test?secret=private",
        "complete",
    )
    .unwrap_err();
    assert_eq!(error.error_code, "BAD_GATEWAY");
    assert_eq!(error.retryable, Some(true));
    assert!(!error.message.contains("private"));
}

#[tokio::test]
async fn put_is_followed_by_owner_completion_and_rejection_cannot_be_success() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let uploaded = Arc::new(AtomicBool::new(false));
    let reject = Arc::new(AtomicBool::new(false));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let file_id = uuid::Uuid::new_v4().to_string();
    let (put_seen, complete_seen, fail) = (uploaded.clone(), uploaded.clone(), reject.clone());
    let id = file_id.clone();
    let prepare_id = file_id.clone();
    let object_url = format!("{origin}/object");
    let app = Router::new()
        .route("/api/bridge-agent/local-app-assets/prepare", axum::routing::post(move |Json(body): Json<Value>| {
            assert_eq!(body["contractVersion"], "1.0.0");
            assert_eq!(body["appId"], "test-app");
            assert_eq!(body["purpose"], "evidence");
            let (id, url) = (prepare_id.clone(), object_url.clone());
            async move { Json(json!({"contractVersion":"1.0.0","errorCode":"0","data":{"contractVersion":"1.0.0","fileId":id,"objectKey":"staging/object","uploadUrl":url,"method":"PUT","headers":{"Content-Type":"image/png"}}})) }
        }))
        .route("/object", put(move |body: axum::body::Bytes| {
            let put_seen = put_seen.clone();
            async move {
                assert_eq!(&body[..], b"image");
                put_seen.store(true, Ordering::SeqCst);
                StatusCode::OK
            }
        }))
        .route("/api/bridge-agent/local-app-assets/complete", axum::routing::post(move |headers: HeaderMap, Json(body): Json<Value>| {
            let (seen, fail, id) = (complete_seen.clone(), fail.clone(), id.clone());
            async move {
                assert!(seen.load(Ordering::SeqCst));
                assert_eq!(headers["authorization"], "Bearer device-token");
                assert_eq!(body["fileId"], id);
                assert_eq!(body["objectKey"], "staging/object");
                if fail.load(Ordering::SeqCst) {
                    Json(json!({"contractVersion":"1.0.0","errorCode":"WORKSPACE_FILE_SERVICE_FORBIDDEN","data":{"message":"grant revoked","retryable":false}}))
                } else {
                    Json(json!({"contractVersion":"1.0.0","errorCode":"0","data":{"contractVersion":"1.0.0","fileId":id,"fileName":"image.png","path":"attachments/connector_asset/image.png","mimeType":"image/png","sizeBytes":5,"downloadUrl":"https://storage.test/committed","expiresAt":"2030-01-01T00:00:00Z"}}))
                }
            }
        }));
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let directory = tempfile::tempdir().unwrap();
    let mut config = AgentConfig::example();
    config.runtime.service_registration_enabled = true;
    config.runtime.event_server_bind = "127.0.0.1:0".into();
    config.relay.url = origin;
    config.relay.token = "device-token".into();
    let registry = Arc::new(RwLock::new(
        ServiceRegistry::from_config(&config, directory.path()).unwrap(),
    ));
    let (apply, _) = mpsc::unbounded_channel();
    let (audit, _) = mpsc::unbounded_channel();
    let server = LocalEventServer::bind(
        &config,
        directory.path().join("config.json"),
        registry,
        None,
        apply,
        audit,
    )
    .await
    .unwrap()
    .unwrap();
    let asset = ValidatedLocalAsset {
        app_id: "test-app".into(),
        purpose: "evidence".into(),
        content_type: "image/png".into(),
        file_name: "image.png".into(),
        bytes: b"image".to_vec(),
        size_bytes: 5,
    };
    let slot = super::super::prepare_local_asset_upload(&server.state, &asset)
        .await
        .unwrap();
    assert_eq!(slot.file_id, file_id);
    super::super::upload_local_asset_bytes(&server.state, &slot, &asset)
        .await
        .unwrap();
    let completed = complete(&server.state, &slot, &asset).await.unwrap();
    assert_eq!(completed.path, "attachments/connector_asset/image.png");
    reject.store(true, Ordering::SeqCst);
    let error = complete(&server.state, &slot, &asset).await.err().unwrap();
    assert_eq!(error.error_code, "WORKSPACE_FILE_SERVICE_FORBIDDEN");
    assert_eq!(error.stage, "complete");
    task.abort();
}

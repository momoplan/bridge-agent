use super::*;
use serde_json::json;

fn source(app: &str) -> InstalledLocalAppSource {
    serde_json::from_value(
        json!({"application":{"environmentKey":"test","appId":app},"version":"1.0.0"}),
    )
    .unwrap()
}
fn request() -> Request {
    serde_json::from_value(
        json!({"requestId":"one","moduleKey":"storage","method":"prepareFileUpload",
        "arguments":{"marker":"fixture"}}),
    )
    .unwrap()
}
fn receipt(source: InstalledLocalAppSource) -> v1::CloudModuleReply {
    v1::CloudModuleReply {
        installed_source: Some(source.into()),
        result: Some(v1::InvokeResult {
            request_id: "one".into(),
            success: true,
            data_json: Some(br#"{"received":true}"#.to_vec()),
            error: None,
            duration_ms: 2,
        }),
    }
}

#[tokio::test]
async fn only_the_same_installation_and_connection_complete_the_pending_call() {
    let calls = CloudCalls::default();
    let (tx, mut frames) = mpsc::channel(4);
    let (other, _unused) = mpsc::channel(4);
    assert!(calls.call(source("one"), request()).await.is_err());
    calls.connect(tx.clone());
    let pending = {
        let calls = calls.clone();
        tokio::spawn(async move { calls.call(source("one"), request()).await })
    };
    let Some(device_frame::Frame::CloudModuleCall(call)) = frames.recv().await.unwrap().frame
    else {
        panic!("expected call")
    };
    assert_eq!(call.method, "prepareFileUpload");
    assert_eq!(call.request_id, "one");
    assert!(calls.call(source("one"), request()).await.is_err());
    calls.receive(&other, receipt(source("one"))).unwrap();
    calls.receive(&tx, receipt(source("two"))).unwrap();
    assert!(!pending.is_finished());
    calls.receive(&tx, receipt(source("one"))).unwrap();
    let reply = pending.await.unwrap().unwrap();
    assert_eq!(reply.data, Some(json!({"received":true})));
    let next = {
        let calls = calls.clone();
        tokio::spawn(async move { calls.call(source("one"), request()).await })
    };
    frames.recv().await.unwrap();
    calls.disconnect(&tx);
    assert!(next.await.unwrap().is_err());
}

#[tokio::test]
async fn a_bad_receipt_fails_only_its_request_and_a_module_denial_is_preserved() {
    let calls = CloudCalls::default();
    let (channel, mut frames) = mpsc::channel(4);
    calls.connect(channel.clone());
    let start = |app: &'static str| {
        let calls = calls.clone();
        tokio::spawn(async move { calls.call(source(app), request()).await })
    };
    let bad = start("first");
    let denied = start("second");
    frames.recv().await.unwrap();
    frames.recv().await.unwrap();

    let mut malformed = receipt(source("first"));
    malformed.result.as_mut().unwrap().data_json = Some(b"invalid JSON".to_vec());
    calls.receive(&channel, malformed).unwrap();
    assert!(bad.await.unwrap().is_err());
    assert!(!denied.is_finished());

    let mut refusal = receipt(source("second"));
    let result = refusal.result.as_mut().unwrap();
    result.success = false;
    result.data_json = None;
    result.error = Some(v1::InvokeError {
        code: "METHOD_DENIED".into(),
        message: "method is unavailable to this installation".into(),
        public_details_json: Some(br#"{"method":"prepareFileUpload"}"#.to_vec()),
    });
    calls.receive(&channel, refusal).unwrap();
    let receipt = denied.await.unwrap().unwrap();
    assert!(!receipt.success);
    let error = receipt.error.unwrap();
    assert_eq!(error.code, "METHOD_DENIED");
    assert_eq!(
        error.public_details,
        Some(json!({"method":"prepareFileUpload"}))
    );
}

#[test]
fn installation_identity_and_runtime_are_not_local_request_fields() {
    let valid = json!({"requestId":"one","moduleKey":"files","method":"prepareFileUpload",
        "arguments":{"messageId":"message-one"}});
    assert!(serde_json::from_value::<Request>(valid.clone()).is_ok());
    for key in [
        "runtimeId",
        "applicationRuntimeId",
        "deviceId",
        "installedSource",
    ] {
        let mut input = valid.clone();
        input[key] = json!("supplied-by-caller");
        assert!(serde_json::from_value::<Request>(input).is_err(), "{key}");
    }
}

use super::*;
use crate::config::AgentConfig;
use axum::{
    body::Bytes,
    extract::{DefaultBodyLimit, State},
    http::HeaderMap,
    routing::{post, put},
    Json, Router,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc, Mutex,
};
use tokio::time::{sleep, Duration, Instant};

const FILE: &str = "00000000-0000-4000-8000-000000000001";
#[derive(Clone, Default)]
struct Owner {
    base: String,
    bytes: Arc<Mutex<Vec<u8>>>,
    commits: Arc<AtomicU64>,
    delay: Arc<AtomicBool>,
    delay_ms: Arc<AtomicU64>,
    reject_complete: Arc<AtomicBool>,
    declared_size: Arc<AtomicU64>,
    mime_type: Arc<Mutex<String>>,
}
fn success(value: Value) -> Json<Value> {
    Json(json!({"contractVersion":"1.0.0","errorCode":"0","data":value}))
}
async fn prepare(State(s): State<Owner>, headers: HeaderMap, Json(v): Json<Value>) -> Json<Value> {
    assert!(headers.get("authorization").is_some());
    assert_eq!(v["contractVersion"], "1.0.0");
    assert!(v.get("userId").is_none());
    *s.mime_type.lock().unwrap() = v["mimeType"].as_str().unwrap().into();
    success(
        json!({"contractVersion":"1.0.0","fileId":FILE,"objectKey":"staging/object","uploadUrl":format!("{}/oss",s.base),"method":"PUT","headers":{"content-type":v["mimeType"]}}),
    )
}
async fn store(State(s): State<Owner>, headers: HeaderMap, body: Bytes) -> axum::http::StatusCode {
    assert!(
        headers.get("authorization").is_none(),
        "device bearer must never reach object storage"
    );
    if s.delay.load(Ordering::Relaxed) {
        sleep(Duration::from_millis(s.delay_ms.load(Ordering::Relaxed))).await;
    }
    *s.bytes.lock().unwrap() = body.to_vec();
    axum::http::StatusCode::OK
}
fn completion(s: &Owner, size: u64) -> Value {
    json!({"contractVersion":"1.0.0","fileId":FILE,"fileName":"file.bin","path":"attachments/device_file/file.bin","mimeType":s.mime_type.lock().unwrap().clone(),"sizeBytes":size,"downloadUrl":format!("{}/oss",s.base),"expiresAt":"2099-01-01T00:00:00Z"})
}
async fn complete(State(s): State<Owner>, Json(v): Json<Value>) -> Json<Value> {
    assert_eq!(v["fileId"], FILE);
    assert_eq!(v["objectKey"], "staging/object");
    s.commits.fetch_add(1, Ordering::Relaxed);
    if s.reject_complete.load(Ordering::Relaxed) {
        return Json(
            json!({"contractVersion":"1.0.0","errorCode":"WORKSPACE_FILE_SERVICE_FORBIDDEN","data":{"message":"设备授权已撤销","retryable":false}}),
        );
    }
    let size = s.bytes.lock().unwrap().len() as u64;
    success(completion(&s, size))
}
async fn download(State(s): State<Owner>) -> Json<Value> {
    success(completion(&s, s.declared_size.load(Ordering::Relaxed)))
}
async fn content(State(s): State<Owner>) -> Vec<u8> {
    s.bytes.lock().unwrap().clone()
}
async fn fixture() -> (Owner, AgentConfig, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let s = Owner {
        base: format!("http://{}", listener.local_addr().unwrap()),
        mime_type: Arc::new(Mutex::new("application/octet-stream".into())),
        delay_ms: Arc::new(AtomicU64::new(400)),
        ..Owner::default()
    };
    let app = Router::new()
        .route("/api/bridge-agent/files/uploads/prepare", post(prepare))
        .route("/api/bridge-agent/files/uploads/complete", post(complete))
        .route("/api/bridge-agent/files/downloads/prepare", post(download))
        .route("/oss", put(store).get(content))
        .layer(DefaultBodyLimit::disable())
        .with_state(s.clone());
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let mut config = AgentConfig::example();
    config.relay.url = s.base.clone();
    config.relay.token = uuid::Uuid::new_v4().to_string();
    (s, config, task)
}
async fn terminal(config: &AgentConfig, id: &Value) -> Value {
    let query = FileMethod::new(config, "queryTransfer").unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let value = query.invoke(json!({"transferId":id})).await.unwrap();
        if ["SUCCEEDED", "FAILED", "CANCELLED"].contains(&value["status"].as_str().unwrap()) {
            return value;
        }
        assert!(Instant::now() < deadline);
        sleep(Duration::from_millis(10)).await;
    }
}
#[tokio::test]
async fn streamed_roundtrip_survives_registry_rebuild_and_checks_hash() {
    let (owner, config, server) = fixture().await;
    owner.delay.store(true, Ordering::Relaxed);
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("input.bin");
    let output = dir.path().join("output.bin");
    let bytes = vec![0x53; 5 * 1024 * 1024 + 19];
    tokio::fs::write(&input, &bytes).await.unwrap();
    let upload = crate::services::ServiceRegistry::from_config(&config, dir.path()).unwrap();
    let before = Instant::now();
    let result = upload
        .invoke(
            "upload".into(),
            "files",
            "upload",
            json!({"localPath":input}),
            None,
        )
        .await;
    assert!(result.success);
    let started = result.data.unwrap();
    assert!(before.elapsed() < Duration::from_millis(200));
    drop(upload);
    let done = terminal(&config, &started["transferId"]).await;
    assert_eq!(done["status"], "SUCCEEDED");
    assert_eq!(done["bytesTransferred"], bytes.len());
    assert_eq!(
        done["fileRef"]["sha256"],
        format!("{:x}", Sha256::digest(&bytes))
    );
    owner
        .declared_size
        .store(bytes.len() as u64, Ordering::Relaxed);
    let start = FileMethod::new(&config, "download")
        .unwrap()
        .invoke(
            json!({"fileId":FILE,"localPath":output,"expectedSha256":done["fileRef"]["sha256"]}),
        )
        .await
        .unwrap();
    assert_eq!(
        terminal(&config, &start["transferId"]).await["status"],
        "SUCCEEDED"
    );
    assert_eq!(tokio::fs::read(&output).await.unwrap(), bytes);
    assert_eq!(owner.commits.load(Ordering::Relaxed), 1);
    server.abort();
}
#[tokio::test]
async fn owner_failure_never_becomes_success_and_cancellation_stops_before_commit() {
    let (owner, config, server) = fixture().await;
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("input.bin");
    tokio::fs::write(&input, b"hello").await.unwrap();
    owner.reject_complete.store(true, Ordering::Relaxed);
    let start = FileMethod::new(&config, "upload")
        .unwrap()
        .invoke(json!({"localPath":input}))
        .await
        .unwrap();
    let failed = terminal(&config, &start["transferId"]).await;
    assert_eq!(failed["status"], "FAILED");
    assert_eq!(failed["error"]["code"], "WORKSPACE_FILE_SERVICE_FORBIDDEN");
    assert!(failed.get("fileRef").is_none());
    owner.delay.store(true, Ordering::Relaxed);
    let start = FileMethod::new(&config, "upload")
        .unwrap()
        .invoke(json!({"localPath":input}))
        .await
        .unwrap();
    FileMethod::new(&config, "cancelTransfer")
        .unwrap()
        .invoke(json!({"transferId":start["transferId"]}))
        .await
        .unwrap();
    assert_eq!(
        terminal(&config, &start["transferId"]).await["status"],
        "CANCELLED"
    );
    assert_eq!(owner.commits.load(Ordering::Relaxed), 1);
    let mut other = config.clone();
    other.relay.token = "different-grant".into();
    assert!(FileMethod::new(&other, "queryTransfer")
        .unwrap()
        .invoke(json!({"transferId":start["transferId"]}))
        .await
        .is_err());
    server.abort();
}
#[tokio::test]
async fn downloads_preserve_existing_file_and_clean_partial_content_on_hash_or_length_failure() {
    let (owner, config, server) = fixture().await;
    *owner.bytes.lock().unwrap() = b"hello".to_vec();
    owner.declared_size.store(5, Ordering::Relaxed);
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("output.bin");
    tokio::fs::write(&output, b"original").await.unwrap();
    let client = FileClient::from_config(&config).unwrap();
    for expected_size in [5, 6] {
        owner.declared_size.store(expected_size, Ordering::Relaxed);
        let error = client
            .download(
                FILE,
                &output,
                Some(&"0".repeat(64)),
                Arc::new(AtomicU64::new(0)),
                Arc::new(AtomicU64::new(0)),
            )
            .await
            .err()
            .unwrap();
        assert_eq!(
            error.code,
            if expected_size == 5 {
                "FILE_HASH_MISMATCH"
            } else {
                "FILE_SIZE_MISMATCH"
            }
        );
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }
    owner.declared_size.store(5, Ordering::Relaxed);
    let pending = client
        .download(
            FILE,
            &output,
            None,
            Arc::new(AtomicU64::new(0)),
            Arc::new(AtomicU64::new(0)),
        )
        .await
        .unwrap();
    assert!(pending.commit(&output, false).is_err());
    assert_eq!(std::fs::read(&output).unwrap(), b"original");
    let pending = client
        .download(
            FILE,
            &output,
            None,
            Arc::new(AtomicU64::new(0)),
            Arc::new(AtomicU64::new(0)),
        )
        .await
        .unwrap();
    pending.commit(&output, true).unwrap();
    assert_eq!(std::fs::read(&output).unwrap(), b"hello");
    server.abort();
}
#[test]
fn default_service_is_discoverable_and_preserves_explicit_disable() {
    let mut config = AgentConfig::example();
    let service = config
        .services
        .iter_mut()
        .find(|s| s.name == "files")
        .unwrap();
    service.enabled = false;
    config.normalize();
    assert!(
        !config
            .services
            .iter()
            .find(|s| s.name == "files")
            .unwrap()
            .enabled
    );
    config.services.retain(|s| s.name != "files");
    assert!(config.normalize());
    let files = config
        .service_definitions()
        .into_iter()
        .find(|s| s.name == "files")
        .unwrap();
    assert_eq!(files.methods.len(), 4);
}

#[tokio::test]
async fn screenshot_uses_shared_upload_and_owner_completion() {
    let (owner, config, server) = fixture().await;
    let client = FileClient::from_config(&config).unwrap();
    let (file, hash) = client
        .upload(
            UploadSource::Bytes(b"png".to_vec()),
            "screen.png",
            "image/png",
            3,
            Purpose::Screenshot,
        )
        .await
        .unwrap();
    assert_eq!(file.file_id, FILE);
    assert_eq!(file.mime_type, "image/png");
    assert_eq!(owner.commits.load(Ordering::Relaxed), 1);
    assert_eq!(hash, format!("{:x}", Sha256::digest(b"png")));
    server.abort();
}

#[tokio::test]
async fn object_transfer_outlives_the_control_request_timeout() {
    let (owner, mut config, server) = fixture().await;
    config.upload.timeout_secs = 1;
    owner.delay.store(true, Ordering::Relaxed);
    owner.delay_ms.store(1400, Ordering::Relaxed);
    let client = FileClient::from_config(&config).unwrap();
    let before = Instant::now();
    let (file, _) = client
        .upload(
            UploadSource::Bytes(b"long".to_vec()),
            "file.bin",
            "application/octet-stream",
            4,
            Purpose::File,
        )
        .await
        .unwrap();
    assert_eq!(file.size_bytes, 4);
    assert!(before.elapsed() > Duration::from_secs(1));
    assert_eq!(owner.commits.load(Ordering::Relaxed), 1);
    server.abort();
}

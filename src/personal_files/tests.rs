use super::*;
use serde_json::json;
use std::sync::Mutex;

fn binding() -> PersonalFileClient {
    PersonalFileClient {
        base_url: "https://platform.example.test".into(),
        environment_key: Some("test-env".into()),
        workspace_id: Some(42),
        device_id: "device-a".into(),
        timeout_secs: 30,
    }
}

fn credential() -> serde_json::Value {
    json!({"environmentKey":"test-env", "clientId":"device-a", "workspaceIds":[42],
        "userId":8, "source":"bridge-agent", "subjectType":"user", "tokenType":"pat", "token":"lc_pat_test"})
}

fn document(entries: Vec<serde_json::Value>) -> serde_json::Value {
    json!({"schemaVersion":2, "environments":{"test-env":{"environmentKey":"test-env", "baseUrl":"https://platform.example.test"}},
        "credentials":entries})
}

fn read(
    document: serde_json::Value,
    binding: &PersonalFileClient,
) -> Result<credentials::DeviceCredential> {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("auth.json");
    std::fs::write(&path, document.to_string()).unwrap();
    credentials::read(&path, binding)
}

#[test]
fn device_binding_ignores_other_logins_devices_workspaces_and_environments() {
    let mut other = credential();
    other["clientId"] = json!("device-b");
    other["userId"] = json!(99);
    let mut foreign = credential();
    foreign["environmentKey"] = json!("foreign-env");
    let mut workspace = credential();
    workspace["workspaceIds"] = json!([43]);
    let login = json!({"token":"lc_pat_login", "workspaceIds":[42], "userId":99});
    let result = read(
        document(vec![other, foreign, workspace, login, credential()]),
        &binding(),
    )
    .unwrap();
    assert_eq!(result.user_id, 8);
    assert_eq!(result.token, "lc_pat_test");
}

#[test]
fn no_fallback_on_missing_ambiguous_or_changed_environment_binding() {
    assert!(read(document(vec![]), &binding()).is_err());
    assert!(read(document(vec![credential(), credential()]), &binding()).is_err());
    let mut changed = binding();
    changed.base_url = "https://attacker.example.test".into();
    assert!(read(document(vec![credential()]), &changed).is_err());
    changed = binding();
    changed.workspace_id = None;
    assert!(read(document(vec![credential()]), &changed).is_err());
    let mut invalid = credential();
    invalid["tokenType"] = json!("login");
    assert!(read(document(vec![invalid]), &binding()).is_err());
}

#[test]
fn owner_endpoints_do_not_use_relay_or_project_service() {
    let url = binding().endpoint("direct-uploads/prepare").unwrap();
    assert_eq!(url.as_str(), "https://platform.example.test/partner/v1/workspace-file-service/api/personal-files/direct-uploads/prepare");
    for raw in [
        "http://platform.example.test",
        "https://user:password@platform.example.test",
        "https://platform.example.test/#fragment",
    ] {
        let mut invalid = binding();
        invalid.base_url = raw.into();
        assert!(invalid.endpoint("direct-uploads/prepare").is_err());
    }
}

struct Transport {
    stages: Mutex<Vec<&'static str>>,
    fail: Option<&'static str>,
}
impl Transport {
    fn step(&self, stage: &'static str) -> Result<()> {
        self.stages.lock().unwrap().push(stage);
        anyhow::ensure!(self.fail != Some(stage), "injected {stage} failure");
        Ok(())
    }
}
fn ticket() -> UploadTicket {
    UploadTicket {
        upload_id: uuid::Uuid::from_u128(1).to_string(),
        file_id: uuid::Uuid::from_u128(2).to_string(),
        upload_url: "https://storage.example.test/staging".into(),
        upload_expires_at: u128::MAX,
        method: "PUT".into(),
        headers: BTreeMap::from([("content-type".into(), "image/png".into())]),
    }
}
impl UploadTransport for Transport {
    async fn prepare(&self, request: &PrepareRequest<'_>) -> Result<UploadTicket> {
        self.step("prepare")?;
        assert_eq!(request.size_bytes, 4);
        Ok(ticket())
    }
    async fn put(&self, _ticket: &UploadTicket, bytes: Vec<u8>) -> Result<()> {
        self.step("put")?;
        assert_eq!(bytes, b"shot");
        Ok(())
    }
    async fn complete(&self, upload_id: &str) -> Result<FileReference> {
        self.step("complete")?;
        assert_eq!(upload_id, ticket().upload_id);
        Ok(FileReference {
            file_id: ticket().file_id,
            path: "attachments/computer_screenshot/shot.png".into(),
            file_name: "shot.png".into(),
            mime_type: "image/png".into(),
            size_bytes: 4,
            source: "computer_screenshot".into(),
        })
    }
}

#[tokio::test]
async fn returns_reference_only_after_transfer_and_completion() {
    let request = PrepareRequest {
        file_name: "shot.png",
        mime_type: "image/png",
        source: "computer_screenshot",
        size_bytes: 4,
    };
    for (failure, stages) in [
        (None, vec!["prepare", "put", "complete"]),
        (Some("prepare"), vec!["prepare"]),
        (Some("put"), vec!["prepare", "put"]),
        (Some("complete"), vec!["prepare", "put", "complete"]),
    ] {
        let transport = Transport {
            stages: Mutex::new(vec![]),
            fail: failure,
        };
        let result = upload_file(&transport, &request, b"shot".to_vec()).await;
        assert_eq!(result.is_ok(), failure.is_none());
        assert_eq!(*transport.stages.lock().unwrap(), stages);
        if let Ok(file) = result {
            assert_eq!(file.file_id, ticket().file_id);
            let output = serde_json::to_string(&file).unwrap();
            assert!(!output.contains("downloadUrl"));
            assert!(!output.contains("storage.example.test"));
        }
    }
}

#[test]
fn rejects_expired_or_malformed_ticket() {
    let mut value = ticket();
    value.upload_expires_at = 0;
    assert!(validate_ticket(&value).is_err());
    value = ticket();
    value.file_id = "oss-object-path".into();
    assert!(validate_ticket(&value).is_err());
    value = ticket();
    value.method = "POST".into();
    assert!(validate_ticket(&value).is_err());
}

#[tokio::test]
async fn public_business_error_is_not_misreported_as_missing_upload_url() {
    let response = axum::http::Response::builder().status(200).body(
        r#"{"contractVersion":"1.0.0","errorCode":"WORKSPACE_FILE_SERVICE_FORBIDDEN","data":{"message":"personal space is frozen","retryable":false}}"#
    ).unwrap();
    let error = http::decode::<FileReference>(response.into(), "prepare")
        .await
        .unwrap_err();
    assert!(error
        .to_string()
        .contains("WORKSPACE_FILE_SERVICE_FORBIDDEN"));
    assert!(!error.to_string().contains("uploadUrl"));
}

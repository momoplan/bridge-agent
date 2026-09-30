use super::*;
use baijimu_cmodel_core::CModelResponse;
use local_app_contract::{ApplicationType, Artifact, ManifestDocument, SourceApp, VersionContent};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn frozen() -> FrozenVersion {
    FrozenVersion::new(
        SourceVersion {
            application: SourceApp {
                environment_key: "source-test".to_owned().try_into().unwrap(),
                app_id: "test-app".parse().unwrap(),
            },
            version: "1.0.0".parse().unwrap(),
        },
        VersionContent {
            application_type: ApplicationType::ManagedTool,
            manifest: ManifestDocument::parse(
                include_str!("../../tests/fixtures/market-managed-tool-3.0.0.json").to_owned(),
            )
            .unwrap(),
            source_revision: "v1.0.0".into(),
            artifacts: vec![Artifact {
                artifact_id: Uuid::from_u128(10),
                platform: "windows".into(),
                architecture: "x86_64".into(),
                file_name: "app.zip".to_owned().try_into().unwrap(),
                size_bytes: 3,
            }],
        },
    )
    .unwrap()
}

#[tokio::test]
async fn private_metadata_and_artifact_requests_bind_the_recipient_workspace() {
    let frozen = frozen();
    let metadata =
        serde_json::to_vec(&CModelResponse::success(Some(frozen.clone())).unwrap()).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/local-apps", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        for body in [metadata, b"zip".to_vec()] {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                request.push(stream.read_u8().await.unwrap());
            }
            let request = String::from_utf8(request).unwrap().to_ascii_lowercase();
            assert!(request.contains("\r\nx-workspace-id: 42\r\n"));
            assert!(request.contains("\r\nauthorization: bearer test-pat\r\n"));
            let headers = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream.write_all(headers.as_bytes()).await.unwrap();
            stream.write_all(&body).await.unwrap();
        }
    });
    // Only this local transport fixture permits HTTP. Production construction
    // validates HTTPS and disables redirects before accepting credentials.
    let reader = EnvironmentReader {
        environment: frozen.source.application.environment_key.clone(),
        endpoint: Url::parse(&endpoint).unwrap(),
        client: workspace_client(42, Duration::from_secs(5))
            .unwrap()
            .https_only(false)
            .build()
            .unwrap(),
    };
    let returned = reader.version("test-pat", &frozen.source).await.unwrap();
    let response = reader
        .artifact(
            "test-pat",
            &returned,
            frozen.content.artifacts[0].artifact_id,
        )
        .await
        .unwrap();
    assert_eq!(response.bytes().await.unwrap().as_ref(), b"zip");
    server.await.unwrap();
}

#[test]
fn private_reader_rejects_missing_workspace_before_network_access() {
    assert!(EnvironmentReader::new(
        "source-test".to_owned().try_into().unwrap(),
        "https://source.example.test/local-apps",
        0,
        Duration::from_secs(5),
    )
    .is_err());
}

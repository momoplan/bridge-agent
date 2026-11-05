use super::*;
use baijimu_cmodel_core::CModelResponse;
use local_app_contract::{
    ApplicationType, Artifact, FrozenVersion, ManifestDocument, MarketPage, SourceApp,
    SourceVersion, VersionContent,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn listing() -> MarketListing {
    MarketListing {
        contract_version: local_app_contract::MARKET_READ_CONTRACT_VERSION
            .parse()
            .unwrap(),
        listing_id: Uuid::from_u128(1),
        presentation: local_app_contract::ApplicationPresentation {
            name: "Test application".into(),
            description: "Workspace binding test".into(),
            icon: None,
        },
        frozen_version: FrozenVersion::new(
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
        .unwrap(),
    }
}

#[tokio::test]
async fn market_catalog_version_and_artifact_requests_bind_the_recipient_workspace() {
    let listing = listing();
    let page = MarketPage {
        items: vec![listing.clone()],
        next_cursor: None,
    };
    let page = serde_json::to_vec(&CModelResponse::success(Some(page)).unwrap()).unwrap();
    let version =
        serde_json::to_vec(&CModelResponse::success(Some(listing.clone())).unwrap()).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        // Artifact reads verify the exact version again before downloading.
        for (index, body) in [page, version.clone(), version, b"zip".to_vec()]
            .into_iter()
            .enumerate()
        {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                request.push(stream.read_u8().await.unwrap());
            }
            let request = String::from_utf8(request).unwrap().to_ascii_lowercase();
            assert!(request.contains("\r\nx-workspace-id: 73\r\n"));
            assert!(request.contains("workspaceid=73"));
            assert!(request.contains("\r\nauthorization: bearer test-pat\r\n"));
            assert!(request.contains("\r\nx-request-id:"));
            assert!(request.contains("\r\nx-trace-id:"));
            let content_type = if index == 3 {
                "application/octet-stream"
            } else {
                "application/json"
            };
            let headers = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream.write_all(headers.as_bytes()).await.unwrap();
            stream.write_all(&body).await.unwrap();
        }
    });
    let mut distribution =
        MarketDistribution::new("https://consumer.example.test/market/", 73).unwrap();
    distribution.base_url = Url::parse(&format!("http://{address}/market/")).unwrap();
    // The shared production builder binds credentials to the configured workspace;
    // only this local transport fixture permits plain HTTP.
    let reader = MarketReader {
        distribution,
        client: workspace_client(73, Duration::from_secs(5))
            .unwrap()
            .https_only(false)
            .build()
            .unwrap(),
    };
    let selection = InstallSource::Market {
        listing_id: listing.listing_id,
        version: listing.frozen_version.source.version.clone(),
        source: listing.frozen_version.source.clone(),
    };
    assert_eq!(
        reader.page("test-pat", None).await.unwrap().items,
        [listing.clone()]
    );
    assert_eq!(
        reader.version("test-pat", &selection).await.unwrap(),
        listing
    );
    let response = reader
        .artifact("test-pat", &selection, Uuid::from_u128(10))
        .await
        .unwrap();
    assert_eq!(response.bytes().await.unwrap().as_ref(), b"zip");
    server.await.unwrap();
}

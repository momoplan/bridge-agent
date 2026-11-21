use super::*;
use baijimu_cmodel_core::CModelResponse;
use local_app_contract::{
    ApplicationType, Artifact, FrozenVersion, ManifestDocument, MarketPage, SourceApp,
    SourceVersion, VersionContent,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
async fn market_update_lookup_uses_the_full_locator_and_rejects_another_bundle() {
    use local_app_contract::bundle::{LocatedAppVersion, LocatedFrozenVersion};

    let id = Uuid::from_u128(1);
    let locator: LocalAppLocator = "source-test/owner-bundle/LOCAL_APP/test-app"
        .parse()
        .unwrap();
    let legacy = listing_with_manifest(
        ApplicationType::Connector,
        include_str!("../../tests/fixtures/market-connector-3.0.0.json"),
    );
    let mut located = LocatedMarketListing {
        contract_version: "5.0.0".parse().unwrap(),
        listing_id: id,
        frozen_version: LocatedFrozenVersion::from_legacy(locator.clone(), legacy.frozen_version)
            .unwrap(),
        presentation: legacy.presentation,
    };
    let encode = |item: LocatedMarketListing| {
        serde_json::to_vec(&CModelResponse::success(Some(vec![item])).unwrap()).unwrap()
    };
    let valid = encode(located.clone());
    located.frozen_version.source.resource_locator = "source-test/other-bundle/LOCAL_APP/test-app"
        .parse()
        .unwrap();
    let wrong = encode(located);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        for (body, expected) in [
            (valid.clone(), format!("listingId={id}")),
            (valid, "resourceLocator=source-test%2Fowner-bundle%2FLOCAL_APP%2Ftest-app&platform=windows&architecture=x86_64".into()),
            (wrong, "resourceLocator=source-test%2Fowner-bundle%2FLOCAL_APP%2Ftest-app&platform=windows&architecture=x86_64".into()),
        ] {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                request.push(stream.read_u8().await.unwrap());
            }
            assert!(String::from_utf8(request).unwrap().contains(&format!("workspaceId=73&{expected}")));
            let header = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
            stream.write_all(header.as_bytes()).await.unwrap();
            stream.write_all(&body).await.unwrap();
        }
    });
    let mut distribution =
        MarketDistribution::new("https://consumer.example.test/market/", 73).unwrap();
    distribution.base_url = Url::parse(&format!("http://{address}/market/")).unwrap();
    let reader = MarketReader {
        distribution,
        client: workspace_client(73, Duration::from_secs(5))
            .unwrap()
            .https_only(false)
            .build()
            .unwrap(),
    };
    let binding = reader
        .located_by_listing("test-pat", id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(binding.frozen_version.source.resource_locator, locator);
    let current = LocatedAppVersion {
        resource_locator: locator.clone(),
        version: "0.9.0".parse().unwrap(),
    };
    let candidate = reader
        .located_for_target("test-pat", &locator, "windows", "x86_64")
        .await
        .unwrap()
        .unwrap();
    assert!(candidate.is_update_for(&current).unwrap());
    assert!(reader
        .located_for_target("test-pat", &locator, "windows", "x86_64")
        .await
        .is_err());
    server.await.unwrap();
}

fn listing() -> MarketListing {
    listing_with_manifest(
        ApplicationType::ManagedTool,
        include_str!("../../tests/fixtures/market-managed-tool-3.0.0.json"),
    )
}

fn listing_with_manifest(application_type: ApplicationType, manifest: &str) -> MarketListing {
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
                application_type,
                manifest: ManifestDocument::parse(manifest.to_owned()).unwrap(),
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
        reader
            .page("test-pat", None)
            .await
            .unwrap()
            .items
            .as_slice(),
        std::slice::from_ref(&listing)
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

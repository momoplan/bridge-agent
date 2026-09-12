use baijimu_cmodel_core::CModelResponse;
use bridge_agent::market_distribution::{resolve_exact, select_upgrade, MarketDistribution};
use local_app_contract::{
    Artifact, FrozenVersion, InstallSource, ManifestDocument, MarketListing, MarketPage, SourceApp,
    SourceVersion, VersionContent,
};
use uuid::Uuid;

fn listing(environment: &str, version: &str, id: u128) -> MarketListing {
    let manifest =
        include_str!("fixtures/market-managed-tool-3.0.0.json").replace("1.0.0", version);
    let source = SourceVersion {
        application: SourceApp {
            environment_key: environment.to_owned().try_into().unwrap(),
            app_id: "test-app".parse().unwrap(),
        },
        version: version.parse().unwrap(),
    };
    let content = VersionContent {
        application_type: local_app_contract::ApplicationType::ManagedTool,
        manifest: ManifestDocument::parse(manifest).unwrap(),
        source_revision: format!("v{version}"),
        artifacts: vec![Artifact {
            artifact_id: Uuid::from_u128(10),
            platform: "linux".into(),
            architecture: "x86_64".into(),
            file_name: "app.zip".to_owned().try_into().unwrap(),
            size_bytes: 256,
        }],
    };
    MarketListing {
        contract_version: local_app_contract::DISTRIBUTION_CONTRACT_VERSION
            .parse()
            .unwrap(),
        presentation: local_app_contract::ApplicationPresentation {
            name: "Example tool".into(),
            description: "Description".into(),
            publisher: None,
            capability: None,
            risk: None,
            icon: None,
        },
        market_key: "public-market".to_owned().try_into().unwrap(),
        listing_id: Uuid::from_u128(id),
        frozen_version: FrozenVersion::new(source, content).unwrap(),
    }
}

fn selection(item: &MarketListing) -> InstallSource {
    InstallSource::Market {
        market_key: item.market_key.clone(),
        listing_id: item.listing_id,
        version: item.frozen_version.source.version.clone(),
        source: item.frozen_version.source.clone(),
    }
}

fn market() -> MarketDistribution {
    MarketDistribution::new(
        "public-market".to_owned().try_into().unwrap(),
        "https://consumer.example.test/api/local-app-market/",
        42,
    )
    .unwrap()
}

#[test]
fn canonical_envelope_preserves_frozen_manifest_bytes() {
    let item = listing("environment-a", "1.0.0", 1);
    let raw = item.frozen_version.content.manifest.as_json().to_owned();
    let wire = serde_json::to_vec(&CModelResponse::success(Some(item.clone())).unwrap()).unwrap();
    let response: CModelResponse<MarketListing> = local_app_contract::decode(&wire).unwrap();
    assert!(response.is_success());
    let returned = response.into_data().unwrap();
    assert_eq!(returned.frozen_version.content.manifest.as_json(), raw);
    assert_eq!(resolve_exact(&selection(&item), &returned).unwrap(), &item);
}

#[test]
fn same_app_and_version_from_other_environment_cannot_resolve_or_upgrade() {
    let installed = listing("environment-a", "1.0.0", 1);
    let other = listing("environment-b", "1.0.0", 1);
    assert!(resolve_exact(&selection(&installed), &other).is_err());
    let newer = listing("environment-b", "1.0.1", 1);
    assert!(select_upgrade(Some(&selection(&installed)), &newer).is_err());
}

#[test]
fn market_and_listing_are_both_checked() {
    let installed = listing("environment-a", "1.0.0", 1);
    let mut other = listing("environment-a", "1.0.1", 2);
    assert!(select_upgrade(Some(&selection(&installed)), &other).is_err());
    other.listing_id = installed.listing_id;
    other.market_key = "other-market".to_owned().try_into().unwrap();
    assert!(select_upgrade(Some(&selection(&installed)), &other).is_err());
    assert!(market().version_url(&selection(&other)).is_err());
}

#[test]
fn upgrade_preserves_stable_listing_and_complete_source_identity() {
    let installed = listing("environment-a", "1.0.0", 1);
    let newer = listing("environment-a", "1.1.0", 1);
    assert_eq!(
        select_upgrade(Some(&selection(&installed)), &newer).unwrap(),
        Some(selection(&newer))
    );
    assert!(resolve_exact(&selection(&installed), &newer).is_err());
}

#[test]
fn missing_or_private_provenance_cannot_fall_back_to_market() {
    let item = listing("environment-a", "1.0.0", 1);
    assert!(select_upgrade(None, &item).is_err());
    let private = InstallSource::Environment {
        source: item.frozen_version.source.clone(),
    };
    assert!(select_upgrade(Some(&private), &item).is_err());
    assert!(resolve_exact(&private, &item).is_err());
    assert!(market().version_url(&private).is_err());
}

#[test]
fn build_metadata_is_exact_identity_but_not_upgrade_precedence() {
    let installed = listing("environment-a", "1.0.0+first", 1);
    let other = listing("environment-a", "1.0.0+second", 1);
    assert!(resolve_exact(&selection(&installed), &other).is_err());
    assert!(select_upgrade(Some(&selection(&installed)), &other)
        .unwrap()
        .is_none());
    let older = listing("environment-a", "0.9.0", 1);
    assert!(select_upgrade(Some(&selection(&installed)), &older)
        .unwrap()
        .is_none());
}

#[test]
fn inconsistent_requested_versions_are_rejected() {
    let item = listing("environment-a", "1.0.0", 1);
    let mut requested = selection(&item);
    if let InstallSource::Market { version, .. } = &mut requested {
        *version = "2.0.0".parse().unwrap();
    }
    assert!(market().version_url(&requested).is_err());
    assert!(resolve_exact(&requested, &item).is_err());
    assert!(select_upgrade(Some(&requested), &item).is_err());
}

#[test]
fn addresses_use_consumer_service_and_do_not_use_manifest_download_sources() {
    let item = listing("environment-a", "1.0.0+build.1", 1);
    let selected = selection(&item);
    let endpoint = market()
        .artifact_url(&selected, &item, Uuid::from_u128(10))
        .unwrap();
    assert_eq!(endpoint.host_str(), Some("consumer.example.test"));
    assert_eq!(endpoint.query(), Some("workspaceId=42"));
    assert_eq!(endpoint.path(), "/api/local-app-market/listings/00000000-0000-0000-0000-000000000001/versions/1.0.0+build.1/artifacts/00000000-0000-0000-0000-00000000000a");
    assert!(market()
        .artifact_url(&selected, &item, Uuid::from_u128(11))
        .is_err());
    assert!(market()
        .artifact_url(&selected, &item, Uuid::nil())
        .is_err());
}

#[test]
fn market_base_rejects_credential_and_ambiguous_targets() {
    for url in [
        "http://example.test",
        "https://user:password@example.test",
        "https://example.test?q=x",
        "https://example.test#x",
        " https://example.test",
        "/relative",
    ] {
        assert!(
            MarketDistribution::new("market".to_owned().try_into().unwrap(), url, 42).is_err(),
            "{url}"
        );
    }
}

#[test]
fn page_preserves_same_app_ids_across_different_sources() {
    let page = MarketPage {
        items: vec![
            listing("environment-a", "1.0.0", 1),
            listing("environment-b", "1.0.0", 2),
        ],
        next_cursor: Some(Uuid::from_u128(2)),
    };
    market().validate_page(None, &page).unwrap();
    assert_eq!(
        market().page_url(page.next_cursor).unwrap().query(),
        Some("workspaceId=42&after=00000000-0000-0000-0000-000000000002")
    );
    market()
        .validate_page(
            page.next_cursor,
            &MarketPage {
                items: vec![],
                next_cursor: None,
            },
        )
        .unwrap();
}

#[test]
fn invalid_pagination_cannot_loop_or_merge_source_applications() {
    let first = listing("environment-a", "1.0.0", 1);
    for page in [
        MarketPage {
            items: vec![first.clone(), first.clone()],
            next_cursor: Some(first.listing_id),
        },
        MarketPage {
            items: vec![first.clone(), listing("environment-a", "1.0.1", 2)],
            next_cursor: Some(Uuid::from_u128(2)),
        },
        MarketPage {
            items: vec![first.clone()],
            next_cursor: Some(Uuid::from_u128(2)),
        },
        MarketPage {
            items: vec![],
            next_cursor: Some(first.listing_id),
        },
    ] {
        assert!(market().validate_page(None, &page).is_err());
    }
    let page = MarketPage {
        items: vec![first],
        next_cursor: Some(Uuid::from_u128(1)),
    };
    assert!(market()
        .validate_page(Some(Uuid::from_u128(1)), &page)
        .is_err());
}

#[test]
fn catalog_rank_order_does_not_require_increasing_uuid_values() {
    let page = MarketPage {
        items: vec![
            listing("environment-a", "1.0.0", 90),
            listing("environment-b", "1.0.0", 5),
        ],
        next_cursor: Some(Uuid::from_u128(5)),
    };
    market()
        .validate_page(Some(Uuid::from_u128(80)), &page)
        .unwrap();
    let next = MarketPage {
        items: vec![listing("environment-c", "1.0.0", 3)],
        next_cursor: None,
    };
    market().validate_page(page.next_cursor, &next).unwrap();
    assert!(market()
        .validate_page(Some(Uuid::from_u128(5)), &page)
        .is_err());
}

#[test]
fn reviewed_presentation_and_distribution_contract_are_validated() {
    let item = listing("environment-a", "1.0.0", 1);
    let mut invalid = item.clone();
    invalid.contract_version = "1.0.0".parse().unwrap();
    assert!(resolve_exact(&selection(&item), &invalid).is_err());
    invalid = item.clone();
    invalid.presentation.name.clear();
    assert!(resolve_exact(&selection(&item), &invalid).is_err());
}

#[test]
fn wrong_contract_or_manifest_identity_fails_before_selection() {
    let expected = listing("environment-a", "1.0.0", 1);
    let mut returned = expected.clone();
    returned.frozen_version.contract_version = "2.0.0".parse().unwrap();
    assert_eq!(
        resolve_exact(&selection(&expected), &returned)
            .unwrap_err()
            .path,
        "contractVersion"
    );
    returned = expected.clone();
    returned.frozen_version.source.version = "1.0.1".parse().unwrap();
    assert_eq!(
        resolve_exact(&selection(&expected), &returned)
            .unwrap_err()
            .path,
        "manifest.version"
    );
}

#[test]
fn closed_contract_rejects_unknown_fields_and_invalid_semver_with_paths() {
    let item = listing("environment-a", "1.0.0", 1);
    let wire = serde_json::to_string(&item).unwrap();
    for invalid in [
        wire.replacen('{', "{\"unexpected\":true,", 1),
        wire.replace("\"1.0.0\"", "\"01.0.0\""),
    ] {
        let error = local_app_contract::decode::<MarketListing>(invalid.as_bytes()).unwrap_err();
        assert!(!error.path.is_empty());
    }
}

#[tokio::test]
async fn http_cmodel_roundtrip_keeps_manifest_bytes_and_owner_errors() {
    use axum::{routing::get, Router};
    let item = listing("environment-a", "1.0.0", 1);
    let expected = item.clone();
    let app = Router::new().route(
        "/listings",
        get(move || {
            let item = item.clone();
            async move { axum::Json(CModelResponse::success(Some(item)).unwrap()) }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let bytes = reqwest::get(format!("http://{addr}/listings"))
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    task.abort();
    let envelope: CModelResponse<MarketListing> = local_app_contract::decode(&bytes).unwrap();
    assert_eq!(envelope.into_data().unwrap(), expected);
    let failure: CModelResponse<MarketListing> = CModelResponse::failure(
        baijimu_cmodel_core::ErrorCode::parse("LOCAL_APP_MARKET_NOT_FOUND").unwrap(),
    )
    .unwrap();
    let wire = serde_json::to_vec(&failure).unwrap();
    let decoded: CModelResponse<MarketListing> = local_app_contract::decode(&wire).unwrap();
    assert!(!decoded.is_success());
    assert!(decoded.into_data().is_none());
}

#[test]
fn connector_golden_and_managed_tool_use_the_same_source_contract() {
    let mut item = listing("environment-a", "1.0.0", 1);
    item.frozen_version.content.application_type = local_app_contract::ApplicationType::Connector;
    item.frozen_version.content.manifest =
        ManifestDocument::parse(include_str!("fixtures/market-connector-3.0.0.json").to_owned())
            .unwrap();
    item.frozen_version.validate().unwrap();
    let requested = selection(&item);
    let wire = serde_json::to_vec(&CModelResponse::success(Some(item.clone())).unwrap()).unwrap();
    let envelope: CModelResponse<MarketListing> = local_app_contract::decode(&wire).unwrap();
    let decoded = envelope.into_data().unwrap();
    assert_eq!(resolve_exact(&requested, &decoded).unwrap(), &item);
}

#[test]
fn universal_package_is_supported_without_crossing_platforms() {
    use bridge_agent::market_distribution::select_artifact;
    let mut item = listing("environment-a", "1.0.0", 1);
    let universal = &mut item.frozen_version.content.artifacts[0];
    universal.platform = "macos".into();
    universal.architecture = "universal".into();
    assert_eq!(
        select_artifact(&item.frozen_version, "macos", "aarch64")
            .unwrap()
            .unwrap()
            .architecture,
        "universal"
    );
    assert!(select_artifact(&item.frozen_version, "windows", "x86_64")
        .unwrap()
        .is_none());
    let mut exact = item.frozen_version.content.artifacts[0].clone();
    exact.architecture = "aarch64".into();
    exact.artifact_id = Uuid::from_u128(11);
    item.frozen_version.content.artifacts.push(exact);
    assert_eq!(
        select_artifact(&item.frozen_version, "macos", "aarch64")
            .unwrap()
            .unwrap()
            .artifact_id,
        Uuid::from_u128(11)
    );
}

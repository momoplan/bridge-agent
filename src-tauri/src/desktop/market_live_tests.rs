use super::*;

/// Read-only acceptance using the same endpoint, authentication and DTOs as the desktop.
/// The release workflow explicitly runs this test and requires its completion receipt.
#[tokio::test]
#[ignore = "requires existing device authentication or the release market access secret"]
async fn current_device_market_reads_and_renders_reviewed_listings() {
    let consumer = live_consumer().await;
    let listings = consumer.listings().await.unwrap();
    assert!(
        !listings.is_empty(),
        "acceptance needs a published market listing"
    );
    let count = listings.len();
    for listing in listings {
        assert_eq!(listing.contract_version.to_string(), "4.0.0");
        let expected = listing.presentation.clone();
        let app = market_listing_presentation(listing.clone()).unwrap();
        assert_eq!(app.name, expected.name);
        assert_eq!(app.description, expected.description);
        let exact = consumer
            .reader
            .version(&consumer.credential, app.install_source.as_ref().unwrap())
            .await
            .unwrap();
        assert_eq!(exact, listing);
        println!(
            "Verified reviewed market item: {} {}",
            app.name, app.version
        );
    }
    println!("MARKET_READ_ACCEPTANCE {{\"contractVersion\":\"4.0.0\",\"listings\":{count},\"versions\":{count}}}");
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReleaseMarketAccess {
    base_url: String,
    environment_key: String,
    workspace_id: u64,
    token: String,
}

async fn live_consumer() -> market_consumer::MarketConsumer {
    if let Some(path) = std::env::var_os("BRIDGE_AGENT_LIVE_CONFIG") {
        return market_consumer::market_consumer(Path::new(&path))
            .await
            .unwrap();
    }
    let raw = std::env::var("BRIDGE_AGENT_MARKET_ACCESS_JSON")
        .expect("release market access configuration is required");
    let access: ReleaseMarketAccess = serde_json::from_str(&raw)
        .unwrap_or_else(|_| panic!("invalid release market access configuration"));
    assert!(!access.environment_key.trim().is_empty());
    let mut config = AgentConfig::example();
    config.platform.base_url = access.base_url.clone();
    config.platform.environment_key = Some(access.environment_key.clone());
    config.platform.workspace_id = Some(access.workspace_id);
    let document = serde_json::from_value(serde_json::json!({
        "environments": {access.environment_key.clone(): {
            "environmentKey": access.environment_key,
            "baseUrl": access.base_url
        }},
        "credentials": [{
            "environmentKey": config.platform.environment_key,
            "tokenType": "pat", "token": access.token,
            "workspaceIds": [access.workspace_id]
        }]
    }))
    .unwrap_or_else(|_| panic!("invalid release market authentication"));
    market_consumer::market_consumer_with_auth(&config, document)
        .await
        .unwrap()
}

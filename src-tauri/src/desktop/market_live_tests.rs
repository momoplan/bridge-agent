use super::*;

/// Opt-in read-only acceptance against the operator's explicitly selected device.
/// Credentials remain in the normal client authentication store.
#[tokio::test]
#[ignore = "requires BRIDGE_AGENT_LIVE_CONFIG and existing workspace authentication"]
async fn current_device_market_reads_and_renders_reviewed_listings() {
    let path =
        std::env::var_os("BRIDGE_AGENT_LIVE_CONFIG").expect("explicit device config path required");
    let consumer = market_consumer::market_consumer(Path::new(&path))
        .await
        .unwrap();
    let listings = consumer.listings().await.unwrap();
    assert!(
        !listings.is_empty(),
        "acceptance needs a published market listing"
    );
    for listing in listings {
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
}

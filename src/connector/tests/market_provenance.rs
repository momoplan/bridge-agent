fn fixture_market_provenance(environment: &str, listing_id: uuid::Uuid, version: &str) -> ConnectorInstallProvenance {
    let source: local_app_contract::SourceVersion = serde_json::from_value(json!({
        "application": {"environmentKey": environment, "appId": "test-app"}, "version": version
    })).unwrap();
    ConnectorInstallProvenance {
        install_source: Some(local_app_contract::InstallSource::Market {
            market_key: local_app_contract::MarketKey::try_from("test-market".to_owned()).unwrap(), listing_id, version: version.parse().unwrap(), source
        }),
        source_reference: None, review_status: "PUBLISHED".into(), source_checksum: None,
    }
}

#[test]
fn market_install_preserves_source_and_rejects_other_environment_before_mutation() {
    let directory = tempdir().unwrap();
    let _env = connector_test_env(directory.path().join("connectors"));
    let config_path = directory.path().join("config.json");
    save_config(&config_path, &AgentConfig::example()).unwrap();
    let source = directory.path().join("source");
    fs::create_dir_all(&source).unwrap();
    fs::write(source.join(CONNECTOR_MANIFEST_FILE), include_str!("../../../tests/fixtures/market-connector-3.0.0.json")).unwrap();
    let listing = uuid::Uuid::from_u128(1);
    let first = fixture_market_provenance("author-a", listing, "1.0.0");
    install_connector_from_path_with_provenance(&source, &config_path, false, first.clone()).unwrap();
    assert_eq!(show_connector("test-app").unwrap().install_source, first.install_source);
    let before = fs::read(&config_path).unwrap();
    let error = install_connector_from_path_with_provenance(&source, &config_path, true,
        fixture_market_provenance("author-b", uuid::Uuid::from_u128(2), "1.0.0")).unwrap_err();
    assert!(error.to_string().contains("different source applications"));
    assert_eq!(fs::read(&config_path).unwrap(), before);
    assert_eq!(show_connector("test-app").unwrap().install_source, first.install_source);
}

#[test]
fn historical_installation_does_not_acquire_a_market_source_from_matching_app_id() {
    let directory = tempdir().unwrap();
    let _env = connector_test_env(directory.path().join("connectors"));
    let config_path = directory.path().join("config.json");
    save_config(&config_path, &AgentConfig::example()).unwrap();
    let source = directory.path().join("source");
    fs::create_dir_all(&source).unwrap();
    fs::write(source.join(CONNECTOR_MANIFEST_FILE), include_str!("../../../tests/fixtures/market-connector-3.0.0.json")).unwrap();
    install_connector_from_path(&source, &config_path, false).unwrap();
    let error = install_connector_from_path_with_provenance(&source, &config_path, true,
        fixture_market_provenance("author-a", uuid::Uuid::from_u128(1), "1.0.0")).unwrap_err();
    assert!(error.to_string().contains("source migration is required"));
    assert!(show_connector("test-app").unwrap().install_source.is_none());
}

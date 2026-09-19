fn fixture_market_provenance(
    environment: &str,
    listing_id: uuid::Uuid,
    version: &str,
) -> ConnectorInstallProvenance {
    let source: local_app_contract::SourceVersion = serde_json::from_value(json!({
        "application": {"environmentKey": environment, "appId": "test-app"}, "version": version
    })).unwrap();
    ConnectorInstallProvenance {
        install_source: Some(local_app_contract::InstallSource::Market {
            market_key: local_app_contract::MarketKey::try_from("test-market".to_owned()).unwrap(), listing_id, version: version.parse().unwrap(), source
        }),
        source_identity_migration: false,
        source_reference: None, review_status: "PUBLISHED".into(), source_checksum: None,
    }
}

fn mark_fixture_connector_stopped(config_path: &Path) {
    let mut config = load_config(config_path).unwrap();
    let app = config
        .local_apps
        .iter_mut()
        .find(|app| app.app_id == "test-app")
        .unwrap();
    app.start_command = None;
    app.stop_command = None;
    save_config(config_path, &config).unwrap();
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
    assert!(error.to_string().contains("explicit source identity migration is required"));
    assert!(show_connector("test-app").unwrap().install_source.is_none());
}

#[test]
fn explicitly_authorized_historical_installation_adopts_source_application() {
    let directory = tempdir().unwrap();
    let _env = connector_test_env(directory.path().join("connectors"));
    let config_path = directory.path().join("config.json");
    save_config(&config_path, &AgentConfig::example()).unwrap();
    let source = directory.path().join("source");
    fs::create_dir_all(&source).unwrap();
    fs::write(
        source.join(CONNECTOR_MANIFEST_FILE),
        include_str!("../../../tests/fixtures/market-connector-3.0.0.json"),
    )
    .unwrap();
    install_connector_from_path(&source, &config_path, false).unwrap();
    mark_fixture_connector_stopped(&config_path);

    let mut provenance =
        fixture_market_provenance("author-a", uuid::Uuid::from_u128(1), "1.0.0");
    provenance.source_identity_migration = true;
    install_connector_from_path_with_provenance(
        &source,
        &config_path,
        true,
        provenance.clone(),
    )
    .unwrap();

    assert_eq!(
        show_connector("test-app").unwrap().install_source,
        provenance.install_source
    );
}

#[test]
fn distribution_route_can_change_within_the_same_source_application() {
    let directory = tempdir().unwrap();
    let _env = connector_test_env(directory.path().join("connectors"));
    let config_path = directory.path().join("config.json");
    save_config(&config_path, &AgentConfig::example()).unwrap();
    let source = directory.path().join("source");
    fs::create_dir_all(&source).unwrap();
    fs::write(
        source.join(CONNECTOR_MANIFEST_FILE),
        include_str!("../../../tests/fixtures/market-connector-3.0.0.json"),
    )
    .unwrap();
    install_connector_from_path_with_provenance(
        &source,
        &config_path,
        false,
        fixture_market_provenance("author-a", uuid::Uuid::from_u128(1), "1.0.0"),
    )
    .unwrap();
    mark_fixture_connector_stopped(&config_path);
    let replacement =
        fixture_market_provenance("author-a", uuid::Uuid::from_u128(2), "1.0.0");
    install_connector_from_path_with_provenance(
        &source,
        &config_path,
        true,
        replacement.clone(),
    )
    .unwrap();
    assert_eq!(
        show_connector("test-app").unwrap().install_source,
        replacement.install_source
    );
}

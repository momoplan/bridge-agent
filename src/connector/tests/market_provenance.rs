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
            listing_id, version: version.parse().unwrap(), source
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
fn installed_connector_with_retired_claims_keeps_its_source_and_can_be_replaced() {
    let directory = tempdir().unwrap();
    let _env = connector_test_env(directory.path().join("connectors"));
    let config_path = directory.path().join("config.json");
    save_config(&config_path, &AgentConfig::example()).unwrap();
    let source = directory.path().join("source");
    fs::create_dir_all(&source).unwrap();
    let manifest = source.join(CONNECTOR_MANIFEST_FILE);
    fs::write(&manifest, include_str!("../../../tests/fixtures/market-connector-3.0.0.json")).unwrap();
    let first = fixture_market_provenance("author-a", uuid::Uuid::from_u128(1), "1.0.0");
    install_connector_from_path_with_provenance(&source, &config_path, false, first.clone()).unwrap();
    mark_fixture_connector_stopped(&config_path);
    let path = install_record_path("test-app").unwrap();
    let mut legacy: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    legacy["manifest"]["publisher"] = json!({ "name": "Historical author claim" });
    legacy["manifest"]["remoteCapabilities"] = json!([]);
    let original = serde_json::to_vec(&legacy).unwrap();
    fs::write(&path, &original).unwrap();

    let installed = show_connector("test-app").unwrap();
    assert_eq!(installed.install_source, first.install_source);
    assert_eq!(list_connectors().unwrap()[0].version, "1.0.0");
    assert_eq!(fs::read(&path).unwrap(), original);
    let foreign = fixture_market_provenance("author-b", uuid::Uuid::from_u128(2), "1.0.0");
    let error = install_connector_from_path_with_provenance(&source, &config_path, true, foreign).unwrap_err();
    assert!(error.to_string().contains("different source applications"));
    assert_eq!(fs::read(&path).unwrap(), original);
    install_connector_from_path_with_provenance(&source, &config_path, true, first.clone()).unwrap();
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

/// Install a source-pinned fixture using the same disk records as the desktop installer.
pub(crate) fn install_bound_call_fixture(config: &mut AgentConfig, root: &Path, base_url: &str) {
    let config_path = root.join("config.json");
    config.local_apps.clear();
    save_config(&config_path, config).unwrap();
    let package = root.join("fixture-package");
    fs::create_dir(&package).unwrap();
    let mut manifest = test_manifest("test-app", "Fixture", "1.0.0");
    manifest["transport"]["baseUrl"] = json!(base_url);
    manifest["methods"][0]["path"] = json!("/invoke");
    manifest["methods"][0]["httpMethod"] = json!("POST");
    fs::write(package.join(CONNECTOR_MANIFEST_FILE), manifest.to_string()).unwrap();
    let source = fixture_market_provenance("test", uuid::Uuid::from_u128(1), "1.0.0");
    install_connector_from_path_with_provenance(&package, &config_path, false, source).unwrap();
    prepare_installed_connector_runtime(&config_path, "test-app").unwrap();
    *config = load_config(&config_path).unwrap();
}

#[test]
fn bound_registration_requires_the_installed_app_identity() {
    let dir = tempdir().unwrap();
    let _env = connector_test_env(dir.path().join("connectors"));
    let mut config = AgentConfig::example();
    install_bound_call_fixture(&mut config, dir.path(), "http://127.0.0.1:8080");
    let app = &config.local_apps[0];
    let mut record = load_install_record(&app.app_id).unwrap();
    let Some(local_app_contract::InstallSource::Market { source, .. }) = &mut record.install_source else {
        panic!("expected a market installation");
    };
    source.application.app_id = "different-app".to_owned().try_into().unwrap();
    assert!(check_registered_source(app, &record).is_err());
}

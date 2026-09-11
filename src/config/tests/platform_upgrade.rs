#[test]
fn upgrade_persists_api_root_without_changing_device_or_credentials() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("agent-config.json");
    let mut config = AgentConfig::example();
    config.platform.workspace_id = Some(77);
    config.platform.environment_key = Some("baijimu".into());
    config.relay.token = "test-upgrade-relay-credential".into();
    config.relay.token_issued_at_epoch_seconds = Some("123".into());
    config.relay.token_expires_at_epoch_seconds = Some("456".into());
    save_config(&path, &config).unwrap();
    let mut document: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    document["platform"]["base_url"] = json!("https://api.baijimu.com/lowcode3");
    fs::write(&path, serde_json::to_vec(&document).unwrap()).unwrap();
    let upgraded = load_config(&path).unwrap();
    assert_eq!(upgraded.platform.base_url, "https://api.baijimu.com");
    assert_eq!(upgraded.platform.workspace_id, config.platform.workspace_id);
    assert_eq!(upgraded.platform.environment_key, config.platform.environment_key);
    assert_eq!(serde_json::to_value(&upgraded.relay).unwrap(), serde_json::to_value(&config.relay).unwrap());
    assert_eq!(serde_json::to_value(&upgraded.device).unwrap(), serde_json::to_value(&config.device).unwrap());
    assert_eq!(serde_json::to_value(&upgraded.services).unwrap(), serde_json::to_value(&config.services).unwrap());
    let persisted = fs::read(&path).unwrap();
    assert!(!String::from_utf8_lossy(&persisted).contains("test-upgrade-relay-credential"));
    assert_eq!(serde_json::from_slice::<serde_json::Value>(&persisted).unwrap()["platform"]["base_url"], "https://api.baijimu.com");
    load_config(&path).unwrap();
    assert_eq!(fs::read(&path).unwrap(), persisted);
}

#[test]
fn private_environment_migration_preserves_origin_port_and_deployment_path() {
    use super::environment::api_base;
    for (old, expected) in [
        ("https://private.example.test:9443/team/lowcode3/", "https://private.example.test:9443/team"),
        ("https://private.example.test:9443/team", "https://private.example.test:9443/team"),
        ("https://api.baijimu.com:9443/lowcode3", "https://api.baijimu.com:9443"),
        ("https://unrelated.example.test/manager", "https://unrelated.example.test/manager"),
    ] {
        assert_eq!(api_base(old), expected);
        assert_eq!(api_base(expected), expected);
    }
}

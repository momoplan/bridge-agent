#[test]
fn test_store_round_trip_and_delete() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("agent-config.json");
    assert!(load_relay_token(&config_path).unwrap().is_none());
    store_relay_token(&config_path, "secret-token").unwrap();
    assert_eq!(
        load_relay_token(&config_path).unwrap().as_deref(),
        Some("secret-token")
    );
    delete_relay_token(&config_path).unwrap();
    assert!(load_relay_token(&config_path).unwrap().is_none());

    store_service_registration_token(&config_path, "register-token").unwrap();
    assert_eq!(
        load_service_registration_token(&config_path)
            .unwrap()
            .as_deref(),
        Some("register-token")
    );
    assert!(load_relay_token(&config_path).unwrap().is_none());
    delete_service_registration_token(&config_path).unwrap();
    assert!(load_service_registration_token(&config_path)
        .unwrap()
        .is_none());
}

#[test]
fn development_credentials_use_a_distinct_sibling_file() {
    let config_path = PathBuf::from("config").join("agent-config.json");
    assert_eq!(
        development_secret_path(&config_path, SecretSlot::Relay),
        PathBuf::from("config")
            .join(".bridge-agent-development")
            .join("agent-config.json.credentials")
    );
    assert_eq!(
        development_secret_path(&config_path, SecretSlot::ServiceRegistration),
        PathBuf::from("config")
            .join(".bridge-agent-development")
            .join("agent-config.json.service-registration")
    );
}

#[cfg(unix)]
#[test]
fn development_credentials_are_private_and_atomic() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let config_dir = dir.path().join("bridge-agent");
    let config_path = config_dir.join("agent-config.development.json");

    store_development_secret(&config_path, SecretSlot::Relay, "first-token").unwrap();
    store_development_secret(&config_path, SecretSlot::Relay, "second-token").unwrap();

    let secret_path = development_secret_path(&config_path, SecretSlot::Relay);
    let secret_dir = secret_path.parent().unwrap();
    assert_eq!(
        load_development_secret(&config_path, SecretSlot::Relay)
            .unwrap()
            .as_deref(),
        Some("second-token")
    );
    assert_eq!(
        std::fs::metadata(secret_dir).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        std::fs::metadata(&secret_path)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert_eq!(std::fs::read_dir(secret_dir).unwrap().count(), 1);

    delete_development_secret(&config_path, SecretSlot::Relay).unwrap();
    assert!(load_development_secret(&config_path, SecretSlot::Relay)
        .unwrap()
        .is_none());
}

#[cfg(unix)]
#[test]
fn credential_read_does_not_repair_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let scratch = tempfile::tempdir().unwrap();
    let config = scratch.path().join("config.json");
    store_development_secret(&config, SecretSlot::Relay, "test-value").unwrap();
    let secret = development_secret_path(&config, SecretSlot::Relay);
    std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(load_development_secret(&config, SecretSlot::Relay).is_err());
    assert_eq!(
        std::fs::metadata(&secret).unwrap().permissions().mode() & 0o777,
        0o644
    );
    super::prepare_relay_credentials(&config).unwrap();
    assert_eq!(
        load_development_secret(&config, SecretSlot::Relay)
            .unwrap()
            .as_deref(),
        Some("test-value")
    );
    assert_eq!(
        std::fs::metadata(&secret).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

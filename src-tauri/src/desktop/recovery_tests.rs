use super::*;

#[test]
fn render_failure_after_ready_is_persisted_as_an_incomplete_startup() {
    let directory = tempfile::tempdir().unwrap();
    let config = directory.path().join("agent-config.json");
    let diagnostics = StartupDiagnostics::for_config_path(&config);
    let health = StartupHealthManager::new(&config, diagnostics.clone());
    health.begin_primary(false, None);
    health.mark_frontend_ready().unwrap();
    health.mark_frontend_failed("render failed".into());
    assert!(!health.snapshot().frontend_ready);
    let persisted: PersistentStartupState =
        serde_json::from_slice(&fs::read(&health.state_path).unwrap()).unwrap();
    assert!(persisted.pending);
    let restarted = StartupHealthManager::new(&config, diagnostics);
    restarted.begin_primary(false, None);
    assert_eq!(restarted.snapshot().consecutive_failures, 1);
}

#[test]
fn recovery_health_does_not_require_parseable_business_config() {
    let directory = tempfile::tempdir().unwrap();
    let config = directory.path().join("agent-config.json");
    fs::write(&config, b"invalid business JSON").unwrap();
    let health = StartupHealthManager::new(&config, StartupDiagnostics::for_config_path(&config));
    health.begin_primary(false, None);
    health.mark_frontend_failed("render failed".into());
    assert!(!health.snapshot().frontend_ready);
    assert_eq!(fs::read(&config).unwrap(), b"invalid business JSON");
}

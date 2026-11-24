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

#[test]
fn unresponsive_page_after_ready_does_not_count_as_an_incomplete_startup() {
    let directory = tempfile::tempdir().unwrap();
    let config = directory.path().join("agent-config.json");
    let diagnostics = StartupDiagnostics::for_config_path(&config);
    let health = StartupHealthManager::new(&config, diagnostics.clone());
    health.begin_primary(false, None);
    health.mark_frontend_ready().unwrap();
    health.mark_frontend_unresponsive(FRONTEND_UNRESPONSIVE_DETAIL);
    assert_eq!(desktop_shell_status(&health), "degraded");
    let persisted: PersistentStartupState =
        serde_json::from_slice(&fs::read(&health.state_path).unwrap()).unwrap();
    assert!(!persisted.pending);
    let restarted = StartupHealthManager::new(&config, diagnostics);
    restarted.begin_primary(false, None);
    assert_eq!(restarted.snapshot().consecutive_failures, 0);
    assert!(!restarted.safe_mode());
}

#[test]
fn answering_again_clears_only_the_unresponsive_mark() {
    let directory = tempfile::tempdir().unwrap();
    let config = directory.path().join("agent-config.json");
    let health = StartupHealthManager::new(&config, StartupDiagnostics::for_config_path(&config));
    health.begin_primary(false, None);
    health.mark_frontend_ready().unwrap();
    health.mark_frontend_unresponsive(FRONTEND_UNRESPONSIVE_DETAIL);
    health.mark_frontend_responsive(FRONTEND_UNRESPONSIVE_DETAIL);
    assert_eq!(desktop_shell_status(&health), "ready");

    health.mark_frontend_failed("render failed".into());
    health.mark_frontend_responsive(FRONTEND_UNRESPONSIVE_DETAIL);
    assert_eq!(desktop_shell_status(&health), "degraded");
}

fn desktop_shell_status(health: &StartupHealthManager) -> String {
    let snapshot = health.snapshot();
    let shell = snapshot
        .components
        .iter()
        .find(|component| component.id == "desktop_shell");
    shell.unwrap().status.clone()
}

/// Runs the watchdog at its real interval; `heartbeat_at` gives the last
/// heartbeat as an offset from the start for each elapsed second.
fn watchdog_actions(
    seconds: u64,
    visible: impl Fn(u64) -> bool,
    heartbeat_at: impl Fn(u64, &[(u64, FrontendWatchdogAction)]) -> Option<u64>,
) -> Vec<(u64, FrontendWatchdogAction)> {
    let start = Instant::now();
    let mut watchdog = FrontendWatchdog::new(start);
    let mut actions = Vec::new();
    let step = FRONTEND_WATCHDOG_INTERVAL.as_secs();
    for elapsed in (step..=seconds).step_by(step as usize) {
        let heartbeat =
            heartbeat_at(elapsed, &actions).map(|offset| start + Duration::from_secs(offset));
        let now = start + Duration::from_secs(elapsed);
        match watchdog.observe(now, visible(elapsed), heartbeat) {
            FrontendWatchdogAction::Wait => {}
            action => actions.push((elapsed, action)),
        }
    }
    actions
}

#[test]
fn covered_window_that_answers_probes_never_alarms() {
    // Page timers are suspended, so the only heartbeats are answers to probes.
    let actions = watchdog_actions(
        600,
        |_| true,
        |_, actions| actions.last().map(|(asked, _)| asked + 1),
    );
    assert!(actions.len() > 10);
    assert!(actions
        .iter()
        .all(|(_, action)| *action == FrontendWatchdogAction::Probe));
}

#[test]
fn page_that_ignores_the_probe_alarms_once_and_recovers_once() {
    let actions = watchdog_actions(
        300,
        |_| true,
        |elapsed, _| (elapsed >= 200).then_some(elapsed - 1),
    );
    assert_eq!(
        actions,
        vec![
            (30, FrontendWatchdogAction::Probe),
            (40, FrontendWatchdogAction::Alarm),
            (200, FrontendWatchdogAction::Recovered),
        ]
    );
}

#[test]
fn hidden_window_is_not_probed_and_reopening_it_alarms_again() {
    let actions = watchdog_actions(300, |elapsed| !(100..200).contains(&elapsed), |_, _| None);
    assert_eq!(
        actions,
        vec![
            (30, FrontendWatchdogAction::Probe),
            (40, FrontendWatchdogAction::Alarm),
            (225, FrontendWatchdogAction::Probe),
            (235, FrontendWatchdogAction::Alarm),
        ]
    );
}

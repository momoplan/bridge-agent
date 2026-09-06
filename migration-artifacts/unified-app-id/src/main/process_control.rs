fn stop_legacy_app(discovery: &LocalControlDiscovery, legacy_identity: &str) -> Result<()> {
    let url = format!(
        "{}/local-apps/{}/stop",
        discovery.base_url.trim_end_matches('/'),
        legacy_identity
    );
    let response = reqwest::blocking::Client::new()
        .post(url)
        .bearer_auth(&discovery.token)
        .send()
        .with_context(|| format!("failed to stop legacy application {legacy_identity}"))?;
    if !response.status().is_success() {
        bail!(
            "legacy application {legacy_identity} stop request returned {}",
            response.status()
        );
    }
    Ok(())
}

fn stop_legacy_bridge(discovery: &LocalControlDiscovery) -> Result<()> {
    let _ = discovery.started_at_epoch_ms;
    let pid = Pid::from_u32(discovery.pid);
    let mut system = System::new_all();
    let Some(process) = system.process(pid) else {
        return Ok(());
    };
    let signalled = process.kill_with(Signal::Term).unwrap_or(false) || process.kill();
    if !signalled {
        bail!(
            "failed to terminate legacy Bridge Agent process {}",
            discovery.pid
        );
    }
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        thread::sleep(Duration::from_millis(200));
        system.refresh_all();
        if system.process(pid).is_none() {
            return Ok(());
        }
    }
    bail!(
        "legacy Bridge Agent process {} did not terminate within 15 seconds",
        discovery.pid
    )
}

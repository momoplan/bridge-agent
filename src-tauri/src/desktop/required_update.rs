//! Forced updates while the app runs. The startup gate only checks once, so a
//! long-running app re-checks hourly and reacts to Relay rejecting its version.
use super::*;
use std::sync::atomic::AtomicBool;

/// Tells the frontend to show the forced-update page with this status.
pub(super) const APP_UPDATE_REQUIRED_EVENT: &str = "app-update-required";

static REQUIRED_UPDATE_ENFORCED: AtomicBool = AtomicBool::new(false);

pub(super) fn start_required_update_watch(app: tauri::AppHandle, diagnostics: StartupDiagnostics) {
    start_periodic_update_recheck(app.clone(), diagnostics.clone());
    start_relay_rejection_watch(app, diagnostics);
}

fn start_periodic_update_recheck(app: tauri::AppHandle, diagnostics: StartupDiagnostics) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(REQUIRED_UPDATE_RECHECK_INTERVAL).await;
            match timeout(STARTUP_UPDATE_ATTEMPT_TIMEOUT, resolve_app_update_status()).await {
                Ok(Ok(status)) => {
                    let startup_health = app.state::<DesktopState>().startup_health.clone();
                    apply_updater_health_status(&startup_health, &status);
                    if startup_update_decision(&status) == StartupUpdateDecision::RequireUpdate {
                        enforce_required_update(&app, status, &diagnostics).await;
                        return;
                    }
                }
                Ok(Err(failure)) => {
                    diagnostics.warn(format!("periodic update check failed: {}", failure.detail))
                }
                Err(_) => diagnostics.warn("periodic update check timed out"),
            }
        }
    });
}

fn start_relay_rejection_watch(app: tauri::AppHandle, diagnostics: StartupDiagnostics) {
    let runtime = app.state::<DesktopState>().runtime.clone();
    let mut events = runtime.subscribe();
    tauri::async_runtime::spawn(async move {
        let mut snapshot = runtime.snapshot().await;
        while snapshot.status != RuntimeStatus::UpdateRequired {
            snapshot = match events.recv().await {
                Ok(RuntimeEvent::SnapshotChanged(snapshot)) => snapshot,
                Ok(RuntimeEvent::LogAppended(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    runtime.snapshot().await
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
            };
        }
        let relay_detail = snapshot
            .last_error
            .unwrap_or_else(|| "Relay 要求升级客户端".to_string());
        diagnostics.warn(format!(
            "relay rejected this client version; checking the release: {relay_detail}"
        ));
        let status = match timeout(STARTUP_UPDATE_CHECK_TIMEOUT, resolve_app_update_status()).await
        {
            Ok(Ok(status)) => relay_required_status(Some(status), relay_detail),
            Ok(Err(_)) | Err(_) => relay_required_status(None, relay_detail),
        };
        enforce_required_update(&app, status, &diagnostics).await;
    });
}

/// Relay's rejection is authoritative for the connection, so the page is shown
/// even when the update check is unreachable or has not caught up yet.
pub(super) fn relay_required_status(
    checked: Option<AppUpdateStatus>,
    relay_detail: String,
) -> AppUpdateStatus {
    let mut status = checked.unwrap_or_else(|| AppUpdateStatus {
        current_version: env!("CARGO_PKG_VERSION").to_string(),
        latest_version: None,
        update_available: true,
        force_update_required: true,
        minimum_supported_version: None,
        force_update_message: None,
        release_url: None,
        release_name: None,
        published_at: None,
        current_target: current_update_target(),
        auto_download_available: false,
        asset_name: None,
    });
    if !status.force_update_required {
        status.force_update_required = true;
        status.update_available = true;
        status.force_update_message = Some(relay_detail);
    } else if status.force_update_message.is_none() {
        status.force_update_message = Some(relay_detail);
    }
    status
}

/// Stops Relay, the local event service and managed connectors, then shows the
/// forced-update page. Runs at most once per process.
pub(super) async fn enforce_required_update(
    app: &tauri::AppHandle,
    status: AppUpdateStatus,
    diagnostics: &StartupDiagnostics,
) {
    if REQUIRED_UPDATE_ENFORCED.swap(true, Ordering::SeqCst) {
        return;
    }
    let (runtime, connector_processes, config_path, startup_health) = {
        let state = app.state::<DesktopState>();
        (
            state.runtime.clone(),
            state.connector_processes.clone(),
            state.config_path.clone(),
            state.startup_health.clone(),
        )
    };
    let detail = updater_required_detail(&status);
    diagnostics.warn(format!(
        "required update while running; stopping business components: {detail}"
    ));
    for failure in connector_processes.stop_all(&config_path).await {
        diagnostics.warn(format!(
            "failed to stop connector for required update: {failure}"
        ));
    }
    if let Err(err) = runtime.stop().await {
        diagnostics.warn(format!(
            "failed to stop runtime for required update: {err:#}"
        ));
    }
    startup_health.set_component("updater", "官方更新器", "degraded", Some(detail));
    mark_business_startup_skipped(&startup_health, "必须先完成客户端升级");
    if let Err(error) = app.emit(APP_UPDATE_REQUIRED_EVENT, &status) {
        diagnostics.warn(format!("failed to emit required update event: {error}"));
    }
    show_main_window(app, Some(diagnostics), MainWindowOpenReason::RequiredUpdate);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checked(force_update_required: bool, message: Option<&str>) -> AppUpdateStatus {
        AppUpdateStatus {
            current_version: "0.17.0".into(),
            latest_version: Some("0.17.3".into()),
            update_available: force_update_required,
            force_update_required,
            minimum_supported_version: Some("0.17.2".into()),
            force_update_message: message.map(str::to_string),
            release_url: Some("https://downloads.example/".into()),
            release_name: None,
            published_at: None,
            current_target: "darwin-aarch64".into(),
            auto_download_available: true,
            asset_name: Some("bridge-agent.app.tar.gz".into()),
        }
    }

    #[test]
    fn relay_rejection_forces_the_update_page_whatever_the_check_says() {
        let unreachable = relay_required_status(None, "Relay 要求升级".into());
        assert!(unreachable.force_update_required && unreachable.update_available);
        assert!(!unreachable.auto_download_available);
        assert_eq!(
            unreachable.force_update_message.as_deref(),
            Some("Relay 要求升级")
        );

        let lagging = relay_required_status(Some(checked(false, None)), "Relay 要求升级".into());
        assert!(lagging.force_update_required && lagging.update_available);
        assert!(
            lagging.auto_download_available,
            "the release can still be installed"
        );
        assert_eq!(
            lagging.force_update_message.as_deref(),
            Some("Relay 要求升级")
        );

        let agreed = relay_required_status(Some(checked(true, Some("官方提示"))), "Relay".into());
        assert_eq!(agreed.force_update_message.as_deref(), Some("官方提示"));
        assert_eq!(agreed.latest_version.as_deref(), Some("0.17.3"));
    }
}

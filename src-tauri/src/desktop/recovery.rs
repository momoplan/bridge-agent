use super::*;
use tauri_plugin_dialog::{DialogExt as _, MessageDialogButtons, MessageDialogKind};

#[derive(Default)]
pub(super) struct RecoveryState {
    pub(super) heartbeat: Mutex<Option<Instant>>,
    pub(super) dialog_open: AtomicBool,
    pub(super) installing: AtomicBool,
}

pub(super) struct UpdateInstallationGuard<'a>(&'a AtomicBool);
impl<'a> UpdateInstallationGuard<'a> {
    pub(super) fn acquire(flag: &'a AtomicBool) -> Result<Self, String> {
        flag.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .map(|_| Self(flag))
            .map_err(|_| "官方更新正在进行，请等待完成".to_string())
    }
}
impl Drop for UpdateInstallationGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

#[tauri::command]
pub(super) fn frontend_heartbeat(state: tauri::State<'_, RecoveryState>) {
    *state
        .heartbeat
        .lock()
        .unwrap_or_else(|error| error.into_inner()) = Some(Instant::now());
}

#[tauri::command]
pub(super) fn report_frontend_failure(state: tauri::State<'_, DesktopState>, detail: String) {
    let detail: String = detail.chars().take(4000).collect();
    state.startup_health.mark_frontend_failed(detail);
}

#[tauri::command]
pub(super) fn open_native_recovery(app: tauri::AppHandle) {
    show_native_recovery(
        &app,
        "可独立检查并安装官方签名更新。更新会保留本机配置和应用数据。",
    );
}

pub(super) fn show_native_recovery(app: &tauri::AppHandle, detail: &str) {
    let state = app.state::<RecoveryState>();
    if state.dialog_open.swap(true, Ordering::SeqCst) {
        return;
    }
    let handle = app.clone();
    app.dialog()
        .message(detail)
        .title("百积木 · 更新与修复")
        .kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::OkCancelCustom(
            "检查官方更新".into(),
            "关闭".into(),
        ))
        .show(move |check| {
            handle
                .state::<RecoveryState>()
                .dialog_open
                .store(false, Ordering::SeqCst);
            if check {
                check_native_update(handle);
            }
        });
}

fn check_native_update(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        match resolve_app_update_status().await {
            Ok(status) if status.update_available && status.auto_download_available => {
                let version = status.latest_version.as_deref().unwrap_or("最新版本");
                let handle = app.clone();
                app.dialog()
                    .message(format!(
                        "发现官方更新 {version}。安装完成后客户端将重启。是否开始下载并安装？"
                    ))
                    .title("安装官方更新")
                    .buttons(MessageDialogButtons::OkCancelCustom(
                        "下载并安装".into(),
                        "取消".into(),
                    ))
                    .show(move |install| {
                        if install {
                            install_native_update(handle);
                        }
                    });
            }
            Ok(status) => {
                let detail = if status.update_available {
                    "当前平台需要使用官方安装包更新。".to_string()
                } else {
                    format!(
                        "当前版本 {}，暂无新版本。可重试界面，或通过系统托盘打开启动日志。",
                        status.current_version
                    )
                };
                app.dialog()
                    .message(detail)
                    .title("百积木 · 更新检查")
                    .show(|_| {});
            }
            Err(error) => {
                app.dialog()
                    .message(format!(
                        "{}\n\n请检查网络后，从系统托盘再次选择“检查更新与修复”。",
                        error.detail
                    ))
                    .title("更新检查失败")
                    .kind(MessageDialogKind::Error)
                    .show(|_| {});
            }
        }
    });
}

fn install_native_update(app: tauri::AppHandle) {
    // Blocking dialogs run off the UI thread. The updater is the same signed installer used by IPC.
    tauri::async_runtime::spawn(async move {
        match install_app_update(app.clone(), app.state::<DesktopState>()).await {
            Ok(result) if result.status == "up_to_date" => {
                app.dialog()
                    .message("当前没有可安装的更新。")
                    .title("百积木 · 更新检查")
                    .show(|_| {});
            }
            Ok(_) => {}
            Err(error) => {
                app.dialog()
                    .message(error)
                    .title("官方更新未完成")
                    .kind(MessageDialogKind::Error)
                    .show(|_| {});
            }
        }
    });
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FrontendWatchdogAction {
    Wait,
    Probe,
    Alarm,
    Recovered,
}

/// Decides when the main page counts as unresponsive. The system suspends page
/// timers while the window is covered or the display sleeps, so a stale
/// heartbeat only makes the shell ask; the alarm needs that question to go
/// unanswered.
pub(super) struct FrontendWatchdog {
    visible_since: Instant,
    probed_at: Option<Instant>,
    notified: bool,
    degraded: bool,
}

impl FrontendWatchdog {
    pub(super) fn new(now: Instant) -> Self {
        Self {
            visible_since: now,
            probed_at: None,
            notified: false,
            degraded: false,
        }
    }

    pub(super) fn observe(
        &mut self,
        now: Instant,
        visible: bool,
        heartbeat: Option<Instant>,
    ) -> FrontendWatchdogAction {
        if !visible {
            self.visible_since = now;
            self.probed_at = None;
            self.notified = false;
        }
        let fresh = heartbeat.is_some_and(|last| {
            now.saturating_duration_since(last) < FRONTEND_HEARTBEAT_STALE_AFTER
        });
        if fresh {
            self.probed_at = None;
            self.notified = false;
            return if std::mem::take(&mut self.degraded) {
                FrontendWatchdogAction::Recovered
            } else {
                FrontendWatchdogAction::Wait
            };
        }
        let settled =
            now.saturating_duration_since(self.visible_since) >= FRONTEND_HEARTBEAT_STALE_AFTER;
        if !visible || self.notified || !settled {
            return FrontendWatchdogAction::Wait;
        }
        match self.probed_at {
            None => {
                self.probed_at = Some(now);
                FrontendWatchdogAction::Probe
            }
            Some(asked) if now.saturating_duration_since(asked) >= FRONTEND_PROBE_TIMEOUT => {
                self.probed_at = None;
                self.notified = true;
                self.degraded = true;
                FrontendWatchdogAction::Alarm
            }
            Some(_) => FrontendWatchdogAction::Wait,
        }
    }
}

pub(super) fn start_frontend_watchdog(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut watchdog = FrontendWatchdog::new(Instant::now());
        loop {
            tokio::time::sleep(FRONTEND_WATCHDOG_INTERVAL).await;
            if app_is_quitting(&app) {
                return;
            }
            let visible = app
                .get_webview_window("main")
                .is_some_and(|window| window.is_visible().unwrap_or(false));
            let heartbeat = *app
                .state::<RecoveryState>()
                .heartbeat
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            apply_frontend_watchdog_action(
                &app,
                watchdog.observe(Instant::now(), visible, heartbeat),
            );
        }
    });
}

fn apply_frontend_watchdog_action(app: &tauri::AppHandle, action: FrontendWatchdogAction) {
    let startup_health = app.state::<DesktopState>().startup_health.clone();
    match action {
        FrontendWatchdogAction::Wait => {}
        FrontendWatchdogAction::Probe => {
            if let Err(error) = app.emit(FRONTEND_LIVENESS_PROBE_EVENT, ()) {
                startup_health
                    .diagnostics
                    .warn(format!("failed to emit frontend liveness probe: {error}"));
            }
        }
        FrontendWatchdogAction::Alarm => {
            startup_health.mark_frontend_unresponsive(FRONTEND_UNRESPONSIVE_DETAIL);
            show_native_recovery(app, FRONTEND_UNRESPONSIVE_DETAIL);
        }
        FrontendWatchdogAction::Recovered => {
            startup_health.mark_frontend_responsive(FRONTEND_UNRESPONSIVE_DETAIL);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn concurrent_installation_is_rejected_and_failures_release_guard() {
        let flag = AtomicBool::new(false);
        let guard = UpdateInstallationGuard::acquire(&flag).unwrap();
        assert!(UpdateInstallationGuard::acquire(&flag).is_err());
        drop(guard);
        assert!(UpdateInstallationGuard::acquire(&flag).is_ok());
    }
}

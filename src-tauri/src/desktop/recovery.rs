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

pub(super) fn start_frontend_watchdog(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut visible_since = Instant::now();
        let mut notified = false;
        loop {
            tokio::time::sleep(Duration::from_secs(5)).await;
            if app_is_quitting(&app) {
                return;
            }
            let visible = app
                .get_webview_window("main")
                .is_some_and(|window| window.is_visible().unwrap_or(false));
            if !visible {
                visible_since = Instant::now();
                notified = false;
                continue;
            }
            let heartbeat = *app
                .state::<RecoveryState>()
                .heartbeat
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let healthy = heartbeat.is_some_and(|last| last.elapsed() < Duration::from_secs(30));
            if healthy {
                notified = false;
                continue;
            }
            if !notified && visible_since.elapsed() >= Duration::from_secs(30) {
                notified = true;
                let detail = "客户端界面长时间未响应。您仍可通过系统窗口检查并安装官方更新，也可从系统托盘打开启动日志。";
                app.state::<DesktopState>()
                    .startup_health
                    .mark_frontend_failed(detail.into());
                show_native_recovery(&app, detail);
            }
        }
    });
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

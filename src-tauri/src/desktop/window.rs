use super::*;

pub(super) fn configure_desktop_command(command: &mut Command) {
    #[cfg(windows)]
    command.creation_flags(WINDOWS_CREATE_NO_WINDOW);
    #[cfg(not(windows))]
    let _ = command;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DesktopLaunchMode {
    Interactive,
    BackgroundAutostart,
}

impl DesktopLaunchMode {
    pub(super) fn from_args<I, S>(args: I, interactive_restart_requested: bool) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        if !interactive_restart_requested
            && args
                .into_iter()
                .any(|arg| arg.as_ref() == OsStr::new(AUTOSTART_BACKGROUND_ARG))
        {
            Self::BackgroundAutostart
        } else {
            Self::Interactive
        }
    }

    pub(super) fn should_show_main_window(self) -> bool {
        matches!(self, Self::Interactive)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MainWindowOpenReason {
    InteractiveStartup,
    RequiredUpdate,
    SecondaryLaunch,
    TrayMenu,
    TrayIcon,
    #[cfg(target_os = "macos")]
    MacosReopen,
}

impl MainWindowOpenReason {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::InteractiveStartup => "interactive_startup",
            Self::RequiredUpdate => "required_update",
            Self::SecondaryLaunch => "secondary_launch",
            Self::TrayMenu => "tray_menu",
            Self::TrayIcon => "tray_icon",
            #[cfg(target_os = "macos")]
            Self::MacosReopen => "macos_reopen",
        }
    }
}

pub(super) fn desktop_window_state_flags() -> StateFlags {
    StateFlags::SIZE
        | StateFlags::POSITION
        | StateFlags::MAXIMIZED
        | StateFlags::DECORATIONS
        | StateFlags::FULLSCREEN
}

#[cfg(any(target_os = "macos", test))]
pub(super) fn should_restore_main_window_on_macos_reopen(has_visible_windows: bool) -> bool {
    !has_visible_windows
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DesktopAutostartPolicy {
    SkipDevelopmentBuild,
    EnableForDesktop,
}

pub(super) fn desktop_autostart_policy(debug_assertions: bool) -> DesktopAutostartPolicy {
    if debug_assertions {
        DesktopAutostartPolicy::SkipDevelopmentBuild
    } else {
        DesktopAutostartPolicy::EnableForDesktop
    }
}

pub(super) fn configure_desktop_autostart(
    app: &tauri::App,
    startup_health: &StartupHealthManager,
    diagnostics: &StartupDiagnostics,
) {
    match desktop_autostart_policy(cfg!(debug_assertions)) {
        DesktopAutostartPolicy::SkipDevelopmentBuild => startup_health.set_component(
            "autostart",
            "登录启动",
            "skipped",
            Some("开发构建不注册系统登录项".to_string()),
        ),
        DesktopAutostartPolicy::EnableForDesktop => {
            let was_enabled = match app.autolaunch().is_enabled() {
                Ok(enabled) => Some(enabled),
                Err(err) => {
                    diagnostics.warn(format!(
                        "failed to inspect autostart before refreshing registration: {err:#}"
                    ));
                    None
                }
            };
            match app.autolaunch().enable() {
                Ok(()) => {
                    diagnostics.info(format!(
                        "official autostart integration {} with background launch argument",
                        if was_enabled == Some(true) {
                            "refreshed"
                        } else {
                            "enabled"
                        }
                    ));
                    startup_health.set_component("autostart", "登录启动", "ready", None);
                }
                Err(err) => {
                    diagnostics.error(format!("failed to register autostart: {err:#}"));
                    startup_health.set_component(
                        "autostart",
                        "登录启动",
                        "degraded",
                        Some(err.to_string()),
                    );
                }
            }
        }
    }
}

#[cfg(target_os = "macos")]
pub(super) fn prompt_accessibility_permission() {
    let key = CFString::new("AXTrustedCheckOptionPrompt");
    let value = CFBoolean::true_value();
    let options = CFDictionary::from_CFType_pairs(&[(key.as_CFType(), value.as_CFType())]);
    let _ = unsafe { AXIsProcessTrustedWithOptions(options.as_concrete_TypeRef().cast()) };
}

pub(super) fn setup_tray(app: &tauri::App, diagnostics: &StartupDiagnostics) -> tauri::Result<()> {
    diagnostics.info("setting up tray icon");
    let show = MenuItem::with_id(app, TRAY_MENU_SHOW, "打开百积木", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, TRAY_MENU_QUIT, "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;
    let icon = app.default_window_icon().cloned();
    let menu_diagnostics = diagnostics.clone();
    let tray_diagnostics = diagnostics.clone();

    let mut tray = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("百积木")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(move |app, event| match event.id().as_ref() {
            TRAY_MENU_SHOW => {
                show_main_window(app, Some(&menu_diagnostics), MainWindowOpenReason::TrayMenu)
            }
            TRAY_MENU_QUIT => quit_app(app),
            _ => {}
        })
        .on_tray_icon_event(move |tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(
                    tray.app_handle(),
                    Some(&tray_diagnostics),
                    MainWindowOpenReason::TrayIcon,
                );
            }
        });

    if let Some(icon) = icon {
        tray = tray.icon(icon);
    } else {
        diagnostics.warn("default window icon is unavailable; building tray without an icon");
    }

    tray.build(app)?;
    diagnostics.info("tray icon setup completed");
    Ok(())
}

#[cfg(target_os = "windows")]
pub(super) fn setup_main_window_icon(app: &tauri::App) -> anyhow::Result<()> {
    let icon = app
        .default_window_icon()
        .cloned()
        .context("default bundled window icon is unavailable")?;
    let window = app
        .get_webview_window("main")
        .context("main webview window is unavailable")?;
    window
        .set_icon(icon)
        .context("failed to bind the bundled icon to the main Windows window")
}

#[cfg(not(target_os = "windows"))]
pub(super) fn setup_main_window_icon(_app: &tauri::App) -> anyhow::Result<()> {
    Ok(())
}

pub(super) fn show_main_window(
    app: &tauri::AppHandle,
    diagnostics: Option<&StartupDiagnostics>,
    reason: MainWindowOpenReason,
) {
    if app_is_quitting(app) {
        if let Some(diagnostics) = diagnostics {
            diagnostics.info(format!(
                "skipping main window open because app is quitting: reason={}",
                reason.as_str()
            ));
        }
        return;
    }
    if let Some(diagnostics) = diagnostics {
        diagnostics.info(format!(
            "main window open requested: reason={}",
            reason.as_str()
        ));
    }
    let Some(window) = app.get_webview_window("main") else {
        if let Some(diagnostics) = diagnostics {
            diagnostics.warn(format!(
                "main window is unavailable during open request: reason={}",
                reason.as_str()
            ));
        }
        return;
    };
    normalize_main_window_layout(&window, diagnostics, WindowLayoutPolicy::Full);
    if window.is_visible().unwrap_or(false) && window.is_focused().unwrap_or(false) {
        if let Some(diagnostics) = diagnostics {
            diagnostics.info(format!(
                "main window open skipped because it is already visible and focused: reason={}",
                reason.as_str()
            ));
        }
        return;
    }
    show_dock_icon(app, diagnostics);
    restore_main_window(&window, diagnostics);
    if let Some(diagnostics) = diagnostics {
        diagnostics.info(format!(
            "main window open completed: reason={} visible={} focused={}",
            reason.as_str(),
            window.is_visible().unwrap_or(false),
            window.is_focused().unwrap_or(false)
        ));
    }
}

pub(super) fn normalize_main_window_layout(
    window: &tauri::WebviewWindow,
    diagnostics: Option<&StartupDiagnostics>,
    policy: WindowLayoutPolicy,
) {
    match fit_main_window_to_work_area(window, policy) {
        Ok(outcome @ WindowLayoutOutcome::Applied { .. }) => {
            if let Some(diagnostics) = diagnostics {
                diagnostics.info(format!("main window work-area normalization {outcome}"));
            }
        }
        Ok(_) => {}
        Err(err) => {
            if let Some(diagnostics) = diagnostics {
                diagnostics.warn(format!(
                    "failed to normalize main window to the monitor work area: {err:#}"
                ));
            }
        }
    }
}

pub(super) fn app_is_quitting(app: &tauri::AppHandle) -> bool {
    app.try_state::<DesktopState>()
        .is_some_and(|state| state.quitting.load(Ordering::SeqCst))
}

pub(super) fn restore_main_window(
    window: &tauri::WebviewWindow,
    diagnostics: Option<&StartupDiagnostics>,
) {
    run_window_action(
        diagnostics,
        "show main window",
        "main window show completed",
        || window.show(),
    );
    run_window_action(
        diagnostics,
        "unminimize main window",
        "main window unminimize completed",
        || window.unminimize(),
    );
    run_window_action(
        diagnostics,
        "focus main window",
        "main window focus completed",
        || window.set_focus(),
    );
    if window.is_visible().unwrap_or(false) {
        if let Some(state) = window.app_handle().try_state::<DesktopState>() {
            state.main_window_visible.store(true, Ordering::SeqCst);
            state.runtime_log_streaming.store(
                state.runtime_log_streaming_requested.load(Ordering::SeqCst),
                Ordering::SeqCst,
            );
        }
        let _ = window.app_handle().emit(MAIN_WINDOW_VISIBILITY_EVENT, true);
    }
}

pub(super) fn run_window_action(
    diagnostics: Option<&StartupDiagnostics>,
    label: &str,
    success_message: &str,
    action: impl FnOnce() -> tauri::Result<()>,
) {
    match panic::catch_unwind(AssertUnwindSafe(action)) {
        Ok(Ok(())) => {
            if let Some(diagnostics) = diagnostics {
                diagnostics.info(success_message);
            }
        }
        Ok(Err(err)) => {
            if let Some(diagnostics) = diagnostics {
                diagnostics.error(format!("failed to {label}: {err:#}"));
            }
            eprintln!("failed to {label}: {err}");
        }
        Err(_) => {
            if let Some(diagnostics) = diagnostics {
                diagnostics.error(format!("{label} panicked; skipping"));
            }
            eprintln!("{label} panicked; skipping");
        }
    }
}

pub(super) fn hide_to_tray(window: &tauri::Window) {
    if let Some(state) = window.app_handle().try_state::<DesktopState>() {
        state.main_window_visible.store(false, Ordering::SeqCst);
        state.runtime_log_streaming.store(false, Ordering::SeqCst);
    }
    let _ = window
        .app_handle()
        .emit(MAIN_WINDOW_VISIBILITY_EVENT, false);
    if let Err(err) = window.hide() {
        eprintln!("failed to hide main window: {err}");
    }
    hide_dock_icon(window.app_handle());
}

pub(super) fn prepare_background_startup(app: &tauri::AppHandle, diagnostics: &StartupDiagnostics) {
    if let Some(state) = app.try_state::<DesktopState>() {
        state.main_window_visible.store(false, Ordering::SeqCst);
        state.runtime_log_streaming.store(false, Ordering::SeqCst);
    }
    if let Some(window) = app.get_webview_window("main") {
        run_window_action(
            Some(diagnostics),
            "hide main window for background autostart",
            "background autostart main window hide completed",
            || window.hide(),
        );
    } else {
        diagnostics.warn("main window is unavailable while preparing background autostart");
    }
    hide_dock_icon(app);
    let _ = app.emit(MAIN_WINDOW_VISIBILITY_EVENT, false);
    diagnostics.info("background autostart prepared without showing or focusing the main window");
}

#[cfg(target_os = "macos")]
pub(super) fn show_dock_icon(app: &tauri::AppHandle, diagnostics: Option<&StartupDiagnostics>) {
    if let Err(err) = app.set_dock_visibility(true) {
        if let Some(diagnostics) = diagnostics {
            diagnostics.error(format!("failed to show dock icon: {err:#}"));
        }
        eprintln!("failed to show dock icon: {err}");
    } else if let Some(diagnostics) = diagnostics {
        diagnostics.info("dock icon show completed");
    }
}

#[cfg(not(target_os = "macos"))]
pub(super) fn show_dock_icon(_app: &tauri::AppHandle, _diagnostics: Option<&StartupDiagnostics>) {}

#[cfg(target_os = "macos")]
pub(super) fn hide_dock_icon(app: &tauri::AppHandle) {
    if let Err(err) = app.set_dock_visibility(false) {
        eprintln!("failed to hide dock icon: {err}");
    }
}

#[cfg(not(target_os = "macos"))]
pub(super) fn hide_dock_icon(_app: &tauri::AppHandle) {}

pub(super) fn quit_app(app: &tauri::AppHandle) {
    let state = app.state::<DesktopState>();
    if state.quitting.swap(true, Ordering::SeqCst) {
        eprintln!("quit requested while app is already quitting");
        return;
    }
    let runtime = state.runtime.clone();
    let connector_processes = state.connector_processes.clone();
    let config_path = state.config_path.clone();
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        for failure in connector_processes.stop_all(&config_path).await {
            eprintln!("failed to stop managed connector before exit: {failure}");
        }
        if let Err(err) = runtime.stop().await {
            eprintln!("failed to stop runtime before exit: {err:#}");
        }
        app.exit(0);
    });
}

pub(super) fn quit_running_instance_requested(args: &[String]) -> bool {
    args.iter().any(|arg| arg == QUIT_RUNNING_INSTANCE_ARG)
}

pub(super) fn background_autostart_requested<I, S>(args: I) -> bool
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    args.into_iter()
        .any(|arg| arg.as_ref() == OsStr::new(AUTOSTART_BACKGROUND_ARG))
}

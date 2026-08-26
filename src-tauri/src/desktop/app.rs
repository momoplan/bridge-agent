use super::*;

struct DesktopLaunch {
    diagnostics: StartupDiagnostics,
    config_path: PathBuf,
    config_path_failure: Option<String>,
    crypto_provider_failure: Option<String>,
    macos_installation_required: bool,
    forced_safe_mode: bool,
    launch_mode: DesktopLaunchMode,
    startup_health: StartupHealthManager,
}

struct DesktopServices {
    runtime: AgentRuntimeManager,
    connector_lifecycles: ConnectorLifecycleManager,
    connector_processes: ConnectorProcessManager,
    registered_services: RegisteredServiceMonitor,
    registered_service_request_rx: RegisteredServiceMonitorReceiver,
    local_apps: LocalAppsChangeNotifier,
    local_app_install_tasks: LocalAppInstallTaskManager,
    runtime_log_streaming_requested: Arc<AtomicBool>,
    runtime_log_streaming: Arc<AtomicBool>,
    main_window_visible: Arc<AtomicBool>,
    quitting: Arc<AtomicBool>,
    local_app_ui: Arc<RwLock<Option<LocalAppUiEndpoint>>>,
}

impl DesktopServices {
    fn new() -> Self {
        let (registered_services, registered_service_request_rx) = RegisteredServiceMonitor::new();
        Self {
            runtime: AgentRuntimeManager::new(),
            connector_lifecycles: ConnectorLifecycleManager::default(),
            connector_processes: ConnectorProcessManager::default(),
            registered_services,
            registered_service_request_rx,
            local_apps: LocalAppsChangeNotifier::default(),
            local_app_install_tasks: LocalAppInstallTaskManager::default(),
            runtime_log_streaming_requested: Arc::new(AtomicBool::new(false)),
            runtime_log_streaming: Arc::new(AtomicBool::new(false)),
            main_window_visible: Arc::new(AtomicBool::new(false)),
            quitting: Arc::new(AtomicBool::new(false)),
            local_app_ui: Arc::new(RwLock::new(None)),
        }
    }

    fn desktop_state(&self, launch: &DesktopLaunch) -> DesktopState {
        DesktopState {
            runtime: self.runtime.clone(),
            connector_lifecycles: self.connector_lifecycles.clone(),
            connector_processes: self.connector_processes.clone(),
            config_path: launch.config_path.clone(),
            quitting: Arc::clone(&self.quitting),
            local_app_ui: Arc::clone(&self.local_app_ui),
            local_apps: self.local_apps.clone(),
            local_app_install_tasks: self.local_app_install_tasks.clone(),
            startup_health: launch.startup_health.clone(),
            registered_services: self.registered_services.clone(),
            runtime_log_streaming_requested: Arc::clone(&self.runtime_log_streaming_requested),
            runtime_log_streaming: Arc::clone(&self.runtime_log_streaming),
            main_window_visible: Arc::clone(&self.main_window_visible),
        }
    }
}

struct DesktopSetup {
    diagnostics: StartupDiagnostics,
    startup_health: StartupHealthManager,
    config_path: PathBuf,
    config_path_failure: Option<String>,
    crypto_provider_failure: Option<String>,
    macos_installation_required: bool,
    forced_safe_mode: bool,
    launch_mode: DesktopLaunchMode,
    runtime: AgentRuntimeManager,
    connector_lifecycles: ConnectorLifecycleManager,
    connector_processes: ConnectorProcessManager,
    registered_services: RegisteredServiceMonitor,
    registered_service_request_rx: RegisteredServiceMonitorReceiver,
    local_apps: LocalAppsChangeNotifier,
    local_app_ui: Arc<RwLock<Option<LocalAppUiEndpoint>>>,
    runtime_log_streaming: Arc<AtomicBool>,
}

#[derive(Clone)]
struct DesktopRun {
    diagnostics: StartupDiagnostics,
    startup_health: StartupHealthManager,
    macos_installation_required: bool,
    launch_mode: DesktopLaunchMode,
}

pub(crate) fn run() {
    let launch = prepare_desktop_launch();
    let run_context = DesktopRun {
        diagnostics: launch.diagnostics.clone(),
        startup_health: launch.startup_health.clone(),
        macos_installation_required: launch.macos_installation_required,
        launch_mode: launch.launch_mode,
    };
    let services = DesktopServices::new();
    let builder = configure_desktop_callbacks(desktop_builder(&launch), &launch, services);
    let app = register_desktop_commands(builder)
        .build(tauri::generate_context!())
        .unwrap_or_else(|err| {
            launch
                .diagnostics
                .error(format!("error while building tauri application: {err:#}"));
            std::process::exit(1);
        });
    app.run(move |app, event| handle_desktop_run_event(app, event, &run_context));
}

fn prepare_desktop_launch() -> DesktopLaunch {
    let bootstrap_diagnostics = StartupDiagnostics::bootstrap();
    install_panic_diagnostics(bootstrap_diagnostics.clone());
    let crypto_provider_failure = install_rustls_crypto_provider().err().map(|err| {
        let detail = format!("failed to install rustls provider: {err:#}");
        bootstrap_diagnostics.error(&detail);
        detail
    });
    let (config_path, config_path_failure) = resolve_desktop_config(&bootstrap_diagnostics);
    let diagnostics = StartupDiagnostics::for_config_path(&config_path);
    install_panic_diagnostics(diagnostics.clone());
    log_startup_environment(&diagnostics, &config_path);
    let process_args = std::env::args_os().collect::<Vec<_>>();
    let forced_safe_mode = process_args
        .iter()
        .any(|arg| arg == OsStr::new("--safe-mode"));
    let interactive_restart_requested = consume_interactive_restart_request(&config_path)
        .unwrap_or_else(|err| {
            diagnostics.warn(err);
            false
        });
    let launch_mode = DesktopLaunchMode::from_args(
        process_args.iter().map(|arg| arg.as_os_str()),
        interactive_restart_requested,
    );
    diagnostics.info(format!(
        "desktop launch mode resolved: mode={} interactive_restart_requested={interactive_restart_requested}",
        match launch_mode {
            DesktopLaunchMode::Interactive => "interactive",
            DesktopLaunchMode::BackgroundAutostart => "background_autostart",
        }
    ));
    let startup_health = StartupHealthManager::new(&config_path, diagnostics.clone());
    DesktopLaunch {
        diagnostics,
        config_path,
        config_path_failure,
        crypto_provider_failure,
        macos_installation_required: macos_installation::required_for_current_executable(),
        forced_safe_mode,
        launch_mode,
        startup_health,
    }
}

fn resolve_desktop_config(diagnostics: &StartupDiagnostics) -> (PathBuf, Option<String>) {
    match default_config_path() {
        Ok(path) => (path, None),
        Err(err) => {
            let detail = format!("failed to determine default config path: {err:#}");
            diagnostics.error(&detail);
            (
                std::env::temp_dir()
                    .join("baijimu-recovery")
                    .join("agent-config.json"),
                Some(detail),
            )
        }
    }
}

fn desktop_builder(launch: &DesktopLaunch) -> tauri::Builder<tauri::Wry> {
    let diagnostics = launch.diagnostics.clone();
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(
            move |app, argv, _cwd| handle_single_instance(app, argv, &diagnostics),
        ))
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(
            tauri_plugin_log::Builder::new()
                .max_file_size(5 * 1024 * 1024)
                .rotation_strategy(tauri_plugin_log::RotationStrategy::KeepSome(5))
                .build(),
        )
        .plugin(
            tauri_plugin_autostart::Builder::new()
                .app_name("BaijimuBridgeAgent")
                .arg(AUTOSTART_BACKGROUND_ARG)
                .build(),
        )
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(desktop_window_state_flags())
                .build(),
        )
}

fn handle_single_instance(
    app: &tauri::AppHandle,
    argv: Vec<String>,
    diagnostics: &StartupDiagnostics,
) {
    if quit_running_instance_requested(&argv) {
        diagnostics.info("quit requested by a secondary desktop process");
        quit_app(app);
    } else if background_autostart_requested(argv.iter().map(String::as_str)) {
        diagnostics.info(
            "background autostart reached an existing instance; keeping main window state unchanged",
        );
    } else {
        show_main_window(
            app,
            Some(diagnostics),
            MainWindowOpenReason::SecondaryLaunch,
        );
    }
}

fn configure_desktop_callbacks(
    builder: tauri::Builder<tauri::Wry>,
    launch: &DesktopLaunch,
    services: DesktopServices,
) -> tauri::Builder<tauri::Wry> {
    let page_diagnostics = launch.diagnostics.clone();
    let page_requested = Arc::clone(&services.runtime_log_streaming_requested);
    let page_streaming = Arc::clone(&services.runtime_log_streaming);
    let window_diagnostics = launch.diagnostics.clone();
    let quitting = Arc::clone(&services.quitting);
    let desktop_state = services.desktop_state(launch);
    let setup = DesktopSetup {
        diagnostics: launch.diagnostics.clone(),
        startup_health: launch.startup_health.clone(),
        config_path: launch.config_path.clone(),
        config_path_failure: launch.config_path_failure.clone(),
        crypto_provider_failure: launch.crypto_provider_failure.clone(),
        macos_installation_required: launch.macos_installation_required,
        forced_safe_mode: launch.forced_safe_mode,
        launch_mode: launch.launch_mode,
        runtime: services.runtime.clone(),
        connector_lifecycles: services.connector_lifecycles.clone(),
        connector_processes: services.connector_processes.clone(),
        registered_services: services.registered_services.clone(),
        registered_service_request_rx: services.registered_service_request_rx,
        local_apps: services.local_apps.clone(),
        local_app_ui: Arc::clone(&services.local_app_ui),
        runtime_log_streaming: Arc::clone(&services.runtime_log_streaming),
    };
    builder
        .manage(desktop_state)
        .on_page_load(move |webview, payload| {
            handle_page_load(
                webview,
                payload,
                &page_requested,
                &page_streaming,
                &page_diagnostics,
            );
        })
        .setup(move |app| setup_desktop(app, setup))
        .on_window_event(move |window, event| {
            handle_window_event(window, event, &quitting, &window_diagnostics);
        })
}

fn handle_page_load<R: tauri::Runtime>(
    webview: &tauri::Webview<R>,
    payload: &tauri::webview::PageLoadPayload<'_>,
    requested: &AtomicBool,
    streaming: &AtomicBool,
    diagnostics: &StartupDiagnostics,
) {
    if webview.label() == "main" && payload.event() == tauri::webview::PageLoadEvent::Started {
        requested.store(false, Ordering::SeqCst);
        streaming.store(false, Ordering::SeqCst);
    }
    diagnostics.info(format!(
        "webview page load {:?}: label={} url={}",
        payload.event(),
        webview.label(),
        payload.url()
    ));
}

fn setup_desktop(
    app: &mut tauri::App,
    setup: DesktopSetup,
) -> Result<(), Box<dyn std::error::Error>> {
    setup.diagnostics.info("tauri setup started");
    if setup.macos_installation_required {
        setup.diagnostics.warn(
            "macOS app is running outside an Applications directory; showing the installation reminder and stopping startup",
        );
        macos_installation::show_reminder(app.handle());
        return Ok(());
    }
    setup
        .startup_health
        .begin_primary(setup.forced_safe_mode, setup.config_path_failure);
    record_crypto_provider_health(&setup.startup_health, setup.crypto_provider_failure);
    setup.startup_health.attach_event_app(app.handle().clone());
    setup.local_apps.attach_event_app(app.handle().clone());
    attach_lifecycle_events(&setup.connector_lifecycles, app.handle().clone());
    forward_runtime_events(
        app.handle().clone(),
        setup.runtime.clone(),
        Arc::clone(&setup.runtime_log_streaming),
    );
    #[cfg(debug_assertions)]
    if std::env::var_os("BRIDGE_AGENT_OPEN_DEVTOOLS").is_some() {
        if let Some(window) = app.get_webview_window("main") {
            window.open_devtools();
        }
    }
    configure_desktop_autostart(app, &setup.startup_health, &setup.diagnostics);
    setup_shell_integrations(app, &setup.startup_health, &setup.diagnostics);
    if setup.launch_mode == DesktopLaunchMode::BackgroundAutostart {
        prepare_background_startup(app.handle(), &setup.diagnostics);
    }
    start_desktop_business_after_update_gate(DesktopBusinessStartup {
        app: app.handle().clone(),
        launch_mode: setup.launch_mode,
        runtime: setup.runtime,
        connector_lifecycles: setup.connector_lifecycles,
        connector_processes: setup.connector_processes,
        config_path: setup.config_path,
        startup_health: setup.startup_health,
        diagnostics: setup.diagnostics.clone(),
        local_app_ui: setup.local_app_ui,
        local_apps: setup.local_apps,
        registered_services: setup.registered_services,
        registered_service_request_rx: setup.registered_service_request_rx,
    });
    setup.diagnostics.info("tauri setup completed");
    Ok(())
}

fn record_crypto_provider_health(health: &StartupHealthManager, failure: Option<String>) {
    if let Some(detail) = failure {
        health.set_component("crypto_provider", "网络加密组件", "degraded", Some(detail));
    } else {
        health.set_component("crypto_provider", "网络加密组件", "ready", None);
    }
}

fn setup_shell_integrations(
    app: &tauri::App,
    health: &StartupHealthManager,
    diagnostics: &StartupDiagnostics,
) {
    if let Err(err) = setup_main_window_icon(app) {
        diagnostics.error(format!(
            "failed to setup the main window icon; Windows may show a generic taskbar icon: {err:#}"
        ));
        health.set_component("window_icon", "应用图标", "degraded", Some(err.to_string()));
    } else {
        diagnostics.info("main window icon setup completed");
        health.set_component("window_icon", "应用图标", "ready", None);
    }
    if let Err(err) = setup_tray(app, diagnostics) {
        diagnostics.error(format!(
            "failed to setup tray; continuing without tray icon: {err:#}"
        ));
        health.set_component("tray", "系统托盘", "degraded", Some(err.to_string()));
    } else {
        health.set_component("tray", "系统托盘", "ready", None);
    }
}

fn handle_window_event(
    window: &tauri::Window,
    event: &WindowEvent,
    quitting: &AtomicBool,
    diagnostics: &StartupDiagnostics,
) {
    if let WindowEvent::CloseRequested { api, .. } = event {
        if quitting.load(Ordering::SeqCst) {
            return;
        }
        api.prevent_close();
        hide_to_tray(window);
    }
    if window.label() != "main" {
        return;
    }
    let Some(webview_window) = window.app_handle().get_webview_window("main") else {
        return;
    };
    match event {
        WindowEvent::ScaleFactorChanged { .. } => normalize_main_window_layout(
            &webview_window,
            Some(diagnostics),
            WindowLayoutPolicy::Full,
        ),
        WindowEvent::Resized(_) => normalize_main_window_layout(
            &webview_window,
            Some(diagnostics),
            WindowLayoutPolicy::OversizedOnly,
        ),
        _ => {}
    }
}

fn register_desktop_commands(builder: tauri::Builder<tauri::Wry>) -> tauri::Builder<tauri::Wry> {
    builder.invoke_handler(tauri::generate_handler![
        load_config,
        python_runtime_status,
        save_config,
        save_service,
        delete_service,
        start_agent,
        stop_agent,
        stop_conflicting_runtime,
        runtime_snapshot,
        apply_saved_config_to_runtime,
        test_capability,
        test_local_app_capability,
        list_logs,
        set_runtime_log_streaming,
        clear_logs,
        reset_example_config,
        recover_invalid_config,
        open_in_browser,
        open_in_edge,
        desktop_permission_status,
        registered_service_statuses,
        local_app_runtime_statuses,
        connector_lifecycle_snapshots,
        start_registered_service,
        stop_registered_service,
        list_connector_apps,
        connector_app_ui_url,
        list_market_connector_apps,
        show_connector_app,
        invoke_connector_management,
        check_connector_app_update,
        start_connector_app_install,
        list_connector_app_install_tasks,
        start_connector_app,
        stop_connector_app,
        uninstall_connector_app,
        request_desktop_permission,
        open_desktop_permission_settings,
        start_browser_auth,
        poll_browser_auth,
        app_version,
        open_app_uninstaller,
        get_startup_health,
        mark_frontend_ready,
        restart_in_normal_mode,
        open_startup_log,
        check_app_update,
        install_app_update,
        baijimu_cli_status,
        install_baijimu_cli_update,
        rollback_baijimu_cli
    ])
}

fn handle_desktop_run_event(app: &tauri::AppHandle, event: tauri::RunEvent, context: &DesktopRun) {
    match event {
        tauri::RunEvent::Ready => handle_desktop_ready(app, context),
        #[cfg(target_os = "macos")]
        tauri::RunEvent::Reopen {
            has_visible_windows,
            ..
        } => {
            context.diagnostics.info(format!(
                "tauri reopen event received: has_visible_windows={has_visible_windows}"
            ));
            if should_restore_main_window_on_macos_reopen(has_visible_windows) {
                show_main_window(
                    app,
                    Some(&context.diagnostics),
                    MainWindowOpenReason::MacosReopen,
                );
            } else {
                context
                    .diagnostics
                    .info("macOS already has a visible main window; skipping forced refocus");
            }
        }
        tauri::RunEvent::ExitRequested { api, .. } => {
            let state = app.state::<DesktopState>();
            if !state.quitting.load(Ordering::SeqCst) {
                api.prevent_exit();
                quit_app(app);
            }
        }
        _ => {}
    }
}

fn handle_desktop_ready(app: &tauri::AppHandle, context: &DesktopRun) {
    context.diagnostics.info("tauri runtime ready");
    if context.macos_installation_required {
        context
            .diagnostics
            .info("macOS installation reminder is active; suppressing the main window");
        return;
    }
    context.startup_health.set_component(
        "desktop_shell",
        "桌面基础壳",
        "starting",
        Some("等待前端就绪确认".to_string()),
    );
    if context.launch_mode.should_show_main_window() {
        show_main_window(
            app,
            Some(&context.diagnostics),
            MainWindowOpenReason::InteractiveStartup,
        );
    } else {
        context
            .diagnostics
            .info("background autostart runtime ready; main window remains hidden and unfocused");
    }
}

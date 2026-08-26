use super::*;

pub(super) struct DesktopState {
    pub(super) runtime: AgentRuntimeManager,
    pub(super) connector_lifecycles: ConnectorLifecycleManager,
    pub(super) connector_processes: ConnectorProcessManager,
    pub(super) config_path: PathBuf,
    pub(super) quitting: Arc<AtomicBool>,
    pub(super) local_app_ui: Arc<RwLock<Option<LocalAppUiEndpoint>>>,
    pub(super) local_apps: LocalAppsChangeNotifier,
    pub(super) local_app_install_tasks: LocalAppInstallTaskManager,
    pub(super) startup_health: StartupHealthManager,
    pub(super) registered_services: RegisteredServiceMonitor,
    pub(super) runtime_log_streaming_requested: Arc<AtomicBool>,
    pub(super) runtime_log_streaming: Arc<AtomicBool>,
    pub(super) main_window_visible: Arc<AtomicBool>,
}

#[derive(Debug, Clone)]
pub(super) struct LocalAppUiEndpoint {
    pub(super) port: u16,
    pub(super) token: String,
}

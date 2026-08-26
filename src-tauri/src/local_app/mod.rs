mod adapters;
mod application;
mod domain;

pub(crate) use adapters::health_http::{LocalAppRuntimeStatus, RegisteredServiceStatus};
pub(crate) use adapters::monitor::{
    collect_local_app_runtime_statuses, RegisteredServiceMonitor, RegisteredServiceMonitorReceiver,
};
pub(crate) use adapters::process::ConnectorProcessManager;
pub(crate) use adapters::runtime_host::{
    connector_local_app_is_healthy, connector_local_app_status, start_connector_and_wait,
    start_connector_with_lifecycle, stop_connector_with_lifecycle,
};
pub(crate) use adapters::tauri_events::{
    attach_lifecycle_events, start_runtime_monitor_with_tauri, LocalAppsChangeNotifier,
    LocalAppsChangeOperation,
};
pub(crate) use adapters::tauri_progress::LocalAppInstallProgressReporter;
pub(crate) use application::install_tasks::{
    LocalAppInstallTask, LocalAppInstallTaskManager, LocalAppInstallTaskOperation,
    LocalAppInstallTaskPhase,
};
pub(crate) use application::lifecycle::ConnectorLifecycleManager;
pub(crate) use domain::{
    ConnectorHealthState, ConnectorLifecycleSnapshot, ConnectorLifecycleState,
    ConnectorManagementNotReady, ConnectorOperationKind,
};

#[cfg(test)]
pub(crate) use adapters::health_http::{
    apply_managed_process_status, check_local_app, format_health_http_error,
    inactive_local_app_status, RegisteredServiceState, HEALTH_ERROR_MESSAGE_MAX_CHARS,
};
#[cfg(test)]
pub(crate) use adapters::monitor::{
    local_app_runtime_statuses_changed, registered_service_statuses_changed,
};
#[cfg(test)]
pub(crate) use adapters::runtime_host::ensure_lifecycle_command_succeeded;
#[cfg(test)]
pub(crate) use application::install_tasks::format_byte_count;

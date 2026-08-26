use serde::Serialize;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, Mutex,
};
use tauri::Emitter;

use super::super::application::lifecycle::ConnectorLifecycleManager;
use super::super::domain::{ConnectorLifecycleSnapshot, CONNECTOR_LIFECYCLE_EVENT};
use super::monitor::{start_runtime_monitor, RegisteredServiceMonitorReceiver};
use super::process::ConnectorProcessManager;

const LOCAL_APPS_CHANGED_EVENT: &str = "local-apps-changed";
const REGISTERED_SERVICES_EVENT: &str = "registered-services-changed";
const LOCAL_APP_RUNTIME_EVENT: &str = "local-app-runtime-changed";

pub(crate) fn attach_lifecycle_events(manager: &ConnectorLifecycleManager, app: tauri::AppHandle) {
    manager.attach_event_sink(move |snapshot: ConnectorLifecycleSnapshot| {
        if let Err(error) = app.emit(CONNECTOR_LIFECYCLE_EVENT, snapshot) {
            log::warn!("failed to emit connector lifecycle snapshot: {error}");
        }
    });
}

pub(crate) fn start_runtime_monitor_with_tauri(
    app: tauri::AppHandle,
    config_path: std::path::PathBuf,
    connector_lifecycles: ConnectorLifecycleManager,
    connector_processes: ConnectorProcessManager,
    request_rx: RegisteredServiceMonitorReceiver,
) {
    let registered_service_app = app.clone();
    start_runtime_monitor(
        config_path,
        connector_lifecycles,
        connector_processes,
        request_rx,
        move |statuses| {
            let _ = registered_service_app.emit(REGISTERED_SERVICES_EVENT, statuses);
        },
        move |statuses| {
            let _ = app.emit(LOCAL_APP_RUNTIME_EVENT, statuses);
        },
    );
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LocalAppsChangedEvent {
    pub(crate) revision: u64,
    pub(crate) operation: LocalAppsChangeOperation,
    pub(crate) app_id: String,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LocalAppsChangeOperation {
    Install,
    Upgrade,
    Sync,
    Uninstall,
}

#[derive(Clone, Default)]
pub(crate) struct LocalAppsChangeNotifier {
    revision: Arc<AtomicU64>,
    event_app: Arc<Mutex<Option<tauri::AppHandle>>>,
}

impl LocalAppsChangeNotifier {
    pub(crate) fn attach_event_app(&self, app: tauri::AppHandle) {
        if let Ok(mut current) = self.event_app.lock() {
            *current = Some(app);
        }
    }

    pub(crate) fn notify(
        &self,
        operation: LocalAppsChangeOperation,
        app_id: &str,
    ) -> LocalAppsChangedEvent {
        let event = LocalAppsChangedEvent {
            revision: self.revision.fetch_add(1, Ordering::SeqCst) + 1,
            operation,
            app_id: app_id.to_string(),
        };
        let app = self
            .event_app
            .lock()
            .ok()
            .and_then(|current| current.clone());
        if let Some(app) = app {
            if let Err(err) = app.emit(LOCAL_APPS_CHANGED_EVENT, event.clone()) {
                log::warn!(
                    "failed to emit local apps changed event: operation={:?} app_id={} error={err}",
                    event.operation,
                    event.app_id
                );
            }
        }
        event
    }
}

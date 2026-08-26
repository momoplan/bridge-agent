use super::super::application::lifecycle::ConnectorLifecycleManager;
use super::super::domain::{ConnectorHealthState, ConnectorLifecycleState};
use super::health_http::{
    check_local_app, check_registered_service, inactive_local_app_status, LocalAppRuntimeStatus,
    RegisteredServiceState, RegisteredServiceStatus,
};
use super::process::ConnectorProcessManager;
use bridge_agent::{ensure_config_exists, load_config as load_agent_config, show_connector};
use reqwest::Client;
use std::path::{Path, PathBuf};
use std::time::Duration;

const MONITOR_INTERVAL: Duration = Duration::from_secs(30);

#[derive(Clone)]
pub(crate) struct RegisteredServiceMonitor {
    request_tx: tokio::sync::mpsc::UnboundedSender<RegisteredServiceMonitorRequest>,
}

impl RegisteredServiceMonitor {
    pub(crate) fn new() -> (Self, RegisteredServiceMonitorReceiver) {
        let (request_tx, request_rx) = tokio::sync::mpsc::unbounded_channel();
        (Self { request_tx }, request_rx)
    }

    pub(crate) fn request_refresh(&self) {
        let _ = self
            .request_tx
            .send(RegisteredServiceMonitorRequest::Refresh);
    }

    pub(crate) async fn statuses(&self) -> Result<Vec<RegisteredServiceStatus>, String> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        self.request_tx
            .send(RegisteredServiceMonitorRequest::RefreshAndRespond(reply_tx))
            .map_err(|_| "本地应用健康监控已停止".to_string())?;
        reply_rx
            .await
            .map_err(|_| "本地应用健康监控未返回结果".to_string())?
    }
}

pub(crate) enum RegisteredServiceMonitorRequest {
    Refresh,
    RefreshAndRespond(tokio::sync::oneshot::Sender<Result<Vec<RegisteredServiceStatus>, String>>),
}

pub(crate) type RegisteredServiceMonitorReceiver =
    tokio::sync::mpsc::UnboundedReceiver<RegisteredServiceMonitorRequest>;

pub(crate) async fn collect_local_app_runtime_statuses(
    config_path: &Path,
    connector_lifecycles: &ConnectorLifecycleManager,
    connector_processes: &ConnectorProcessManager,
) -> Result<Vec<LocalAppRuntimeStatus>, String> {
    ensure_config_exists(config_path).map_err(|err| err.to_string())?;
    let config = load_agent_config(config_path).map_err(|err| err.to_string())?;
    let client = Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
        .map_err(|err| err.to_string())?;
    let mut statuses = Vec::with_capacity(config.local_apps.len());
    for app in config.local_apps {
        let process_running = connector_processes.managed_running(&app.app_id).await;
        let runtime_active = connector_processes.runtime_active(&app.app_id).await;
        let app_id = app.app_id.clone();
        let status = if runtime_active {
            check_local_app(&client, app, process_running).await
        } else {
            inactive_local_app_status(app, process_running)
        };
        let version = show_connector(&app_id)
            .ok()
            .map(|record| record.manifest.version);
        let pid = connector_processes.managed_pid(&app_id).await;
        let (lifecycle, health) = match (runtime_active, status.status) {
            (false, _) => (
                ConnectorLifecycleState::Stopped,
                ConnectorHealthState::Unhealthy,
            ),
            (true, RegisteredServiceState::Healthy) => (
                ConnectorLifecycleState::Ready,
                ConnectorHealthState::Healthy,
            ),
            (true, RegisteredServiceState::Unhealthy) => (
                ConnectorLifecycleState::Degraded,
                ConnectorHealthState::Unhealthy,
            ),
            (true, RegisteredServiceState::NotConfigured) => (
                ConnectorLifecycleState::Ready,
                ConnectorHealthState::NotConfigured,
            ),
            (true, RegisteredServiceState::Unknown) => (
                ConnectorLifecycleState::Recovering,
                ConnectorHealthState::Unknown,
            ),
        };
        connector_lifecycles.observe(
            &app_id,
            lifecycle,
            health,
            version,
            pid,
            status.detail.clone(),
        )?;
        statuses.push(status);
    }
    Ok(statuses)
}

async fn collect_registered_service_statuses(
    config_path: &Path,
) -> Result<Vec<RegisteredServiceStatus>, String> {
    ensure_config_exists(config_path).map_err(|err| err.to_string())?;
    let config = load_agent_config(config_path).map_err(|err| err.to_string())?;
    let client = Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
        .map_err(|err| err.to_string())?;
    let mut statuses = Vec::new();

    for service in config.services {
        if service.health_check.is_none() && service.start_command.is_none() {
            continue;
        }
        statuses.push(check_registered_service(&client, service).await);
    }

    Ok(statuses)
}

pub(crate) fn registered_service_statuses_changed(
    previous: &[RegisteredServiceStatus],
    current: &[RegisteredServiceStatus],
) -> bool {
    previous.len() != current.len()
        || previous.iter().zip(current).any(|(left, right)| {
            left.service != right.service
                || left.status != right.status
                || left.detail != right.detail
                || left.health_check_configured != right.health_check_configured
                || left.start_command_configured != right.start_command_configured
                || left.stop_command_configured != right.stop_command_configured
        })
}

pub(crate) fn local_app_runtime_statuses_changed(
    previous: &[LocalAppRuntimeStatus],
    current: &[LocalAppRuntimeStatus],
) -> bool {
    previous.len() != current.len()
        || previous.iter().zip(current).any(|(left, right)| {
            left.app_id != right.app_id
                || left.status != right.status
                || left.detail != right.detail
                || left.health_check_configured != right.health_check_configured
                || left.start_command_configured != right.start_command_configured
                || left.stop_command_configured != right.stop_command_configured
                || left.process_managed != right.process_managed
                || left.process_running != right.process_running
        })
}

pub(crate) fn start_runtime_monitor(
    config_path: PathBuf,
    connector_lifecycles: ConnectorLifecycleManager,
    connector_processes: ConnectorProcessManager,
    mut request_rx: RegisteredServiceMonitorReceiver,
    publish_registered_services: impl Fn(Vec<RegisteredServiceStatus>) + Send + Sync + 'static,
    publish_local_apps: impl Fn(Vec<LocalAppRuntimeStatus>) + Send + Sync + 'static,
) {
    tauri::async_runtime::spawn(async move {
        let mut previous: Option<Vec<RegisteredServiceStatus>> = None;
        let mut previous_local_apps: Option<Vec<LocalAppRuntimeStatus>> = None;
        let mut interval = tokio::time::interval_at(
            tokio::time::Instant::now() + MONITOR_INTERVAL,
            MONITOR_INTERVAL,
        );
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            let mut responders = Vec::new();
            tokio::select! {
                _ = interval.tick() => {}
                _ = connector_processes.changed() => {}
                request = request_rx.recv() => {
                    match request {
                        Some(RegisteredServiceMonitorRequest::Refresh) => {}
                        Some(RegisteredServiceMonitorRequest::RefreshAndRespond(reply)) => {
                            responders.push(reply);
                        }
                        None => break,
                    }
                }
            }

            while let Ok(request) = request_rx.try_recv() {
                if let RegisteredServiceMonitorRequest::RefreshAndRespond(reply) = request {
                    responders.push(reply);
                }
            }

            let result = collect_registered_service_statuses(&config_path).await;
            match result.as_ref() {
                Ok(current) => {
                    let changed = previous
                        .as_deref()
                        .is_none_or(|last| registered_service_statuses_changed(last, current));
                    if changed {
                        log::debug!("registered service status changed");
                    }
                    if responders.is_empty() {
                        publish_registered_services(current.clone());
                    }
                    previous = Some(current.clone());
                }
                Err(err) => {
                    log::warn!("failed to refresh registered service statuses: {err}");
                }
            }
            for responder in responders {
                let _ = responder.send(result.clone());
            }

            match collect_local_app_runtime_statuses(
                &config_path,
                &connector_lifecycles,
                &connector_processes,
            )
            .await
            {
                Ok(current) => {
                    let changed = previous_local_apps
                        .as_deref()
                        .is_none_or(|last| local_app_runtime_statuses_changed(last, &current));
                    if changed {
                        log::debug!("local app runtime status changed");
                        publish_local_apps(current.clone());
                    }
                    previous_local_apps = Some(current);
                }
                Err(err) => {
                    log::warn!("failed to refresh local app runtime statuses: {err}");
                }
            }
        }
    });
}

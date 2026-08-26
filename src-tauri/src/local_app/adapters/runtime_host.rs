use super::super::application::lifecycle::ConnectorLifecycleManager;
use super::super::domain::ConnectorOperationKind;
use super::health_http::{check_local_app, LocalAppRuntimeStatus, RegisteredServiceState};
use super::process::ConnectorProcessManager;
use crate::managed_tool_dependency;
use bridge_agent::{
    load_config as load_agent_config, show_connector, sync_installed_connector,
    ConnectorStartResult,
};
use reqwest::Client;
use std::path::Path;
use std::time::{Duration, Instant};

pub(crate) fn ensure_lifecycle_command_succeeded(
    action: &str,
    result: &ConnectorStartResult,
) -> Result<(), String> {
    let lifecycle = &result.lifecycle;
    if lifecycle.configured && lifecycle.exit_code == Some(0) {
        Ok(())
    } else {
        let detail = if !lifecycle.configured {
            "命令未配置".to_string()
        } else if !lifecycle.stderr.trim().is_empty() {
            lifecycle.stderr.trim().to_string()
        } else {
            format!("退出码 {:?}", lifecycle.exit_code)
        };
        Err(format!("{action}失败：{}: {detail}", lifecycle.app_id))
    }
}

pub(crate) async fn connector_local_app_is_healthy(
    config_path: &Path,
    app_id: &str,
    connector_processes: &ConnectorProcessManager,
) -> Result<bool, String> {
    let config = load_agent_config(config_path).map_err(|err| err.to_string())?;
    if !config.local_apps.iter().any(|app| app.app_id == app_id) {
        // The install record is authoritative; local_apps is derived state.
        sync_installed_connector(config_path, app_id).map_err(|err| err.to_string())?;
    }
    let process_running = connector_processes.managed_running(app_id).await;
    let status = connector_local_app_status(config_path, app_id, process_running).await?;
    Ok(status.status == RegisteredServiceState::Healthy)
}

pub(crate) async fn connector_local_app_status(
    config_path: &Path,
    app_id: &str,
    process_running: Option<bool>,
) -> Result<LocalAppRuntimeStatus, String> {
    let config = load_agent_config(config_path).map_err(|err| err.to_string())?;
    let client = Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
        .map_err(|err| err.to_string())?;
    let app = config
        .local_apps
        .into_iter()
        .find(|app| app.app_id == app_id)
        .ok_or_else(|| format!("本地应用 `{app_id}` 不在当前配置中"))?;
    Ok(check_local_app(&client, app, process_running).await)
}

async fn wait_for_connector_health(
    config_path: &Path,
    app_id: &str,
    expected_healthy: bool,
) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let status = connector_local_app_status(config_path, app_id, None).await?;
        let matches = if expected_healthy {
            !status.health_check_configured || status.status == RegisteredServiceState::Healthy
        } else {
            !status.health_check_configured || status.status != RegisteredServiceState::Healthy
        };
        if matches {
            return Ok(());
        }
        if Instant::now() >= deadline {
            let details = format!(
                "{}={:?} ({})",
                status.app_id,
                status.status,
                status.detail.as_deref().unwrap_or("无详情")
            );
            let expected = if expected_healthy { "健康" } else { "停止" };
            return Err(format!("等待应用进入{expected}状态超时：{details}"));
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

pub(crate) async fn start_connector_and_wait(
    connector_processes: &ConnectorProcessManager,
    config_path: &Path,
    app_id: &str,
    action: &str,
    bundled_cli: Option<&Path>,
) -> Result<ConnectorStartResult, String> {
    let record = show_connector(app_id).map_err(|err| err.to_string())?;
    let dependency_env = managed_tool_dependency::ensure_ready(
        &record.manifest,
        bridge_agent::ConnectorManagedToolDependencyPhase::Start,
        bundled_cli,
    )
    .await
    .map_err(|err| format!("{action}前的应用依赖检查失败: {err:#}"))?;
    let result = connector_processes
        .start(app_id, config_path, dependency_env)
        .await?;
    if let Err(error) = ensure_lifecycle_command_succeeded(action, &result) {
        return Err(cleanup_failed_connector_start(
            connector_processes,
            config_path,
            &result.app_id,
            error,
        )
        .await);
    }
    if connector_processes.managed_running(&result.app_id).await == Some(false) {
        return Err(cleanup_failed_connector_start(
            connector_processes,
            config_path,
            &result.app_id,
            format!("{action}失败：宿主管理进程已提前退出"),
        )
        .await);
    }
    if let Err(error) = wait_for_connector_health(config_path, &result.app_id, true).await {
        return Err(cleanup_failed_connector_start(
            connector_processes,
            config_path,
            &result.app_id,
            error,
        )
        .await);
    }
    Ok(result)
}

pub(crate) async fn start_connector_with_lifecycle(
    connector_lifecycles: &ConnectorLifecycleManager,
    connector_processes: &ConnectorProcessManager,
    config_path: &Path,
    app_id: &str,
    action: &str,
    bundled_cli: Option<&Path>,
) -> Result<ConnectorStartResult, String> {
    let version = show_connector(app_id)
        .map_err(|error| error.to_string())?
        .manifest
        .version;
    let operation = connector_lifecycles
        .begin(
            app_id,
            ConnectorOperationKind::Start,
            Some(version.clone()),
            action,
        )
        .await?;
    match start_connector_and_wait(
        connector_processes,
        config_path,
        app_id,
        action,
        bundled_cli,
    )
    .await
    {
        Ok(result) => {
            connector_lifecycles.complete_ready(
                operation,
                Some(version),
                connector_processes.managed_pid(app_id).await,
                "应用已启动并通过就绪检查",
            )?;
            Ok(result)
        }
        Err(error) => {
            connector_lifecycles.fail(operation, &error)?;
            Err(error)
        }
    }
}

async fn cleanup_failed_connector_start(
    connector_processes: &ConnectorProcessManager,
    config_path: &Path,
    app_id: &str,
    error: String,
) -> String {
    match connector_processes.stop(app_id, config_path).await {
        Ok(_) => format!("{error}；已回收未通过启动验证的应用进程"),
        Err(cleanup_error) => {
            format!("{error}；回收未通过启动验证的应用进程也失败: {cleanup_error}")
        }
    }
}

async fn stop_connector_and_wait(
    connector_processes: &ConnectorProcessManager,
    config_path: &Path,
    app_id: &str,
    action: &str,
) -> Result<ConnectorStartResult, String> {
    let result = connector_processes.stop(app_id, config_path).await?;
    ensure_lifecycle_command_succeeded(action, &result)?;
    wait_for_connector_health(config_path, &result.app_id, false).await?;
    Ok(result)
}

pub(crate) async fn stop_connector_with_lifecycle(
    connector_lifecycles: &ConnectorLifecycleManager,
    connector_processes: &ConnectorProcessManager,
    config_path: &Path,
    app_id: &str,
    action: &str,
) -> Result<ConnectorStartResult, String> {
    let version = show_connector(app_id)
        .ok()
        .map(|record| record.manifest.version);
    let operation = connector_lifecycles
        .begin(
            app_id,
            ConnectorOperationKind::Stop,
            version.clone(),
            action,
        )
        .await?;
    match stop_connector_and_wait(connector_processes, config_path, app_id, action).await {
        Ok(result) => {
            connector_lifecycles.complete_stopped(operation, version, "应用已停止")?;
            Ok(result)
        }
        Err(error) => {
            connector_lifecycles.fail(operation, &error)?;
            Err(error)
        }
    }
}

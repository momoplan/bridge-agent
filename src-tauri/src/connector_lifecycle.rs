use serde::Serialize;
use std::collections::BTreeMap;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, Mutex,
};
use tauri::Emitter;

pub(crate) const CONNECTOR_LIFECYCLE_EVENT: &str = "connector-lifecycle-changed";

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ConnectorLifecycleState {
    Absent,
    Installing,
    Stopped,
    Starting,
    Ready,
    Degraded,
    Stopping,
    Upgrading,
    Uninstalling,
    Recovering,
    Failed,
}

impl ConnectorLifecycleState {
    fn operation_active(self) -> bool {
        matches!(
            self,
            Self::Installing
                | Self::Starting
                | Self::Stopping
                | Self::Upgrading
                | Self::Uninstalling
                | Self::Recovering
        )
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ConnectorOperationKind {
    Install,
    Upgrade,
    Start,
    Stop,
    Uninstall,
}

impl ConnectorOperationKind {
    pub(crate) fn lifecycle(self) -> ConnectorLifecycleState {
        match self {
            Self::Install => ConnectorLifecycleState::Installing,
            Self::Upgrade => ConnectorLifecycleState::Upgrading,
            Self::Start => ConnectorLifecycleState::Starting,
            Self::Stop => ConnectorLifecycleState::Stopping,
            Self::Uninstall => ConnectorLifecycleState::Uninstalling,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ConnectorHealthState {
    NotConfigured,
    Healthy,
    Unhealthy,
    Unknown,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConnectorOperationSnapshot {
    pub(crate) id: String,
    pub(crate) kind: ConnectorOperationKind,
    pub(crate) phase: String,
    pub(crate) progress_percent: Option<u8>,
    pub(crate) started_at_epoch_ms: u64,
    pub(crate) updated_at_epoch_ms: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConnectorLifecycleSnapshot {
    pub(crate) schema_version: u32,
    pub(crate) connector_id: String,
    pub(crate) lifecycle: ConnectorLifecycleState,
    pub(crate) operation: Option<ConnectorOperationSnapshot>,
    pub(crate) health: ConnectorHealthState,
    pub(crate) desired_version: Option<String>,
    pub(crate) observed_version: Option<String>,
    pub(crate) desired_generation: u64,
    pub(crate) observed_generation: u64,
    pub(crate) pid: Option<u32>,
    pub(crate) detail: Option<String>,
    pub(crate) error: Option<String>,
    pub(crate) updated_at_epoch_ms: u64,
}

impl ConnectorLifecycleSnapshot {
    fn new(connector_id: &str, now: u64) -> Self {
        Self {
            schema_version: 1,
            connector_id: connector_id.to_string(),
            lifecycle: ConnectorLifecycleState::Absent,
            operation: None,
            health: ConnectorHealthState::Unknown,
            desired_version: None,
            observed_version: None,
            desired_generation: 0,
            observed_generation: 0,
            pid: None,
            detail: None,
            error: None,
            updated_at_epoch_ms: now,
        }
    }

    pub(crate) fn management_ready(&self) -> bool {
        self.lifecycle == ConnectorLifecycleState::Ready
            && self.desired_generation != 0
            && self.desired_generation == self.observed_generation
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConnectorManagementNotReady {
    pub(crate) code: &'static str,
    pub(crate) message: String,
    pub(crate) lifecycle: ConnectorLifecycleSnapshot,
}

#[derive(Clone, Default)]
pub(crate) struct ConnectorLifecycleManager {
    entries: Arc<Mutex<BTreeMap<String, ConnectorLifecycleSnapshot>>>,
    next_generation: Arc<AtomicU64>,
    event_app: Arc<Mutex<Option<tauri::AppHandle>>>,
}

impl ConnectorLifecycleManager {
    pub(crate) fn attach_event_app(&self, app: tauri::AppHandle) {
        if let Ok(mut current) = self.event_app.lock() {
            *current = Some(app);
        }
    }

    pub(crate) fn list(&self) -> Vec<ConnectorLifecycleSnapshot> {
        self.entries
            .lock()
            .map(|entries| entries.values().cloned().collect())
            .unwrap_or_default()
    }

    pub(crate) fn begin(
        &self,
        connector_id: &str,
        kind: ConnectorOperationKind,
        desired_version: Option<String>,
        phase: impl Into<String>,
    ) -> Result<ConnectorOperationSnapshot, String> {
        let connector_id = normalized_connector_id(connector_id)?;
        let now = now_ms();
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| "本地应用生命周期状态锁已损坏".to_string())?;
        let entry = entries
            .entry(connector_id.clone())
            .or_insert_with(|| ConnectorLifecycleSnapshot::new(&connector_id, now));
        if let Some(active) = entry.operation.as_ref() {
            return Err(format!(
                "应用 `{connector_id}` 正在执行 {:?}（阶段：{}）",
                active.kind, active.phase
            ));
        }
        let generation = self.next_generation.fetch_add(1, Ordering::Relaxed) + 1;
        let operation = ConnectorOperationSnapshot {
            id: uuid::Uuid::new_v4().to_string(),
            kind,
            phase: phase.into(),
            progress_percent: None,
            started_at_epoch_ms: now,
            updated_at_epoch_ms: now,
        };
        entry.lifecycle = kind.lifecycle();
        entry.operation = Some(operation.clone());
        entry.desired_generation = generation;
        if desired_version.is_some() {
            entry.desired_version = desired_version;
        }
        entry.detail = Some(operation.phase.clone());
        entry.error = None;
        entry.updated_at_epoch_ms = now;
        let snapshot = entry.clone();
        drop(entries);
        self.emit(snapshot);
        Ok(operation)
    }

    pub(crate) fn advance(
        &self,
        connector_id: &str,
        operation_id: &str,
        lifecycle: ConnectorLifecycleState,
        phase: impl Into<String>,
        progress_percent: Option<u8>,
    ) -> Result<ConnectorLifecycleSnapshot, String> {
        if !lifecycle.operation_active() {
            return Err("运行中操作只能进入生命周期过渡态".to_string());
        }
        self.update_operation(connector_id, operation_id, |entry, operation, now| {
            let phase = phase.into();
            entry.lifecycle = lifecycle;
            entry.detail = Some(phase.clone());
            operation.phase = phase;
            operation.progress_percent = progress_percent;
            operation.updated_at_epoch_ms = now;
        })
    }

    pub(crate) fn complete_ready(
        &self,
        connector_id: &str,
        operation_id: &str,
        observed_version: Option<String>,
        pid: Option<u32>,
        detail: impl Into<String>,
    ) -> Result<ConnectorLifecycleSnapshot, String> {
        self.complete(connector_id, operation_id, move |entry| {
            let detail = detail.into();
            entry.lifecycle = ConnectorLifecycleState::Ready;
            entry.health = ConnectorHealthState::Healthy;
            entry.observed_generation = entry.desired_generation;
            entry.observed_version = observed_version;
            entry.desired_version = entry.observed_version.clone();
            entry.pid = pid;
            entry.detail = Some(detail);
            entry.error = None;
        })
    }

    pub(crate) fn complete_stopped(
        &self,
        connector_id: &str,
        operation_id: &str,
        observed_version: Option<String>,
        detail: impl Into<String>,
    ) -> Result<ConnectorLifecycleSnapshot, String> {
        self.complete(connector_id, operation_id, move |entry| {
            entry.lifecycle = ConnectorLifecycleState::Stopped;
            entry.health = ConnectorHealthState::Unhealthy;
            entry.observed_generation = entry.desired_generation;
            entry.observed_version = observed_version;
            entry.pid = None;
            entry.detail = Some(detail.into());
            entry.error = None;
        })
    }

    pub(crate) fn complete_absent(
        &self,
        connector_id: &str,
        operation_id: &str,
    ) -> Result<ConnectorLifecycleSnapshot, String> {
        self.complete(connector_id, operation_id, |entry| {
            entry.lifecycle = ConnectorLifecycleState::Absent;
            entry.health = ConnectorHealthState::NotConfigured;
            entry.observed_generation = entry.desired_generation;
            entry.desired_version = None;
            entry.observed_version = None;
            entry.pid = None;
            entry.detail = Some("应用已卸载".to_string());
            entry.error = None;
        })
    }

    pub(crate) fn fail(
        &self,
        connector_id: &str,
        operation_id: &str,
        error: impl Into<String>,
    ) -> Result<ConnectorLifecycleSnapshot, String> {
        self.complete(connector_id, operation_id, move |entry| {
            let error = error.into();
            entry.lifecycle = ConnectorLifecycleState::Failed;
            entry.health = ConnectorHealthState::Unknown;
            entry.pid = None;
            entry.detail = Some("生命周期操作失败".to_string());
            entry.error = Some(error);
        })
    }

    pub(crate) fn observe(
        &self,
        connector_id: &str,
        lifecycle: ConnectorLifecycleState,
        health: ConnectorHealthState,
        version: Option<String>,
        pid: Option<u32>,
        detail: Option<String>,
    ) -> Result<ConnectorLifecycleSnapshot, String> {
        let connector_id = normalized_connector_id(connector_id)?;
        let now = now_ms();
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| "本地应用生命周期状态锁已损坏".to_string())?;
        let entry = entries
            .entry(connector_id.clone())
            .or_insert_with(|| ConnectorLifecycleSnapshot::new(&connector_id, now));
        if entry.operation.is_some() {
            return Ok(entry.clone());
        }
        if entry.desired_generation == 0 {
            entry.desired_generation = self.next_generation.fetch_add(1, Ordering::Relaxed) + 1;
        }
        entry.lifecycle = lifecycle;
        entry.health = health;
        entry.desired_version = version.clone();
        entry.observed_version = version;
        entry.observed_generation = entry.desired_generation;
        entry.pid = pid;
        entry.detail = detail;
        if lifecycle != ConnectorLifecycleState::Failed {
            entry.error = None;
        }
        entry.updated_at_epoch_ms = now;
        let snapshot = entry.clone();
        drop(entries);
        self.emit(snapshot.clone());
        Ok(snapshot)
    }

    pub(crate) fn require_management_ready(
        &self,
        connector_id: &str,
    ) -> Result<ConnectorLifecycleSnapshot, Box<ConnectorManagementNotReady>> {
        let snapshot = self
            .entries
            .lock()
            .ok()
            .and_then(|entries| entries.get(connector_id.trim()).cloned())
            .unwrap_or_else(|| ConnectorLifecycleSnapshot::new(connector_id.trim(), now_ms()));
        if snapshot.management_ready() {
            return Ok(snapshot);
        }
        Err(Box::new(ConnectorManagementNotReady {
            code: "connector_not_ready",
            message: format!(
                "应用 `{}` 当前为 {:?}，本机管理接口尚未就绪",
                snapshot.connector_id, snapshot.lifecycle
            ),
            lifecycle: snapshot,
        }))
    }

    fn update_operation(
        &self,
        connector_id: &str,
        operation_id: &str,
        update: impl FnOnce(&mut ConnectorLifecycleSnapshot, &mut ConnectorOperationSnapshot, u64),
    ) -> Result<ConnectorLifecycleSnapshot, String> {
        let now = now_ms();
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| "本地应用生命周期状态锁已损坏".to_string())?;
        let entry = entries
            .get_mut(connector_id.trim())
            .ok_or_else(|| format!("应用 `{}` 没有生命周期记录", connector_id.trim()))?;
        let mut operation = entry
            .operation
            .take()
            .ok_or_else(|| format!("应用 `{}` 当前没有运行中的操作", connector_id.trim()))?;
        if operation.id != operation_id {
            entry.operation = Some(operation);
            return Err(format!(
                "应用 `{}` 的生命周期操作代际已变化",
                connector_id.trim()
            ));
        }
        update(entry, &mut operation, now);
        entry.operation = Some(operation);
        entry.updated_at_epoch_ms = now;
        let snapshot = entry.clone();
        drop(entries);
        self.emit(snapshot.clone());
        Ok(snapshot)
    }

    fn complete(
        &self,
        connector_id: &str,
        operation_id: &str,
        update: impl FnOnce(&mut ConnectorLifecycleSnapshot),
    ) -> Result<ConnectorLifecycleSnapshot, String> {
        let now = now_ms();
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| "本地应用生命周期状态锁已损坏".to_string())?;
        let entry = entries
            .get_mut(connector_id.trim())
            .ok_or_else(|| format!("应用 `{}` 没有生命周期记录", connector_id.trim()))?;
        let operation = entry
            .operation
            .as_ref()
            .ok_or_else(|| format!("应用 `{}` 当前没有运行中的操作", connector_id.trim()))?;
        if operation.id != operation_id {
            return Err(format!(
                "应用 `{}` 的生命周期操作代际已变化",
                connector_id.trim()
            ));
        }
        update(entry);
        entry.operation = None;
        entry.updated_at_epoch_ms = now;
        let snapshot = entry.clone();
        drop(entries);
        self.emit(snapshot.clone());
        Ok(snapshot)
    }

    fn emit(&self, snapshot: ConnectorLifecycleSnapshot) {
        let app = self
            .event_app
            .lock()
            .ok()
            .and_then(|current| current.clone());
        if let Some(app) = app {
            if let Err(error) = app.emit(CONNECTOR_LIFECYCLE_EVENT, snapshot) {
                log::warn!("failed to emit connector lifecycle snapshot: {error}");
            }
        }
    }
}

fn normalized_connector_id(connector_id: &str) -> Result<String, String> {
    let connector_id = connector_id.trim();
    if connector_id.is_empty() {
        return Err("本地应用 ID 不能为空".to_string());
    }
    Ok(connector_id.to_string())
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn management_is_available_only_for_the_observed_ready_generation() {
        let manager = ConnectorLifecycleManager::default();
        let operation = manager
            .begin(
                "connector.test",
                ConnectorOperationKind::Start,
                Some("1.0.0".to_string()),
                "starting",
            )
            .unwrap();

        assert_eq!(
            manager
                .require_management_ready("connector.test")
                .unwrap_err()
                .code,
            "connector_not_ready"
        );

        manager
            .complete_ready(
                "connector.test",
                &operation.id,
                Some("1.0.0".to_string()),
                Some(42),
                "ready",
            )
            .unwrap();
        let ready = manager.require_management_ready("connector.test").unwrap();
        assert_eq!(ready.lifecycle, ConnectorLifecycleState::Ready);
        assert_eq!(ready.desired_generation, ready.observed_generation);
    }

    #[test]
    fn active_operation_serializes_lifecycle_changes() {
        let manager = ConnectorLifecycleManager::default();
        manager
            .begin(
                "connector.test",
                ConnectorOperationKind::Upgrade,
                Some("2.0.0".to_string()),
                "upgrading",
            )
            .unwrap();

        let error = manager
            .begin(
                "connector.test",
                ConnectorOperationKind::Start,
                None,
                "starting",
            )
            .unwrap_err();
        assert!(error.contains("正在执行"));
    }

    #[test]
    fn health_observation_cannot_overwrite_an_active_transition() {
        let manager = ConnectorLifecycleManager::default();
        manager
            .begin(
                "connector.test",
                ConnectorOperationKind::Upgrade,
                Some("2.0.0".to_string()),
                "switching",
            )
            .unwrap();

        let snapshot = manager
            .observe(
                "connector.test",
                ConnectorLifecycleState::Ready,
                ConnectorHealthState::Healthy,
                Some("1.0.0".to_string()),
                Some(10),
                Some("stale health result".to_string()),
            )
            .unwrap();
        assert_eq!(snapshot.lifecycle, ConnectorLifecycleState::Upgrading);
        assert!(snapshot.operation.is_some());
    }
}

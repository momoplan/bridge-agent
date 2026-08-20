use serde::Serialize;
use std::collections::BTreeMap;
use std::ops::Deref;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, Mutex,
};
use tauri::Emitter;
use tokio::sync::{OwnedRwLockReadGuard, OwnedRwLockWriteGuard, RwLock};

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

pub(crate) struct ConnectorManagementPermit {
    pub(crate) lifecycle: ConnectorLifecycleSnapshot,
    _permit: OwnedRwLockReadGuard<()>,
}

pub(crate) struct ConnectorLifecycleOperation {
    pub(crate) snapshot: ConnectorOperationSnapshot,
    manager: ConnectorLifecycleManager,
    connector_id: String,
    armed: bool,
    _permit: OwnedRwLockWriteGuard<()>,
}

impl Deref for ConnectorLifecycleOperation {
    type Target = ConnectorOperationSnapshot;

    fn deref(&self) -> &Self::Target {
        &self.snapshot
    }
}

impl Drop for ConnectorLifecycleOperation {
    fn drop(&mut self) {
        if self.armed {
            self.manager.cancel_if_active(
                &self.connector_id,
                &self.snapshot.id,
                "生命周期操作在完成前被取消",
            );
        }
    }
}

struct PendingConnectorLifecycleOperation {
    manager: ConnectorLifecycleManager,
    connector_id: String,
    operation_id: String,
    previous: Option<ConnectorLifecycleSnapshot>,
    armed: bool,
}

impl PendingConnectorLifecycleOperation {
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for PendingConnectorLifecycleOperation {
    fn drop(&mut self) {
        if self.armed {
            self.manager.restore_if_active(
                &self.connector_id,
                &self.operation_id,
                self.previous.clone(),
            );
        }
    }
}

#[derive(Clone, Default)]
pub(crate) struct ConnectorLifecycleManager {
    entries: Arc<Mutex<BTreeMap<String, ConnectorLifecycleSnapshot>>>,
    access_gates: Arc<Mutex<BTreeMap<String, Arc<RwLock<()>>>>>,
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

    pub(crate) async fn begin(
        &self,
        connector_id: &str,
        kind: ConnectorOperationKind,
        desired_version: Option<String>,
        phase: impl Into<String>,
    ) -> Result<ConnectorLifecycleOperation, String> {
        let connector_id = normalized_connector_id(connector_id)?;
        let access_gate = self.access_gate(&connector_id)?;
        let now = now_ms();
        let phase = phase.into();
        let (operation, previous, snapshot) = {
            let mut entries = self
                .entries
                .lock()
                .map_err(|_| "本地应用生命周期状态锁已损坏".to_string())?;
            let previous = entries.get(&connector_id).cloned();
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
                phase,
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
            (operation, previous, entry.clone())
        };
        self.emit(snapshot);
        let mut pending = PendingConnectorLifecycleOperation {
            manager: self.clone(),
            connector_id: connector_id.clone(),
            operation_id: operation.id.clone(),
            previous,
            armed: true,
        };
        let permit = access_gate.write_owned().await;
        pending.disarm();
        Ok(ConnectorLifecycleOperation {
            snapshot: operation,
            manager: self.clone(),
            connector_id,
            armed: true,
            _permit: permit,
        })
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
        operation: ConnectorLifecycleOperation,
        observed_version: Option<String>,
        pid: Option<u32>,
        detail: impl Into<String>,
    ) -> Result<ConnectorLifecycleSnapshot, String> {
        self.complete(operation, move |entry| {
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
        operation: ConnectorLifecycleOperation,
        observed_version: Option<String>,
        detail: impl Into<String>,
    ) -> Result<ConnectorLifecycleSnapshot, String> {
        self.complete(operation, move |entry| {
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
        operation: ConnectorLifecycleOperation,
    ) -> Result<ConnectorLifecycleSnapshot, String> {
        self.complete(operation, |entry| {
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
        operation: ConnectorLifecycleOperation,
        error: impl Into<String>,
    ) -> Result<ConnectorLifecycleSnapshot, String> {
        self.complete(operation, move |entry| {
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

    fn require_management_ready(
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

    pub(crate) fn try_management_permit(
        &self,
        connector_id: &str,
    ) -> Result<ConnectorManagementPermit, Box<ConnectorManagementNotReady>> {
        let connector_id = connector_id.trim();
        let permit = self
            .access_gate(connector_id)
            .ok()
            .and_then(|gate| gate.try_read_owned().ok())
            .ok_or_else(|| self.management_not_ready(connector_id))?;
        let lifecycle = self.require_management_ready(connector_id)?;
        Ok(ConnectorManagementPermit {
            lifecycle,
            _permit: permit,
        })
    }

    fn access_gate(&self, connector_id: &str) -> Result<Arc<RwLock<()>>, String> {
        let connector_id = normalized_connector_id(connector_id)?;
        let mut access_gates = self
            .access_gates
            .lock()
            .map_err(|_| "本地应用访问门禁锁已损坏".to_string())?;
        Ok(access_gates
            .entry(connector_id)
            .or_insert_with(|| Arc::new(RwLock::new(())))
            .clone())
    }

    fn management_not_ready(&self, connector_id: &str) -> Box<ConnectorManagementNotReady> {
        match self.require_management_ready(connector_id) {
            Err(error) => error,
            Ok(lifecycle) => Box::new(ConnectorManagementNotReady {
                code: "connector_not_ready",
                message: format!(
                    "应用 `{}` 正在切换生命周期，本机管理接口暂不可用",
                    lifecycle.connector_id
                ),
                lifecycle,
            }),
        }
    }

    fn restore_if_active(
        &self,
        connector_id: &str,
        operation_id: &str,
        previous: Option<ConnectorLifecycleSnapshot>,
    ) {
        let Ok(mut entries) = self.entries.lock() else {
            return;
        };
        let is_active = entries
            .get(connector_id)
            .and_then(|entry| entry.operation.as_ref())
            .is_some_and(|operation| operation.id == operation_id);
        if !is_active {
            return;
        }
        let snapshot = match previous {
            Some(previous) => {
                entries.insert(connector_id.to_string(), previous.clone());
                previous
            }
            None => {
                entries.remove(connector_id);
                ConnectorLifecycleSnapshot::new(connector_id, now_ms())
            }
        };
        drop(entries);
        self.emit(snapshot);
    }

    fn cancel_if_active(&self, connector_id: &str, operation_id: &str, error: &str) {
        let Ok(mut entries) = self.entries.lock() else {
            return;
        };
        let Some(entry) = entries.get_mut(connector_id) else {
            return;
        };
        if entry
            .operation
            .as_ref()
            .map(|operation| operation.id.as_str())
            != Some(operation_id)
        {
            return;
        }
        entry.lifecycle = ConnectorLifecycleState::Failed;
        entry.health = ConnectorHealthState::Unknown;
        entry.operation = None;
        entry.pid = None;
        entry.detail = Some("生命周期操作被取消".to_string());
        entry.error = Some(error.to_string());
        entry.updated_at_epoch_ms = now_ms();
        let snapshot = entry.clone();
        drop(entries);
        self.emit(snapshot);
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
        mut lifecycle_operation: ConnectorLifecycleOperation,
        update: impl FnOnce(&mut ConnectorLifecycleSnapshot),
    ) -> Result<ConnectorLifecycleSnapshot, String> {
        let connector_id = lifecycle_operation.connector_id.clone();
        let operation_id = lifecycle_operation.snapshot.id.clone();
        let now = now_ms();
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| "本地应用生命周期状态锁已损坏".to_string())?;
        let entry = entries
            .get_mut(&connector_id)
            .ok_or_else(|| format!("应用 `{connector_id}` 没有生命周期记录"))?;
        let operation = entry
            .operation
            .as_ref()
            .ok_or_else(|| format!("应用 `{connector_id}` 当前没有运行中的操作"))?;
        if operation.id != operation_id {
            return Err(format!("应用 `{connector_id}` 的生命周期操作代际已变化"));
        }
        update(entry);
        entry.operation = None;
        entry.updated_at_epoch_ms = now;
        let snapshot = entry.clone();
        drop(entries);
        lifecycle_operation.armed = false;
        drop(lifecycle_operation);
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

    #[tokio::test]
    async fn management_is_available_only_for_the_observed_ready_generation() {
        let manager = ConnectorLifecycleManager::default();
        let operation = manager
            .begin(
                "connector.test",
                ConnectorOperationKind::Start,
                Some("1.0.0".to_string()),
                "starting",
            )
            .await
            .unwrap();

        assert_eq!(
            manager
                .require_management_ready("connector.test")
                .unwrap_err()
                .code,
            "connector_not_ready"
        );

        manager
            .complete_ready(operation, Some("1.0.0".to_string()), Some(42), "ready")
            .unwrap();
        let ready = manager.try_management_permit("connector.test").unwrap();
        assert_eq!(ready.lifecycle.lifecycle, ConnectorLifecycleState::Ready);
        assert_eq!(
            ready.lifecycle.desired_generation,
            ready.lifecycle.observed_generation
        );
    }

    #[tokio::test]
    async fn active_operation_serializes_lifecycle_changes() {
        let manager = ConnectorLifecycleManager::default();
        let operation = manager
            .begin(
                "connector.test",
                ConnectorOperationKind::Upgrade,
                Some("2.0.0".to_string()),
                "upgrading",
            )
            .await
            .unwrap();

        let error = manager
            .begin(
                "connector.test",
                ConnectorOperationKind::Start,
                None,
                "starting",
            )
            .await
            .err()
            .expect("a second lifecycle operation must be rejected");
        assert!(error.contains("正在执行"));
        manager.fail(operation, "test cleanup").unwrap();
    }

    #[tokio::test]
    async fn health_observation_cannot_overwrite_an_active_transition() {
        let manager = ConnectorLifecycleManager::default();
        let operation = manager
            .begin(
                "connector.test",
                ConnectorOperationKind::Upgrade,
                Some("2.0.0".to_string()),
                "switching",
            )
            .await
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
        manager.fail(operation, "test cleanup").unwrap();
    }

    #[tokio::test]
    async fn upgrade_drains_inflight_management_and_rejects_new_calls() {
        let manager = ConnectorLifecycleManager::default();
        let start = manager
            .begin(
                "connector.test",
                ConnectorOperationKind::Start,
                Some("1.0.0".to_string()),
                "starting",
            )
            .await
            .unwrap();
        manager
            .complete_ready(start, Some("1.0.0".to_string()), Some(42), "ready")
            .unwrap();
        let inflight = manager.try_management_permit("connector.test").unwrap();

        let upgrade_manager = manager.clone();
        let upgrade_task = tokio::spawn(async move {
            upgrade_manager
                .begin(
                    "connector.test",
                    ConnectorOperationKind::Upgrade,
                    Some("2.0.0".to_string()),
                    "upgrading",
                )
                .await
        });
        tokio::task::yield_now().await;

        let transitioning = manager
            .list()
            .into_iter()
            .find(|snapshot| snapshot.connector_id == "connector.test")
            .unwrap();
        assert_eq!(transitioning.lifecycle, ConnectorLifecycleState::Upgrading);
        assert!(!upgrade_task.is_finished());
        assert!(manager.try_management_permit("connector.test").is_err());

        drop(inflight);
        let upgrade = upgrade_task.await.unwrap().unwrap();
        assert!(manager.try_management_permit("connector.test").is_err());
        manager.fail(upgrade, "test cleanup").unwrap();
    }

    #[tokio::test]
    async fn cancelled_upgrade_wait_restores_the_previous_ready_state() {
        let manager = ConnectorLifecycleManager::default();
        let start = manager
            .begin(
                "connector.test",
                ConnectorOperationKind::Start,
                Some("1.0.0".to_string()),
                "starting",
            )
            .await
            .unwrap();
        manager
            .complete_ready(start, Some("1.0.0".to_string()), Some(42), "ready")
            .unwrap();
        let inflight = manager.try_management_permit("connector.test").unwrap();

        let upgrade_manager = manager.clone();
        let upgrade_task = tokio::spawn(async move {
            upgrade_manager
                .begin(
                    "connector.test",
                    ConnectorOperationKind::Upgrade,
                    Some("2.0.0".to_string()),
                    "upgrading",
                )
                .await
        });
        tokio::task::yield_now().await;
        upgrade_task.abort();
        let cancelled = upgrade_task.await;
        assert!(matches!(cancelled, Err(error) if error.is_cancelled()));

        let restored = manager
            .list()
            .into_iter()
            .find(|snapshot| snapshot.connector_id == "connector.test")
            .unwrap();
        assert_eq!(restored.lifecycle, ConnectorLifecycleState::Ready);
        assert!(restored.operation.is_none());
        drop(inflight);
        assert!(manager.try_management_permit("connector.test").is_ok());
    }

    #[tokio::test]
    async fn connector_access_gates_are_isolated_by_connector_id() {
        let manager = ConnectorLifecycleManager::default();
        let first_start = manager
            .begin(
                "connector.first",
                ConnectorOperationKind::Start,
                Some("1.0.0".to_string()),
                "starting",
            )
            .await
            .unwrap();
        manager
            .complete_ready(first_start, Some("1.0.0".to_string()), Some(42), "ready")
            .unwrap();
        let first_request = manager.try_management_permit("connector.first").unwrap();

        let second_start = manager
            .begin(
                "connector.second",
                ConnectorOperationKind::Start,
                Some("1.0.0".to_string()),
                "starting",
            )
            .await
            .unwrap();
        manager
            .complete_ready(second_start, Some("1.0.0".to_string()), Some(84), "ready")
            .unwrap();
        assert!(manager.try_management_permit("connector.second").is_ok());
        drop(first_request);
    }
}

use super::super::domain::{
    ConnectorHealthState, ConnectorLifecycleSnapshot, ConnectorLifecycleState,
    ConnectorManagementNotReady, ConnectorOperationKind, ConnectorOperationSnapshot,
};
use std::collections::BTreeMap;
use std::ops::Deref;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, Mutex,
};
use tokio::sync::{OwnedRwLockReadGuard, OwnedRwLockWriteGuard, RwLock};

type LifecycleEventSink = Arc<dyn Fn(ConnectorLifecycleSnapshot) + Send + Sync>;

pub(crate) struct ConnectorManagementPermit {
    pub(crate) lifecycle: ConnectorLifecycleSnapshot,
    _permit: OwnedRwLockReadGuard<()>,
}

pub(crate) struct ConnectorLifecycleOperation {
    pub(crate) snapshot: ConnectorOperationSnapshot,
    manager: ConnectorLifecycleManager,
    app_id: String,
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
                &self.app_id,
                &self.snapshot.id,
                "生命周期操作在完成前被取消",
            );
        }
    }
}

struct PendingConnectorLifecycleOperation {
    manager: ConnectorLifecycleManager,
    app_id: String,
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
            self.manager
                .restore_if_active(&self.app_id, &self.operation_id, self.previous.clone());
        }
    }
}

#[derive(Clone, Default)]
pub(crate) struct ConnectorLifecycleManager {
    entries: Arc<Mutex<BTreeMap<String, ConnectorLifecycleSnapshot>>>,
    access_gates: Arc<Mutex<BTreeMap<String, Arc<RwLock<()>>>>>,
    next_generation: Arc<AtomicU64>,
    event_sink: Arc<Mutex<Option<LifecycleEventSink>>>,
}

impl ConnectorLifecycleManager {
    pub(crate) fn attach_event_sink(
        &self,
        sink: impl Fn(ConnectorLifecycleSnapshot) + Send + Sync + 'static,
    ) {
        if let Ok(mut current) = self.event_sink.lock() {
            *current = Some(Arc::new(sink));
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
        app_id: &str,
        kind: ConnectorOperationKind,
        desired_version: Option<String>,
        phase: impl Into<String>,
    ) -> Result<ConnectorLifecycleOperation, String> {
        let app_id = normalized_app_id(app_id)?;
        let access_gate = self.access_gate(&app_id)?;
        let now = now_ms();
        let phase = phase.into();
        let (operation, previous, snapshot) = {
            let mut entries = self
                .entries
                .lock()
                .map_err(|_| "本地应用生命周期状态锁已损坏".to_string())?;
            let previous = entries.get(&app_id).cloned();
            let entry = entries
                .entry(app_id.clone())
                .or_insert_with(|| ConnectorLifecycleSnapshot::new(&app_id, now));
            if let Some(active) = entry.operation.as_ref() {
                return Err(format!(
                    "应用 `{app_id}` 正在执行 {:?}（阶段：{}）",
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
            app_id: app_id.clone(),
            operation_id: operation.id.clone(),
            previous,
            armed: true,
        };
        let permit = access_gate.write_owned().await;
        pending.disarm();
        Ok(ConnectorLifecycleOperation {
            snapshot: operation,
            manager: self.clone(),
            app_id,
            armed: true,
            _permit: permit,
        })
    }

    pub(crate) fn advance(
        &self,
        app_id: &str,
        operation_id: &str,
        lifecycle: ConnectorLifecycleState,
        phase: impl Into<String>,
        progress_percent: Option<u8>,
    ) -> Result<ConnectorLifecycleSnapshot, String> {
        if !lifecycle.operation_active() {
            return Err("运行中操作只能进入生命周期过渡态".to_string());
        }
        self.update_operation(app_id, operation_id, |entry, operation, now| {
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
        app_id: &str,
        lifecycle: ConnectorLifecycleState,
        health: ConnectorHealthState,
        version: Option<String>,
        pid: Option<u32>,
        detail: Option<String>,
    ) -> Result<ConnectorLifecycleSnapshot, String> {
        let app_id = normalized_app_id(app_id)?;
        let now = now_ms();
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| "本地应用生命周期状态锁已损坏".to_string())?;
        let entry = entries
            .entry(app_id.clone())
            .or_insert_with(|| ConnectorLifecycleSnapshot::new(&app_id, now));
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
        app_id: &str,
    ) -> Result<ConnectorLifecycleSnapshot, Box<ConnectorManagementNotReady>> {
        let snapshot = self
            .entries
            .lock()
            .ok()
            .and_then(|entries| entries.get(app_id.trim()).cloned())
            .unwrap_or_else(|| ConnectorLifecycleSnapshot::new(app_id.trim(), now_ms()));
        if snapshot.management_ready() {
            return Ok(snapshot);
        }
        Err(Box::new(ConnectorManagementNotReady {
            code: "connector_not_ready",
            message: format!(
                "应用 `{}` 当前为 {:?}，本机管理接口尚未就绪",
                snapshot.app_id, snapshot.lifecycle
            ),
            lifecycle: snapshot,
        }))
    }

    pub(crate) fn try_management_permit(
        &self,
        app_id: &str,
    ) -> Result<ConnectorManagementPermit, Box<ConnectorManagementNotReady>> {
        let app_id = app_id.trim();
        let permit = self
            .access_gate(app_id)
            .ok()
            .and_then(|gate| gate.try_read_owned().ok())
            .ok_or_else(|| self.management_not_ready(app_id))?;
        let lifecycle = self.require_management_ready(app_id)?;
        Ok(ConnectorManagementPermit {
            lifecycle,
            _permit: permit,
        })
    }

    fn access_gate(&self, app_id: &str) -> Result<Arc<RwLock<()>>, String> {
        let app_id = normalized_app_id(app_id)?;
        let mut access_gates = self
            .access_gates
            .lock()
            .map_err(|_| "本地应用访问门禁锁已损坏".to_string())?;
        Ok(access_gates
            .entry(app_id)
            .or_insert_with(|| Arc::new(RwLock::new(())))
            .clone())
    }

    fn management_not_ready(&self, app_id: &str) -> Box<ConnectorManagementNotReady> {
        match self.require_management_ready(app_id) {
            Err(error) => error,
            Ok(lifecycle) => Box::new(ConnectorManagementNotReady {
                code: "connector_not_ready",
                message: format!(
                    "应用 `{}` 正在切换生命周期，本机管理接口暂不可用",
                    lifecycle.app_id
                ),
                lifecycle,
            }),
        }
    }

    fn restore_if_active(
        &self,
        app_id: &str,
        operation_id: &str,
        previous: Option<ConnectorLifecycleSnapshot>,
    ) {
        let Ok(mut entries) = self.entries.lock() else {
            return;
        };
        let is_active = entries
            .get(app_id)
            .and_then(|entry| entry.operation.as_ref())
            .is_some_and(|operation| operation.id == operation_id);
        if !is_active {
            return;
        }
        let snapshot = match previous {
            Some(previous) => {
                entries.insert(app_id.to_string(), previous.clone());
                previous
            }
            None => {
                entries.remove(app_id);
                ConnectorLifecycleSnapshot::new(app_id, now_ms())
            }
        };
        drop(entries);
        self.emit(snapshot);
    }

    fn cancel_if_active(&self, app_id: &str, operation_id: &str, error: &str) {
        let Ok(mut entries) = self.entries.lock() else {
            return;
        };
        let Some(entry) = entries.get_mut(app_id) else {
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
        app_id: &str,
        operation_id: &str,
        update: impl FnOnce(&mut ConnectorLifecycleSnapshot, &mut ConnectorOperationSnapshot, u64),
    ) -> Result<ConnectorLifecycleSnapshot, String> {
        let now = now_ms();
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| "本地应用生命周期状态锁已损坏".to_string())?;
        let entry = entries
            .get_mut(app_id.trim())
            .ok_or_else(|| format!("应用 `{}` 没有生命周期记录", app_id.trim()))?;
        let mut operation = entry
            .operation
            .take()
            .ok_or_else(|| format!("应用 `{}` 当前没有运行中的操作", app_id.trim()))?;
        if operation.id != operation_id {
            entry.operation = Some(operation);
            return Err(format!("应用 `{}` 的生命周期操作代际已变化", app_id.trim()));
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
        let app_id = lifecycle_operation.app_id.clone();
        let operation_id = lifecycle_operation.snapshot.id.clone();
        let now = now_ms();
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| "本地应用生命周期状态锁已损坏".to_string())?;
        let entry = entries
            .get_mut(&app_id)
            .ok_or_else(|| format!("应用 `{app_id}` 没有生命周期记录"))?;
        let operation = entry
            .operation
            .as_ref()
            .ok_or_else(|| format!("应用 `{app_id}` 当前没有运行中的操作"))?;
        if operation.id != operation_id {
            return Err(format!("应用 `{app_id}` 的生命周期操作代际已变化"));
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
        let sink = self
            .event_sink
            .lock()
            .ok()
            .and_then(|current| current.clone());
        if let Some(sink) = sink {
            sink(snapshot);
        }
    }
}

include!("lifecycle/helpers.rs");

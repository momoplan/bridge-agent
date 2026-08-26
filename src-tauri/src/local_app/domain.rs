use serde::Serialize;

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
    pub(crate) fn operation_active(self) -> bool {
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
    pub(crate) app_id: String,
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
    pub(crate) fn new(app_id: &str, now: u64) -> Self {
        Self {
            // Existing desktop IPC contract. Changing its shape is outside this refactor.
            schema_version: 1,
            app_id: app_id.to_string(),
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

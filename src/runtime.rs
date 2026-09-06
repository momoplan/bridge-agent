use crate::config::{load_config, resolve_config_base_dir, AgentConfig};
use crate::event_server::LocalEventServer;
use crate::logging::{FileLogConfig, FileLogSink, LogEntry, LogMetadata};
use crate::power::SystemSleepPrevention;
use crate::process_identity::is_bridge_agent_process_name;
use crate::protocol::{
    AgentCapabilities, AgentMessage, EventAck, LocalAppEventEmitted,
    AGENT_PROTOCOL_FEATURE_LOCAL_APP_CAPABILITIES_V3, AGENT_PROTOCOL_FEATURE_LOCAL_APP_EVENTS_V2,
    AGENT_PROTOCOL_FEATURE_REGISTERED_ACK, AGENT_PROTOCOL_VERSION,
};
use crate::services::ServiceRegistry;
#[cfg(windows)]
use crate::windows_process::{
    inspect_windows_process, terminate_windows_process, windows_process_is_running,
};
use anyhow::{bail, Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{hash_map::Entry, HashMap, VecDeque};
use std::fs;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::{broadcast, mpsc, oneshot, watch, Mutex, RwLock};
use tokio::task::JoinHandle;
use tokio::time::{interval_at, sleep, timeout, Duration};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::{
    http::StatusCode, protocol::Message, Error as WebSocketError,
};
use tracing::{error, info, warn};
use url::Url;

const RELAY_KEEPALIVE_INTERVAL_SECS: u64 = 25;
const RELAY_HEARTBEAT_TIMEOUT_SECS: u64 = 75;
const RELAY_CONNECT_TIMEOUT_SECS: u64 = 15;
const LOCAL_EVENT_QUEUE_CAPACITY: usize = 1024;
const LOCAL_EVENT_SERVER_RETRY_INTERVAL_SECS: u64 = 2;
const RUNTIME_LOCK_DIR: &str = ".bridge-agent-locks";
const RETIRED_EVENT_OUTBOX_DIR: &str = "event-outbox";
const RUNTIME_STOP_TIMEOUT_SECS: u64 = 15;
const RUNTIME_ABORT_TIMEOUT_SECS: u64 = 2;
const DEFAULT_LOG_LIMIT: usize = 500;
const RELAY_SEEN_EVENT_INTERVAL_MS: u64 = 5_000;
const RELAY_AUTHORIZATION_REQUIRED_MESSAGE: &str = "Relay 授权已失效，请重新授权此设备后继续使用。";
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeStatus {
    Stopped,
    Starting,
    Connecting,
    Online,
    Backoff,
    AuthorizationRequired,
    Stopping,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeSnapshot {
    pub revision: u64,
    pub status: RuntimeStatus,
    pub config_path: Option<String>,
    pub agent_id: Option<String>,
    pub relay_url: Option<String>,
    pub relay_registered: bool,
    pub relay_registered_at: Option<u64>,
    pub last_relay_seen_at: Option<u64>,
    pub log_file_path: Option<String>,
    pub last_error: Option<String>,
    pub last_event_at: u64,
}

#[derive(Debug, Clone)]
pub enum RuntimeEvent {
    SnapshotChanged(RuntimeSnapshot),
    LogAppended(LogEntry),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeProcessInfo {
    pub pid: u32,
    pub parent_pid: Option<u32>,
    pub name: Option<String>,
    pub executable_path: Option<String>,
    pub command_line: Option<String>,
    pub running: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeLockConflict {
    pub pid: u32,
    pub agent_id: String,
    pub config_path: String,
    pub lock_path: String,
    pub process: RuntimeProcessInfo,
}

impl std::fmt::Display for RuntimeLockConflict {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "bridge-agent runtime is already running for agent `{}` with config `{}` (pid {}, lock {})",
            self.agent_id, self.config_path, self.pid, self.lock_path
        )
    }
}

impl std::error::Error for RuntimeLockConflict {}

#[derive(Clone, Default)]
pub struct AgentRuntimeManager {
    inner: Arc<RuntimeInner>,
}

struct RuntimeInner {
    lifecycle: Mutex<()>,
    state: Mutex<ManagedState>,
    logs: Mutex<VecDeque<LogEntry>>,
    file_log: Mutex<Option<FileLogSink>>,
    events: broadcast::Sender<RuntimeEvent>,
    next_log_sequence: AtomicU64,
}

impl Default for RuntimeInner {
    fn default() -> Self {
        let (events, _) = broadcast::channel(512);
        Self {
            lifecycle: Mutex::new(()),
            state: Mutex::new(ManagedState::default()),
            logs: Mutex::new(VecDeque::new()),
            file_log: Mutex::new(None),
            events,
            next_log_sequence: AtomicU64::new(0),
        }
    }
}

struct ManagedState {
    snapshot: RuntimeSnapshot,
    task: Option<JoinHandle<()>>,
    shutdown: Option<watch::Sender<bool>>,
    apply: Option<mpsc::UnboundedSender<RuntimeRegistryUpdate>>,
    last_relay_seen_event_at: Option<u64>,
}

pub(crate) struct RuntimeRegistryUpdate {
    pub(crate) registry: ServiceRegistry,
    pub(crate) services: Vec<crate::protocol::ServiceDefinition>,
    pub(crate) local_apps: Vec<crate::protocol::LocalAppDefinition>,
}

pub(crate) struct RuntimeAuditLog {
    pub(crate) level: String,
    pub(crate) message: String,
    pub(crate) metadata: LogMetadata,
}

pub(crate) struct LocalAppEventSubmission {
    pub(crate) event: LocalAppEventEmitted,
    pub(crate) response: oneshot::Sender<Result<EventAck, String>>,
}

type PendingEventKey = (String, String);
type PendingEventWaiters = HashMap<PendingEventKey, Vec<oneshot::Sender<Result<EventAck, String>>>>;

fn register_event_waiter(
    pending_events: &mut PendingEventWaiters,
    key: PendingEventKey,
    waiter: oneshot::Sender<Result<EventAck, String>>,
) -> bool {
    match pending_events.entry(key) {
        Entry::Vacant(entry) => {
            entry.insert(vec![waiter]);
            true
        }
        Entry::Occupied(mut entry) => {
            entry.get_mut().retain(|existing| !existing.is_closed());
            let should_forward = entry.get().is_empty();
            entry.get_mut().push(waiter);
            should_forward
        }
    }
}

fn prune_closed_event_waiters(pending_events: &mut PendingEventWaiters) {
    pending_events.retain(|_, waiters| {
        waiters.retain(|waiter| !waiter.is_closed());
        !waiters.is_empty()
    });
}

fn remove_retired_event_storage(config_base_dir: &Path) -> Result<()> {
    let path = config_base_dir.join(RETIRED_EVENT_OUTBOX_DIR);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(()),
        Err(err) => {
            return Err(err).with_context(|| {
                format!("failed to inspect retired event storage {}", path.display())
            });
        }
    };
    if metadata.file_type().is_dir() {
        fs::remove_dir_all(&path)
    } else {
        fs::remove_file(&path)
    }
    .with_context(|| format!("failed to remove retired event storage {}", path.display()))
}

impl Default for ManagedState {
    fn default() -> Self {
        Self {
            snapshot: RuntimeSnapshot {
                revision: 0,
                status: RuntimeStatus::Stopped,
                config_path: None,
                agent_id: None,
                relay_url: None,
                relay_registered: false,
                relay_registered_at: None,
                last_relay_seen_at: None,
                log_file_path: None,
                last_error: None,
                last_event_at: now_ms(),
            },
            task: None,
            shutdown: None,
            apply: None,
            last_relay_seen_event_at: None,
        }
    }
}

include!("runtime/manager.rs");
include!("runtime/event_server_runtime.rs");
include!("runtime/runner.rs");
include!("runtime/relay.rs");
include!("runtime/instance_lock.rs");
include!("runtime/process.rs");

#[cfg(test)]
mod tests {
    use super::{
        build_agent_url, command_line_starts_with_bridge_agent, decode_relay_message,
        is_relay_authorization_error, process_looks_like_bridge_agent, prune_closed_event_waiters,
        read_runtime_lock, register_event_waiter, remove_retired_event_storage,
        runtime_lock_owner_is_active, runtime_lock_path, runtime_start_is_active,
        terminate_runtime_lock_owner, AgentRuntimeManager, PendingEventWaiters, RuntimeEvent,
        RuntimeInstanceLock, RuntimeLockDocument, RuntimeProcessInfo, RuntimeStatus, StatusCode,
        WebSocketError, RELAY_AUTHORIZATION_REQUIRED_MESSAGE, RETIRED_EVENT_OUTBOX_DIR,
    };
    use crate::config::{AgentConfig, ServiceConfig, ServiceHealthCheck};
    use crate::logging::LogMetadata;
    use crate::protocol::AgentMessage;
    use futures_util::{SinkExt, StreamExt};
    use serde_json::{json, Value};
    use std::collections::BTreeMap;
    use std::fs;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    use tempfile::tempdir;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::sync::oneshot;
    use tokio_tungstenite::{accept_async, tungstenite::Message};

    include!("runtime/tests/core.rs");
    include!("runtime/tests/locks.rs");
    include!("runtime/tests/authorization.rs");
    include!("runtime/tests/startup.rs");
    include!("runtime/tests/process.rs");
}

//! Durable device-owned delivery. Relay is a transport, never the retry owner.
mod store;
#[cfg(test)]
mod tests;

use anyhow::{bail, Result};
use relay::contracts::{channel::DeviceEvent, device_events::*};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StoragePolicy {
    pub automatic_cleanup: bool,
    pub retention_days: u32,
    pub capacity_bytes: u64,
    pub diagnostic_retention_days: u32,
}
impl Default for StoragePolicy {
    fn default() -> Self {
        Self {
            automatic_cleanup: true,
            retention_days: 7,
            capacity_bytes: 256 * 1024 * 1024,
            diagnostic_retention_days: 7,
        }
    }
}
impl StoragePolicy {
    pub fn validate(&self) -> Result<()> {
        if !(1..=3650).contains(&self.retention_days)
            || !(1..=3650).contains(&self.diagnostic_retention_days)
        {
            bail!("event retention days must be between 1 and 3650");
        }
        if !(1024 * 1024..=100 * 1024 * 1024 * 1024).contains(&self.capacity_bytes) {
            bail!("event capacity must be between 1 MiB and 100 GiB");
        }
        Ok(())
    }
    fn cutoff(&self, now: i64) -> Option<i64> {
        self.automatic_cleanup
            .then(|| now.saturating_sub(i64::from(self.retention_days) * 86400))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct QueueIdentity {
    pub environment: String,
    pub workspace_id: u64,
    pub device_id: String,
}
impl QueueIdentity {
    /// Saved browser authorization is the identity source, including while Relay is offline.
    pub fn from_authorized_config(config: &crate::AgentConfig) -> Option<Self> {
        if config.relay.token.trim().is_empty() || config.relay.agent_id.trim().is_empty() {
            return None;
        }
        Self::from_config(config).ok()
    }

    pub fn from_config(config: &crate::AgentConfig) -> Result<Self> {
        Ok(Self {
            environment: config
                .platform
                .environment_key
                .clone()
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(|| anyhow::anyhow!("event storage requires an environment identity"))?,
            workspace_id: config
                .platform
                .workspace_id
                .filter(|id| *id > 0)
                .ok_or_else(|| anyhow::anyhow!("event storage requires a workspace identity"))?,
            device_id: config.relay.agent_id.clone(),
        })
    }
}

#[derive(Debug, Default, Serialize)]
pub struct StorageStats {
    pub pending_events: u64,
    pub logical_bytes: u64,
    pub physical_bytes: u64,
    pub oldest_received_at: Option<i64>,
    pub expiring_events: u64,
    pub expiring_bytes: u64,
    pub last_cleanup_at: Option<i64>,
    pub last_cleanup_expired: u64,
}

#[derive(Clone)]
pub struct EventQueue {
    inner: Arc<Mutex<Store>>,
}
struct Store {
    db: Connection,
    path: PathBuf,
    identity: QueueIdentity,
    scope: String,
    subscriptions: Option<DeviceSubscriptions>,
    last_maintenance: i64,
    subscription_received: std::time::Instant,
}
impl EventQueue {
    pub fn open(path: &Path, identity: QueueIdentity) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
            std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(false)
                .mode(0o600)
                .open(path)?;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        }
        let db = Connection::open(path)?;
        db.busy_timeout(std::time::Duration::from_secs(5))?;
        db.execute_batch("PRAGMA auto_vacuum=INCREMENTAL; PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA secure_delete=ON;
            CREATE TABLE IF NOT EXISTS events (
                scope TEXT NOT NULL, app_id TEXT NOT NULL, event_id TEXT NOT NULL,
                body TEXT NOT NULL, received_at INTEGER NOT NULL,
                routed INTEGER NOT NULL, next_attempt INTEGER NOT NULL, attempts INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY(scope,app_id,event_id));
            CREATE INDEX IF NOT EXISTS events_due ON events(scope,next_attempt);
            CREATE TABLE IF NOT EXISTS deliveries (
                scope TEXT NOT NULL, app_id TEXT NOT NULL, event_id TEXT NOT NULL, consumer_id TEXT NOT NULL,
                PRIMARY KEY(scope,app_id,event_id,consumer_id));
            CREATE TABLE IF NOT EXISTS diagnostics (
                id INTEGER PRIMARY KEY, app_id TEXT NOT NULL, event_id TEXT NOT NULL,
                outcome TEXT NOT NULL, finished_at INTEGER NOT NULL);
            CREATE INDEX IF NOT EXISTS events_retention ON events(received_at);
            CREATE INDEX IF NOT EXISTS diagnostics_retention ON diagnostics(finished_at);
            CREATE VIEW IF NOT EXISTS event_sizes AS SELECT e.*,
                length(CAST(body AS BLOB))+512+(SELECT COALESCE(SUM(length(CAST(d.consumer_id AS BLOB))+128),0) FROM deliveries d WHERE d.scope=e.scope AND d.app_id=e.app_id AND d.event_id=e.event_id) AS bytes
                FROM events e;
            CREATE TABLE IF NOT EXISTS cleanup (id INTEGER PRIMARY KEY CHECK(id=1), at INTEGER NOT NULL, expired INTEGER NOT NULL);")?;
        let scope = serde_json::to_string(&identity)?;
        Ok(Self {
            inner: Arc::new(Mutex::new(Store {
                db,
                path: path.into(),
                identity,
                scope,
                subscriptions: None,
                last_maintenance: 0,
                subscription_received: std::time::Instant::now(),
            })),
        })
    }
    /// All SQLite work runs off the async executor. Commit errors never produce acceptance.
    async fn run<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Store) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let inner = self.inner.clone();
        tokio::task::spawn_blocking(move || {
            let mut store = inner
                .lock()
                .map_err(|_| anyhow::anyhow!("event storage lock poisoned"))?;
            f(&mut store)
        })
        .await?
    }
    pub async fn admit(
        &self,
        event: LocalAppEventEmitted,
        policy: StoragePolicy,
    ) -> Result<LocalEventAccepted> {
        self.run(move |store| store.admit(event, &policy, now()))
            .await
    }
    pub async fn subscriptions(&self, snapshot: Option<DeviceSubscriptions>) -> Result<()> {
        self.run(move |store| {
            if let Some(snapshot) = &snapshot {
                if snapshot.workspace_id != store.identity.workspace_id
                    || snapshot.device_id != store.identity.device_id
                {
                    bail!("subscription snapshot belongs to another device");
                }
                let mut ids = std::collections::HashSet::new();
                for group in &snapshot.groups {
                    if !relay::routing::valid_identity(&group.consumer_id)
                        || !ids.insert(&group.consumer_id)
                    {
                        bail!("invalid or duplicate subscription group");
                    }
                    for selector in &group.subscriptions {
                        selector.validate().map_err(anyhow::Error::msg)?;
                    }
                }
            }
            store.subscriptions = snapshot;
            store.subscription_received = std::time::Instant::now();
            Ok(())
        })
        .await
    }
    pub async fn due(
        &self,
        policy: StoragePolicy,
        limit: usize,
    ) -> Result<Vec<LocalAppEventEmitted>> {
        self.run(move |store| store.due(&policy, now(), limit))
            .await
    }
    pub async fn acknowledge(&self, ack: EventAck) -> Result<()> {
        self.run(move |store| store.acknowledge(ack, now())).await
    }
    pub async fn statistics(&self, policy: StoragePolicy) -> Result<StorageStats> {
        self.run(move |store| store.statistics(&policy, now()))
            .await
    }
    pub async fn cleanup(&self, policy: StoragePolicy) -> Result<()> {
        self.run(move |store| store.cleanup(&policy, now())).await
    }
}
pub fn database_path(config_path: &Path) -> Result<PathBuf> {
    Ok(crate::config::resolve_config_base_dir(config_path)
        .join("event-delivery")
        .join("events.sqlite3"))
}
fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn targets(snapshot: &DeviceSubscriptions, event: &LocalAppEventEmitted) -> Vec<String> {
    let envelope = DeviceEvent {
        workspace_id: snapshot.workspace_id,
        device_id: snapshot.device_id.clone(),
        event_id: event.event_id.clone(),
        app_id: event.app_id.clone(),
        event: event.event.clone(),
        payload: event.payload.clone(),
        occurred_at: event.occurred_at.clone(),
    };
    snapshot
        .groups
        .iter()
        .filter(|g| g.subscriptions.iter().any(|s| s.matches(&envelope)))
        .map(|g| g.consumer_id.clone())
        .collect()
}

/// Read only the public storage policy; event ingestion must never fetch device secrets.
pub async fn read_policy(path: &Path) -> Result<StoragePolicy> {
    #[derive(Deserialize)]
    struct Document {
        runtime: Runtime,
    }
    #[derive(Deserialize)]
    struct Runtime {
        #[serde(default)]
        event_delivery: StoragePolicy,
    }
    let document: Document = serde_json::from_slice(&tokio::fs::read(path).await?)?;
    document.runtime.event_delivery.validate()?;
    Ok(document.runtime.event_delivery)
}

pub use relay::contracts::device_events::LOCAL_EVENT_HANDOFF_CAPABILITY;

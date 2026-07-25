use crate::protocol::LocalAppEventEmitted;
use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

const EVENT_OUTBOX_DIR: &str = "event-outbox";

#[derive(Debug, Clone)]
pub(crate) struct EventOutbox {
    dir: PathBuf,
}

impl EventOutbox {
    pub(crate) fn new(config_base_dir: &Path) -> Result<Self> {
        let dir = config_base_dir.join(EVENT_OUTBOX_DIR);
        fs::create_dir_all(&dir)
            .with_context(|| format!("failed to create event outbox {}", dir.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
        }
        Ok(Self { dir })
    }

    pub(crate) fn enqueue(&self, event: &LocalAppEventEmitted) -> Result<()> {
        let destination = self.event_path(&event.event_id);
        if destination.exists() {
            return Ok(());
        }
        let temporary = destination.with_extension(format!("json.tmp-{}", std::process::id()));
        let bytes = serde_json::to_vec(event)?;
        fs::write(&temporary, bytes)
            .with_context(|| format!("failed to write event outbox {}", temporary.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600))?;
        }
        fs::rename(&temporary, &destination)
            .with_context(|| format!("failed to commit event outbox {}", destination.display()))?;
        Ok(())
    }

    pub(crate) fn pending(&self) -> Result<Vec<LocalAppEventEmitted>> {
        let mut paths = fs::read_dir(&self.dir)
            .with_context(|| format!("failed to read event outbox {}", self.dir.display()))?
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("json"))
            .collect::<Vec<_>>();
        paths.sort();

        paths
            .into_iter()
            .map(|path| {
                let content = fs::read(&path)
                    .with_context(|| format!("failed to read event outbox {}", path.display()))?;
                serde_json::from_slice(&content)
                    .with_context(|| format!("failed to parse event outbox {}", path.display()))
            })
            .collect()
    }

    pub(crate) fn acknowledge(&self, event_id: &str) -> Result<bool> {
        let path = self.event_path(event_id);
        match fs::remove_file(&path) {
            Ok(()) => Ok(true),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(err) => {
                Err(err).with_context(|| format!("failed to acknowledge event {}", path.display()))
            }
        }
    }

    fn event_path(&self, event_id: &str) -> PathBuf {
        let digest = Sha256::digest(event_id.as_bytes());
        self.dir.join(format!("{digest:x}.json"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn outbox_persists_and_acknowledges_events() {
        let dir = tempfile::tempdir().unwrap();
        let outbox = EventOutbox::new(dir.path()).unwrap();
        let event = LocalAppEventEmitted {
            event_id: "evt-1".to_string(),
            connector_id: "com.baijimu.connector.test".to_string(),
            installation_id: "lai-test".to_string(),
            event: "message.received".to_string(),
            payload: json!({"ok": true}),
            occurred_at: None,
        };

        outbox.enqueue(&event).unwrap();
        outbox.enqueue(&event).unwrap();
        assert_eq!(outbox.pending().unwrap().len(), 1);
        assert!(outbox.acknowledge("evt-1").unwrap());
        assert!(!outbox.acknowledge("evt-1").unwrap());
        assert!(outbox.pending().unwrap().is_empty());
    }
}

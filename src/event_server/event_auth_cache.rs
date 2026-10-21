use crate::connector::{
    authorize_connector_event, connector_event_token_path, constant_time_text_eq,
    install_record_path,
};
use crate::event_delivery::{read_policy, StoragePolicy};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::sync::Mutex;
use std::time::SystemTime;

struct EventAuthCache {
    inner: Mutex<CacheState>,
}

struct CacheState {
    config_stamp: Option<FileStamp>,
    policy: Option<StoragePolicy>,
    apps: HashMap<String, CachedApp>,
}

#[derive(Clone, PartialEq, Eq)]
struct FileStamp {
    modified: Option<SystemTime>,
    len: u64,
}

struct CachedApp {
    token_stamp: FileStamp,
    record_stamp: FileStamp,
    token: String,
    events: HashSet<String>,
}

enum AuthLookup {
    Authorized(StoragePolicy),
    Rejected(String),
    Miss,
}

enum EventAuthFailure {
    Forbidden(anyhow::Error),
    Unavailable(anyhow::Error),
}

impl EventAuthCache {
    fn new() -> Self {
        Self {
            inner: Mutex::new(CacheState {
                config_stamp: None,
                policy: None,
                apps: HashMap::new(),
            }),
        }
    }

    fn invalidate(&self) {
        let mut state = self.lock();
        state.config_stamp = None;
        state.policy = None;
        state.apps.clear();
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, CacheState> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    async fn authorize(
        &self,
        config_path: &Path,
        app_id: &str,
        event_name: &str,
        token: &str,
    ) -> Result<StoragePolicy, EventAuthFailure> {
        let Some(config_stamp) = file_stamp(config_path) else {
            return Err(EventAuthFailure::Forbidden(anyhow::anyhow!(
                "failed to read config {}",
                config_path.display()
            )));
        };
        let token_path = connector_event_token_path(app_id).map_err(EventAuthFailure::Forbidden)?;
        let record_path = install_record_path(app_id).map_err(EventAuthFailure::Forbidden)?;
        match self.lookup(
            &config_stamp,
            app_id,
            event_name,
            token,
            file_stamp(&token_path).as_ref(),
            file_stamp(&record_path).as_ref(),
        ) {
            AuthLookup::Authorized(policy) => return Ok(policy),
            AuthLookup::Rejected(message) => {
                return Err(EventAuthFailure::Forbidden(anyhow::anyhow!(message)))
            }
            AuthLookup::Miss => {}
        }

        authorize_connector_event(config_path, app_id, event_name, token)
            .map_err(EventAuthFailure::Forbidden)?;
        let policy = read_policy(config_path)
            .await
            .map_err(EventAuthFailure::Unavailable)?;
        self.remember(
            config_path,
            &config_stamp,
            app_id,
            &token_path,
            &record_path,
            &policy,
        );
        Ok(policy)
    }

    fn lookup(
        &self,
        config_stamp: &FileStamp,
        app_id: &str,
        event_name: &str,
        token: &str,
        token_stamp: Option<&FileStamp>,
        record_stamp: Option<&FileStamp>,
    ) -> AuthLookup {
        let state = self.lock();
        if state.config_stamp.as_ref() != Some(config_stamp) {
            return AuthLookup::Miss;
        }
        let Some(policy) = state.policy.clone() else {
            return AuthLookup::Miss;
        };
        let (Some(token_stamp), Some(record_stamp)) = (token_stamp, record_stamp) else {
            return AuthLookup::Miss;
        };
        let Some(app) = state.apps.get(app_id) else {
            return AuthLookup::Miss;
        };
        if &app.token_stamp != token_stamp || &app.record_stamp != record_stamp {
            return AuthLookup::Miss;
        }
        if !constant_time_text_eq(&app.token, token.trim()) {
            return AuthLookup::Rejected("invalid connector event credential".to_string());
        }
        let event_name = event_name.trim();
        if !app.events.contains(event_name) {
            return AuthLookup::Rejected(format!(
                "local app event `{event_name}` is not declared by connector `{app_id}`"
            ));
        }
        AuthLookup::Authorized(policy)
    }

    fn remember(
        &self,
        config_path: &Path,
        config_stamp: &FileStamp,
        app_id: &str,
        token_path: &Path,
        record_path: &Path,
        policy: &StoragePolicy,
    ) {
        let Ok(token) = fs::read_to_string(token_path) else {
            return;
        };
        let Ok(config) = crate::config::load_config(config_path) else {
            return;
        };
        let (Some(token_stamp), Some(record_stamp)) =
            (file_stamp(token_path), file_stamp(record_path))
        else {
            return;
        };
        let events = declared_events(&config, app_id);
        let mut state = self.lock();
        if state.config_stamp.as_ref() != Some(config_stamp) {
            state.apps.clear();
            state.config_stamp = Some(config_stamp.clone());
            state.policy = Some(policy.clone());
        } else if state.policy.is_none() {
            state.policy = Some(policy.clone());
        }
        state.apps.insert(
            app_id.to_string(),
            CachedApp {
                token_stamp,
                record_stamp,
                token: token.trim().to_string(),
                events,
            },
        );
    }

    #[cfg(test)]
    fn insert_for_test(
        &self,
        config_stamp: FileStamp,
        policy: StoragePolicy,
        app_id: &str,
        app: CachedApp,
    ) {
        let mut state = self.lock();
        state.config_stamp = Some(config_stamp);
        state.policy = Some(policy);
        state.apps.insert(app_id.to_string(), app);
    }
}

fn declared_events(config: &crate::config::AgentConfig, app_id: &str) -> HashSet<String> {
    config
        .local_apps
        .iter()
        .find(|app| app.app_id == app_id && app.enabled)
        .map(|app| {
            app.events
                .iter()
                .filter(|event| event.enabled)
                .map(|event| event.name.clone())
                .collect()
        })
        .unwrap_or_default()
}

fn file_stamp(path: &Path) -> Option<FileStamp> {
    let metadata = fs::metadata(path).ok()?;
    Some(FileStamp {
        modified: metadata.modified().ok(),
        len: metadata.len(),
    })
}

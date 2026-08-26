use super::*;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct StartupComponentHealth {
    pub(super) id: String,
    pub(super) label: String,
    pub(super) status: String,
    pub(super) detail: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct StartupHealthSnapshot {
    pub(super) revision: u64,
    pub(super) safe_mode: bool,
    pub(super) forced_safe_mode: bool,
    pub(super) consecutive_failures: u32,
    pub(super) frontend_ready: bool,
    pub(super) startup_log_path: String,
    pub(super) components: Vec<StartupComponentHealth>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PersistentStartupState {
    pub(super) pending: bool,
    pub(super) consecutive_failures: u32,
    pub(super) version: Option<String>,
    pub(super) started_at_ms: Option<u64>,
    pub(super) ready_at_ms: Option<u64>,
}

#[derive(Clone)]
pub(super) struct StartupHealthManager {
    pub(super) inner: Arc<Mutex<StartupHealthSnapshot>>,
    pub(super) event_app: Arc<Mutex<Option<tauri::AppHandle>>>,
    pub(super) state_path: PathBuf,
    pub(super) diagnostics: StartupDiagnostics,
}

impl StartupHealthManager {
    pub(super) fn new(config_path: &Path, diagnostics: StartupDiagnostics) -> Self {
        let base_dir = resolve_config_base_dir(config_path);
        Self {
            inner: Arc::new(Mutex::new(StartupHealthSnapshot {
                revision: 0,
                safe_mode: false,
                forced_safe_mode: false,
                consecutive_failures: 0,
                frontend_ready: false,
                startup_log_path: diagnostics.primary_path.display().to_string(),
                components: Vec::new(),
            })),
            event_app: Arc::new(Mutex::new(None)),
            state_path: base_dir.join(STARTUP_STATE_FILE_NAME),
            diagnostics,
        }
    }

    pub(super) fn begin_primary(&self, forced_safe_mode: bool, bootstrap_failure: Option<String>) {
        let previous = fs::read(&self.state_path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<PersistentStartupState>(&bytes).ok())
            .unwrap_or_default();
        let consecutive_failures = if previous.pending {
            previous.consecutive_failures.saturating_add(1)
        } else {
            0
        };
        let safe_mode = forced_safe_mode
            || bootstrap_failure.is_some()
            || consecutive_failures >= SAFE_MODE_FAILURE_THRESHOLD;
        {
            let mut health = match self.inner.lock() {
                Ok(health) => health,
                Err(poisoned) => poisoned.into_inner(),
            };
            *health = StartupHealthSnapshot {
                revision: 0,
                safe_mode,
                forced_safe_mode,
                consecutive_failures,
                frontend_ready: false,
                startup_log_path: self.diagnostics.primary_path.display().to_string(),
                components: Vec::new(),
            };
        }
        self.set_component("desktop_shell", "桌面基础壳", "starting", None);
        if let Some(detail) = bootstrap_failure {
            self.set_component("configuration_path", "配置目录", "degraded", Some(detail));
        } else {
            self.set_component("configuration_path", "配置目录", "ready", None);
        }
        if safe_mode {
            self.diagnostics.warn(format!(
                "safe mode enabled: forced={forced_safe_mode} consecutive_failures={consecutive_failures}"
            ));
        }
        self.write_pending_state();
    }

    pub(super) fn snapshot(&self) -> StartupHealthSnapshot {
        match self.inner.lock() {
            Ok(health) => health.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    pub(super) fn safe_mode(&self) -> bool {
        match self.inner.lock() {
            Ok(health) => health.safe_mode,
            Err(poisoned) => poisoned.into_inner().safe_mode,
        }
    }

    pub(super) fn attach_event_app(&self, app: tauri::AppHandle) {
        match self.event_app.lock() {
            Ok(mut target) => *target = Some(app),
            Err(poisoned) => *poisoned.into_inner() = Some(app),
        }
        self.emit_snapshot(self.snapshot());
    }

    pub(super) fn emit_snapshot(&self, snapshot: StartupHealthSnapshot) {
        let app = match self.event_app.lock() {
            Ok(target) => target.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        };
        if let Some(app) = app {
            let _ = app.emit(STARTUP_HEALTH_EVENT, snapshot);
        }
    }

    pub(super) fn set_component(
        &self,
        id: &str,
        label: &str,
        status: &str,
        detail: Option<String>,
    ) {
        let snapshot = {
            let mut health = match self.inner.lock() {
                Ok(health) => health,
                Err(poisoned) => poisoned.into_inner(),
            };
            if let Some(component) = health.components.iter_mut().find(|item| item.id == id) {
                component.label = label.to_string();
                component.status = status.to_string();
                component.detail = detail;
            } else {
                health.components.push(StartupComponentHealth {
                    id: id.to_string(),
                    label: label.to_string(),
                    status: status.to_string(),
                    detail,
                });
            }
            health.revision = health.revision.saturating_add(1);
            health.clone()
        };
        self.emit_snapshot(snapshot);
    }

    pub(super) fn mark_frontend_ready(&self) -> Result<StartupHealthSnapshot, String> {
        {
            let mut health = self
                .inner
                .lock()
                .map_err(|_| "启动状态锁已损坏".to_string())?;
            health.frontend_ready = true;
            if let Some(component) = health
                .components
                .iter_mut()
                .find(|item| item.id == "desktop_shell")
            {
                component.status = "ready".to_string();
                component.detail = None;
            }
            health.revision = health.revision.saturating_add(1);
        }
        let snapshot = self.snapshot();
        self.emit_snapshot(snapshot.clone());
        self.write_ready_state()?;
        self.diagnostics
            .info("frontend readiness handshake completed");
        Ok(snapshot)
    }

    pub(super) fn reset_for_normal_restart(&self) -> Result<(), String> {
        write_startup_state(
            &self.state_path,
            &PersistentStartupState {
                pending: false,
                consecutive_failures: 0,
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
                started_at_ms: None,
                ready_at_ms: Some(now_ms()),
            },
        )
    }

    pub(super) fn write_pending_state(&self) {
        let snapshot = self.snapshot();
        if let Err(err) = write_startup_state(
            &self.state_path,
            &PersistentStartupState {
                pending: true,
                consecutive_failures: snapshot.consecutive_failures,
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
                started_at_ms: Some(now_ms()),
                ready_at_ms: None,
            },
        ) {
            self.diagnostics
                .warn(format!("failed to persist startup pending state: {err}"));
        }
    }

    pub(super) fn write_ready_state(&self) -> Result<(), String> {
        write_startup_state(
            &self.state_path,
            &PersistentStartupState {
                pending: false,
                consecutive_failures: 0,
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
                started_at_ms: None,
                ready_at_ms: Some(now_ms()),
            },
        )
    }
}

pub(super) fn write_startup_state(
    path: &Path,
    state: &PersistentStartupState,
) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| format!("创建启动状态目录失败: {err}"))?;
    }
    let temporary_path = path.with_extension("json.tmp");
    let bytes =
        serde_json::to_vec_pretty(state).map_err(|err| format!("序列化启动状态失败: {err}"))?;
    fs::write(&temporary_path, bytes).map_err(|err| format!("写入启动状态失败: {err}"))?;
    match fs::rename(&temporary_path, path) {
        Ok(()) => Ok(()),
        Err(first_err) if path.exists() => {
            fs::remove_file(path)
                .map_err(|err| format!("替换启动状态失败（rename: {first_err}; remove: {err}）"))?;
            fs::rename(&temporary_path, path).map_err(|err| format!("提交启动状态失败: {err}"))
        }
        Err(err) => Err(format!("提交启动状态失败: {err}")),
    }
}

#[tauri::command]
pub(super) fn get_startup_health(state: tauri::State<'_, DesktopState>) -> StartupHealthSnapshot {
    state.startup_health.snapshot()
}

#[tauri::command]
pub(super) fn mark_frontend_ready(
    state: tauri::State<'_, DesktopState>,
) -> Result<StartupHealthSnapshot, String> {
    state.startup_health.mark_frontend_ready()
}

#[tauri::command]
pub(super) fn restart_in_normal_mode(
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopState>,
) -> Result<(), String> {
    if state.startup_health.snapshot().forced_safe_mode {
        return Err("当前进程由 --safe-mode 参数启动，请移除该参数后重新启动应用".to_string());
    }
    state.startup_health.reset_for_normal_restart()?;
    state
        .startup_health
        .diagnostics
        .info("normal mode restart requested from recovery UI");
    request_interactive_restart(&state.config_path)?;
    app.restart();
}

#[tauri::command]
pub(super) fn open_startup_log(state: tauri::State<'_, DesktopState>) -> Result<(), String> {
    let path = state.startup_health.snapshot().startup_log_path;
    open::that(path).map_err(|err| format!("打开启动日志失败: {err}"))
}

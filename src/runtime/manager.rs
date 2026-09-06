impl AgentRuntimeManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<RuntimeEvent> {
        self.inner.events.subscribe()
    }

    pub async fn push_desktop_log(&self, level: &str, message: &str, metadata: LogMetadata) {
        push_log_entry(&self.inner, 500, level, message, metadata).await;
    }

    pub async fn start_from_path(&self, path: &Path) -> Result<RuntimeSnapshot> {
        let config = load_config(path)?;
        self.start(config, path).await
    }

    pub async fn start(&self, config: AgentConfig, config_path: &Path) -> Result<RuntimeSnapshot> {
        let _lifecycle = self.inner.lifecycle.lock().await;
        if let Some(snapshot) = self.active_start_snapshot(&config, config_path).await {
            return Ok(snapshot);
        }
        self.stop_if_running().await?;
        let log_limit = config.runtime.log_limit;
        let config_base_dir = resolve_config_base_dir(config_path);
        remove_retired_event_storage(&config_base_dir)?;
        let runtime_lock = RuntimeInstanceLock::acquire(config_path, &config.relay.agent_id)?;
        let file_log = create_runtime_file_log(&config, &config_base_dir)?;
        let log_file_path = file_log
            .as_ref()
            .map(|sink| sink.path().display().to_string());
        let ws_url = build_agent_url(
            &config.relay.url,
            &config.relay.agent_id,
            &config.relay.token,
        )?;
        let snapshot = RuntimeSnapshot {
            revision: 0,
            status: RuntimeStatus::Starting,
            config_path: Some(config_path.display().to_string()),
            agent_id: Some(config.relay.agent_id.clone()),
            relay_url: Some(config.relay.url.clone()),
            relay_registered: false,
            relay_registered_at: None,
            last_relay_seen_at: None,
            log_file_path: log_file_path.clone(),
            last_error: None,
            last_event_at: now_ms(),
        };

        let snapshot_event = {
            let mut state = self.inner.state.lock().await;
            let mut snapshot = snapshot;
            snapshot.revision = state.snapshot.revision.saturating_add(1);
            state.snapshot = snapshot.clone();
            state.last_relay_seen_event_at = None;
            snapshot
        };
        publish_runtime_event(&self.inner, RuntimeEvent::SnapshotChanged(snapshot_event));
        {
            let mut active_file_log = self.inner.file_log.lock().await;
            *active_file_log = file_log;
        }
        self.push_log("info", "runtime starting", log_limit).await;

        let registry = match ServiceRegistry::from_config_initial(&config, &config_base_dir) {
            Ok(registry) => Arc::new(RwLock::new(registry)),
            Err(err) => {
                let message = format!("initial runtime preparation failed: {err:#}");
                self.force_stopped_with_error(message.clone()).await;
                self.push_log("error", &message, log_limit).await;
                return Err(err);
            }
        };

        let handles = spawn_runtime_task(
            Arc::clone(&self.inner),
            config,
            config_path,
            log_limit,
            runtime_lock,
            ws_url,
            registry,
        );

        let mut state = self.inner.state.lock().await;
        state.shutdown = Some(handles.shutdown);
        state.apply = Some(handles.apply);
        state.task = Some(handles.task);
        let snapshot = state.snapshot.clone();
        drop(state);

        let refresh_manager = self.clone();
        let refresh_path = config_path.to_path_buf();
        tokio::spawn(async move {
            if let Err(err) = refresh_manager
                .apply_capabilities_from_path(&refresh_path)
                .await
            {
                refresh_manager
                    .push_log(
                        "warn",
                        &format!("post-connect capability readiness refresh failed: {err:#}"),
                        log_limit,
                    )
                    .await;
            }
        });

        Ok(snapshot)
    }

    pub async fn apply_capabilities_from_path(&self, path: &Path) -> Result<RuntimeSnapshot> {
        let config = load_config(path)?;
        let config_base_dir = resolve_config_base_dir(path);
        let registry = ServiceRegistry::from_config_checked(&config, &config_base_dir).await?;
        let services = registry.definitions();
        let local_apps = registry.local_app_definitions();
        let update = RuntimeRegistryUpdate {
            registry,
            services,
            local_apps,
        };

        let snapshot = {
            let mut state = self.inner.state.lock().await;
            if state.snapshot.status == RuntimeStatus::Stopped {
                return Ok(state.snapshot.clone());
            }
            let apply = state
                .apply
                .as_ref()
                .context("runtime is running but cannot accept config updates")?
                .clone();
            apply
                .send(update)
                .context("failed to send runtime config update")?;
            state.snapshot.revision = state.snapshot.revision.saturating_add(1);
            state.snapshot.last_event_at = now_ms();
            state.snapshot.clone()
        };
        publish_runtime_event(&self.inner, RuntimeEvent::SnapshotChanged(snapshot.clone()));

        self.push_log(
            "info",
            "runtime capabilities update scheduled",
            config.runtime.log_limit,
        )
        .await;
        Ok(snapshot)
    }

    pub async fn stop(&self) -> Result<RuntimeSnapshot> {
        let _lifecycle = self.inner.lifecycle.lock().await;
        self.stop_if_running().await?;
        Ok(self.snapshot().await)
    }

    pub async fn snapshot(&self) -> RuntimeSnapshot {
        self.inner.state.lock().await.snapshot.clone()
    }

    pub async fn logs(&self, limit: usize) -> Vec<LogEntry> {
        let logs = self.inner.logs.lock().await;
        logs.iter()
            .rev()
            .take(limit)
            .cloned()
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect()
    }

    pub async fn clear_logs(&self) -> u64 {
        let mut logs = self.inner.logs.lock().await;
        let cleared_through = self.inner.next_log_sequence.load(Ordering::Relaxed);
        logs.clear();
        let file_log = self.inner.file_log.lock().await.clone();
        if let Some(file_log) = file_log {
            if let Err(err) = file_log.clear() {
                warn!("failed to clear file log: {err:#}");
            }
        }
        cleared_through
    }

    async fn stop_if_running(&self) -> Result<()> {
        let (shutdown, task, apply, snapshot) = {
            let mut state = self.inner.state.lock().await;
            if state.snapshot.status == RuntimeStatus::Stopped {
                return Ok(());
            }
            state.snapshot.revision = state.snapshot.revision.saturating_add(1);
            state.snapshot.status = RuntimeStatus::Stopping;
            state.snapshot.last_event_at = now_ms();
            (
                state.shutdown.take(),
                state.task.take(),
                state.apply.take(),
                state.snapshot.clone(),
            )
        };
        publish_runtime_event(&self.inner, RuntimeEvent::SnapshotChanged(snapshot));

        if let Some(shutdown) = shutdown {
            let _ = shutdown.send(true);
        }
        drop(apply);
        if let Some(task) = task {
            self.wait_for_runtime_stop(task).await;
        }
        Ok(())
    }

    async fn wait_for_runtime_stop(&self, mut task: JoinHandle<()>) {
        match timeout(Duration::from_secs(RUNTIME_STOP_TIMEOUT_SECS), &mut task).await {
            Ok(Ok(())) => {}
            Ok(Err(err)) => {
                let message = format!("runtime task ended before stop completed: {err:#}");
                self.force_stopped_with_error(message.clone()).await;
                self.push_log("error", &message, DEFAULT_LOG_LIMIT).await;
            }
            Err(_) => {
                let message = format!(
                    "runtime stop timed out after {RUNTIME_STOP_TIMEOUT_SECS}s; aborting runtime task"
                );
                self.push_log("warn", &message, DEFAULT_LOG_LIMIT).await;
                task.abort();
                match timeout(Duration::from_secs(RUNTIME_ABORT_TIMEOUT_SECS), task).await {
                    Ok(Ok(())) => {
                        self.push_log(
                            "warn",
                            "runtime task aborted after stop timeout",
                            DEFAULT_LOG_LIMIT,
                        )
                        .await;
                    }
                    Ok(Err(err)) if err.is_cancelled() => {
                        self.push_log(
                            "warn",
                            "runtime task cancelled after stop timeout",
                            DEFAULT_LOG_LIMIT,
                        )
                        .await;
                    }
                    Ok(Err(err)) => {
                        self.push_log(
                            "error",
                            &format!(
                                "runtime task failed while aborting after stop timeout: {err:#}"
                            ),
                            DEFAULT_LOG_LIMIT,
                        )
                        .await;
                    }
                    Err(_) => {
                        self.push_log(
                            "error",
                            &format!(
                                "runtime task did not finish within {RUNTIME_ABORT_TIMEOUT_SECS}s after abort"
                            ),
                            DEFAULT_LOG_LIMIT,
                        )
                        .await;
                    }
                }
                self.force_stopped_with_error(message).await;
            }
        }
    }

    async fn force_stopped_with_error(&self, last_error: String) {
        let snapshot = {
            let mut state = self.inner.state.lock().await;
            state.snapshot.status = RuntimeStatus::Stopped;
            state.snapshot.relay_registered = false;
            state.snapshot.relay_registered_at = None;
            state.snapshot.last_relay_seen_at = None;
            state.snapshot.last_error = Some(last_error);
            state.snapshot.revision = state.snapshot.revision.saturating_add(1);
            state.snapshot.last_event_at = now_ms();
            state.last_relay_seen_event_at = None;
            state.snapshot.clone()
        };
        publish_runtime_event(&self.inner, RuntimeEvent::SnapshotChanged(snapshot));
    }

    async fn active_start_snapshot(
        &self,
        config: &AgentConfig,
        config_path: &Path,
    ) -> Option<RuntimeSnapshot> {
        let state = self.inner.state.lock().await;
        let snapshot = &state.snapshot;
        if !runtime_start_is_active(snapshot.status) {
            return None;
        }
        if snapshot.agent_id.as_deref() != Some(config.relay.agent_id.as_str()) {
            return None;
        }
        if snapshot.config_path.as_deref() != Some(&config_path.display().to_string()) {
            return None;
        }
        Some(snapshot.clone())
    }

    async fn push_log(&self, level: &str, message: &str, limit: usize) {
        push_log_entry(&self.inner, limit, level, message, LogMetadata::default()).await;
    }
}

struct RuntimeTaskHandles {
    shutdown: watch::Sender<bool>,
    apply: mpsc::UnboundedSender<RuntimeRegistryUpdate>,
    task: JoinHandle<()>,
}

fn create_runtime_file_log(config: &AgentConfig, config_base_dir: &Path) -> Result<Option<FileLogSink>> {
    FileLogSink::from_config(
        &FileLogConfig {
            enabled: config.runtime.log_file_enabled,
            dir: config.runtime.log_file_dir.as_ref().map(PathBuf::from),
            max_bytes: config.runtime.log_file_max_bytes,
            max_files: config.runtime.log_file_max_files,
        },
        config_base_dir,
    )
}

fn spawn_runtime_task(
    inner: Arc<RuntimeInner>,
    config: AgentConfig,
    config_path: &Path,
    log_limit: usize,
    runtime_lock: RuntimeInstanceLock,
    ws_url: Url,
    registry: Arc<RwLock<ServiceRegistry>>,
) -> RuntimeTaskHandles {
    let (shutdown, shutdown_rx) = watch::channel(false);
    let (apply, apply_rx) = mpsc::unbounded_channel();
    let (event_tx, event_rx) = mpsc::channel(LOCAL_EVENT_QUEUE_CAPACITY);
    let (audit_tx, audit_rx) = mpsc::unbounded_channel();
    let event_server = LocalEventServerRuntime {
        inner: Arc::clone(&inner),
        log_limit,
        config: config.clone(),
        config_path: config_path.to_path_buf(),
        registry: Arc::clone(&registry),
        event_tx,
        apply_tx: apply.clone(),
        audit_tx,
    };
    let runner = RuntimeRunner {
        inner: Arc::clone(&inner),
        log_limit,
        config,
        config_path: config_path.display().to_string(),
        ws_url,
        registry,
    };
    let task = tokio::spawn(
        RuntimeTask {
            inner,
            log_limit,
            _runtime_lock: runtime_lock,
            event_server,
            runner,
            shutdown_rx,
            apply_rx,
            event_rx,
            audit_rx,
        }
        .run(),
    );
    RuntimeTaskHandles { shutdown, apply, task }
}

struct RuntimeTask {
    inner: Arc<RuntimeInner>,
    log_limit: usize,
    _runtime_lock: RuntimeInstanceLock,
    event_server: LocalEventServerRuntime,
    runner: RuntimeRunner,
    shutdown_rx: watch::Receiver<bool>,
    apply_rx: mpsc::UnboundedReceiver<RuntimeRegistryUpdate>,
    event_rx: mpsc::Receiver<LocalAppEventSubmission>,
    audit_rx: mpsc::UnboundedReceiver<RuntimeAuditLog>,
}

impl RuntimeTask {
    async fn run(mut self) {
        let system_sleep_prevention = acquire_system_sleep_prevention(&self.inner, self.log_limit).await;
        let event_server_task = tokio::spawn(self.event_server.run(self.shutdown_rx.clone()));
        if let Err(err) = self.runner.run(
            self.shutdown_rx,
            &mut self.apply_rx,
            &mut self.event_rx,
            &mut self.audit_rx,
        ).await {
            self.runner.update_snapshot(
                RuntimeStatus::Stopped,
                Some(err.to_string()),
                self.runner.config.relay.agent_id.clone(),
                self.runner.config.relay.url.clone(),
                self.runner.config_path.clone(),
            ).await;
            self.runner.push_log("error", &format!("runtime stopped with error: {err:#}")).await;
        }
        if !event_server_task.is_finished() {
            event_server_task.abort();
        }
        let _ = event_server_task.await;
        if system_sleep_prevention.as_ref().is_some_and(SystemSleepPrevention::is_active) {
            push_log_entry(
                &self.inner,
                self.log_limit,
                "info",
                "system idle sleep prevention released",
                LogMetadata::category("power").outcome("released"),
            ).await;
        }
    }
}

async fn acquire_system_sleep_prevention(
    inner: &Arc<RuntimeInner>,
    log_limit: usize,
) -> Option<SystemSleepPrevention> {
    match SystemSleepPrevention::acquire("百积木保持连接在线") {
        Ok(assertion) => {
            if assertion.is_active() {
                push_log_entry(
                    inner,
                    log_limit,
                    "info",
                    "system idle sleep prevention enabled while runtime is active",
                    LogMetadata::category("power").outcome("enabled"),
                ).await;
            }
            Some(assertion)
        }
        Err(err) => {
            push_log_entry(
                inner,
                log_limit,
                "warn",
                &format!("failed to enable system idle sleep prevention: {err:#}"),
                LogMetadata::category("power").outcome("failed"),
            ).await;
            None
        }
    }
}

impl ShellExecutionStore {
    async fn start(&self, prepared: PreparedShellExec) -> ShellExecutionSnapshot {
        let execution_id = Uuid::new_v4().to_string();
        let started_at_epoch_ms = current_epoch_ms();
        let (cancel_tx, cancel_rx) = oneshot::channel();
        let record = ShellExecutionRecord {
            execution_id: execution_id.clone(),
            command: prepared.command_args.clone(),
            cwd: prepared.cwd.display().to_string(),
            status: ShellExecutionStatus::Running,
            exit_code: None,
            stdout: String::new(),
            stderr: String::new(),
            timed_out: false,
            success: None,
            error: None,
            timeout_secs: prepared.timeout_secs,
            started_at_epoch_ms,
            completed_at_epoch_ms: None,
            duration_ms: None,
            cancel_requested: false,
            cancel_tx: Some(cancel_tx),
        };
        let snapshot = record.snapshot();
        let store = self.clone();
        let task_execution_id = execution_id.clone();

        self.insert(record).await;

        tokio::spawn(async move {
            let live_output = ShellLiveOutput {
                store: store.clone(),
                execution_id: task_execution_id.clone(),
            };
            let completed = run_shell_command(prepared, Some(cancel_rx), Some(live_output)).await;
            store.complete(&task_execution_id, completed).await;
        });

        snapshot
    }

    async fn insert(&self, record: ShellExecutionRecord) {
        let mut entries = self.entries.lock().await;
        entries.insert(record.execution_id.clone(), record);
        prune_shell_executions(&mut entries);
    }

    async fn get(&self, execution_id: &str) -> Option<ShellExecutionSnapshot> {
        let entries = self.entries.lock().await;
        entries
            .get(execution_id)
            .map(ShellExecutionRecord::snapshot)
    }

    async fn wait_for_terminal_snapshot(
        &self,
        execution_id: &str,
        wait_duration: Duration,
    ) -> Option<ShellExecutionSnapshot> {
        let started = Instant::now();
        loop {
            let snapshot = self.get(execution_id).await?;
            if !matches!(snapshot.status, ShellExecutionStatus::Running) {
                return Some(snapshot);
            }
            if started.elapsed() >= wait_duration {
                return Some(snapshot);
            }
            sleep(Duration::from_millis(25)).await;
        }
    }

    async fn cancel(&self, execution_id: &str) -> Option<ShellExecutionSnapshot> {
        let mut entries = self.entries.lock().await;
        let record = entries.get_mut(execution_id)?;
        if matches!(record.status, ShellExecutionStatus::Running) {
            record.cancel_requested = true;
            if let Some(cancel_tx) = record.cancel_tx.take() {
                let _ = cancel_tx.send(());
            }
        }
        Some(record.snapshot())
    }

    async fn complete(&self, execution_id: &str, completed: CompletedShellExec) {
        let mut entries = self.entries.lock().await;
        if let Some(record) = entries.get_mut(execution_id) {
            record.status = completed.status;
            record.exit_code = completed.exit_code;
            record.stdout = completed.stdout;
            record.stderr = completed.stderr;
            record.timed_out = completed.timed_out;
            record.success = Some(completed.success);
            record.error = completed.error;
            record.completed_at_epoch_ms = Some(completed.completed_at_epoch_ms);
            record.duration_ms = Some(completed.duration_ms);
            record.cancel_tx = None;
        }
        prune_shell_executions(&mut entries);
    }

    async fn append_output(&self, execution_id: &str, stream: ShellOutputStream, chunk: &str) {
        if chunk.is_empty() {
            return;
        }

        let mut entries = self.entries.lock().await;
        let Some(record) = entries.get_mut(execution_id) else {
            return;
        };
        if !matches!(record.status, ShellExecutionStatus::Running) {
            return;
        }

        let output = match stream {
            ShellOutputStream::Stdout => &mut record.stdout,
            ShellOutputStream::Stderr => &mut record.stderr,
        };
        output.push_str(chunk);
        trim_string_to_max_bytes(output, RUNNING_SHELL_OUTPUT_TAIL_BYTES);
    }
}

impl ShellExecData {
    fn from_snapshot(
        snapshot: &ShellExecutionSnapshot,
        recommended_action: Option<&'static str>,
        recommended_service: Option<String>,
        recommended_method: Option<&'static str>,
    ) -> Self {
        Self {
            execution_id: snapshot.execution_id.clone(),
            status: snapshot.status.clone(),
            exit_code: snapshot.exit_code,
            stdout: snapshot.stdout.clone(),
            stderr: snapshot.stderr.clone(),
            timed_out: snapshot.timed_out,
            timeout_secs: snapshot.timeout_secs,
            started_at_epoch_ms: snapshot.started_at_epoch_ms,
            completed_at_epoch_ms: snapshot.completed_at_epoch_ms,
            duration_ms: snapshot.duration_ms,
            recommended_action,
            recommended_service,
            recommended_method,
        }
    }
}

impl ShellExecutionRecord {
    fn snapshot(&self) -> ShellExecutionSnapshot {
        ShellExecutionSnapshot {
            execution_id: self.execution_id.clone(),
            command: self.command.clone(),
            cwd: self.cwd.clone(),
            status: self.status.clone(),
            exit_code: self.exit_code,
            stdout: self.stdout.clone(),
            stderr: self.stderr.clone(),
            timed_out: self.timed_out,
            success: self.success,
            error: self.error.clone(),
            timeout_secs: self.timeout_secs,
            started_at_epoch_ms: self.started_at_epoch_ms,
            completed_at_epoch_ms: self.completed_at_epoch_ms,
            duration_ms: self.duration_ms,
            cancel_requested: self.cancel_requested,
        }
    }
}

fn prune_shell_executions(entries: &mut BTreeMap<String, ShellExecutionRecord>) {
    const MAX_SHELL_EXECUTIONS: usize = 100;
    if entries.len() <= MAX_SHELL_EXECUTIONS {
        return;
    }

    let mut removable = entries
        .values()
        .filter(|record| !matches!(record.status, ShellExecutionStatus::Running))
        .map(|record| (record.started_at_epoch_ms, record.execution_id.clone()))
        .collect::<Vec<_>>();
    removable.sort_by_key(|(started_at, _)| *started_at);

    for (_, execution_id) in removable {
        if entries.len() <= MAX_SHELL_EXECUTIONS {
            break;
        }
        entries.remove(&execution_id);
    }
}

fn trim_string_to_max_bytes(value: &mut String, max_bytes: usize) {
    if value.len() <= max_bytes {
        return;
    }

    let mut start = value.len().saturating_sub(max_bytes);
    while start < value.len() && !value.is_char_boundary(start) {
        start += 1;
    }
    value.drain(..start);
}

#[cfg(not(test))]
fn shell_exec_sync_wait_duration() -> Duration {
    Duration::from_secs(30)
}

#[cfg(test)]
fn shell_exec_sync_wait_duration() -> Duration {
    Duration::from_millis(100)
}

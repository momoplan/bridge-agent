struct LocalEventServerRuntime {
    inner: Arc<RuntimeInner>,
    log_limit: usize,
    config: AgentConfig,
    config_path: PathBuf,
    registry: Arc<RwLock<ServiceRegistry>>,
    event_tx: mpsc::Sender<LocalAppEventSubmission>,
    apply_tx: mpsc::UnboundedSender<RuntimeRegistryUpdate>,
    audit_tx: mpsc::UnboundedSender<RuntimeAuditLog>,
}

impl LocalEventServerRuntime {
    async fn run(self, mut shutdown_rx: watch::Receiver<bool>) {
        let mut previous_error = None::<String>;

        loop {
            if *shutdown_rx.borrow() {
                return;
            }

            match LocalEventServer::bind(
                &self.config,
                self.config_path.clone(),
                Arc::clone(&self.registry),
                self.event_tx.clone(),
                self.apply_tx.clone(),
                self.audit_tx.clone(),
            )
            .await
            {
                Ok(None) => return,
                Ok(Some(server)) => {
                    let bind_addr = server.bind_addr();
                    let recovered = previous_error.take().is_some();
                    push_log_entry(
                        &self.inner,
                        self.log_limit,
                        "info",
                        &if recovered {
                            format!("local event server recovered and is listening on {bind_addr}")
                        } else {
                            format!("local event server listening on {bind_addr}")
                        },
                        LogMetadata::category("event_server").outcome(if recovered {
                            "recovered"
                        } else {
                            "listening"
                        }),
                    )
                    .await;

                    match server.serve(shutdown_rx.clone()).await {
                        Ok(()) => return,
                        Err(_) if *shutdown_rx.borrow() => return,
                        Err(err) => {
                            let detail = format!("{err:#}");
                            push_log_entry(
                                &self.inner,
                                self.log_limit,
                                "error",
                                &format!(
                                    "local event server stopped; relay remains active and the listener will retry: {detail}"
                                ),
                                LogMetadata::category("event_server").outcome("stopped"),
                            )
                            .await;
                            previous_error = Some(detail);
                        }
                    }
                }
                Err(err) => {
                    let detail = format!("{err:#}");
                    if previous_error.as_deref() != Some(detail.as_str()) {
                        push_log_entry(
                            &self.inner,
                            self.log_limit,
                            "warn",
                            &format!(
                                "local event server unavailable; relay remains active and binding will retry: {detail}"
                            ),
                            LogMetadata::category("event_server").outcome("bind_failed"),
                        )
                        .await;
                    }
                    previous_error = Some(detail);
                }
            }

            tokio::select! {
                changed = shutdown_rx.changed() => {
                    if changed.is_err() || *shutdown_rx.borrow() {
                        return;
                    }
                }
                _ = sleep(Duration::from_secs(LOCAL_EVENT_SERVER_RETRY_INTERVAL_SECS)) => {}
            }
        }
    }
}

fn runtime_start_is_active(status: RuntimeStatus) -> bool {
    matches!(
        status,
        RuntimeStatus::Starting
            | RuntimeStatus::Connecting
            | RuntimeStatus::Backoff
            | RuntimeStatus::AuthorizationRequired
    )
}

fn is_relay_authorization_error(error: &WebSocketError) -> bool {
    matches!(
        error,
        WebSocketError::Http(response)
            if matches!(response.status(), StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN)
    )
}

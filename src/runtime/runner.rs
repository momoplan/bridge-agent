struct RuntimeRunner {
    inner: Arc<RuntimeInner>,
    log_limit: usize,
    config: AgentConfig,
    config_path: String,
    ws_url: Url,
    registry: Arc<RwLock<ServiceRegistry>>,
}

impl RuntimeRunner {
    async fn run(
        &self,
        mut shutdown_rx: watch::Receiver<bool>,
        apply_rx: &mut mpsc::UnboundedReceiver<RuntimeRegistryUpdate>,
        event_rx: &mut mpsc::Receiver<LocalAppEventSubmission>,
        audit_rx: &mut mpsc::UnboundedReceiver<RuntimeAuditLog>,
    ) -> Result<()> {
        loop {
            if *shutdown_rx.borrow() {
                break;
            }

            self.update_snapshot(
                RuntimeStatus::Connecting,
                None,
                self.config.relay.agent_id.clone(),
                self.config.relay.url.clone(),
                self.config_path.clone(),
            )
            .await;
            self.push_log("info", &format!("connecting to {}", self.config.relay.url))
                .await;

            if !self
                .connect_once(&mut shutdown_rx, apply_rx, event_rx, audit_rx)
                .await
            {
                break;
            }
            if self
                .wait_for_reconnect(&mut shutdown_rx, apply_rx, audit_rx)
                .await
            {
                break;
            }
        }

        self.update_snapshot(
            RuntimeStatus::Stopped,
            None,
            self.config.relay.agent_id.clone(),
            self.config.relay.url.clone(),
            self.config_path.clone(),
        )
        .await;
        self.push_log("info", "runtime stopped").await;
        Ok(())
    }

    async fn connect_once(
        &self,
        shutdown_rx: &mut watch::Receiver<bool>,
        apply_rx: &mut mpsc::UnboundedReceiver<RuntimeRegistryUpdate>,
        event_rx: &mut mpsc::Receiver<LocalAppEventSubmission>,
        audit_rx: &mut mpsc::UnboundedReceiver<RuntimeAuditLog>,
    ) -> bool {
        match timeout(
            Duration::from_secs(RELAY_CONNECT_TIMEOUT_SECS),
            connect_async(self.ws_url.as_str()),
        )
        .await
        {
            Ok(Ok((stream, _))) => {
                self.push_log("info", "connected to relay, waiting for registration")
                    .await;
                if let Err(err) = self
                    .handle_connection(stream, shutdown_rx, apply_rx, event_rx, audit_rx)
                    .await
                {
                    self.enter_backoff(err.to_string(), &format!("connection ended: {err:#}"))
                        .await;
                }
                true
            }
            Ok(Err(err)) if is_relay_authorization_error(&err) => {
                self.update_snapshot(
                    RuntimeStatus::AuthorizationRequired,
                    Some(RELAY_AUTHORIZATION_REQUIRED_MESSAGE.to_string()),
                    self.config.relay.agent_id.clone(),
                    self.config.relay.url.clone(),
                    self.config_path.clone(),
                )
                .await;
                self.push_log(
                    "warn",
                    &format!("relay rejected the saved device credential; reconnect paused until reauthorization: {err}"),
                )
                .await;
                wait_until_shutdown(shutdown_rx).await;
                false
            }
            Ok(Err(err)) => {
                self.enter_backoff(err.to_string(), &format!("connect failed: {err}"))
                    .await;
                true
            }
            Err(_) => {
                let message = format!("relay connect timed out after {RELAY_CONNECT_TIMEOUT_SECS}s");
                self.enter_backoff(message.clone(), &message).await;
                true
            }
        }
    }

    async fn enter_backoff(&self, error: String, log_message: &str) {
        self.update_snapshot(
            RuntimeStatus::Backoff,
            Some(error),
            self.config.relay.agent_id.clone(),
            self.config.relay.url.clone(),
            self.config_path.clone(),
        )
        .await;
        self.push_log("warn", log_message).await;
    }

    async fn wait_for_reconnect(
        &self,
        shutdown_rx: &mut watch::Receiver<bool>,
        apply_rx: &mut mpsc::UnboundedReceiver<RuntimeRegistryUpdate>,
        audit_rx: &mut mpsc::UnboundedReceiver<RuntimeAuditLog>,
    ) -> bool {
        tokio::select! {
            _ = shutdown_rx.changed() => true,
            Some(update) = apply_rx.recv() => {
                self.apply_registry_update(update).await;
                false
            }
            Some(audit) = audit_rx.recv() => {
                self.push_audit_log(audit).await;
                false
            }
            _ = sleep(Duration::from_secs(self.config.relay.reconnect_secs)) => false,
        }
    }

    async fn handle_connection(
        &self,
        stream: RelayWebSocket,
        shutdown_rx: &mut watch::Receiver<bool>,
        apply_rx: &mut mpsc::UnboundedReceiver<RuntimeRegistryUpdate>,
        event_rx: &mut mpsc::Receiver<LocalAppEventSubmission>,
        audit_rx: &mut mpsc::UnboundedReceiver<RuntimeAuditLog>,
    ) -> Result<()> {
        let (mut write, read) = stream.split();
        let capabilities = self.current_capabilities().await;
        write_json(&mut write, &capabilities).await?;
        let keepalive_interval = Duration::from_secs(RELAY_KEEPALIVE_INTERVAL_SECS);
        RelayConnection {
            runner: self,
            write,
            read,
            keepalive: interval_at(
                tokio::time::Instant::now() + keepalive_interval,
                keepalive_interval,
            ),
            last_relay_seen: tokio::time::Instant::now(),
            pending_events: PendingEventWaiters::new(),
        }
        .run(shutdown_rx, apply_rx, event_rx, audit_rx)
        .await
    }

    async fn current_capabilities(&self) -> AgentMessage {
        let registry = self.registry.read().await;
        AgentMessage::Capabilities(AgentCapabilities {
            agent_id: self.config.relay.agent_id.clone(),
            protocol_version: AGENT_PROTOCOL_VERSION.to_string(),
            protocol_features: vec![
                AGENT_PROTOCOL_FEATURE_REGISTERED_ACK.to_string(),
                AGENT_PROTOCOL_FEATURE_LOCAL_APP_EVENTS_V2.to_string(),
                AGENT_PROTOCOL_FEATURE_LOCAL_APP_CAPABILITIES_V3.to_string(),
            ],
            services: registry.definitions(),
            local_apps: registry.local_app_definitions(),
        })
    }

    async fn apply_registry_update(&self, update: RuntimeRegistryUpdate) -> AgentMessage {
        {
            let mut registry = self.registry.write().await;
            *registry = update.registry;
        }
        AgentMessage::Capabilities(AgentCapabilities {
            agent_id: self.config.relay.agent_id.clone(),
            protocol_version: AGENT_PROTOCOL_VERSION.to_string(),
            protocol_features: vec![
                AGENT_PROTOCOL_FEATURE_REGISTERED_ACK.to_string(),
                AGENT_PROTOCOL_FEATURE_LOCAL_APP_EVENTS_V2.to_string(),
                AGENT_PROTOCOL_FEATURE_LOCAL_APP_CAPABILITIES_V3.to_string(),
            ],
            services: update.services,
            local_apps: update.local_apps,
        })
    }

    async fn update_snapshot(
        &self,
        status: RuntimeStatus,
        last_error: Option<String>,
        agent_id: String,
        relay_url: String,
        config_path: String,
    ) {
        let snapshot = {
            let mut state = self.inner.state.lock().await;
            let revision = state.snapshot.revision.saturating_add(1);
            state.snapshot = RuntimeSnapshot {
                revision,
                status,
                config_path: Some(config_path),
                agent_id: Some(agent_id),
                relay_url: Some(relay_url),
                relay_registered: false,
                relay_registered_at: None,
                last_relay_seen_at: None,
                log_file_path: state.snapshot.log_file_path.clone(),
                last_error,
                last_event_at: now_ms(),
            };
            state.last_relay_seen_event_at = None;
            state.snapshot.clone()
        };
        publish_runtime_event(&self.inner, RuntimeEvent::SnapshotChanged(snapshot));
    }
}

impl RelayConnection<'_> {
    async fn send_registry_update(&mut self, update: RuntimeRegistryUpdate) -> Result<()> {
        let capabilities = self.runner.apply_registry_update(update).await;
        write_json(&mut self.write, &capabilities).await?;
        self.runner
            .push_log("info", "runtime capabilities updated and sent to relay")
            .await;
        Ok(())
    }

    async fn send_local_event(&mut self, submission: LocalAppEventSubmission) -> Result<()> {
        if submission.response.is_closed() {
            return Ok(());
        }
        let event = submission.event;
        let event_id = event.event_id.clone();
        let app_id = event.app_id.clone();
        let event_name = event.event.clone();
        let event_key = (app_id.clone(), event_id.clone());
        if !register_event_waiter(&mut self.pending_events, event_key, submission.response) {
            return Ok(());
        }
        write_json(&mut self.write, &AgentMessage::LocalAppEventEmitted(event)).await?;
        self.runner
            .push_log_with_metadata(
                "info",
                &format!("local app event {app_id}.{event_name} sent to relay"),
                LogMetadata::category("local_app_event")
                    .event(event_name)
                    .event_id(event_id)
                    .outcome("sent"),
            )
            .await;
        Ok(())
    }

    async fn send_keepalive(&mut self) -> Result<()> {
        prune_closed_event_waiters(&mut self.pending_events);
        if self.last_relay_seen.elapsed() > Duration::from_secs(RELAY_HEARTBEAT_TIMEOUT_SECS) {
            bail!(
                "relay heartbeat timed out after {}s without server frame",
                RELAY_HEARTBEAT_TIMEOUT_SECS
            );
        }
        self.write.send(Message::Ping(Vec::new().into())).await?;
        Ok(())
    }

    async fn handle_frame(&mut self, message: Message) -> Result<()> {
        match message {
            Message::Text(text) => {
                self.mark_relay_seen().await;
                let Some(incoming) = decode_relay_message(&text)
                    .with_context(|| format!("invalid relay message: {text}"))?
                else {
                    self.runner
                        .push_log("warn", "ignored unsupported relay message type")
                        .await;
                    return Ok(());
                };
                self.handle_agent_message(incoming).await?;
            }
            Message::Ping(payload) => {
                self.mark_relay_seen().await;
                self.write.send(Message::Pong(payload)).await?;
            }
            Message::Close(_) => bail!("relay closed websocket"),
            Message::Pong(_) => self.mark_relay_seen().await,
            Message::Binary(_) | Message::Frame(_) => {}
        }
        Ok(())
    }

    async fn mark_relay_seen(&mut self) {
        self.last_relay_seen = tokio::time::Instant::now();
        self.runner.update_relay_seen().await;
    }

    async fn handle_agent_message(&mut self, message: AgentMessage) -> Result<()> {
        match message {
            AgentMessage::RegisteredAck(ack) => self.handle_registered_ack(ack).await,
            AgentMessage::EventAck(ack) => self.handle_event_ack(ack).await,
            AgentMessage::InvokeRequest(request) => self.handle_invoke_request(request).await?,
            AgentMessage::LocalAppInvokeRequest(request) => {
                self.handle_local_app_invoke_request(request).await?
            }
            AgentMessage::Error(err) => {
                self.runner
                    .push_log("warn", &format!("relay error: {}", err.message))
                    .await;
            }
            AgentMessage::Capabilities(_)
            | AgentMessage::InvokeResult(_)
            | AgentMessage::LocalAppInvokeResult(_)
            | AgentMessage::LocalAppEventEmitted(_) => {}
        }
        Ok(())
    }

    async fn handle_registered_ack(&self, ack: crate::protocol::RegisteredAck) {
        self.runner.update_registered_snapshot(&ack).await;
        self.runner
            .push_log(
                "info",
                &format!(
                    "registered on relay as {} connection {}",
                    ack.agent_id, ack.connection_id
                ),
            )
            .await;
    }

    async fn handle_event_ack(&mut self, ack: EventAck) {
        let Some(waiters) = self
            .pending_events
            .remove(&(ack.app_id.clone(), ack.event_id.clone()))
        else {
            return;
        };
        for waiter in waiters {
            let _ = waiter.send(Ok(ack.clone()));
        }
        let outcome = if ack.matched_subscription_count == 0 {
            "ignored"
        } else if ack.duplicate {
            "deduplicated"
        } else {
            "persisted"
        };
        self.runner
            .push_log_with_metadata(
                "info",
                &format!(
                    "local app event {} acknowledged by relay with {} matching subscription(s)",
                    ack.event_id, ack.matched_subscription_count
                ),
                LogMetadata::category("local_app_event")
                    .event_id(ack.event_id)
                    .outcome(outcome),
            )
            .await;
    }

    async fn handle_invoke_request(&mut self, request: crate::protocol::InvokeRequest) -> Result<()> {
        let service = request.service.clone();
        let method = request.method.clone();
        let request_id = request.request_id.clone();
        self.runner.push_log_with_metadata(
            "info",
            &format!("invoke {service}.{method} started"),
            LogMetadata::category("invoke").service(service.clone()).method(method.clone()).request_id(request_id.clone()).outcome("started"),
        ).await;
        let result = self.runner.registry.read().await.invoke(
            request.request_id,
            &service,
            &method,
            request.arguments,
            request.timeout_secs,
        ).await;
        let (level, outcome, suffix) = if result.success {
            ("info", "succeeded", String::new())
        } else {
            let error = result.error.as_ref().map(|err| format!("{}: {}", err.code, err.message)).unwrap_or_else(|| "unknown error".to_string());
            ("warn", "failed", format!(": {error}"))
        };
        self.runner.push_log_with_metadata(
            level,
            &format!("invoke {service}.{method} {outcome} in {}ms{suffix}", result.duration_ms),
            LogMetadata::category("invoke").service(service).method(method).request_id(request_id).outcome(outcome).duration_ms(result.duration_ms),
        ).await;
        write_json(&mut self.write, &AgentMessage::InvokeResult(result)).await
    }

    async fn handle_local_app_invoke_request(
        &mut self,
        request: crate::protocol::LocalAppInvokeRequest,
    ) -> Result<()> {
        let app_id = request.app_id.clone();
        let method = request.method.clone();
        let request_id = request.request_id.clone();
        self.runner.push_log_with_metadata(
            "info",
            &format!("local app invoke {app_id}.{method} started"),
            LogMetadata::category("local_app_invoke").method(method.clone()).request_id(request_id.clone()).outcome("started"),
        ).await;
        let result = self.runner.registry.read().await.invoke_local_app(
            request.request_id,
            request.workspace_id,
            &app_id,
            &method,
            request.arguments,
            request.timeout_secs,
        ).await;
        let outcome = if result.success { "succeeded" } else { "failed" };
        self.runner.push_log_with_metadata(
            if result.success { "info" } else { "warn" },
            &format!("local app invoke {app_id}.{method} {outcome} in {}ms", result.duration_ms),
            LogMetadata::category("local_app_invoke").method(method).request_id(request_id).outcome(outcome).duration_ms(result.duration_ms),
        ).await;
        write_json(&mut self.write, &AgentMessage::LocalAppInvokeResult(result)).await
    }
}

type RelayWebSocket = tokio_tungstenite::WebSocketStream<
    tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
>;
type RelayWrite = futures_util::stream::SplitSink<RelayWebSocket, Message>;
type RelayRead = futures_util::stream::SplitStream<RelayWebSocket>;

async fn wait_until_shutdown(shutdown_rx: &mut watch::Receiver<bool>) {
    while !*shutdown_rx.borrow() && shutdown_rx.changed().await.is_ok() {}
}

struct RelayConnection<'a> {
    runner: &'a RuntimeRunner,
    write: RelayWrite,
    read: RelayRead,
    keepalive: tokio::time::Interval,
    last_relay_seen: tokio::time::Instant,
    pending_events: PendingEventWaiters,
}

impl RelayConnection<'_> {
    async fn run(
        mut self,
        shutdown_rx: &mut watch::Receiver<bool>,
        apply_rx: &mut mpsc::UnboundedReceiver<RuntimeRegistryUpdate>,
        event_rx: &mut mpsc::Receiver<LocalAppEventSubmission>,
        audit_rx: &mut mpsc::UnboundedReceiver<RuntimeAuditLog>,
    ) -> Result<()> {
        loop {
            tokio::select! {
                _ = shutdown_rx.changed() => {
                    self.write.send(Message::Close(None)).await.ok();
                    break;
                }
                Some(update) = apply_rx.recv() => {
                    self.send_registry_update(update).await?;
                }
                Some(submission) = event_rx.recv() => {
                    self.send_local_event(submission).await?;
                }
                Some(audit) = audit_rx.recv() => {
                    self.runner.push_audit_log(audit).await;
                }
                _ = self.keepalive.tick() => {
                    self.send_keepalive().await?;
                }
                message = self.read.next() => {
                    self.handle_frame(message.context("relay websocket ended")??).await?;
                }
            }
        }
        Ok(())
    }
}

impl RuntimeRunner {
    async fn update_registered_snapshot(&self, ack: &crate::protocol::RegisteredAck) {
        let snapshot = {
            let mut state = self.inner.state.lock().await;
            let now = now_ms();
            state.snapshot.revision = state.snapshot.revision.saturating_add(1);
            state.snapshot.status = RuntimeStatus::Online;
            state.snapshot.relay_registered = true;
            state.snapshot.relay_registered_at = Some(ack.registered_at_epoch_seconds);
            state.snapshot.last_relay_seen_at = Some(now);
            state.snapshot.last_error = None;
            state.snapshot.last_event_at = now;
            state.last_relay_seen_event_at = Some(now);
            state.snapshot.clone()
        };
        publish_runtime_event(&self.inner, RuntimeEvent::SnapshotChanged(snapshot));
    }

    async fn update_relay_seen(&self) {
        let snapshot = {
            let mut state = self.inner.state.lock().await;
            let now = now_ms();
            state.snapshot.revision = state.snapshot.revision.saturating_add(1);
            state.snapshot.last_relay_seen_at = Some(now);
            let should_emit = state
                .last_relay_seen_event_at
                .is_none_or(|last| now.saturating_sub(last) >= RELAY_SEEN_EVENT_INTERVAL_MS);
            if should_emit {
                state.last_relay_seen_event_at = Some(now);
                Some(state.snapshot.clone())
            } else {
                None
            }
        };
        if let Some(snapshot) = snapshot {
            publish_runtime_event(&self.inner, RuntimeEvent::SnapshotChanged(snapshot));
        }
    }

    async fn push_log(&self, level: &str, message: &str) {
        push_log_entry(
            &self.inner,
            self.log_limit,
            level,
            message,
            LogMetadata::default(),
        )
        .await;
    }

    async fn push_log_with_metadata(&self, level: &str, message: &str, metadata: LogMetadata) {
        push_log_entry(&self.inner, self.log_limit, level, message, metadata).await;
    }

    async fn push_audit_log(&self, audit: RuntimeAuditLog) {
        push_log_entry(
            &self.inner,
            self.log_limit,
            &audit.level,
            &audit.message,
            audit.metadata,
        )
        .await;
    }
}

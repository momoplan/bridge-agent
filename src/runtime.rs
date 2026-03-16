use crate::config::{load_config, resolve_config_base_dir, AgentConfig};
use crate::protocol::{AgentCapabilities, AgentMessage};
use crate::services::ServiceRegistry;
use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde::Serialize;
use std::collections::VecDeque;
use std::path::Path;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::{watch, Mutex};
use tokio::task::JoinHandle;
use tokio::time::{sleep, Duration};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::protocol::Message;
use tracing::{error, info, warn};
use url::Url;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeStatus {
    Stopped,
    Starting,
    Connecting,
    Online,
    Backoff,
    Stopping,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeSnapshot {
    pub status: RuntimeStatus,
    pub config_path: Option<String>,
    pub agent_id: Option<String>,
    pub relay_url: Option<String>,
    pub last_error: Option<String>,
    pub last_event_at: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct LogEntry {
    pub timestamp_ms: u64,
    pub level: String,
    pub message: String,
}

#[derive(Clone, Default)]
pub struct AgentRuntimeManager {
    inner: Arc<RuntimeInner>,
}

#[derive(Default)]
struct RuntimeInner {
    state: Mutex<ManagedState>,
    logs: Mutex<VecDeque<LogEntry>>,
}

struct ManagedState {
    snapshot: RuntimeSnapshot,
    task: Option<JoinHandle<()>>,
    shutdown: Option<watch::Sender<bool>>,
}

impl Default for ManagedState {
    fn default() -> Self {
        Self {
            snapshot: RuntimeSnapshot {
                status: RuntimeStatus::Stopped,
                config_path: None,
                agent_id: None,
                relay_url: None,
                last_error: None,
                last_event_at: now_ms(),
            },
            task: None,
            shutdown: None,
        }
    }
}

impl AgentRuntimeManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn start_from_path(&self, path: &Path) -> Result<RuntimeSnapshot> {
        let config = load_config(path)?;
        self.start(config, path).await
    }

    pub async fn start(&self, config: AgentConfig, config_path: &Path) -> Result<RuntimeSnapshot> {
        self.stop_if_running().await?;
        let log_limit = config.runtime.log_limit;
        let registry = Arc::new(ServiceRegistry::from_config(
            &config,
            &resolve_config_base_dir(config_path),
        )?);
        let ws_url = build_agent_url(
            &config.relay.url,
            &config.relay.agent_id,
            &config.relay.token,
        )?;
        let snapshot = RuntimeSnapshot {
            status: RuntimeStatus::Starting,
            config_path: Some(config_path.display().to_string()),
            agent_id: Some(config.relay.agent_id.clone()),
            relay_url: Some(ws_url.to_string()),
            last_error: None,
            last_event_at: now_ms(),
        };

        {
            let mut state = self.inner.state.lock().await;
            state.snapshot = snapshot;
        }
        self.push_log("info", "runtime starting", log_limit).await;

        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let inner = Arc::clone(&self.inner);
        let config_path_string = config_path.display().to_string();
        let task = tokio::spawn(async move {
            let runner = RuntimeRunner {
                inner,
                log_limit,
                config,
                config_path: config_path_string,
                ws_url,
                registry,
            };
            if let Err(err) = runner.run(shutdown_rx).await {
                runner
                    .update_snapshot(
                        RuntimeStatus::Stopped,
                        Some(err.to_string()),
                        runner.config.relay.agent_id.clone(),
                        runner.ws_url.to_string(),
                        runner.config_path.clone(),
                    )
                    .await;
                runner
                    .push_log("error", &format!("runtime stopped with error: {err:#}"))
                    .await;
            }
        });

        let mut state = self.inner.state.lock().await;
        state.shutdown = Some(shutdown_tx);
        state.task = Some(task);
        Ok(state.snapshot.clone())
    }

    pub async fn stop(&self) -> Result<RuntimeSnapshot> {
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

    pub async fn clear_logs(&self) {
        self.inner.logs.lock().await.clear();
    }

    async fn stop_if_running(&self) -> Result<()> {
        let (shutdown, task) = {
            let mut state = self.inner.state.lock().await;
            if state.snapshot.status == RuntimeStatus::Stopped {
                return Ok(());
            }
            state.snapshot.status = RuntimeStatus::Stopping;
            state.snapshot.last_event_at = now_ms();
            (state.shutdown.take(), state.task.take())
        };

        if let Some(shutdown) = shutdown {
            let _ = shutdown.send(true);
        }
        if let Some(task) = task {
            let _ = task.await;
        }
        Ok(())
    }

    async fn push_log(&self, level: &str, message: &str, limit: usize) {
        emit_tracing(level, message);
        let mut logs = self.inner.logs.lock().await;
        logs.push_back(LogEntry {
            timestamp_ms: now_ms(),
            level: level.to_string(),
            message: message.to_string(),
        });
        while logs.len() > limit {
            logs.pop_front();
        }
    }
}

struct RuntimeRunner {
    inner: Arc<RuntimeInner>,
    log_limit: usize,
    config: AgentConfig,
    config_path: String,
    ws_url: Url,
    registry: Arc<ServiceRegistry>,
}

impl RuntimeRunner {
    async fn run(&self, mut shutdown_rx: watch::Receiver<bool>) -> Result<()> {
        loop {
            if *shutdown_rx.borrow() {
                break;
            }

            self.update_snapshot(
                RuntimeStatus::Connecting,
                None,
                self.config.relay.agent_id.clone(),
                self.ws_url.to_string(),
                self.config_path.clone(),
            )
            .await;
            self.push_log("info", &format!("connecting to {}", self.config.relay.url))
                .await;

            match connect_async(self.ws_url.as_str()).await {
                Ok((stream, _)) => {
                    self.update_snapshot(
                        RuntimeStatus::Online,
                        None,
                        self.config.relay.agent_id.clone(),
                        self.ws_url.to_string(),
                        self.config_path.clone(),
                    )
                    .await;
                    self.push_log("info", "connected to relay").await;

                    if let Err(err) = self.handle_connection(stream, &mut shutdown_rx).await {
                        self.update_snapshot(
                            RuntimeStatus::Backoff,
                            Some(err.to_string()),
                            self.config.relay.agent_id.clone(),
                            self.ws_url.to_string(),
                            self.config_path.clone(),
                        )
                        .await;
                        self.push_log("warn", &format!("connection ended: {err:#}"))
                            .await;
                    }
                }
                Err(err) => {
                    self.update_snapshot(
                        RuntimeStatus::Backoff,
                        Some(err.to_string()),
                        self.config.relay.agent_id.clone(),
                        self.ws_url.to_string(),
                        self.config_path.clone(),
                    )
                    .await;
                    self.push_log("warn", &format!("connect failed: {err}"))
                        .await;
                }
            }

            tokio::select! {
                _ = shutdown_rx.changed() => {
                    break;
                }
                _ = sleep(Duration::from_secs(self.config.relay.reconnect_secs)) => {}
            }
        }

        self.update_snapshot(
            RuntimeStatus::Stopped,
            None,
            self.config.relay.agent_id.clone(),
            self.ws_url.to_string(),
            self.config_path.clone(),
        )
        .await;
        self.push_log("info", "runtime stopped").await;
        Ok(())
    }

    async fn handle_connection(
        &self,
        stream: tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        shutdown_rx: &mut watch::Receiver<bool>,
    ) -> Result<()> {
        let (mut write, mut read) = stream.split();
        let capabilities = AgentMessage::Capabilities(AgentCapabilities {
            agent_id: self.config.relay.agent_id.clone(),
            services: self.registry.definitions(),
        });
        write_json(&mut write, &capabilities).await?;

        loop {
            tokio::select! {
                _ = shutdown_rx.changed() => {
                    write.send(Message::Close(None)).await.ok();
                    break;
                }
                message = read.next() => {
                    let Some(message) = message else {
                        break;
                    };
                    match message? {
                        Message::Text(text) => {
                            let incoming: AgentMessage = serde_json::from_str(&text)
                                .with_context(|| format!("invalid relay message: {text}"))?;
                            match incoming {
                                AgentMessage::InvokeRequest(request) => {
                                    self.push_log(
                                        "info",
                                        &format!("invoke {}.{}", request.service, request.method),
                                    )
                                    .await;
                                    let response = AgentMessage::InvokeResult(
                                        self.registry
                                            .invoke(
                                                request.request_id,
                                                &request.service,
                                                &request.method,
                                                request.arguments,
                                                request.timeout_secs,
                                            )
                                            .await,
                                    );
                                    write_json(&mut write, &response).await?;
                                }
                                AgentMessage::Error(err) => {
                                    self.push_log("warn", &format!("relay error: {}", err.message))
                                        .await;
                                }
                                AgentMessage::Capabilities(_) | AgentMessage::InvokeResult(_) => {}
                            }
                        }
                        Message::Ping(payload) => {
                            write.send(Message::Pong(payload)).await?;
                        }
                        Message::Close(_) => break,
                        Message::Binary(_) | Message::Pong(_) | Message::Frame(_) => {}
                    }
                }
            }
        }
        Ok(())
    }

    async fn update_snapshot(
        &self,
        status: RuntimeStatus,
        last_error: Option<String>,
        agent_id: String,
        relay_url: String,
        config_path: String,
    ) {
        let mut state = self.inner.state.lock().await;
        state.snapshot = RuntimeSnapshot {
            status,
            config_path: Some(config_path),
            agent_id: Some(agent_id),
            relay_url: Some(relay_url),
            last_error,
            last_event_at: now_ms(),
        };
    }

    async fn push_log(&self, level: &str, message: &str) {
        emit_tracing(level, message);
        let mut logs = self.inner.logs.lock().await;
        logs.push_back(LogEntry {
            timestamp_ms: now_ms(),
            level: level.to_string(),
            message: message.to_string(),
        });
        while logs.len() > self.log_limit {
            logs.pop_front();
        }
    }
}

async fn write_json<S>(sink: &mut S, message: &AgentMessage) -> Result<()>
where
    S: futures_util::Sink<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin,
{
    let payload = serde_json::to_string(message)?;
    sink.send(Message::Text(payload.into())).await?;
    Ok(())
}

fn build_agent_url(base: &str, agent_id: &str, token: &str) -> Result<Url> {
    let base = base.trim_end_matches('/');
    let mut url = Url::parse(&format!("{base}/{agent_id}"))?;
    url.query_pairs_mut().append_pair("token", token);
    Ok(url)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn emit_tracing(level: &str, message: &str) {
    match level {
        "error" => error!("{message}"),
        "warn" => warn!("{message}"),
        _ => info!("{message}"),
    }
}

#[cfg(test)]
mod tests {
    use super::build_agent_url;

    #[test]
    fn build_url_injects_token() {
        let url = build_agent_url("ws://127.0.0.1:8080/ws/agent", "devbox", "secret").unwrap();
        assert_eq!(
            url.as_str(),
            "ws://127.0.0.1:8080/ws/agent/devbox?token=secret"
        );
    }
}

use relay::device_channel::{
    decode_json, decode_json_or_null,
    v1::{
        self,
        device_channel_server::{DeviceChannel, DeviceChannelServer},
        device_frame::Frame as DeviceFrameKind,
        relay_frame::Frame as RelayFrameKind,
    },
    MAX_FRAME_BYTES,
};

type FakeRelayFrames = std::pin::Pin<
    Box<dyn futures_util::Stream<Item = Result<v1::RelayFrame, tonic::Status>> + Send>,
>;

/// A real gRPC Relay on a loopback port. Each accepted device stream is handed
/// to the test as a [`RelaySession`].
struct FakeRelay {
    /// The configured relay URL, in the WebSocket form existing configs use.
    url: String,
    sessions: tokio::sync::mpsc::UnboundedReceiver<RelaySession>,
    attempts: Arc<AtomicUsize>,
    server: tokio::task::JoinHandle<()>,
}

struct FakeRelayService {
    sessions: tokio::sync::mpsc::UnboundedSender<RelaySession>,
    attempts: Arc<AtomicUsize>,
    reject: Option<tonic::Code>,
}

#[tonic::async_trait]
impl DeviceChannel for FakeRelayService {
    type ConnectStream = FakeRelayFrames;

    async fn connect(
        &self,
        request: tonic::Request<tonic::Streaming<v1::DeviceFrame>>,
    ) -> Result<tonic::Response<FakeRelayFrames>, tonic::Status> {
        self.attempts.fetch_add(1, Ordering::SeqCst);
        if let Some(code) = self.reject {
            return Err(tonic::Status::new(code, "rejected by test relay"));
        }
        let (relay, frames) = tokio::sync::mpsc::channel(64);
        let _ = self.sessions.send(RelaySession {
            metadata: request.metadata().clone(),
            device: request.into_inner(),
            relay: Some(relay),
        });
        Ok(tonic::Response::new(Box::pin(futures_util::stream::unfold(
            frames,
            |mut frames| async move { frames.recv().await.map(|frame| (Ok(frame), frames)) },
        ))))
    }
}

impl FakeRelay {
    fn start() -> Self {
        Self::serve(None)
    }

    fn rejecting(code: tonic::Code) -> Self {
        Self::serve(Some(code))
    }

    fn serve(reject: Option<tonic::Code>) -> Self {
        let incoming =
            tonic::transport::server::TcpIncoming::bind("127.0.0.1:0".parse().unwrap()).unwrap();
        let addr = incoming.local_addr().unwrap();
        let (sessions_tx, sessions) = tokio::sync::mpsc::unbounded_channel();
        let attempts = Arc::new(AtomicUsize::new(0));
        let service = DeviceChannelServer::new(FakeRelayService {
            sessions: sessions_tx,
            attempts: attempts.clone(),
            reject,
        })
        .max_decoding_message_size(MAX_FRAME_BYTES)
        .max_encoding_message_size(MAX_FRAME_BYTES);
        let server = tokio::spawn(async move {
            let _ = tonic::transport::Server::builder()
                .serve_with_incoming(service, incoming)
                .await;
        });
        Self {
            url: format!("ws://{addr}/ws/agent"),
            sessions,
            attempts,
            server,
        }
    }

    async fn accept(&mut self) -> RelaySession {
        tokio::time::timeout(std::time::Duration::from_secs(10), self.sessions.recv())
            .await
            .expect("device did not open a relay stream")
            .expect("fake relay stopped")
    }

    fn attempts(&self) -> usize {
        self.attempts.load(Ordering::SeqCst)
    }
}

impl Drop for FakeRelay {
    fn drop(&mut self) {
        self.server.abort();
    }
}

/// One device stream seen from the Relay side.
struct RelaySession {
    metadata: tonic::metadata::MetadataMap,
    device: tonic::Streaming<v1::DeviceFrame>,
    relay: Option<tokio::sync::mpsc::Sender<v1::RelayFrame>>,
}

impl RelaySession {
    /// Sends a relay message given in its JSON wire form.
    async fn send(&self, message: Value) {
        let message: AgentMessage = serde_json::from_value(message).unwrap();
        self.send_frame(relay_frame(message)).await;
    }

    async fn send_frame(&self, frame: v1::RelayFrame) {
        self.relay
            .as_ref()
            .expect("relay stream already closed")
            .send(frame)
            .await
            .unwrap();
    }

    async fn heartbeat(&self) {
        self.send_frame(v1::RelayFrame {
            frame: Some(RelayFrameKind::Heartbeat(v1::Heartbeat {})),
        })
        .await;
    }

    /// Ends the relay's response stream, as Relay does when it drops a device.
    fn close(&mut self) {
        self.relay = None;
    }

    /// The next device frame, heartbeats included.
    async fn next_frame(&mut self) -> DeviceFrameKind {
        tokio::time::timeout(std::time::Duration::from_secs(3), self.device.message())
            .await
            .expect("device frame did not arrive in time")
            .expect("device stream failed")
            .expect("device stream ended")
            .frame
            .expect("device frame is empty")
    }

    /// The next device message in its JSON wire form, skipping heartbeats.
    async fn receive(&mut self) -> Value {
        loop {
            if let Some(message) = device_message(self.next_frame().await) {
                return serde_json::to_value(message).unwrap();
            }
        }
    }
}

/// Relay's side of the frame mapping, so tests speak the same protocol.
fn relay_frame(message: AgentMessage) -> v1::RelayFrame {
    let frame = match message {
        AgentMessage::RegisteredAck(ack) => RelayFrameKind::RegisteredAck(v1::RegisteredAck {
            agent_id: ack.agent_id,
            workspace_id: ack.workspace_id,
            connection_id: ack.connection_id,
            registered_at_epoch_seconds: ack.registered_at_epoch_seconds,
            heartbeat_timeout_secs: ack.heartbeat_timeout_secs,
        }),
        AgentMessage::InvokeRequest(request) => RelayFrameKind::InvokeRequest(v1::InvokeRequest {
            request_id: request.request_id,
            service: request.service,
            method: request.method,
            arguments_json: serde_json::to_vec(&request.arguments).unwrap(),
            timeout_secs: request.timeout_secs,
        }),
        AgentMessage::LocalAppInvokeRequest(request) => {
            RelayFrameKind::LocalAppInvokeRequest(v1::LocalAppInvokeRequest {
                request_id: request.request_id,
                workspace_id: request.workspace_id,
                app_id: request.app_id,
                method: request.method,
                arguments_json: serde_json::to_vec(&request.arguments).unwrap(),
                timeout_secs: request.timeout_secs,
            })
        }
        AgentMessage::EventAck(ack) => RelayFrameKind::EventAck(ack.into()),
        AgentMessage::DeviceSubscriptions(snapshot) => {
            RelayFrameKind::DeviceSubscriptions(snapshot.into())
        }
        AgentMessage::StreamAttach(attach) => RelayFrameKind::StreamAttach(attach.into()),
        AgentMessage::StreamDetach(detach) => RelayFrameKind::StreamDetach(detach.into()),
        AgentMessage::Error(error) => RelayFrameKind::Error(v1::ProtocolError {
            request_id: error.request_id,
            message: error.message,
        }),
        other => panic!("relay does not send {other:?}"),
    };
    v1::RelayFrame { frame: Some(frame) }
}

/// Relay's decoding of a device frame; `None` is a heartbeat.
fn device_message(frame: DeviceFrameKind) -> Option<AgentMessage> {
    use crate::protocol::{
        EventDefinition, InvokeError, LocalAppDefinition, MethodDefinition, ResponseMode,
        ServiceDefinition,
    };
    let methods = |methods: Vec<v1::MethodDefinition>| -> Vec<MethodDefinition> {
        methods
            .into_iter()
            .map(|method| MethodDefinition {
                name: method.name,
                description: method.description,
                input_schema: decode_json_or_null("inputSchemaJson", &method.input_schema_json)
                    .unwrap(),
                response_mode: match v1::ResponseMode::try_from(method.response_mode).unwrap() {
                    v1::ResponseMode::Cmodel => ResponseMode::Cmodel,
                    v1::ResponseMode::Plain => ResponseMode::Plain,
                    v1::ResponseMode::Passthrough => ResponseMode::Passthrough,
                },
            })
            .collect()
    };
    let result = |result: v1::InvokeResult| crate::protocol::InvokeResult {
        request_id: result.request_id,
        success: result.success,
        data: result
            .data_json
            .map(|data| decode_json("dataJson", &data).unwrap()),
        error: result.error.map(|error| InvokeError {
            public_details: error
                .public_details_json
                .map(|details| decode_json("publicDetailsJson", &details).unwrap()),
            code: error.code,
            message: error.message,
        }),
        duration_ms: result.duration_ms,
    };
    Some(match frame {
        DeviceFrameKind::Capabilities(capabilities) => {
            AgentMessage::Capabilities(AgentCapabilities {
                agent_id: capabilities.agent_id,
                protocol_version: capabilities.protocol_version,
                protocol_features: capabilities.protocol_features,
                services: capabilities
                    .services
                    .into_iter()
                    .map(|service| ServiceDefinition {
                        name: service.name,
                        description: service.description,
                        methods: methods(service.methods),
                    })
                    .collect(),
                local_apps: capabilities
                    .local_apps
                    .into_iter()
                    .map(|app| LocalAppDefinition {
                        app_id: app.app_id,
                        name: app.name,
                        version: app.version,
                        description: app.description,
                        methods: methods(app.methods),
                        events: app
                            .events
                            .into_iter()
                            .map(|event| EventDefinition {
                                name: event.name,
                                description: event.description,
                                payload_schema: decode_json_or_null(
                                    "payloadSchemaJson",
                                    &event.payload_schema_json,
                                )
                                .unwrap(),
                            })
                            .collect(),
                    })
                    .collect(),
            })
        }
        DeviceFrameKind::InvokeResult(invoke) => AgentMessage::InvokeResult(result(invoke)),
        DeviceFrameKind::LocalAppInvokeResult(invoke) => {
            AgentMessage::LocalAppInvokeResult(result(invoke))
        }
        DeviceFrameKind::LocalAppEventEmitted(event) => {
            AgentMessage::LocalAppEventEmitted(event.try_into().unwrap())
        }
        DeviceFrameKind::StreamOutput(output) => {
            AgentMessage::StreamOutput(output.try_into().unwrap())
        }
        DeviceFrameKind::StreamResync(resync) => AgentMessage::StreamResync(resync.into()),
        DeviceFrameKind::StreamClosed(closed) => {
            AgentMessage::StreamClosed(closed.try_into().unwrap())
        }
        DeviceFrameKind::Error(error) => AgentMessage::Error(crate::protocol::ProtocolError {
            request_id: error.request_id,
            message: error.message,
        }),
        DeviceFrameKind::Heartbeat(_) => return None,
    })
}

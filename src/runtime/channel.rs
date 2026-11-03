// Device ↔ Relay transport: one gRPC bidirectional stream per connection
// (`baijimu.relay.device.v1.DeviceChannel/Connect`). tonic and h2 own the
// socket; this file only opens the stream and translates frames.

/// Device frames buffered toward the HTTP/2 stream before senders wait.
const RELAY_OUTBOUND_BUFFER: usize = 16;

struct RelayChannel {
    outbound: mpsc::Sender<DeviceFrame>,
    inbound: tonic::Streaming<RelayFrame>,
}

enum RelayInbound {
    Message(AgentMessage),
    Heartbeat,
    /// A frame kind this client does not know, sent by a newer Relay.
    Unsupported,
    UpdateRequired(RelayUpdateRequired),
}

/// The gRPC origin of the configured relay URL. Existing configs keep the
/// WebSocket form (`wss://host/ws/agent`); only scheme, host and port are used.
fn relay_endpoint(relay_url: &str) -> Result<Url> {
    let mut url =
        Url::parse(relay_url.trim()).with_context(|| format!("invalid relay url {relay_url}"))?;
    let scheme = match url.scheme() {
        "wss" | "https" => "https",
        "ws" | "http" => "http",
        other => bail!("unsupported relay url scheme {other}"),
    };
    if url.set_scheme(scheme).is_err()
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        bail!("relay url must be an absolute URL without credentials");
    }
    url.set_path("/");
    url.set_query(None);
    url.set_fragment(None);
    Ok(url)
}

/// Opens the device stream with `capabilities` as its first frame. A Relay
/// rejection surfaces as a `tonic::Status` inside the error.
async fn open_relay_channel(
    endpoint: &Url,
    agent_id: &str,
    token: &str,
    capabilities: AgentMessage,
) -> Result<RelayChannel> {
    let mut builder = Endpoint::from_shared(endpoint.to_string())?
        .connect_timeout(Duration::from_secs(RELAY_CONNECT_TIMEOUT_SECS));
    if endpoint.scheme() == "https" {
        builder = builder.tls_config(ClientTlsConfig::new().with_webpki_roots())?;
    }
    let mut client = DeviceChannelClient::new(builder.connect_lazy())
        .max_decoding_message_size(MAX_FRAME_BYTES)
        .max_encoding_message_size(MAX_FRAME_BYTES);

    let (outbound, receiver) = mpsc::channel(RELAY_OUTBOUND_BUFFER);
    outbound
        .send(encode_device_frame(capabilities)?)
        .await
        .expect("receiver is held below");
    let frames = futures_util::stream::unfold(receiver, |mut receiver| async move {
        receiver.recv().await.map(|frame| (frame, receiver))
    });
    let mut request = tonic::Request::new(frames);
    let metadata = request.metadata_mut();
    metadata.insert(
        "authorization",
        format!("Bearer {token}")
            .parse()
            .context("device token is not valid metadata")?,
    );
    metadata.insert(
        DEVICE_ID_METADATA,
        agent_id.parse().context("device id is not valid metadata")?,
    );
    let inbound = client.connect(request).await?.into_inner();
    Ok(RelayChannel { outbound, inbound })
}

fn is_relay_authorization_error(status: &tonic::Status) -> bool {
    matches!(
        status.code(),
        tonic::Code::Unauthenticated | tonic::Code::PermissionDenied
    )
}

fn heartbeat_frame() -> DeviceFrame {
    DeviceFrame {
        frame: Some(DeviceFrameKind::Heartbeat(v1::Heartbeat {})),
    }
}

fn encode_device_frame(message: AgentMessage) -> Result<DeviceFrame> {
    let frame = match message {
        AgentMessage::Capabilities(capabilities) => {
            DeviceFrameKind::Capabilities(v1::Capabilities {
                agent_id: capabilities.agent_id,
                protocol_version: capabilities.protocol_version,
                protocol_features: capabilities.protocol_features,
                services: capabilities
                    .services
                    .into_iter()
                    .map(|service| v1::ServiceDefinition {
                        name: service.name,
                        description: service.description,
                        methods: encode_methods(service.methods),
                    })
                    .collect(),
                local_apps: capabilities
                    .local_apps
                    .into_iter()
                    .map(encode_local_app)
                    .collect(),
                client_version: Some(CLIENT_VERSION.to_string()),
                client_platform: Some(client_platform().to_string()),
                client_arch: Some(client_arch().to_string()),
                client_os_version: client_os_version(),
            })
        }
        AgentMessage::InvokeResult(result) => DeviceFrameKind::InvokeResult(encode_result(result)),
        AgentMessage::LocalAppInvokeResult(result) => {
            DeviceFrameKind::LocalAppInvokeResult(encode_result(result))
        }
        AgentMessage::LocalAppEventEmitted(event) => {
            DeviceFrameKind::LocalAppEventEmitted(event.into())
        }
        AgentMessage::StreamOutput(output) => DeviceFrameKind::StreamOutput(output.into()),
        AgentMessage::StreamResync(resync) => DeviceFrameKind::StreamResync(resync.into()),
        AgentMessage::StreamClosed(closed) => DeviceFrameKind::StreamClosed(closed.into()),
        AgentMessage::Error(error) => DeviceFrameKind::Error(v1::ProtocolError {
            request_id: error.request_id,
            message: error.message,
        }),
        AgentMessage::RegisteredAck(_)
        | AgentMessage::InvokeRequest(_)
        | AgentMessage::LocalAppInvokeRequest(_)
        | AgentMessage::EventAck(_)
        | AgentMessage::DeviceSubscriptions(_)
        | AgentMessage::StreamAttach(_)
        | AgentMessage::StreamDetach(_) => bail!("relay-originated message cannot be sent to relay"),
    };
    Ok(DeviceFrame { frame: Some(frame) })
}

fn encode_methods(methods: Vec<MethodDefinition>) -> Vec<v1::MethodDefinition> {
    methods
        .into_iter()
        .map(|method| v1::MethodDefinition {
            name: method.name,
            description: method.description,
            input_schema_json: encode_json(&method.input_schema),
            response_mode: match method.response_mode {
                ResponseMode::Cmodel => v1::ResponseMode::Cmodel,
                ResponseMode::Plain => v1::ResponseMode::Plain,
                ResponseMode::Passthrough => v1::ResponseMode::Passthrough,
            } as i32,
        })
        .collect()
}

fn encode_local_app(app: LocalAppDefinition) -> v1::LocalAppDefinition {
    v1::LocalAppDefinition {
        app_id: app.app_id,
        name: app.name,
        version: app.version,
        description: app.description,
        methods: encode_methods(app.methods),
        events: app
            .events
            .into_iter()
            .map(|event| v1::EventDefinition {
                name: event.name,
                description: event.description,
                payload_schema_json: encode_json(&event.payload_schema),
            })
            .collect(),
    }
}

/// A JSON `null` here was read back as an absent field over WebSocket; keep
/// that meaning instead of sending a present `null`.
fn encode_optional_json(value: Option<Value>) -> Option<Vec<u8>> {
    value
        .filter(|value| !value.is_null())
        .map(|value| encode_json(&value))
}

fn encode_result(result: InvokeResult) -> v1::InvokeResult {
    v1::InvokeResult {
        request_id: result.request_id,
        success: result.success,
        data_json: encode_optional_json(result.data),
        error: result.error.map(|error| v1::InvokeError {
            public_details_json: encode_optional_json(error.public_details),
            code: error.code,
            message: error.message,
        }),
        duration_ms: result.duration_ms,
    }
}

fn decode_relay_frame(frame: RelayFrame) -> Result<RelayInbound> {
    let Some(frame) = frame.frame else {
        return Ok(RelayInbound::Unsupported);
    };
    Ok(RelayInbound::Message(match frame {
        RelayFrameKind::RegisteredAck(ack) => AgentMessage::RegisteredAck(RegisteredAck {
            agent_id: ack.agent_id,
            workspace_id: ack.workspace_id,
            connection_id: ack.connection_id,
            registered_at_epoch_seconds: ack.registered_at_epoch_seconds,
            heartbeat_timeout_secs: ack.heartbeat_timeout_secs,
        }),
        RelayFrameKind::InvokeRequest(request) => AgentMessage::InvokeRequest(InvokeRequest {
            request_id: request.request_id,
            service: request.service,
            method: request.method,
            arguments: decode_json_or_null("argumentsJson", &request.arguments_json)?,
            timeout_secs: request.timeout_secs,
        }),
        RelayFrameKind::LocalAppInvokeRequest(request) => {
            AgentMessage::LocalAppInvokeRequest(LocalAppInvokeRequest {
                request_id: request.request_id,
                workspace_id: request.workspace_id,
                app_id: request.app_id,
                method: request.method,
                arguments: decode_json_or_null("argumentsJson", &request.arguments_json)?,
                timeout_secs: request.timeout_secs,
            })
        }
        RelayFrameKind::EventAck(ack) => AgentMessage::EventAck(ack.try_into()?),
        RelayFrameKind::DeviceSubscriptions(snapshot) => {
            AgentMessage::DeviceSubscriptions(snapshot.try_into()?)
        }
        RelayFrameKind::StreamAttach(attach) => AgentMessage::StreamAttach(attach.into()),
        RelayFrameKind::StreamDetach(detach) => AgentMessage::StreamDetach(detach.into()),
        RelayFrameKind::Error(error) => AgentMessage::Error(ProtocolError {
            request_id: error.request_id,
            message: error.message,
        }),
        RelayFrameKind::Heartbeat(_) => return Ok(RelayInbound::Heartbeat),
        RelayFrameKind::UpdateRequired(notice) => {
            return Ok(RelayInbound::UpdateRequired(RelayUpdateRequired {
                minimum_supported_version: notice.minimum_supported_version,
                message: notice.message,
            }))
        }
    }))
}

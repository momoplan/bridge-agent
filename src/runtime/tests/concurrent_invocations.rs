use crate::event_delivery::{EventQueue, QueueIdentity};
use super::{LocalAppEventSubmission, RuntimeInner, RuntimeRegistryUpdate, RuntimeRunner};
use crate::services::ServiceRegistry;
use tokio::sync::{mpsc, watch, Notify, RwLock};

struct InvocationHarness {
    capabilities: v1::Capabilities,
    socket: RelaySession,
    _relay: FakeRelay,
    apply: mpsc::UnboundedSender<RuntimeRegistryUpdate>,
    events: mpsc::Sender<LocalAppEventSubmission>,
    shutdown: watch::Sender<bool>,
    task: tokio::task::JoinHandle<anyhow::Result<()>>,
    http: tokio::task::JoinHandle<()>,
    entered: Arc<AtomicUsize>,
    release: Arc<Notify>,
    registry: ServiceRegistry,
    _dir: tempfile::TempDir,
}

impl InvocationHarness {
    async fn new() -> Self {
        Self::with_config(|_, _, _| {}).await
    }

    async fn with_config(
        configure: impl FnOnce(&mut AgentConfig, &std::path::Path, &str),
    ) -> Self {
        let entered = Arc::new(AtomicUsize::new(0));
        let release = Arc::new(Notify::new());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handler_entered = entered.clone();
        let handler_release = release.clone();
        let app = axum::Router::new().route(
            "/invoke",
            axum::routing::post(move |axum::Json(body): axum::Json<Value>| {
                let entered = handler_entered.clone();
                let release = handler_release.clone();
                async move {
                    if body["slow"] == true {
                        entered.fetch_add(1, Ordering::SeqCst);
                        release.notified().await;
                    }
                    axum::Json(json!({"ok":true,"data":{"echo":body["echo"]}}))
                }
            }),
        );
        let http = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let dir = tempdir().unwrap();
        let token_path = dir.path().join("token");
        fs::write(&token_path, "bjm_app_test_concurrent_private_runtime_token").unwrap();
        let method = json!({"name":"call","description":"test","enabled":true,
            "input_schema":{},"response_mode":"cmodel","binding":{
                "type":"http","url":format!("http://{addr}/invoke"),
                "http_method":"POST","headers":{},"timeout_secs":60}});
        let mut config = AgentConfig::example();
        config.relay.agent_id = "dev_test".into();
        config.platform.environment_key = Some("test".into());
        config.platform.workspace_id = Some(1);
        config.services = vec![serde_json::from_value(json!({
            "name":"test-service","description":"test","enabled":true,"methods":[method.clone()]
        }))
        .unwrap()];
        config.local_apps = vec![serde_json::from_value(json!({
            "appId":"test-app","name":"test","version":"1.0.0","description":"test",
            "enabled":true,"methods":[method],"events":[],"startCommand":{
                "type":"shell_command","command":["unused"],"env":{
                    "BAIJIMU_LOCAL_APP_TOKEN_FILE":token_path.to_string_lossy()
                },"timeout_secs":1}
        }))
        .unwrap()];
        configure(&mut config, dir.path(), &format!("http://{addr}"));
        let registry = ServiceRegistry::from_config(&config, dir.path()).unwrap();
        let mut relay = FakeRelay::start();
        let relay_endpoint = relay_endpoint(&relay.url).unwrap();
        let (shutdown, mut shutdown_rx) = watch::channel(false);
        let (apply, mut apply_rx) = mpsc::unbounded_channel();
        let (events, mut event_rx) = mpsc::channel(8);
        let (audit_tx, mut audit_rx) = mpsc::unbounded_channel();
        let runner = RuntimeRunner {
            event_queue: Some(EventQueue::open(&dir.path().join("events.sqlite"), QueueIdentity {
                environment: "test".into(), workspace_id:42, device_id: config.relay.agent_id.clone(),
            }).unwrap()),
            inner: Arc::new(RuntimeInner::default()),
            log_limit: 100,
            config,
            config_path: dir.path().join("config.json").display().to_string(),
            relay_endpoint: relay_endpoint.clone(),
            registry: Arc::new(RwLock::new(registry.clone())),
        };
        let task = tokio::spawn(async move {
            let _keep_audit = audit_tx;
            let capabilities = runner.current_capabilities().await;
            let channel =
                open_relay_channel(&relay_endpoint, "dev_test", "test-token", capabilities)
                    .await
                    .unwrap();
            runner
                .handle_connection(
                    channel,
                    &mut shutdown_rx,
                    &mut apply_rx,
                    &mut event_rx,
                    &mut audit_rx,
                )
                .await
        });
        let mut socket = relay.accept().await;
        let DeviceFrameKind::Capabilities(capabilities) = socket.next_frame().await else {
            panic!("expected the initial device registration");
        };
        Self {
            capabilities,
            socket,
            _relay: relay,
            apply,
            events,
            shutdown,
            task,
            http,
            entered,
            release,
            registry,
            _dir: dir,
        }
    }

    async fn call(&mut self, id: &str, local: bool, slow: bool) {
        let value = if local {
            json!({"type":"local_app_invoke_request","requestId":id,
            "workspaceId":1,"appId":"test-app","method":"call",
            "arguments":{"slow":slow,"echo":id},"timeoutSecs":60})
        } else {
            json!({"type":"invoke_request","request_id":id,"service":"test-service",
            "method":"call","arguments":{"slow":slow,"echo":id},"timeout_secs":60})
        };
        self.socket.send(value).await;
    }

    async fn receive(&mut self) -> Value {
        tokio::time::timeout(std::time::Duration::from_secs(2), self.socket.receive())
            .await
            .expect("relay control/result blocked behind slow invocation")
    }

    async fn entered(&self, count: usize) {
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            while self.entered.load(Ordering::SeqCst) < count {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("slow request did not reach HTTP endpoint");
    }
}

impl Drop for InvocationHarness {
    fn drop(&mut self) {
        self.task.abort();
        self.http.abort();
    }
}

#[tokio::test]
async fn slow_local_call_does_not_block_results_heartbeats_events_or_registry_refresh() {
    let mut h = InvocationHarness::new().await;
    h.call("slow", true, true).await;
    h.entered(1).await;
    h.call("fast", false, false).await;
    h.socket.heartbeat().await;
    h.apply
        .send(RuntimeRegistryUpdate {
            services: h.registry.definitions(),
            local_apps: h.registry.local_app_definitions(),
            registry: h.registry.clone(),
        })
        .unwrap();
    let (response, ack_rx) = oneshot::channel();
    h.events
        .send(LocalAppEventSubmission {
            event: serde_json::from_value(json!({"eventId":"evt-concurrent","appId":"test-app",
            "event":"changed","payload":{},"targetConsumers":["worker"]}))
            .unwrap(),
            response,
        })
        .await
        .unwrap();
    let mut fast = false;
    let mut registry = false;
    let mut event = false;
    while !(fast && registry && event) {
        let v = h.receive().await;
        match v["type"].as_str().unwrap() {
            "invoke_result" => {
                assert_eq!(v["request_id"], "fast");
                assert_eq!(v["success"], true);
                fast = true;
            }
            "capabilities" => registry = true,
            "local_app_event_emitted" => {
                event = true;
                h.socket
                    .send(json!({"type":"event_ack","eventId":"evt-concurrent",
                        "appId":"test-app","receipts":[{"consumerId":"worker",
                        "receipt":{"status":"processed","duplicate":false}}]}))
                    .await;
            }
            kind => panic!("unexpected response {kind}"),
        }
    }
    let ack = tokio::time::timeout(std::time::Duration::from_secs(2), ack_rx)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(ack.event_id, "evt-concurrent");
    h.release.notify_one();
    let result = h.receive().await;
    assert_eq!(result["type"], "local_app_invoke_result");
    assert_eq!(result["request_id"], "slow");
    assert_eq!(result["success"], true);
}

#[tokio::test]
async fn invocation_capacity_rejects_before_execution_and_shutdown_does_not_wait() {
    let mut h = InvocationHarness::new().await;
    for i in 0..super::MAX_RELAY_INVOCATIONS {
        h.call(&format!("slow-{i}"), true, true).await;
    }
    h.entered(super::MAX_RELAY_INVOCATIONS).await;
    for local in [true, false] {
        h.call("over-capacity", local, false).await;
        let v = h.receive().await;
        assert_eq!(v["error"]["code"], "AGENT_BUSY");
        assert_eq!(v["request_id"], "over-capacity");
        assert_eq!(
            v["type"],
            if local {
                "local_app_invoke_result"
            } else {
                "invoke_result"
            }
        );
    }
    // A full invocation set still leaves the stream reading relay frames.
    h.socket.heartbeat().await;
    h.call("still-busy", false, false).await;
    assert_eq!(h.receive().await["error"]["code"], "AGENT_BUSY");
    h.shutdown.send(true).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(2), &mut h.task)
        .await
        .expect("shutdown waited for active calls")
        .unwrap()
        .unwrap();
    assert_eq!(
        h.entered.load(Ordering::SeqCst),
        super::MAX_RELAY_INVOCATIONS
    );
}

#[tokio::test]
async fn relay_disconnect_drops_active_invocations_without_waiting_or_replay() {
    let mut h = InvocationHarness::new().await;
    h.call("slow-before-disconnect", true, true).await;
    h.entered(1).await;
    h.socket.close();
    let result = tokio::time::timeout(std::time::Duration::from_secs(2), &mut h.task)
        .await
        .expect("disconnect waited for active calls")
        .unwrap();
    assert!(result.is_err());
    assert_eq!(h.entered.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn bound_call_cannot_dispatch_to_an_unproven_app_by_name() {
    let mut h = InvocationHarness::new().await;
    h.socket.send_frame(v1::RelayFrame {
        frame: Some(RelayFrameKind::BoundLocalAppInvokeRequest(v1::BoundLocalAppInvokeRequest {
            request_id: "bound".into(),
            workspace_id: 1,
            application_runtime_id: "runtime-a".into(),
            local_app_locator: "private/example/LOCAL_APP/selected".into(),
            source: Some(v1::LocalAppSource {
                environment_key: "private".into(),
                app_id: "test-app".into(),
                version: "1.0.0".into(),
            }),
            method: "call".into(),
            arguments_json: br#"{"slow":true}"#.to_vec(),
        })),
    }).await;
    let result = h.receive().await;
    assert_eq!(result["type"], "local_app_invoke_result");
    assert_eq!(result["request_id"], "bound");
    assert_eq!(result["success"], false);
    assert_eq!(result["error"]["code"], "BOUND_LOCAL_APP_DENIED");
    assert_eq!(h.entered.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn registered_bound_call_returns_installed_connectors_reply() {
    use crate::connector::tests::{connector_test_env, install_bound_call_fixture};
    let installation = tempdir().unwrap();
    let _env = connector_test_env(installation.path().join("connectors"));
    let mut harness = InvocationHarness::with_config(install_bound_call_fixture).await;
    let app = harness.capabilities.local_apps.iter().find(|app| app.app_id == "test-app").unwrap();
    let source = app.installed_source.clone().expect("device must register its installed source");
    assert_eq!(source.environment_key, "test");
    assert_eq!(source.app_id, app.app_id);
    assert_eq!(source.version, "1.0.0");
    assert!(app.methods.iter().any(|method| method.name == "ping"));
    harness.socket.send(json!({
        "type": "registered_ack", "agent_id": "dev_test", "workspace_id": 1,
        "connection_id": "bound-connection", "registered_at_epoch_seconds": 1,
        "heartbeat_timeout_secs": 75
    })).await;
    let call = v1::BoundLocalAppInvokeRequest {
        request_id: "registered-call".into(),
        workspace_id: 1,
        application_runtime_id: "test-runtime".into(),
        local_app_locator: "test/example/LOCAL_APP/selected".into(),
        source: Some(source),
        method: "ping".into(),
        arguments_json: serde_json::to_vec(&json!({"echo": "device-result"})).unwrap(),
    };
    harness.socket.send_frame(v1::RelayFrame {
        frame: Some(RelayFrameKind::BoundLocalAppInvokeRequest(call)),
    }).await;
    let reply = harness.receive().await;
    assert_eq!(reply["type"], "local_app_invoke_result");
    assert_eq!(reply["request_id"], "registered-call");
    assert_eq!(reply["success"], true, "{reply}");
    assert_eq!(reply["data"]["echo"], "device-result");
}

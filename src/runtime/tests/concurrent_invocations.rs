use super::{LocalAppEventSubmission, RuntimeInner, RuntimeRegistryUpdate, RuntimeRunner};
use crate::services::ServiceRegistry;
use tokio::sync::{mpsc, watch, Notify, RwLock};

type TestSocket = tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>;

struct InvocationHarness {
    socket: TestSocket,
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
        let registry = ServiceRegistry::from_config(&config, dir.path()).unwrap();
        let relay = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let ws_url =
            url::Url::parse(&format!("ws://{}/relay", relay.local_addr().unwrap())).unwrap();
        let (shutdown, mut shutdown_rx) = watch::channel(false);
        let (apply, mut apply_rx) = mpsc::unbounded_channel();
        let (events, mut event_rx) = mpsc::channel(8);
        let (audit_tx, mut audit_rx) = mpsc::unbounded_channel();
        let runner = RuntimeRunner {
            inner: Arc::new(RuntimeInner::default()),
            log_limit: 100,
            config,
            config_path: dir.path().join("config.json").display().to_string(),
            ws_url: ws_url.clone(),
            registry: Arc::new(RwLock::new(registry.clone())),
        };
        let task = tokio::spawn(async move {
            let _keep_audit = audit_tx;
            let (stream, _) = tokio_tungstenite::connect_async(ws_url.as_str())
                .await
                .unwrap();
            runner
                .handle_connection(
                    stream,
                    &mut shutdown_rx,
                    &mut apply_rx,
                    &mut event_rx,
                    &mut audit_rx,
                )
                .await
        });
        let (stream, _) = relay.accept().await.unwrap();
        let mut socket = accept_async(stream).await.unwrap();
        assert!(matches!(
            socket.next().await.unwrap().unwrap(),
            Message::Text(_)
        ));
        Self {
            socket,
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
        self.socket
            .send(Message::Text(value.to_string().into()))
            .await
            .unwrap();
    }

    async fn receive(&mut self) -> Message {
        tokio::time::timeout(std::time::Duration::from_secs(2), self.socket.next())
            .await
            .expect("relay control/result blocked behind slow invocation")
            .unwrap()
            .unwrap()
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
async fn slow_local_call_does_not_block_results_ping_events_or_registry_refresh() {
    let mut h = InvocationHarness::new().await;
    h.call("slow", true, true).await;
    h.entered(1).await;
    h.call("fast", false, false).await;
    h.socket
        .send(Message::Ping(vec![1, 2, 3].into()))
        .await
        .unwrap();
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
            "event":"changed","payload":{}}))
            .unwrap(),
            response,
        })
        .await
        .unwrap();
    let mut fast = false;
    let mut pong = false;
    let mut registry = false;
    let mut event = false;
    while !(fast && pong && registry && event) {
        match h.receive().await {
            Message::Pong(data) => {
                assert_eq!(data.as_ref(), &[1, 2, 3]);
                pong = true;
            }
            Message::Text(text) => {
                let v: Value = serde_json::from_str(&text).unwrap();
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
                            .send(Message::Text(
                                json!({"type":"event_ack","eventId":"evt-concurrent",
                            "appId":"test-app","matchedSubscriptionCount":1})
                                .to_string()
                                .into(),
                            ))
                            .await
                            .unwrap();
                    }
                    kind => panic!("unexpected response {kind}"),
                }
            }
            other => panic!("unexpected frame {other:?}"),
        }
    }
    let ack = tokio::time::timeout(std::time::Duration::from_secs(2), ack_rx)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(ack.event_id, "evt-concurrent");
    h.release.notify_one();
    let Message::Text(text) = h.receive().await else {
        panic!("missing slow result")
    };
    let result: Value = serde_json::from_str(&text).unwrap();
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
        let Message::Text(text) = h.receive().await else {
            panic!("missing rejection")
        };
        let v: Value = serde_json::from_str(&text).unwrap();
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
    h.socket.send(Message::Ping(vec![9].into())).await.unwrap();
    assert!(matches!(h.receive().await, Message::Pong(_)));
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
    h.socket.send(Message::Close(None)).await.unwrap();
    let result = tokio::time::timeout(std::time::Duration::from_secs(2), &mut h.task)
        .await
        .expect("disconnect waited for active calls")
        .unwrap();
    assert!(result.is_err());
    assert_eq!(h.entered.load(Ordering::SeqCst), 1);
}

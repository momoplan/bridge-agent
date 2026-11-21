// Bound active calls without parking the relay reader behind a slow endpoint.
// Excess requests are explicitly rejected before execution; there is no retry queue.
const MAX_RELAY_INVOCATIONS: usize = 64;

impl<'a> RelayConnection<'a> {
    async fn queue_invocation(&mut self, request: AgentMessage) -> Result<()> {
        if self.invocations.len() >= MAX_RELAY_INVOCATIONS {
            let (request_id, local) = match request {
                AgentMessage::InvokeRequest(r) => (r.request_id, false),
                AgentMessage::BoundLocalAppInvokeRequest(r) => (r.request_id, true),
                AgentMessage::LocalAppInvokeRequest(r) => (r.request_id, true),
                _ => unreachable!("only invocation requests are queued"),
            };
            let result = crate::protocol::InvokeResult {
                request_id,
                success: false,
                data: None,
                error: Some(crate::protocol::InvokeError {
                    public_details: None,
                    code: "AGENT_BUSY".into(),
                    message: "Agent concurrent invocation limit reached; request was not executed"
                        .into(),
                }),
                duration_ms: 0,
            };
            let message = if local {
                AgentMessage::LocalAppInvokeResult(result)
            } else {
                AgentMessage::InvokeResult(result)
            };
            return self.send(message).await;
        }
        let future = match request {
            AgentMessage::BoundLocalAppInvokeRequest(r) => self.runner.handle_bound_invoke(*r, self.registered_workspace).boxed(),
            AgentMessage::InvokeRequest(r) => self.runner.handle_invoke_request(r).boxed(),
            AgentMessage::LocalAppInvokeRequest(r) => {
                self.runner.handle_local_app_invoke_request(r).boxed()
            }
            _ => unreachable!("only invocation requests are queued"),
        };
        self.invocations.push(future);
        Ok(())
    }
}

impl RuntimeRunner {
    async fn handle_bound_invoke(&self, request: crate::protocol::BoundLocalAppInvokeRequest, registered_workspace: Option<u64>) -> AgentMessage {
        let registry = self.registry.read().await.clone();
        let target = if registered_workspace == Some(request.workspace_id) {
            registry.bound_app_id(&request.source, &request.method)
        } else {
            Err(anyhow::anyhow!("bound call outside the registered device workspace"))
        };
        let result = match target {
            Ok(app_id) => registry.invoke_local_app(request.request_id, registered_workspace, app_id, &request.method, request.arguments, None).await,
            Err(_) => InvokeResult {
                request_id: request.request_id,
                success: false,
                data: None,
                error: Some(crate::protocol::InvokeError { code: "BOUND_LOCAL_APP_DENIED".into(), message: "Device registration or installed source does not authorize this method".into(), public_details: None }),
                duration_ms: 0,
            }
        };
        AgentMessage::LocalAppInvokeResult(result)
    }

    async fn handle_invoke_request(&self, request: crate::protocol::InvokeRequest) -> AgentMessage {
        let service = request.service.clone();
        let method = request.method.clone();
        let request_id = request.request_id.clone();
        self.push_log_with_metadata(
            "info",
            &format!("invoke {service}.{method} started"),
            LogMetadata::category("invoke")
                .service(service.clone())
                .method(method.clone())
                .request_id(request_id.clone())
                .outcome("started"),
        )
        .await;
        let registry = self.registry.read().await.clone();
        let result = registry
            .invoke(
                request.request_id,
                &service,
                &method,
                request.arguments,
                request.timeout_secs,
            )
            .await;
        let (level, outcome, suffix) = if result.success {
            ("info", "succeeded", String::new())
        } else {
            let error = result
                .error
                .as_ref()
                .map(|err| format!("{}: {}", err.code, err.message))
                .unwrap_or_else(|| "unknown error".to_string());
            ("warn", "failed", format!(": {error}"))
        };
        self.push_log_with_metadata(
            level,
            &format!(
                "invoke {service}.{method} {outcome} in {}ms{suffix}",
                result.duration_ms
            ),
            LogMetadata::category("invoke")
                .service(service)
                .method(method)
                .request_id(request_id)
                .outcome(outcome)
                .duration_ms(result.duration_ms),
        )
        .await;
        AgentMessage::InvokeResult(result)
    }

    async fn handle_local_app_invoke_request(
        &self,
        request: crate::protocol::LocalAppInvokeRequest,
    ) -> AgentMessage {
        let app_id = request.app_id.clone();
        let method = request.method.clone();
        let request_id = request.request_id.clone();
        self.push_log_with_metadata(
            "info",
            &format!("local app invoke {app_id}.{method} started"),
            LogMetadata::category("local_app_invoke")
                .method(method.clone())
                .request_id(request_id.clone())
                .outcome("started"),
        )
        .await;
        let registry = self.registry.read().await.clone();
        let result = registry
            .invoke_local_app(
                request.request_id,
                request.workspace_id,
                &app_id,
                &method,
                request.arguments,
                request.timeout_secs,
            )
            .await;
        let outcome = if result.success {
            "succeeded"
        } else {
            "failed"
        };
        self.push_log_with_metadata(
            if result.success { "info" } else { "warn" },
            &format!(
                "local app invoke {app_id}.{method} {outcome} in {}ms",
                result.duration_ms
            ),
            LogMetadata::category("local_app_invoke")
                .method(method)
                .request_id(request_id)
                .outcome(outcome)
                .duration_ms(result.duration_ms),
        )
        .await;
        AgentMessage::LocalAppInvokeResult(result)
    }
}

impl HttpMethod {
    async fn invoke(&self, arguments: Value, timeout_secs: Option<u64>) -> Result<ServiceOutcome> {
        self.invoke_with_workspace(arguments, timeout_secs, None)
            .await
    }

    async fn invoke_with_workspace(
        &self,
        arguments: Value,
        timeout_secs: Option<u64>,
        workspace_id: Option<u64>,
    ) -> Result<ServiceOutcome> {
        let timeout_secs = timeout_secs.unwrap_or(self.timeout_secs);
        let mut request = self
            .client
            .request(self.http_method.clone(), &self.url)
            .timeout(Duration::from_secs(timeout_secs));

        for (key, value) in &self.headers {
            request = request.header(key, value);
        }
        if let Some(workspace_id) = workspace_id {
            request = request.header(BAIJIMU_WORKSPACE_ID_HEADER, workspace_id.to_string());
        }

        if matches!(self.http_method, Method::GET | Method::DELETE) {
            let query = query_pairs_from_json(&arguments);
            request = request.query(&query);
        } else {
            request = request.json(&arguments);
        }

        let response = request.send().await.with_context(|| {
            format!(
                "failed to call local http binding for {}.{}",
                self.service_name, self.method_name
            )
        })?;

        let status = response.status();
        let headers = collect_headers(response.headers());
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .to_string();
        let bytes = response.bytes().await?;
        if self.response_mode == ResponseMode::Passthrough {
            return Ok(passthrough_http_outcome(
                status.as_u16(),
                headers,
                bytes.as_ref(),
            ));
        }

        let body = decode_response_body(&bytes, &content_type);
        if self.response_mode == ResponseMode::Plain {
            return Ok(ServiceOutcome {
                success: true,
                data: Some(body),
                error: None,
            });
        }

        if let Some(outcome) =
            normalize_connector_http_outcome(status.is_success(), status.as_u16(), &body)
        {
            return Ok(outcome);
        }

        let operation = format!(
            "local HTTP binding {}.{}",
            self.service_name, self.method_name
        );
        let outcome = normalize_cmodel_http_response(status, &bytes, &operation);
        Ok(ServiceOutcome {
            success: outcome.success,
            data: outcome.data,
            error: outcome.error,
        })
    }
}

fn normalize_connector_http_outcome(
    http_success: bool,
    status_code: u16,
    body: &Value,
) -> Option<ServiceOutcome> {
    if let Some(success) = body
        .get("success")
        .or_else(|| body.get("ok"))
        .and_then(Value::as_bool)
    {
        return Some(ServiceOutcome {
            success: http_success && success,
            data: body.get("data").cloned(),
            error: if http_success && success {
                None
            } else {
                Some(extract_invoke_error(body).unwrap_or_else(|| InvokeError {
                    code: if http_success {
                        "INVOKE_FAILED".to_string()
                    } else {
                        "HTTP_REQUEST_FAILED".to_string()
                    },
                    message: if http_success {
                        "local connector returned an unsuccessful response".to_string()
                    } else {
                        format!("local endpoint returned status {status_code}")
                    },
                }))
            },
        });
    }

    None
}

fn passthrough_http_outcome(
    status: u16,
    headers: BTreeMap<String, String>,
    bytes: &[u8],
) -> ServiceOutcome {
    let (body, body_encoding) = match std::str::from_utf8(bytes) {
        Ok(text) => (text.to_string(), "utf8"),
        Err(_) => (BASE64_STANDARD.encode(bytes), "base64"),
    };
    ServiceOutcome {
        success: true,
        data: Some(json!({
            "status": status,
            "headers": headers,
            "body": body,
            "bodyEncoding": body_encoding,
        })),
        error: None,
    }
}

fn extract_invoke_error(body: &Value) -> Option<InvokeError> {
    let error = body.get("error")?;
    if error.is_null() {
        return None;
    }
    if let Some(message) = error.as_str() {
        return Some(InvokeError {
            code: "INVOKE_FAILED".to_string(),
            message: message.to_string(),
        });
    }
    Some(InvokeError {
        code: error
            .get("code")
            .and_then(Value::as_str)
            .unwrap_or("INVOKE_FAILED")
            .to_string(),
        message: error
            .get("message")
            .or_else(|| error.get("value"))
            .and_then(Value::as_str)
            .unwrap_or("local connector returned an error")
            .to_string(),
    })
}

use super::ServiceRegistry;
use crate::config::{AgentConfig, HttpBinding, MethodBinding, MethodConfig, ServiceConfig};
use crate::protocol::ResponseMode;
use axum::{routing::post, Json, Router};
use serde_json::json;
use std::collections::BTreeMap;
use tokio::net::TcpListener;

#[tokio::test]
async fn http_binding_preserves_http_200_cmodel_failure_details() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new().route(
                "/invoke",
                post(|| async {
                    Json(json!({
                        "contractVersion": "1.0.0",
                        "errorCode": "CUSTOM_CONNECTOR_FAILURE",
                        "data": {
                            "message": "当前账户余额不足",
                            "retryable": false
                        }
                    }))
                }),
            ),
        )
        .await
        .unwrap();
    });

    let registry = registry_for_url(&format!("http://{addr}/invoke"));
    let result = registry
        .invoke(
            "req-http-cmodel-failure".to_string(),
            "localTool",
            "fetch",
            json!({}),
            None,
        )
        .await;

    assert!(!result.success);
    let error = result.error.unwrap();
    assert_eq!(error.code, "CUSTOM_CONNECTOR_FAILURE");
    assert_eq!(error.message, "当前账户余额不足");
    server.abort();
}

#[tokio::test]
async fn http_binding_accepts_legacy_non_200_cmodel_failure() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new().route(
                "/invoke",
                post(|| async {
                    (
                        axum::http::StatusCode::NOT_FOUND,
                        Json(json!({
                            "contractVersion": "1.0.0",
                            "errorCode": "RESOURCE_NOT_FOUND",
                            "data": null
                        })),
                    )
                }),
            ),
        )
        .await
        .unwrap();
    });

    let registry = registry_for_url(&format!("http://{addr}/invoke"));
    let result = registry
        .invoke(
            "req-http-cmodel-legacy-failure".to_string(),
            "localTool",
            "fetch",
            json!({}),
            None,
        )
        .await;

    assert!(!result.success);
    let error = result.error.unwrap();
    assert_eq!(error.code, "RESOURCE_NOT_FOUND");
    assert_eq!(
        error.message,
        "local endpoint returned CModel failure RESOURCE_NOT_FOUND"
    );
    server.abort();
}

fn registry_for_url(url: &str) -> ServiceRegistry {
    let current_dir = std::env::current_dir().unwrap();
    let mut config = AgentConfig::example();
    config.services.push(ServiceConfig {
        name: "localTool".to_string(),
        description: "Local HTTP tool.".to_string(),
        enabled: true,
        health_check: None,
        start_command: None,
        stop_command: None,
        methods: vec![MethodConfig {
            name: "fetch".to_string(),
            description: "Fetch data.".to_string(),
            enabled: true,
            input_schema: json!({"type": "object"}),
            response_mode: ResponseMode::Cmodel,
            binding: MethodBinding::Http(HttpBinding {
                url: url.to_string(),
                http_method: "POST".to_string(),
                headers: BTreeMap::new(),
                timeout_secs: Some(5),
            }),
        }],
    });
    ServiceRegistry::from_config(&config, &current_dir).unwrap()
}

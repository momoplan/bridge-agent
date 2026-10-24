use super::{
    failure::ConsumerFailure,
    reader::{artifact_response, decode},
};

fn response(status: u16, body: &str) -> reqwest::Response {
    axum::http::Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .header("x-request-id", "header-request")
        .header("x-trace-id", "header-trace")
        .body(body.to_owned())
        .unwrap()
        .into()
}

#[tokio::test]
async fn business_failure_keeps_details_through_string_command_boundary() {
    let body = r#"{"contractVersion":"1.0.0","errorCode":"TEST_DENIED","data":{"message":"无权读取所选版本","retryable":false,"requestId":"owner-request","traceId":"trace-1","violations":[{"field":"version"}]}}"#;
    for status in [200, 403] {
        let error = decode::<bool>(response(status, body)).await.unwrap_err();
        let failure = error.downcast_ref::<ConsumerFailure>().unwrap();
        assert_eq!(failure.error_code.as_deref(), Some("TEST_DENIED"));
        assert_eq!(failure.retryable, Some(false));
        let message = error.to_string();
        for expected in [
            "无权读取所选版本",
            "TEST_DENIED",
            "owner-request",
            "trace-1",
            "violations",
            "version",
        ] {
            assert!(message.contains(expected), "missing {expected}: {message}");
        }
        assert!(artifact_response(response(status, body))
            .await
            .unwrap_err()
            .to_string()
            .contains("TEST_DENIED"));
    }
}

#[tokio::test]
async fn null_detail_and_non_json_failure_remain_explicit_and_correlated() {
    let error = decode::<bool>(response(
        200,
        r#"{"contractVersion":"1.0.0","errorCode":"TEST_FAILED","data":null}"#,
    ))
    .await
    .unwrap_err();
    assert!(error.to_string().contains("TEST_FAILED"));
    assert!(error.to_string().contains("header-request"));
    let error = decode::<bool>(response(503, "<html>private-infrastructure-data</html>"))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("header-request"));
    assert!(!error.to_string().contains("private-infrastructure-data"));
    let failure = error.downcast_ref::<ConsumerFailure>().unwrap();
    assert_eq!(failure.trace_id.as_deref(), Some("header-trace"));
    assert_eq!(failure.status, Some(503));
    assert!(decode::<bool>(response(
        200,
        r#"{"contractVersion":"1.0.0","errorCode":"0","data":true}"#
    ))
    .await
    .unwrap());
}

#[tokio::test]
async fn success_envelope_instead_of_artifact_retains_correlation() {
    let error = artifact_response(response(
        200,
        r#"{"contractVersion":"1.0.0","errorCode":"0","data":true}"#,
    ))
    .await
    .unwrap_err();
    let failure = error.downcast_ref::<ConsumerFailure>().unwrap();
    assert_eq!(failure.request_id.as_deref(), Some("header-request"));
    assert_eq!(failure.trace_id.as_deref(), Some("header-trace"));
    assert_eq!(failure.status, Some(200));
    assert!(failure.error_code.is_none());
}

#[tokio::test]
async fn interrupted_response_keeps_both_correlation_ids() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = [0; 4096];
        socket.read(&mut request).await.unwrap();
        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\nx-request-id: read-request\r\nx-trace-id: read-trace\r\n\r\n{").await.unwrap();
        socket.shutdown().await.unwrap();
    });
    let response = reqwest::get(format!("http://{address}")).await.unwrap();
    let error = decode::<bool>(response).await.unwrap_err();
    let failure = error.downcast_ref::<ConsumerFailure>().unwrap();
    assert_eq!(failure.request_id.as_deref(), Some("read-request"));
    assert_eq!(failure.trace_id.as_deref(), Some("read-trace"));
    assert_eq!(failure.status, Some(200));
    assert!(failure.error_code.is_none());
    server.await.unwrap();
}

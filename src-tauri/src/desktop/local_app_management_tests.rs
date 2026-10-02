use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn mock_response(
    response: Vec<u8>,
) -> (ResolvedManagementRequest, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/management/test", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        let mut chunk = [0; 1024];
        while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
            let read = socket.read(&mut chunk).await.unwrap();
            assert!(read > 0, "request ended before its headers");
            request.extend_from_slice(&chunk[..read]);
            assert!(request.len() <= 4096, "unexpected mock request size");
        }
        let _ = socket.write_all(&response).await;
    });
    (
        ResolvedManagementRequest {
            url,
            method: "GET".into(),
            token: "test-management-token".into(),
        },
        task,
    )
}

#[tokio::test]
async fn forwards_application_error_code_and_state() {
    let body = r#"{"ok":false,"error":{"code":"STATE_COMMITTING","message":"正在提交"},"data":{"retryable":true}}"#;
    let response = format!("HTTP/1.1 409 Conflict\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}", body.len()).into_bytes();
    let (request, server) = mock_response(response).await;
    let error = send_connector_management_request(request, None)
        .await
        .unwrap_err();
    assert_eq!(error.code, "STATE_COMMITTING");
    assert!(error.message.contains("正在提交"));
    assert_eq!(error.data.unwrap()["retryable"], true);
    server.await.unwrap();
}

#[tokio::test]
async fn chunked_body_is_bounded_before_collecting_entire_response() {
    let size = LOCAL_APP_UI_MAX_MANAGEMENT_RESPONSE_BYTES + 1;
    let mut response =
        format!("HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n{size:x}\r\n").into_bytes();
    response.extend(vec![b'x'; size]);
    response.extend_from_slice(b"\r\n0\r\n\r\n");
    let (request, server) = mock_response(response).await;
    let error = send_connector_management_request(request, None)
        .await
        .unwrap_err();
    assert_eq!(error.code, "connector_management_response_too_large");
    server.await.unwrap();
}

#[tokio::test]
async fn connection_failure_has_an_actionable_code() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/management/test", listener.local_addr().unwrap());
    drop(listener);
    let error = send_connector_management_request(
        ResolvedManagementRequest {
            url,
            method: "GET".into(),
            token: "test-token".into(),
        },
        None,
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, "connector_management_unreachable");
    assert!(!error.message.contains("test-token"));
}

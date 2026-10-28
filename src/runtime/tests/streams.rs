mod streams {
    use super::*;
    use crate::protocol::{StreamAttach, StreamCloseCode};

    #[test]
    fn sse_parser_handles_chunked_crlf_comments_and_multiline_data() {
        let mut parser = SseParser::default();
        assert!(parser.push(b": keepalive\r\nevent: notif").unwrap().is_empty());
        let events = parser
            .push(b"ication\r\nid: s:1\r\ndata: {\"a\":\r\ndata: 1}\r\n\r\nid: s:2\ndata: {}\n\n")
            .unwrap();
        assert_eq!(
            events,
            vec![
                SseEvent {
                    event: Some("notification".into()),
                    id: Some("s:1".into()),
                    data: "{\"a\":\n1}".into(),
                },
                SseEvent {
                    event: None,
                    id: Some("s:2".into()),
                    data: "{}".into(),
                },
            ]
        );
        assert!(parser.push(b"event: x\n\n").unwrap().is_empty(), "no data, no dispatch");
    }

    #[test]
    fn decode_relay_frame_accepts_stream_control_frames() {
        let decode = |message: Value| {
            match decode_relay_frame(relay_frame(serde_json::from_value(message).unwrap())).unwrap() {
                RelayInbound::Message(message) => message,
                _ => panic!("expected a relay message"),
            }
        };
        match decode(json!({"type":"stream_attach","attachId":"a","workspaceId":7,"appId":"app","after":"s:3"})) {
            AgentMessage::StreamAttach(attach) => {
                assert_eq!(attach.after.as_deref(), Some("s:3"));
                assert_eq!(attach.workspace_id, 7);
            }
            other => panic!("unexpected {other:?}"),
        }
        assert!(matches!(
            decode(json!({"type":"stream_detach","attachId":"a"})),
            AgentMessage::StreamDetach(_)
        ));
    }

    fn attach(after: Option<&str>) -> StreamAttach {
        StreamAttach {
            attach_id: "attach".into(),
            workspace_id: 7,
            app_id: "app".into(),
            after: after.map(str::to_string),
        }
    }

    async fn serve_once(response: &'static str) -> (String, oneshot::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/streams/notifications", listener.local_addr().unwrap());
        let (request_tx, request_rx) = oneshot::channel();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut buffer = [0_u8; 1024];
            while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                let read = socket.read(&mut buffer).await.unwrap();
                request.extend_from_slice(&buffer[..read]);
            }
            let _ = request_tx.send(String::from_utf8(request).unwrap());
            socket.write_all(response.as_bytes()).await.unwrap();
            socket.shutdown().await.ok();
        });
        (url, request_rx)
    }

    fn target(url: String) -> LocalAppStreamTarget {
        LocalAppStreamTarget {
            url,
            headers: BTreeMap::from([("Authorization".to_string(), "Bearer secret".to_string())]),
        }
    }

    async fn next_frame(pumps: &mut StreamPumps) -> AgentMessage {
        let frame = tokio::time::timeout(std::time::Duration::from_secs(5), pumps.frames.recv())
            .await
            .unwrap()
            .unwrap();
        pumps.forwarded(&frame);
        frame
    }

    #[tokio::test]
    async fn pump_forwards_app_entries_and_reports_stream_end() {
        let (url, request) = serve_once(concat!(
            "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n",
            ": keepalive\n\n",
            "event: notification\nid: s:4\ndata: {\"sequence\":4,\"method\":\"item/started\"}\n\n",
            "event: resync\nid: t:9\ndata: {\"streamId\":\"t\",\"latestSequence\":9}\n\n",
            "event: other\nid: t:10\ndata: {}\n\n",
        ))
        .await;
        let mut pumps = StreamPumps::new();
        assert!(pumps.attach(attach(Some("s:3")), Some(target(url))).is_none());
        let request = request.await.unwrap().to_ascii_lowercase();
        assert!(request.starts_with("get /streams/notifications "), "{request}");
        assert!(request.contains("authorization: bearer secret"));
        assert!(request.contains("x-baijimu-workspace-id: 7"));
        assert!(request.contains("last-event-id: s:3"));
        assert!(request.contains("accept: text/event-stream"));

        match next_frame(&mut pumps).await {
            AgentMessage::StreamOutput(output) => {
                assert_eq!(output.attach_id, "attach");
                assert_eq!(output.id, "s:4");
                assert_eq!(output.data, json!({"sequence":4,"method":"item/started"}));
            }
            other => panic!("unexpected {other:?}"),
        }
        assert!(matches!(next_frame(&mut pumps).await, AgentMessage::StreamResync(r) if r.id == "t:9"));
        assert!(matches!(
            next_frame(&mut pumps).await,
            AgentMessage::StreamClosed(c) if c.code == StreamCloseCode::StreamEnded
        ));
        assert!(pumps.running.is_empty());
    }

    #[tokio::test]
    async fn pump_maps_app_rejections_to_close_codes() {
        let (url, _request) = serve_once(concat!(
            "HTTP/1.1 400 Bad Request\r\ncontent-type: application/json\r\nconnection: close\r\n\r\n",
            r#"{"ok":false,"error":{"code":"INVALID_STREAM_CURSOR","message":"bad"}}"#,
        ))
        .await;
        let mut pumps = StreamPumps::new();
        pumps.attach(attach(Some("bad")), Some(target(url)));
        assert!(matches!(
            next_frame(&mut pumps).await,
            AgentMessage::StreamClosed(c) if c.code == StreamCloseCode::InvalidStreamCursor
        ));

        let (url, _request) = serve_once(
            "HTTP/1.1 401 Unauthorized\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
        )
        .await;
        pumps.attach(attach(None), Some(target(url)));
        assert!(matches!(
            next_frame(&mut pumps).await,
            AgentMessage::StreamClosed(c) if c.code == StreamCloseCode::AppUnavailable
        ));

        let (url, _request) = serve_once(concat!(
            "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n",
            "event: notification\nid: s:1\ndata: not json\n\n",
        ))
        .await;
        pumps.attach(attach(None), Some(target(url)));
        assert!(matches!(
            next_frame(&mut pumps).await,
            AgentMessage::StreamClosed(c) if c.code == StreamCloseCode::AppUnavailable
        ));
    }

    #[tokio::test]
    async fn attach_without_declared_stream_is_unsupported_and_detach_aborts() {
        let mut pumps = StreamPumps::new();
        let closed = pumps.attach(attach(None), None).unwrap();
        assert!(matches!(closed, AgentMessage::StreamClosed(c) if c.code == StreamCloseCode::StreamUnsupported));

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/s", listener.local_addr().unwrap());
        let (accepted_tx, accepted_rx) = oneshot::channel();
        tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let _ = accepted_tx.send(());
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            drop(socket);
        });
        pumps.attach(attach(None), Some(target(url)));
        accepted_rx.await.unwrap();
        pumps.detach("attach");
        assert!(pumps.running.is_empty());
        assert!(pumps.frames.try_recv().is_err(), "a detached pump reports nothing");
    }
}

use lilia_protocol::{read_frame, write_frame, Operation, Request};

#[tokio::test]
async fn frame_round_trip() {
    let (mut client, mut server) = tokio::io::duplex(4_096);
    let request = Request {
        id: "018f0000-0000-7000-8000-000000000001".into(),
        token: "secret".into(),
        deadline_ms: Some(1_000),
        operation: Operation::Handshake { major: 1, minor: 0 },
    };
    let expected_id = request.id.clone();
    let writer = tokio::spawn(async move { write_frame(&mut client, &request).await });
    let decoded: Request = read_frame(&mut server).await.expect("decode request");
    writer.await.expect("writer task").expect("write request");
    assert_eq!(decoded.id, expected_id);
    assert!(matches!(
        decoded.operation,
        Operation::Handshake { major: 1, minor: 0 }
    ));
}

#[tokio::test]
async fn rejects_empty_frame() {
    use tokio::io::AsyncWriteExt;

    let (mut client, mut server) = tokio::io::duplex(4);
    client.write_u32(0).await.expect("write empty length");
    let error = read_frame::<Request>(&mut server)
        .await
        .expect_err("empty frame rejected");
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
}

#[tokio::test]
async fn subscribe_and_event_frame_round_trip() {
    use lilia_core::DatabaseEvent;
    use lilia_protocol::{Response, ResponseValue};

    let (mut client, mut server) = tokio::io::duplex(4_096);
    let request = Request {
        id: "sub-123".into(),
        token: "tok".into(),
        deadline_ms: None,
        operation: Operation::Subscribe {
            namespace: Some("users".into()),
        },
    };
    write_frame(&mut client, &request)
        .await
        .expect("write request");
    let decoded_req: Request = read_frame(&mut server).await.expect("read request");
    assert_eq!(decoded_req.id, "sub-123");
    assert!(matches!(
        decoded_req.operation,
        Operation::Subscribe { namespace: Some(ref ns) } if ns == "users"
    ));

    let response = Response {
        id: "sub-123".into(),
        result: Ok(ResponseValue::Event(DatabaseEvent::KvMutated {
            namespace: "users".into(),
            key: b"alice".to_vec(),
            version: Some(42),
            deleted: false,
        })),
    };
    write_frame(&mut server, &response)
        .await
        .expect("write response");
    let decoded_resp: Response = read_frame(&mut client).await.expect("read response");
    assert_eq!(decoded_resp.id, "sub-123");
    match decoded_resp.result.unwrap() {
        ResponseValue::Event(DatabaseEvent::KvMutated {
            namespace,
            key,
            version,
            deleted,
        }) => {
            assert_eq!(namespace, "users");
            assert_eq!(key, b"alice");
            assert_eq!(version, Some(42));
            assert!(!deleted);
        }
        other => panic!("unexpected response value: {other:?}"),
    }
}

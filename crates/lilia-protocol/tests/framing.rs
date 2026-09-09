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

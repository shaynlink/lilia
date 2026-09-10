use super::*;

const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

#[cfg(unix)]
#[test]
fn token_symlink_is_rejected_without_touching_target() {
    use std::os::unix::fs::PermissionsExt;
    let directory = tempfile::tempdir().unwrap();
    let directory = directory.path().canonicalize().unwrap();
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
    let target = directory.join("valuable-file");
    fs::write(&target, b"preserve me").unwrap();
    let token = directory.join("token");
    std::os::unix::fs::symlink(&target, &token).unwrap();
    assert!(create_token(&token).is_err());
    assert_eq!(fs::read(&target).unwrap(), b"preserve me");
}

fn mutation() -> Operation {
    Operation::Batch {
        operations: vec![lilia_core::BatchOperation::KvSet {
            namespace: "test".into(),
            key: b"key".to_vec(),
            value: b"value".to_vec(),
            if_version: Some(0),
            expires_at_ms: None,
        }],
    }
}

fn database() -> (tempfile::TempDir, Arc<Database>) {
    let directory = tempfile::tempdir().unwrap();
    let database = Arc::new(
        Database::open(DatabaseOptions::durable(
            directory.path().join("test.lilia"),
        ))
        .unwrap(),
    );
    (directory, database)
}

async fn exchange(
    stream: &mut tokio::io::DuplexStream,
    token: &str,
    operation: Operation,
    deadline_ms: Option<u64>,
) -> Response {
    let id = Uuid::new_v4().to_string();
    write_frame(
        stream,
        &Request {
            id: id.clone(),
            token: token.into(),
            deadline_ms,
            operation,
        },
    )
    .await
    .unwrap();
    let response: Response = tokio::time::timeout(Duration::from_secs(5), read_frame(stream))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(response.id, id);
    if let Err(error) = &response.result {
        assert_eq!(error.request_id.to_string(), id);
        assert!(!error.message.contains(TOKEN));
    }
    response
}

#[tokio::test]
async fn rejected_shutdown_never_signals_and_connection_remains_usable() {
    let (_directory, database) = database();
    let shutdown = Arc::new(Notify::new());
    let (mut client, server) = tokio::io::duplex(8192);
    let handler = tokio::spawn(handle_stream(server, database, TOKEN, shutdown.clone()));
    for token in [
        "",
        "wrong",
        "1123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
    ] {
        let error = exchange(&mut client, token, Operation::Shutdown, None)
            .await
            .result
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Unauthorized);
        assert!(!error.retryable);
        assert!(
            tokio::time::timeout(Duration::from_millis(10), shutdown.notified())
                .await
                .is_err()
        );
    }
    let error = exchange(&mut client, TOKEN, Operation::Shutdown, Some(0))
        .await
        .result
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::Timeout);
    assert!(error.retryable);
    assert!(
        tokio::time::timeout(Duration::from_millis(10), shutdown.notified())
            .await
            .is_err()
    );
    assert!(matches!(
        exchange(&mut client, TOKEN, Operation::IntegrityCheck, None)
            .await
            .result,
        Ok(ResponseValue::Integrity(true))
    ));
    assert!(matches!(
        exchange(&mut client, TOKEN, Operation::Shutdown, None)
            .await
            .result,
        Ok(ResponseValue::Shutdown)
    ));
    tokio::time::timeout(Duration::from_secs(1), shutdown.notified())
        .await
        .unwrap();
    handler.await.unwrap().unwrap();
}

#[tokio::test]
async fn rejected_mutations_have_no_effect_and_valid_requests_still_work() {
    let (_directory, database) = database();
    let (mut client, server) = tokio::io::duplex(8192);
    let handler = tokio::spawn(handle_stream(
        server,
        database.clone(),
        TOKEN,
        Arc::new(Notify::new()),
    ));
    for (token, deadline, expected) in [
        ("wrong", None, ErrorCode::Unauthorized),
        ("wrong", Some(0), ErrorCode::Unauthorized),
        (TOKEN, Some(0), ErrorCode::Timeout),
    ] {
        assert_eq!(
            exchange(&mut client, token, mutation(), deadline)
                .await
                .result
                .unwrap_err()
                .code,
            expected
        );
        assert!(database.kv_get("test", b"key").unwrap().is_none());
    }
    assert!(matches!(
        exchange(
            &mut client,
            TOKEN,
            Operation::Handshake {
                major: PROTOCOL_MAJOR,
                minor: PROTOCOL_MINOR
            },
            None
        )
        .await
        .result,
        Ok(ResponseValue::Handshake { .. })
    ));
    assert_eq!(
        exchange(
            &mut client,
            TOKEN,
            Operation::Handshake {
                major: PROTOCOL_MAJOR + 1,
                minor: 0
            },
            None
        )
        .await
        .result
        .unwrap_err()
        .code,
        ErrorCode::Unsupported
    );
    assert!(matches!(
        exchange(&mut client, TOKEN, mutation(), Some(u64::MAX))
            .await
            .result,
        Ok(ResponseValue::Mutations(_))
    ));
    assert_eq!(
        database.kv_get("test", b"key").unwrap().unwrap().value,
        b"value"
    );
    drop(client);
    handler.await.unwrap().unwrap();
}

#[test]
fn request_expiring_in_worker_queue_never_mutates_database() {
    let (_directory, database) = database();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(1)
        .build()
        .unwrap();
    runtime.block_on(async {
        let (release, wait) = std::sync::mpsc::channel();
        let (entered, ready) = tokio::sync::oneshot::channel();
        let blocker = tokio::task::spawn_blocking(move || {
            entered.send(()).unwrap();
            wait.recv_timeout(Duration::from_secs(5)).unwrap();
        });
        ready.await.unwrap();
        let execution = execute(database.clone(), mutation(), Instant::now(), Some(100));
        tokio::pin!(execution);
        tokio::select! {
            result = &mut execution => panic!("request must wait for worker: {result:?}"),
            () = tokio::time::sleep(Duration::from_millis(150)) => {},
        }
        release.send(()).unwrap();
        blocker.await.unwrap();
        let error = execution.await.unwrap_err();
        assert_eq!(error.code, ErrorCode::Timeout);
        assert!(error.retryable);
        assert!(database.kv_get("test", b"key").unwrap().is_none());
        assert!(matches!(
            execute(database, mutation(), Instant::now(), None).await,
            Ok(ResponseValue::Mutations(_))
        ));
    });
}

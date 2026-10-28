#![allow(clippy::too_many_lines)]

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use lilia_core::{BatchOperation, Direction, ErrorCode};
use lilia_storage_sqlite::{Database, DatabaseOptions};
use rusqlite::Connection;
use serde_json::json;

struct KillOnDrop(Child);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn multi_model_batch(id: &str, val: u8) -> Vec<BatchOperation> {
    vec![
        BatchOperation::KvSet {
            namespace: "chaos_kv".into(),
            key: format!("k_{id}").into_bytes(),
            value: vec![val; 2048],
            if_version: None,
            expires_at_ms: None,
        },
        BatchOperation::JsonPut {
            space: "chaos_json".into(),
            id: format!("doc_{id}"),
            value: json!({ "id": id, "val": val, "payload": "resilience test payload" }),
            if_version: None,
        },
        BatchOperation::GraphSetNode {
            graph: "chaos_graph".into(),
            id: format!("node_{id}"),
            kind: "service".into(),
            metadata: json!({ "val": val }),
        },
        BatchOperation::GraphAddEdge {
            graph: "chaos_graph".into(),
            source: format!("node_{id}"),
            target: "gateway".into(),
            kind: "routes_to".into(),
        },
        BatchOperation::FtsIndex {
            collection: "chaos_fts".into(),
            doc_id: format!("fts_{id}"),
            content: format!("Cluster node {id} active with resilience validation status {val}"),
        },
    ]
}

// Subprocess worker invoked by chaos tests
#[test]
#[ignore = "subprocess helper for crash testing"]
fn chaos_child_worker() {
    let path_str = std::env::var("CHAOS_DB_PATH").expect("CHAOS_DB_PATH missing");
    let scenario = std::env::var("CHAOS_SCENARIO").expect("CHAOS_SCENARIO missing");
    let path = PathBuf::from(path_str);

    let db = Database::open(DatabaseOptions::durable(&path)).expect("open durable db in child");

    match scenario.as_str() {
        "multimodel_uncommitted" => {
            // Setup gateway node for graph edge target
            db.batch(&[BatchOperation::GraphSetNode {
                graph: "chaos_graph".into(),
                id: "gateway".into(),
                kind: "gateway".into(),
                metadata: json!({ "cluster": "alpha" }),
            }])
            .unwrap();

            // Write 5 confirmed multi-model batches
            for i in 0..5u8 {
                db.batch(&multi_model_batch(&i.to_string(), i))
                    .expect("write confirmed batch");
            }

            fs::write(path.with_extension("confirmed"), b"ready").unwrap();

            // Now open an internal writer connection to stage dirty uncommitted pages
            // into the WAL without committing the transaction.
            let mut conn = Connection::open(&path).expect("open raw sqlite in child");
            conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;")
                .unwrap();
            let tx = conn.transaction().expect("begin raw transaction");

            // Execute uncommitted operations across all tables
            let uncommitted = multi_model_batch("uncommitted", 99);
            for op in &uncommitted {
                match op {
                    BatchOperation::KvSet {
                        namespace,
                        key,
                        value,
                        ..
                    } => {
                        tx.execute(
                            "INSERT INTO _lilia_kv(namespace, key, value, version) VALUES(?1, ?2, ?3, 1)",
                            rusqlite::params![namespace, key, value],
                        )
                        .unwrap();
                    }
                    BatchOperation::JsonPut {
                        space, id, value, ..
                    } => {
                        let json_str = serde_json::to_string(value).unwrap();
                        tx.execute(
                            "INSERT INTO _lilia_json(space, id, value, version) VALUES(?1, ?2, ?3, 1)",
                            rusqlite::params![space, id, json_str],
                        )
                        .unwrap();
                    }
                    BatchOperation::GraphSetNode {
                        graph,
                        id,
                        kind,
                        metadata,
                    } => {
                        let meta_str = serde_json::to_string(metadata).unwrap();
                        tx.execute(
                            "INSERT INTO _lilia_graph_nodes(graph, id, kind, metadata) VALUES(?1, ?2, ?3, ?4)",
                            rusqlite::params![graph, id, kind, meta_str],
                        )
                        .unwrap();
                    }
                    BatchOperation::GraphAddEdge {
                        graph,
                        source,
                        target,
                        kind,
                    } => {
                        tx.execute(
                            "INSERT INTO _lilia_graph_edges(graph, source, target, kind) VALUES(?1, ?2, ?3, ?4)",
                            rusqlite::params![graph, source, target, kind],
                        )
                        .unwrap();
                    }
                    BatchOperation::FtsIndex {
                        collection,
                        doc_id,
                        content,
                    } => {
                        tx.execute(
                            "INSERT INTO _lilia_fts(collection, doc_id, content) VALUES(?1, ?2, ?3)",
                            rusqlite::params![collection, doc_id, content],
                        )
                        .unwrap();
                    }
                    _ => {}
                }
            }

            // Flush dirty pages to the WAL file
            tx.cache_flush().expect("cache flush uncommitted pages");

            // Signal parent that uncommitted dirty pages are in WAL
            fs::write(path.with_extension("staged"), b"uncommitted_staged").unwrap();

            // Park child until killed by parent
            loop {
                std::thread::park_timeout(Duration::from_secs(1));
            }
        }
        "stress_cycle" => {
            let cycle_idx: u8 = std::env::var("CHAOS_CYCLE_INDEX")
                .unwrap_or_else(|_| "0".into())
                .parse()
                .unwrap();

            // Write confirmed batch for this cycle
            db.batch(&multi_model_batch(&cycle_idx.to_string(), cycle_idx))
                .unwrap();

            // Stage dirty uncommitted mutation
            let mut conn = Connection::open(&path).unwrap();
            conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;")
                .unwrap();
            let tx = conn.transaction().unwrap();
            tx.execute(
                "INSERT INTO _lilia_kv(namespace, key, value, version) VALUES('chaos_kv', ?1, ?2, 1)",
                rusqlite::params![
                    format!("dirty_{cycle_idx}").into_bytes(),
                    vec![255u8; 1024]
                ],
            )
            .unwrap();
            tx.cache_flush().unwrap();

            fs::write(path.with_extension("staged"), b"cycle_staged").unwrap();

            loop {
                std::thread::park_timeout(Duration::from_secs(1));
            }
        }
        "wal_commits" => {
            db.batch(&[BatchOperation::GraphSetNode {
                graph: "chaos_graph".into(),
                id: "gateway".into(),
                kind: "gateway".into(),
                metadata: json!({ "cluster": "alpha" }),
            }])
            .unwrap();

            let count: u8 = std::env::var("CHAOS_BATCH_COUNT")
                .unwrap_or_else(|_| "3".into())
                .parse()
                .unwrap();

            for i in 0..count {
                db.batch(&multi_model_batch(&i.to_string(), i))
                    .expect("write confirmed batch");
            }

            fs::write(path.with_extension("ready"), b"ready").unwrap();

            loop {
                std::thread::park_timeout(Duration::from_secs(1));
            }
        }
        _ => panic!("unknown chaos scenario {scenario}"),
    }
}

fn spawn_and_kill_worker(db_path: &Path, scenario: &str, extra_env: &[(&str, &str)]) {
    let mut cmd = Command::new(std::env::current_exe().unwrap());
    cmd.args(["--exact", "chaos_child_worker", "--ignored", "--nocapture"])
        .env("CHAOS_DB_PATH", db_path)
        .env("CHAOS_SCENARIO", scenario);
    for (k, v) in extra_env {
        cmd.env(k, v);
    }
    let mut child = KillOnDrop(cmd.spawn().unwrap());
    let ready_flag = db_path.with_extension("ready");
    let deadline = Instant::now() + Duration::from_secs(25);
    while !ready_flag.exists() {
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "child exited prematurely"
        );
        assert!(
            Instant::now() < deadline,
            "timeout waiting for child ready flag"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let _ = fs::remove_file(&ready_flag);
    child.0.kill().unwrap();
    let _ = child.0.wait();
}

// 1. Simulation de panne de courant au milieu d'un flush WAL (trame tronquée / torn frame)
#[test]
fn test_torn_wal_frame_power_loss_recovery() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("power_loss.lilia");
    let wal_path = dir.path().join("power_loss.lilia-wal");

    // Spawn child worker that writes 4 confirmed batches and is abruptly killed with SIGKILL
    spawn_and_kill_worker(&db_path, "wal_commits", &[("CHAOS_BATCH_COUNT", "4")]);

    assert!(wal_path.exists(), "WAL file must exist");
    let initial_wal_len = fs::metadata(&wal_path).unwrap().len();
    assert!(initial_wal_len >= 32, "WAL must have at least header");

    // SIMULATE POWER LOSS MID-FRAME FLUSH:
    // A standard SQLite WAL frame is 24 bytes header + 4096 bytes page = 4120 bytes.
    // We simulate physical power cut while writing the next frame: only 512 bytes are written.
    let mut wal_file = OpenOptions::new().append(true).open(&wal_path).unwrap();
    let incomplete_frame = vec![0xAA; 512];
    wal_file.write_all(&incomplete_frame).unwrap();
    wal_file.sync_all().unwrap();
    drop(wal_file);

    let torn_wal_len = fs::metadata(&wal_path).unwrap().len();
    assert_eq!(torn_wal_len, initial_wal_len + 512);

    // RECOVERY:
    // Open Database in durable mode. SQLite WAL recovery must:
    // 1. Detect EOF mid-frame.
    // 2. Discard the partial/torn frame.
    // 3. Replay all preceding confirmed commit frames up to batch 3.
    // 4. Leave the database 100% consistent and intact.
    let db = Database::open(DatabaseOptions::durable(&db_path)).unwrap();

    // Verify all confirmed data is present
    for i in 0..4u8 {
        let kv = db
            .kv_get("chaos_kv", format!("k_{i}").as_bytes())
            .unwrap()
            .unwrap();
        assert_eq!(kv.value, vec![i; 2048]);
        assert!(db
            .json_get("chaos_json", &format!("doc_{i}"))
            .unwrap()
            .is_some());
        assert!(db
            .graph_get_node("chaos_graph", &format!("node_{i}"))
            .unwrap()
            .is_some());
    }

    // Verify integrity check passes
    assert!(
        db.integrity_check().unwrap(),
        "Integrity check must pass after torn WAL recovery"
    );

    // Verify new writes succeed immediately
    db.batch(&multi_model_batch("after_torn_recovery", 5))
        .unwrap();
    assert!(db
        .kv_get("chaos_kv", b"k_after_torn_recovery")
        .unwrap()
        .is_some());

    db.close(Duration::from_secs(5)).unwrap();
}

// 2. Simulation de corruption de checksum de trame WAL (bit-flip / torn sector)
#[test]
fn test_corrupt_wal_frame_checksum_recovery() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("wal_checksum.lilia");
    let wal_path = dir.path().join("wal_checksum.lilia-wal");

    // Spawn child worker that writes 2 confirmed batches and is abruptly killed with SIGKILL
    spawn_and_kill_worker(&db_path, "wal_commits", &[("CHAOS_BATCH_COUNT", "2")]);

    assert!(wal_path.exists());
    let mut wal_bytes = fs::read(&wal_path).unwrap();
    assert!(wal_bytes.len() > 32 + 4120);

    // Corrupt the checksum of the last WAL frame:
    // Frame header is 24 bytes, checksum is at offset 16..24 of the frame header.
    let last_frame_offset = wal_bytes.len() - 4120;
    for b in &mut wal_bytes[last_frame_offset + 16..last_frame_offset + 24] {
        *b ^= 0xFF; // Invert bits in checksum
    }
    fs::write(&wal_path, &wal_bytes).unwrap();

    // RECOVERY:
    // SQLite must detect the frame checksum mismatch, stop reading at the corrupted frame,
    // recover all valid preceding frames (batch 0), and successfully open in a consistent state.
    let db = Database::open(DatabaseOptions::durable(&db_path)).unwrap();

    // Batch 0 must be intact
    assert!(db.kv_get("chaos_kv", b"k_0").unwrap().is_some());
    assert!(db.json_get("chaos_json", "doc_0").unwrap().is_some());

    // Integrity check must pass
    assert!(db.integrity_check().unwrap());

    // Writing new data works
    db.batch(&multi_model_batch("after_corrupt", 3)).unwrap();
    assert!(db.kv_get("chaos_kv", b"k_after_corrupt").unwrap().is_some());
}

// 3. Troncature du header WAL (coupure d'alimentation à l'initialisation du WAL)
#[test]
fn test_truncated_wal_header_recovery() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("wal_header.lilia");
    let wal_path = dir.path().join("wal_header.lilia-wal");

    {
        let db = Database::open(DatabaseOptions::durable(&db_path)).unwrap();
        db.batch(&multi_model_batch("init", 1)).unwrap();
        db.close(Duration::from_secs(5)).unwrap();
    }

    // Replace WAL with an incomplete 8-byte header (normal is 32 bytes)
    fs::write(&wal_path, [0x37, 0x7F, 0x06, 0x82, 0x00, 0x00, 0x00, 0x00]).unwrap();

    // Reopen: SQLite ignores the invalid WAL header and falls back safely to the main DB
    let db = Database::open(DatabaseOptions::durable(&db_path)).unwrap();
    assert!(db.integrity_check().unwrap());
    assert!(db.kv_get("chaos_kv", b"k_init").unwrap().is_some());

    // Subsequent writes recreate a valid WAL
    db.batch(&multi_model_batch("after_header_trunc", 2))
        .unwrap();
    assert!(db
        .kv_get("chaos_kv", b"k_after_header_trunc")
        .unwrap()
        .is_some());
}

// 4. Coupure brutale de processus (SIGKILL) pendant un commit multi-modèle
#[test]
fn test_multimodel_killed_process_atomicity() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("killed_multimodel.lilia");

    let mut child = KillOnDrop(
        Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "chaos_child_worker", "--ignored", "--nocapture"])
            .env("CHAOS_DB_PATH", &db_path)
            .env("CHAOS_SCENARIO", "multimodel_uncommitted")
            .spawn()
            .unwrap(),
    );

    // Wait until child has confirmed batches 0..5 and staged uncommitted batch in WAL
    let staged_flag = db_path.with_extension("staged");
    let deadline = Instant::now() + Duration::from_secs(25);
    while !staged_flag.exists() {
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "child exited prematurely"
        );
        assert!(
            Instant::now() < deadline,
            "timeout waiting for child staging"
        );
        std::thread::sleep(Duration::from_millis(20));
    }

    // KILL child process with SIGKILL (immediate abnormal termination)
    child.0.kill().unwrap();
    let _ = child.0.wait();

    let wal_path = dir.path().join("killed_multimodel.lilia-wal");
    assert!(
        wal_path.exists(),
        "WAL file must exist with dirty uncommitted pages"
    );

    // REOPEN DATABASE:
    let db = Database::open(DatabaseOptions::durable(&db_path)).unwrap();

    // 1. Verify all confirmed batches 0..5 are intact across KV, JSON, Graph and FTS
    for i in 0..5u8 {
        let kv = db
            .kv_get("chaos_kv", format!("k_{i}").as_bytes())
            .unwrap()
            .unwrap();
        assert_eq!(kv.value, vec![i; 2048]);

        let doc = db
            .json_get("chaos_json", &format!("doc_{i}"))
            .unwrap()
            .unwrap();
        assert_eq!(doc.value["val"], i);

        let node = db
            .graph_get_node("chaos_graph", &format!("node_{i}"))
            .unwrap()
            .unwrap();
        assert_eq!(node.kind, "service");

        let hits = db
            .fts_search("chaos_fts", &format!("node {i}"), 10)
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].doc_id, format!("fts_{i}"));
    }

    // Graph reachability across confirmed nodes
    let reachability = db
        .graph_reachability("chaos_graph", &["node_0".into()], Direction::Outgoing)
        .unwrap();
    assert!(reachability.contains(&"gateway".to_string()));

    // 2. ATOMICITY CHECK:
    // Verify that NONE of the 6th uncommitted transaction's mutations leaked into ANY model
    assert!(
        db.kv_get("chaos_kv", b"k_uncommitted").unwrap().is_none(),
        "uncommitted KV must be rolled back"
    );
    assert!(
        db.json_get("chaos_json", "doc_uncommitted")
            .unwrap()
            .is_none(),
        "uncommitted JSON must be rolled back"
    );
    assert!(
        db.graph_get_node("chaos_graph", "node_uncommitted")
            .unwrap()
            .is_none(),
        "uncommitted Graph node must be rolled back"
    );
    assert_eq!(
        db.fts_search("chaos_fts", "status 99", 10).unwrap().len(),
        0,
        "uncommitted FTS entry must be rolled back"
    );

    // 3. INTEGRITY CHECK:
    assert!(db.integrity_check().unwrap(), "Integrity check must pass");

    // 4. WRITE RECOVERY:
    // Verify subsequent multi-model write succeeds
    db.batch(&multi_model_batch("confirmed_post_crash", 10))
        .unwrap();
    assert!(db
        .kv_get("chaos_kv", b"k_confirmed_post_crash")
        .unwrap()
        .is_some());
    assert!(db
        .json_get("chaos_json", "doc_confirmed_post_crash")
        .unwrap()
        .is_some());

    db.close(Duration::from_secs(5)).unwrap();
}

// 5. Stress test en boucle de crashs et récupérations consécutifs
#[test]
fn test_stress_crash_recovery_cycle() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("stress_crash.lilia");

    for cycle in 0..5u8 {
        let staged_flag = db_path.with_extension("staged");
        let _ = fs::remove_file(&staged_flag);

        let mut child = KillOnDrop(
            Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "chaos_child_worker", "--ignored", "--nocapture"])
                .env("CHAOS_DB_PATH", &db_path)
                .env("CHAOS_SCENARIO", "stress_cycle")
                .env("CHAOS_CYCLE_INDEX", cycle.to_string())
                .spawn()
                .unwrap(),
        );

        let deadline = Instant::now() + Duration::from_secs(20);
        while !staged_flag.exists() {
            assert!(
                child.0.try_wait().unwrap().is_none(),
                "child exited early in cycle {cycle}"
            );
            assert!(Instant::now() < deadline, "timeout in cycle {cycle}");
            std::thread::sleep(Duration::from_millis(15));
        }

        // Kill writer during dirty WAL state
        child.0.kill().unwrap();
        let _ = child.0.wait();

        // Reopen and verify cumulative integrity
        let db = Database::open(DatabaseOptions::durable(&db_path)).unwrap();
        assert!(
            db.integrity_check().unwrap(),
            "integrity check failed in cycle {cycle}"
        );

        // All confirmed batches up to this cycle must be present
        for c in 0..=cycle {
            assert!(
                db.kv_get("chaos_kv", format!("k_{c}").as_bytes())
                    .unwrap()
                    .is_some(),
                "cycle {c} data missing after recovery"
            );
        }

        // Uncommitted dirty write must be absent
        assert!(
            db.kv_get("chaos_kv", format!("dirty_{cycle}").as_bytes())
                .unwrap()
                .is_none(),
            "dirty uncommitted data must not leak in cycle {cycle}"
        );

        db.close(Duration::from_secs(5)).unwrap();
    }
}

// 6. Saturation disque (ENOSPC) : rollback atomique, libération des verrous et auto-guérison
#[test]
fn test_enospc_batch_rollback_and_healing() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("enospc_test.lilia");

    let db = Database::open(DatabaseOptions::durable(&db_path)).unwrap();
    db.batch(&multi_model_batch("initial", 1)).unwrap();

    // Restrict max page quota to simulate out-of-disk condition
    {
        let writer = db.lock_writer().unwrap();
        let current_pages: u32 = writer
            .query_row("PRAGMA page_count", [], |row| row.get(0))
            .unwrap();
        writer
            .pragma_update(None, "max_page_count", current_pages + 1)
            .unwrap();
    }

    // Attempt a large multi-model batch that exceeds the quota
    let mut oversized = multi_model_batch("oversized", 2);
    oversized.push(BatchOperation::KvSet {
        namespace: "chaos_kv".into(),
        key: b"huge_payload".to_vec(),
        value: vec![0xFF; 512 * 1024], // 512 KB payload requires > 128 pages
        if_version: None,
        expires_at_ms: None,
    });

    let err = db.batch(&oversized).unwrap_err();
    assert_eq!(err.code, ErrorCode::DiskFull);
    assert!(!err.retryable);

    // Verify writer lock was released cleanly (not poisoned)
    {
        let writer = db.lock_writer();
        assert!(writer.is_ok(), "writer lock must be released after ENOSPC");
    }

    // Verify concurrent readers continue to work
    assert!(db.kv_get("chaos_kv", b"k_initial").unwrap().is_some());
    assert!(db.kv_get("chaos_kv", b"huge_payload").unwrap().is_none());

    // SIMULATE DISK SPACE RESTORATION (expand quota)
    {
        let writer = db.lock_writer().unwrap();
        writer
            .pragma_update(None, "max_page_count", 1_000_000)
            .unwrap();
    }

    // Engine immediately heals and accepts new batches without restart
    db.batch(&multi_model_batch("after_space_freed", 3))
        .unwrap();
    assert!(db
        .kv_get("chaos_kv", b"k_after_space_freed")
        .unwrap()
        .is_some());

    assert!(db.integrity_check().unwrap());
    db.close(Duration::from_secs(5)).unwrap();
}

// 7. Saturation disque lors d'un backup : nettoyage de staging et préservation de la source
#[test]
fn test_enospc_backup_cleanup_and_isolation() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("backup_enospc.lilia");

    let db = Database::open(DatabaseOptions::durable(&db_path)).unwrap();
    db.batch(&multi_model_batch("data", 42)).unwrap();

    // Make the parent directory read-only to provoke permission/IO/full failure on staging
    let read_only_dir = dir.path().join("readonly_staging");
    fs::create_dir(&read_only_dir).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&read_only_dir, fs::Permissions::from_mode(0o400)).unwrap();
    }
    #[cfg(windows)]
    {
        // On Windows, replace directory with a regular file so creating child staging directories fails with IO error
        fs::remove_dir(&read_only_dir).unwrap();
        fs::write(&read_only_dir, b"not_a_directory").unwrap();
    }

    let backup_target = read_only_dir.join("snap.lilia");
    let res = db.backup(&backup_target);
    assert!(
        res.is_err(),
        "backup must fail on write-prohibited destination"
    );

    // Verify staging directory was cleaned up and no orphan .lilia-backup-* exists
    let staging_remnants: Vec<_> = fs::read_dir(dir.path())
        .unwrap()
        .filter_map(std::result::Result::ok)
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with(".lilia-backup-")
        })
        .collect();
    assert_eq!(
        staging_remnants.len(),
        0,
        "staging directory must be cleaned up on failure"
    );

    // Source database must remain 100% operational and healthy
    assert!(db.integrity_check().unwrap());
    assert!(db.kv_get("chaos_kv", b"k_data").unwrap().is_some());

    // Normal backup to valid path succeeds
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&read_only_dir, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let valid_backup = dir.path().join("valid_snap.lilia");
    db.backup(&valid_backup).unwrap();
    assert!(valid_backup.exists());

    let backup_db = Database::open(DatabaseOptions::durable(valid_backup)).unwrap();
    assert!(backup_db.integrity_check().unwrap());
    assert!(backup_db.kv_get("chaos_kv", b"k_data").unwrap().is_some());
}

// 8. Détection de corruption physique : refus strict sans réinitialisation silencieuse
#[test]
fn test_silent_corruption_rejection() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("corrupted.lilia");

    {
        let db = Database::open(DatabaseOptions::durable(&db_path)).unwrap();
        db.batch(&multi_model_batch("critical_payload", 77))
            .unwrap();
        db.close(Duration::from_secs(5)).unwrap();
    }

    let original_bytes = fs::read(&db_path).unwrap();
    assert!(original_bytes.len() >= 4096);

    // Physically corrupt the SQLite header bytes (offset 0..16: "SQLite format 3\0")
    let mut corrupted_bytes = original_bytes.clone();
    corrupted_bytes[0..16].copy_from_slice(b"CORRUPTED_HEADER");
    fs::write(&db_path, &corrupted_bytes).unwrap();

    // Reopen must FAIL explicitly with ErrorCode::Corrupt
    let open_result = Database::open(DatabaseOptions::durable(&db_path));
    assert!(
        open_result.is_err(),
        "must refuse to open corrupted database"
    );
    let err = open_result.err().unwrap();
    assert_eq!(err.code, ErrorCode::Corrupt);
    assert!(!err.retryable);

    // CRITICAL: The damaged file must NOT be overwritten or silently recreated!
    let current_bytes = fs::read(&db_path).unwrap();
    assert_eq!(
        current_bytes, corrupted_bytes,
        "damaged database bytes must be preserved for forensics without silent re-creation"
    );
}

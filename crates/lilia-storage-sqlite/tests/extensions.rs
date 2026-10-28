use lilia_core::{BatchOperation, DatabaseEvent, Direction};
use lilia_storage_sqlite::{Database, DatabaseOptions};
use serde_json::json;

#[test]
fn test_secondary_indexing_and_prefix_scan() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(DatabaseOptions::durable(dir.path().join("idx_test.lilia"))).unwrap();

    // Register index on space "items" for $.status
    db.create_json_index("items", "by_status", "$.status")
        .unwrap();

    // Insert documents
    db.batch(&[
        BatchOperation::JsonPut {
            space: "items".into(),
            id: "repoA:1".into(),
            value: json!({ "title": "First", "status": "todo" }),
            if_version: None,
        },
        BatchOperation::JsonPut {
            space: "items".into(),
            id: "repoA:2".into(),
            value: json!({ "title": "Second", "status": "in_progress" }),
            if_version: None,
        },
        BatchOperation::JsonPut {
            space: "items".into(),
            id: "repoB:1".into(),
            value: json!({ "title": "Third", "status": "todo" }),
            if_version: None,
        },
    ])
    .unwrap();

    // Query by index "by_status" with value "todo"
    let todos = db
        .json_scan_by_index("items", "by_status", "todo", None, 10)
        .unwrap();
    assert_eq!(todos.len(), 2);
    assert_eq!(todos[0].id, "repoA:1");
    assert_eq!(todos[1].id, "repoB:1");

    // Prefix scan on repoA
    let repo_a_items = db.json_scan_prefix("items", "repoA:", None, 10).unwrap();
    assert_eq!(repo_a_items.len(), 2);
    assert_eq!(repo_a_items[0].id, "repoA:1");
    assert_eq!(repo_a_items[1].id, "repoA:2");

    // Updating a document updates the index
    db.batch(&[BatchOperation::JsonPut {
        space: "items".into(),
        id: "repoA:1".into(),
        value: json!({ "title": "First updated", "status": "done" }),
        if_version: None,
    }])
    .unwrap();

    let todos_after = db
        .json_scan_by_index("items", "by_status", "todo", None, 10)
        .unwrap();
    assert_eq!(todos_after.len(), 1);
    assert_eq!(todos_after[0].id, "repoB:1");

    let dones = db
        .json_scan_by_index("items", "by_status", "done", None, 10)
        .unwrap();
    assert_eq!(dones.len(), 1);
    assert_eq!(dones[0].id, "repoA:1");

    // Deleting a document removes it from index
    db.batch(&[BatchOperation::JsonDelete {
        space: "items".into(),
        id: "repoA:1".into(),
        if_version: None,
    }])
    .unwrap();

    let dones_after = db
        .json_scan_by_index("items", "by_status", "done", None, 10)
        .unwrap();
    assert_eq!(dones_after.len(), 0);
}

#[test]
fn test_json_find_and_index_lifecycle() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(DatabaseOptions::durable(dir.path().join("idx_find.lilia"))).unwrap();

    // 1. Insert documents
    db.batch(&[
        BatchOperation::JsonPut {
            space: "users".into(),
            id: "u1".into(),
            value: json!({ "name": "Alice", "role": "admin", "age": 30, "tags": ["rust", "db"] }),
            if_version: None,
        },
        BatchOperation::JsonPut {
            space: "users".into(),
            id: "u2".into(),
            value: json!({ "name": "Bob", "role": "viewer", "age": 25, "tags": ["js"] }),
            if_version: None,
        },
        BatchOperation::JsonPut {
            space: "users".into(),
            id: "u3".into(),
            value: json!({ "name": "Charlie", "role": "admin", "age": 35, "tags": ["db"] }),
            if_version: None,
        },
    ])
    .unwrap();

    // 2. Query without index (fallback scan)
    let mut q1 = serde_json::Map::new();
    q1.insert("role".into(), json!("admin"));
    let res1 = db.json_find("users", &q1, None, 10).unwrap();
    assert_eq!(res1.len(), 2);
    assert_eq!(res1[0].id, "u1");
    assert_eq!(res1[1].id, "u3");

    // 3. Create index on "role" and list indexes
    db.create_json_index("users", "role", "$.role").unwrap();
    let idxs = db.list_json_indexes("users").unwrap();
    assert_eq!(idxs.len(), 1);
    assert_eq!(idxs[0].index_name, "role");
    assert_eq!(idxs[0].json_path, "$.role");

    // 4. Query with index acceleration
    let res2 = db.json_find("users", &q1, None, 10).unwrap();
    assert_eq!(res2.len(), 2);
    assert_eq!(res2[0].id, "u1");
    assert_eq!(res2[1].id, "u3");

    // 5. Multi-field query (index on role, filter on age)
    let mut q2 = serde_json::Map::new();
    q2.insert("role".into(), json!("admin"));
    q2.insert("age".into(), json!(35));
    let res3 = db.json_find("users", &q2, None, 10).unwrap();
    assert_eq!(res3.len(), 1);
    assert_eq!(res3[0].id, "u3");

    // 6. Multikey array index test
    db.create_json_index("users", "tags", "$.tags").unwrap();
    let mut q3 = serde_json::Map::new();
    q3.insert("tags".into(), json!("db"));
    let res4 = db.json_find("users", &q3, None, 10).unwrap();
    assert_eq!(res4.len(), 2);
    assert_eq!(res4[0].id, "u1");
    assert_eq!(res4[1].id, "u3");

    // 7. Drop index
    let dropped = db.drop_json_index("users", "role").unwrap();
    assert!(dropped);
    let idxs_after = db.list_json_indexes("users").unwrap();
    assert_eq!(idxs_after.len(), 1);
    assert_eq!(idxs_after[0].index_name, "tags");
}

#[test]
fn test_graph_and_reachability() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(DatabaseOptions::durable(
        dir.path().join("graph_test.lilia"),
    ))
    .unwrap();

    // Build dependency graph:
    // app -> lib_core -> lib_utils
    // app -> lib_ui -> lib_utils
    // worker -> lib_core
    db.graph_set_node("repo_graph", "app", "project", &json!({ "lang": "rust" }))
        .unwrap();
    db.graph_set_node(
        "repo_graph",
        "worker",
        "project",
        &json!({ "lang": "rust" }),
    )
    .unwrap();
    db.graph_set_node(
        "repo_graph",
        "lib_core",
        "library",
        &json!({ "lang": "rust" }),
    )
    .unwrap();
    db.graph_set_node(
        "repo_graph",
        "lib_ui",
        "library",
        &json!({ "lang": "typescript" }),
    )
    .unwrap();
    db.graph_set_node(
        "repo_graph",
        "lib_utils",
        "library",
        &json!({ "lang": "rust" }),
    )
    .unwrap();

    db.graph_add_edge("repo_graph", "app", "lib_core", "depends_on")
        .unwrap();
    db.graph_add_edge("repo_graph", "app", "lib_ui", "depends_on")
        .unwrap();
    db.graph_add_edge("repo_graph", "worker", "lib_core", "depends_on")
        .unwrap();
    db.graph_add_edge("repo_graph", "lib_core", "lib_utils", "depends_on")
        .unwrap();
    db.graph_add_edge("repo_graph", "lib_ui", "lib_utils", "depends_on")
        .unwrap();

    // Verify node retrieval
    let node = db.graph_get_node("repo_graph", "app").unwrap().unwrap();
    assert_eq!(node.kind, "project");
    assert_eq!(node.metadata["lang"], "rust");

    // Test Outgoing reachability from "app" (what does app depend on?)
    let deps = db
        .graph_reachability("repo_graph", &["app".into()], Direction::Outgoing)
        .unwrap();
    assert_eq!(deps, vec!["app", "lib_core", "lib_ui", "lib_utils"]);

    // Test Incoming reachability (impact of changing "lib_utils": who depends on it?)
    let impact = db
        .graph_reachability("repo_graph", &["lib_utils".into()], Direction::Incoming)
        .unwrap();
    assert_eq!(
        impact,
        vec!["app", "lib_core", "lib_ui", "lib_utils", "worker"]
    );

    // Impact of changing "lib_ui" only
    let ui_impact = db
        .graph_reachability("repo_graph", &["lib_ui".into()], Direction::Incoming)
        .unwrap();
    assert_eq!(ui_impact, vec!["app", "lib_ui"]);
}

#[test]
fn test_fts5_indexing_and_search() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(DatabaseOptions::durable(dir.path().join("fts_test.lilia"))).unwrap();

    db.fts_index(
        "docs",
        "arch.md",
        "Worklens connects repository architecture, Git worktrees and GitHub delivery",
    )
    .unwrap();
    db.fts_index(
        "docs",
        "lilia.md",
        "LiliaDB is a local-first multimodel database in Rust with SQLite WAL storage",
    )
    .unwrap();
    db.fts_index(
        "logs",
        "proc_1",
        "INFO Starting compilation of lilia-storage-sqlite crate in release mode",
    )
    .unwrap();
    db.fts_index(
        "logs",
        "proc_2",
        "ERROR Failed to bind socket address already in use",
    )
    .unwrap();

    // Search in docs
    let hits_arch = db.fts_search("docs", "worktrees", 10).unwrap();
    assert_eq!(hits_arch.len(), 1);
    assert_eq!(hits_arch[0].doc_id, "arch.md");
    assert!(hits_arch[0].snippet.contains("<b>worktrees</b>"));

    // Search in logs
    let hits_err = db.fts_search("logs", "Failed socket", 10).unwrap();
    assert_eq!(hits_err.len(), 1);
    assert_eq!(hits_err[0].doc_id, "proc_2");

    // Deletion
    db.fts_delete("docs", "arch.md").unwrap();
    let hits_deleted = db.fts_search("docs", "worktrees", 10).unwrap();
    assert_eq!(hits_deleted.len(), 0);
}

#[test]
fn test_cdc_broadcast_events() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(DatabaseOptions::durable(dir.path().join("cdc_test.lilia"))).unwrap();

    let mut subscriber = db.subscribe();

    db.batch(&[
        BatchOperation::KvSet {
            namespace: "settings".into(),
            key: b"active_theme".to_vec(),
            value: b"dark".to_vec(),
            if_version: None,
            expires_at_ms: None,
        },
        BatchOperation::JsonPut {
            space: "work_items".into(),
            id: "item_1".into(),
            value: json!({ "title": "Refactor" }),
            if_version: None,
        },
    ])
    .unwrap();

    // Check received events
    let event1 = subscriber.try_recv().unwrap();
    match event1 {
        DatabaseEvent::KvMutated {
            namespace,
            key,
            version,
            deleted,
        } => {
            assert_eq!(namespace, "settings");
            assert_eq!(key, b"active_theme");
            assert_eq!(version, Some(1));
            assert!(!deleted);
        }
        _ => panic!("Expected KvMutated"),
    }

    let event2 = subscriber.try_recv().unwrap();
    match event2 {
        DatabaseEvent::JsonMutated {
            space,
            id,
            version,
            deleted,
        } => {
            assert_eq!(space, "work_items");
            assert_eq!(id, "item_1");
            assert_eq!(version, Some(1));
            assert!(!deleted);
        }
        _ => panic!("Expected JsonMutated"),
    }
}

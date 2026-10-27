# LiliaDB Code Examples

This directory provides runnable, end-to-end examples demonstrating how to use the LiliaDB Node.js SDK (`@liliadb/node`).

## Requirements

- Node.js 22 or newer (for Node.js examples)
- Rust 1.80+ / Cargo (for native Rust examples)
- Project dependencies built (`pnpm build` or `cargo build`)

## Node.js Examples (`@liliadb/node`)

### 1. Basic Operations (`node-basic.mjs`)
Demonstrates core functionality in embedded mode:
- Initializing and opening an embedded database file
- Writing and reading Key/Value entries (binary `Uint8Array` support)
- Storing, reading, and updating structured JSON documents
- Multi-model atomic batch transactions combining KV mutations and JSON documents with optimistic concurrency checks (`ifVersion`)
- Database integrity checks and clean lifecycle closing

**Run:**
```sh
node examples/node-basic.mjs
```

---

### 2. Streaming & Large Scans (`node-streaming.mjs`)
Demonstrates high-performance data traversal on large datasets:
- Seeding hundreds of records using batch transactions
- Asynchronous iteration (`for await ... of`) using `db.kv.scanStream()` and `db.json.scanStream()`
- Customizing page chunking with `batchSize`
- Bounding results with a global `limit`
- Resuming scans using keyset pagination with cursor `after`
- Integrating with Node.js standard Streams (`stream.pipeline` and `createReadStream()`)

**Run:**
```sh
node examples/node-streaming.mjs
```

---

### 3. Local Daemon Mode (`node-daemon.mjs`)
Demonstrates client-daemon architecture over high-performance local IPC:
- Spawning a private local daemon
- Connecting to the daemon over Unix domain socket (or Windows named pipe) using bearer tokens
- Executing Key/Value and JSON operations remotely
- Streaming records over IPC

**Run:**
```sh
node examples/node-daemon.mjs
```

---

### 4. Declarative SQL Querying (`node-sql.mjs`)
Demonstrates SQL queries across structured views:
- Querying documents via `lilia_json` view using `json_extract()`
- Running aggregations (`COUNT`, `AVG`, `GROUP BY`)
- Typed record projections with `db.sql.query<T>()`
- Raw matrix projections (`columns`, `rows`) with `db.sql.raw()`
- Executing mutations and updates via `db.sql.execute()`

**Run:**
```sh
node examples/node-sql.mjs
```

---

### 5. Live Events & CDC Streaming (`node-watch.mjs`)
Demonstrates reactive Change Data Capture (CDC) with `db.watch()`:
- Subscribing to live mutations across KV, JSON, Graph, and FTS models
- Unified namespace filtering (`db.watch("orders", callback)`)
- Building reactive in-memory caches and real-time UI synchronization
- Unsubscribing cleanly via `subscription.unsubscribe()`

**Run:**
```sh
node examples/node-watch.mjs
```

---

## Native Rust Examples (`crates/lilia-storage-sqlite/examples`)

Native Rust applications can embed LiliaDB directly with zero FFI overhead.

### 1. Basic Embedded Usage (`basic.rs`)
Demonstrates:
- Opening an embedded SQLite-backed LiliaDB engine (`Database::open`)
- KV operations with TTL and conditional version matching (`if_version: Some(0)`)
- JSON document put/get with structured serde `json!` values
- Multi-model atomic transactions combining KV mutations and JSON documents
- Online snapshot backup (`db.backup(&backup_path)`) and storage integrity checks (`db.check_integrity()`)

**Run:**
```sh
cargo run --example basic -p lilia-storage-sqlite
```

---

### 2. Keyset Streaming & Bounded Pagination (`streaming.rs`)
Demonstrates:
- Efficient keyset pagination using the `after` cursor (`db.json_scan("telemetry", cursor, 50)`)
- Bounded scans without offset drift under concurrent insertions
- Low memory footprint streaming across hundreds of records in chunked batches

**Run:**
```sh
cargo run --example streaming -p lilia-storage-sqlite
```

---

### 3. Custom Encryption Codec & Raw Inspection (`encryption.rs`)
Demonstrates:
- Implementing the pluggable `ValueCodec` trait (`name`, `encode`, `decode`)
- Mounting a codec via `DatabaseOptions::durable(&path).with_codec(codec)`
- Transparent encryption on write and decryption on read for KV values and JSON documents
- Verifying ciphertext at rest by directly querying SQLite raw tables (`lilia_kv`, `lilia_json`)

**Run:**
```sh
cargo run --example encryption -p lilia-storage-sqlite
```

---

### 4. Declarative SQL Querying (`sql.rs`)
Demonstrates:
- Querying documents and KV pairs declaratively using `db.sql_query()`
- Parameterized queries with positional bindings (`?1`, `?2`)
- Aggregations and grouping on JSON fields with `json_extract()`
- Transparent on-the-fly decryption via custom SQLite scalar functions (`lilia_decode_kv`, `lilia_decode_json`)
- Routing read-only queries to the reader pool and write mutations to the exclusive writer queue

**Run:**
```sh
cargo run --example sql -p lilia-storage-sqlite
```

---

## Python Examples (`packages/python`)

Zero-dependency official Python SDK connecting to `lilia-daemon` over the local HTTP gateway.

See [`examples/python/README.md`](file:///Users/shayn/Development/liliadb/examples/python/README.md) for full instructions.

### 1. Basic Key/Value Operations (`01_kv_basic.py`)
Demonstrates:
- Connecting to `lilia-daemon` over HTTP with Bearer token authentication
- Setting and retrieving Key/Value records
- Optimistic Concurrency Control (`if_version`)
- Key expiration with TTL (`expires_at_ms`)

### 2. JSON Document Store & Scans (`02_json_documents.py`)
Demonstrates:
- Storing and retrieving structured JSON objects
- Transparent keyset auto-pagination generators (`client.json.scan()`)
- Multi-model atomic batch mutations

### 3. Declarative SQL Analytics (`03_sql_analytics.py`)
Demonstrates:
- Executing SQL queries over `lilia_json`
- Analytical aggregations and filtering
- Extracting results with `to_dicts()`

### 4. AI Agent Memory & Sessions (`04_ai_agent_memory.py`)
Demonstrates:
- Building a stateful memory and session layer for AI agents using KV facts and JSON conversation logs

### 5. Live Events & CDC Streaming (`05_cdc_live_events.py`)
Demonstrates:
- Subscribing to real-time database mutations over HTTP SSE via `client.watch()`
- Filtering CDC events by namespace across KV, JSON, Graph, and FTS models
- Live state synchronization for autonomous AI agents and cache invalidation
- Graceful subscription management with `.unsubscribe()`




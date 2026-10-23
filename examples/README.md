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


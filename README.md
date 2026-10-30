# LiliaDB

[![CI](https://github.com/shaynlink/lilia/actions/workflows/ci.yml/badge.svg)](https://github.com/shaynlink/lilia/actions/workflows/ci.yml)
[![Compat Runtimes](https://github.com/shaynlink/lilia/actions/workflows/compat-runtimes.yml/badge.svg)](https://github.com/shaynlink/lilia/actions/workflows/compat-runtimes.yml)
[![PyPI version](https://img.shields.io/pypi/v/liliadb.svg)](https://pypi.org/project/liliadb/)
[![npm version](https://img.shields.io/npm/v/@liliadb/node.svg)](https://www.npmjs.com/package/@liliadb/node)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)

LiliaDB is a local-first multimodel embedded & daemon database written in Rust. It provides
durable Key/Value, JSON documents, Graph, and Full-Text Search through the same SQLite ACID
transaction boundary, with embedded Rust, Node.js, Bun, Deno, Python, CLI, and MCP entry points.

---

## Status: v0.1.0-alpha.4

This repository contains the `0.1.0-alpha.4` release featuring:
- **Global Package Manager Distribution**:
  - **Official Homebrew Tap** (`brew tap liliadb/tap && brew install lilia`) with daemon background service support (`brew services start liliadb`).
  - **Official PyPI Package** (`pip install liliadb`) with automated Trusted Publishing (OIDC) and zero external dependencies.
  - **Universal npm SDK** (`@liliadb/node`) supporting Node.js (>=22), Bun (>=1.0), and Deno (>=2.0) with multi-platform prebuilds (`darwin-arm64`, `darwin-x64`, `linux-arm64`, `linux-x64`, `win32-x64`).
- **Real-Time CDC & Live Event Streaming**: Sub-millisecond reactive subscriptions via `db.watch()` in Node/Bun/Deno, `client.watch()` in Python, and Server-Sent Events (`/v1/watch` SSE) over HTTP.
- **Graph & Full-Text Search (FTS5)**: Embedded graph traversal (`setNode`, `addEdge`, `neighbors`, `reachability`) and BM25-ranked full-text search with highlighting and snippets across all entry points.
- **Encryption at Rest**: Native authenticated encryption (AES-256-GCM AEAD) via `plugins/lilia-plugin-codec`, ensuring zero plaintext is stored on disk or in WAL frames, with transparent decryption across KV, JSON, and SQL views.
- **Declarative SQL Engine**: Integrated via `plugins/lilia-plugin-sql`, providing read/write SQL queries, preconfigured views (`lilia_kv`, `lilia_json`, `lilia_graph_nodes`, `lilia_graph_edges`), and custom scalar functions.
- **Physical Crash & Chaos Resilience**: Proven tolerance against physical power loss (torn WAL frame recovery), bit-flips in WAL frame checksums, abrupt process termination (`SIGKILL`), and disk exhaustion (`ENOSPC`) with atomic self-healing.
- **Windows Security Hardening**: Restrictive, protected DACLs applied to IPC directories, authentication tokens, database files, and backup staging, guaranteeing multi-user access denial parity with Unix (`0700`/`0600`).
- **Keyset Streaming & Async Iteration**: Native async generator `scanStream()` and `createReadStream()` in `@liliadb/node` for unbounded, memory-efficient traversals without offset drift.
- **Calibrated Performance Benchmarks**: Formal throughput and latency metrics (up to 281k ops/s in Rust native and 171k ops/s via HTTP gateway).

---

## Installation

### 1. Homebrew (macOS & Linux)
Install the standalone CLI (`lilia`), Daemon (`lilia-daemon`), and MCP server (`lilia-mcp`):

```bash
# Add the official tap and install
brew tap liliadb/tap
brew install lilia

# Or run the daemon as a background service
brew services start liliadb
```

### 2. Python (PyPI)
Install the official zero-dependency Python SDK:

```bash
pip install liliadb
```

### 3. JavaScript / TypeScript (Node.js, Bun, Deno)
Install the universal Node-API & client SDK:

```bash
# Node.js (npm / pnpm / yarn)
pnpm add @liliadb/node

# Bun
bun add @liliadb/node

# Deno
deno add npm:@liliadb/node
```

---

## Quick Start

### CLI (Command-Line Interface)
```sh
# Initialize database and insert JSON document
lilia --database ./demo.lilia json put users ada '{"name":"Ada Lovelace","active":true}' --output json

# Query document
lilia --database ./demo.lilia json get users ada --output json

# Query using SQL analytics
lilia --database ./demo.lilia sql "SELECT id, json_extract(value, '$.name') AS name FROM lilia_json WHERE space = 'users'"

# Create atomic online backup snapshot
lilia --database ./demo.lilia backup ./snapshots/demo.lilia --output json
```

### Node.js / Bun / Deno SDK (`@liliadb/node`)
```typescript
import { Database } from "@liliadb/node";

// Embedded in-process database (Node.js / Bun)
const db = await Database.open({
  path: "./app.lilia",
  durability: "durable", // "durable" | "balanced" | "performance"
});

// Key/Value operations
const enc = new TextEncoder();
await db.kv.set("session", enc.encode("tok_abc"), enc.encode("user_42"));
const session = await db.kv.get("session", enc.encode("tok_abc"));

// JSON document operations
await db.json.put("users", "usr_101", { name: "Alice", role: "admin" });
const user = await db.json.get("users", "usr_101");

// SQL analytical queries
const admins = await db.sql.query("SELECT id, value FROM lilia_json WHERE space = 'users'");

await db.close();
```

### Python SDK (`liliadb`)
```python
from liliadb import LiliaClient

# Connect to local daemon via HTTP REST gateway
client = LiliaClient(endpoint="http://127.0.0.1:4444", token="my-token")

# JSON document store
client.json.put("customers", "cust_1", {"name": "Grace Hopper", "status": "active"})
doc = client.json.get("customers", "cust_1")
print(doc.value)

# Key/Value store
client.kv.set("configs", "theme", "dark")
entry = client.kv.get("configs", "theme")
print(entry.value)
```

---

## Key Features

### 1. Real-Time CDC & Live Watch Subscriptions
Subscribe to live change events in real time with namespace-level filtering.

**TypeScript (Node / Bun / Deno):**
```typescript
const sub = await db.watch("orders", (event) => {
  console.log(`[CDC] ${event.type}: ${event.id || event.key} (deleted: ${event.deleted})`);
});

// Mutate data -> trigger reactive event
await db.json.put("orders", "ord_100", { total: 49.99 });

// Cleanup
sub.unsubscribe();
```

**Python:**
```python
with client.watch(namespace="orders", callback=lambda e: print("Event:", e.type, e.id)) as sub:
    client.json.put("orders", "ord_100", {"total": 49.99})
```

**HTTP Server-Sent Events (SSE):**
```bash
curl -N http://127.0.0.1:4444/v1/watch?namespace=orders \
  -H "Authorization: Bearer <TOKEN>"
```

---

### 2. Embedded Graph & Full-Text Search (FTS5)

**Graph Traversal:**
```typescript
// Directed relationship graph
await db.graph.setNode("social", "user_1", "User", { name: "Alice" });
await db.graph.setNode("social", "user_2", "User", { name: "Bob" });
await db.graph.addEdge("social", "user_1", "user_2", "FOLLOWS");

// Find neighbors and reachability
const friends = await db.graph.neighbors("social", "user_1", { direction: "outgoing" });
const network = await db.graph.reachability("social", ["user_1"], { direction: "outgoing" });
```

**Full-Text Search:**
```typescript
// Index text document
await db.fts.index("articles", "art_1", "LiliaDB delivers unified SQLite multi-model storage.");

// Search with BM25 ranking & snippet extraction
const hits = await db.fts.search("articles", "multi-model storage", { limit: 5 });
for (const hit of hits) {
  console.log(`Hit ${hit.docId} (rank: ${hit.rank}): ${hit.snippet}`);
}
```

---

### 3. Encryption at Rest (AES-256-GCM AEAD)
Start the CLI or Daemon with native authenticated encryption at rest:

```sh
# 256-bit encryption key (hex or passphrase)
export LILIA_CODEC_KEY="0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"

# Stored KV and JSON values are encrypted on disk and in WAL frames
lilia --database ./secure.lilia --plugin ./target/release/liblilia_plugin_codec.dylib \
  kv set vault secret_key "confidential_token"

# Decrypt transparently via SQL views
lilia --database ./secure.lilia --plugin ./target/release/liblilia_plugin_codec.dylib \
  sql "SELECT key, lilia_decode_kv(value) FROM lilia_kv"
```

---

### 4. Local Daemon & HTTP REST Gateway
Run the background daemon on Unix with both IPC socket and loopback HTTP gateway:

```sh
lilia-daemon \
  --database ./data.lilia \
  --endpoint ./data.lilia.sock \
  --token-file ./data.lilia.token \
  --http-port 4444
```

See [HTTP REST API Reference](docs/http_api.md) for endpoint details.

---

### 5. Model Context Protocol (MCP) Server
Run the MCP server to give AI agents access to LiliaDB:

```sh
lilia-mcp --database primary=./app.lilia
```

Exposed Tools: `lilia_kv_get`, `lilia_kv_set`, `lilia_json_get`, `lilia_json_put`, `lilia_sql_query`, `lilia_backup`, `lilia_integrity_check`, `lilia_checkpoint_stats`.

---

## Performance Benchmarks

Calibrated throughput and latency distributions measured on Apple M3 Max (14 cores, macOS 25.6, SQLite WAL `Durability::Durable`):

| Surface | Workload | Batch Size | Throughput (ops/sec) | Latency $p_{50}$ | Latency $p_{99}$ |
| :--- | :---: | :---: | :---: | :---: | :---: |
| **Rust Native Embedded** | 10,000 | 1 | **8,380 ops/s** | 85.17 µs | 266.42 µs |
| **Daemon via IPC (Socket)** | 10,000 | 1 | **5,711 ops/s** | 155.23 µs | 509.61 µs |
| **Daemon via HTTP Loopback** | 10,000 | 1 | **4,519 ops/s** | 161.96 µs | 375.02 µs |
| **Rust Native Embedded** | 10,000 | 100 | **158,192 ops/s** | 3.51 µs | 89.10 µs |
| **Daemon via HTTP Loopback** | 10,000 | 100 | **116,798 ops/s** | 7.89 µs | 11.29 µs |
| **Rust Native Bulk Ingest** | 100,000 | 500 | **281,016 ops/s** | 3.20 µs | 5.33 µs |
| **Daemon via HTTP Bulk Ingest** | 100,000 | 500 | **171,051 ops/s** | 5.42 µs | 7.81 µs |

Read the full report in [Official Performance Benchmarks](docs/benchmarks.md) and the [Comparative Benchmark: LiliaDB vs Native SQLite & Ecosystems](docs/benchmarks_comparison.md).

---

## Workspace Structure

- `apps/`: CLI (`lilia-cli`), local daemon (`lilia-daemon`), and MCP server (`lilia-mcp`).
- `crates/`:
  - `lilia-core`: Storage primitives, error models, and AES-256-GCM AEAD codec.
  - `lilia-storage-sqlite`: SQLite engine adapter, migrations, views, and benchmarks (`lilia_bench`).
  - `lilia-protocol`: Binary MessagePack IPC framing, handshake, and streaming request/response envelopes.
  - `lilia-plugin-api`: Plugin C-ABI, trust store, and ed25519 signature verification.
  - `lilia-graph`: Graph reachability and adjacency engine.
  - `lilia-node-native`: High-performance Node-API native bindings.
- `plugins/`: Native plugins (`lilia-plugin-codec`, `lilia-plugin-sql`, `lilia-plugin-kv`, `lilia-plugin-json`).
- `packages/`:
  - `@liliadb/node`: Universal Node.js, Bun, and Deno SDK (`packages/node`) and native prebuilds (`packages/node-*`).
  - `liliadb`: Official zero-dependency Python SDK (`packages/python`).
- `Formula/`: Homebrew packaging (`Formula/liliadb.rb`, `Formula/lilia.rb`).
- `examples/`: Runnable examples for Node.js, Bun, Python, and native Rust.

Documentation: [Architecture](docs/architecture.md) | [Node.js SDK](docs/node_sdk.md) | [Python SDK](docs/python_sdk.md) | [HTTP REST API](docs/http_api.md) | [Benchmarks](docs/benchmarks.md) | [Comparative Benchmarks](docs/benchmarks_comparison.md) | [Plugins](docs/plugins.md) | [Security Policy](SECURITY.md)

---

## License

Apache-2.0.

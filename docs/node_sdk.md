# LiliaDB Node.js SDK (`@liliadb/node`)

Official TypeScript and Node.js SDK for **LiliaDB**, the local-first multimodel embedded and daemon database written in Rust.

---

## 1. Key Features

- **Embedded & Daemon Modes**: Direct in-process execution via native Node-API bindings or connection to a local daemon over Unix domain sockets or Windows named pipes.
- **Multimodel Engine**: Unified Key/Value store, JSON documents with dynamic secondary indexing, embedded Graph traversal, Full-Text Search (FTS5), and declarative SQL queries.
- **Multi-Runtime Support**: First-class support for **Node.js** (>= 22), **Bun** (>= 1.0), and **Deno** (>= 2.0).
- **Exact 64-bit Versioning**: Safe optimistic concurrency control (`ifVersion`) using JavaScript native `bigint` without IEEE-754 precision loss.
- **Asynchronous Keyset Streaming**: Memory-bounded `for await` async iteration (`scanStream`) and standard Node.js Streams (`createReadStream`) with transparent keyset pagination.
- **Real-Time CDC Subscriptions**: Live change event bus (`db.watch()`) across all mutations with namespace filtering.
- **Atomic Multi-Model Transactions**: Apply KV mutations, JSON documents, graph edges, and search indexes in a single ACID commit (`db.batch()`).

---

## 2. Installation

Install via your preferred package manager:

```bash
# pnpm
pnpm add @liliadb/node

# npm
npm install @liliadb/node

# Bun
bun add @liliadb/node

# Deno 2
deno add npm:@liliadb/node
```

> **Prebuilt Native Addons**: Binary prebuilds are automatically resolved and downloaded for `darwin-arm64`, `darwin-x64`, `linux-arm64-gnu`, `linux-x64-gnu`, and `win32-x64-msvc`.

---

## 3. Quick Start

```typescript
import { Database } from "@liliadb/node";

// 1. Open an embedded database
const db = await Database.open({
  path: "./app.lilia",
  durability: "durable", // "durable" | "balanced" | "performance"
});

// 2. Key/Value Store (Binary safe)
const enc = new TextEncoder();
const dec = new TextDecoder();
await db.kv.set("session", enc.encode("tok_abc"), enc.encode("user_42"));
const entry = await db.kv.get("session", enc.encode("tok_abc"));
if (entry) {
  console.log("Session:", dec.decode(entry.value), "version:", entry.version);
}

// 3. JSON Document Store
await db.json.put("users", "usr_101", { name: "Alice", role: "admin" });
const user = await db.json.get("users", "usr_101");
console.log("User:", user?.value);

// 4. Embedded Graph
await db.graph.setNode("social", "usr_101", "User", { name: "Alice" });
await db.graph.setNode("social", "usr_102", "User", { name: "Bob" });
await db.graph.addEdge("social", "usr_101", "usr_102", "FOLLOWS");
const friends = await db.graph.neighbors("social", "usr_101", { direction: "outgoing" });
console.log("Friends:", friends);

// 5. Full-Text Search (FTS5)
await db.fts.index("articles", "art_1", "LiliaDB delivers local-first multimodel database storage.");
const hits = await db.fts.search("articles", "multimodel storage");
for (const hit of hits) {
  console.log(`Hit ${hit.docId} (rank ${hit.rank}): ${hit.snippet}`);
}

// 6. Declarative SQL Analytics
const admins = await db.sql.query("SELECT id, value FROM lilia_json WHERE space = 'users'");
console.log("Admins:", admins);

// 7. Clean Shutdown
await db.close();
```

---

## 4. Modes of Operation

### Embedded Mode (Default)
Runs in-process via Node-API native bindings. SQLite WAL operations run on background threads without blocking the JavaScript event loop:

```typescript
const db = await Database.open({
  path: "app.lilia",
  durability: "durable", // "durable" | "balanced" | "performance"
  busyTimeoutMs: 5000,
  writerQueueCapacity: 64,
  readPoolSize: 4,
  checkpointPolicy: {
    walBytes: 4 * 1024 * 1024, // 4MB
    intervalMs: 1000,
  },
});
```

### Daemon Mode
Connects over local IPC to an isolated `lilia-daemon` background service:

```typescript
const db = await Database.open({
  mode: "daemon",
  path: "app.lilia",
  endpoint: "app.lilia.sock", // or \\.\pipe\lilia on Windows
  tokenFile: "app.lilia.token",
});
```

---

## 5. API Reference

### Key / Value Model (`db.kv`)

Keys and values are binary `Uint8Array` slices:

```typescript
const enc = new TextEncoder();
const dec = new TextDecoder();

// Put with version check (OCC) and millisecond TTL
const write = await db.kv.set(
  "sessions",
  enc.encode("sess_01"),
  enc.encode("active"),
  {
    ifVersion: 0n,                     // Only insert if key does not exist
    expiresAtMs: Date.now() + 3600_000, // 1 hour TTL
  }
);

// Get entry
const entry = await db.kv.get("sessions", enc.encode("sess_01"));
if (entry) {
  console.log(dec.decode(entry.value), entry.version, entry.expiresAtMs);
}

// Delete entry
await db.kv.delete("sessions", enc.encode("sess_01"), { ifVersion: write.version });

// In-memory bounded scan
const entries = await db.kv.scan("sessions", { limit: 100 });
```

---

### JSON Document Model (`db.json`)

Documents are stored as structured JSON objects:

```typescript
// Put document
const put = await db.json.put("customers", "cust_100", {
  name: "Grace Hopper",
  plan: "enterprise",
  credits: 5000,
  tags: ["vip", "core"],
}, { ifVersion: 0n });

// Get document
const doc = await db.json.get("customers", "cust_100");
console.log(doc?.value);

// Scan documents
const list = await db.json.scan("customers", { limit: 50 });

// --- Dynamic Secondary Indexing ---
// Create an index on a JSON field or path (backfills existing docs and auto-indexes arrays)
await db.json.createIndex("customers", "plan");
await db.json.createIndex("customers", "email", { name: "idx_custom_email" });

// List indexes
const indexes = await db.json.listIndexes("customers");

// Find documents matching query criteria (auto-accelerated by index when available)
const enterpriseCustomers = await db.json.find("customers", { plan: "enterprise" }, { limit: 10 });

// Stream search results with automatic pagination
for await (const customer of db.json.findStream("customers", { plan: "enterprise" }, { batchSize: 50 })) {
  console.log(customer.id, customer.value);
}

// Drop an index
await db.json.dropIndex("customers", "idx_custom_email");
```

---

### Embedded Graph Engine (`db.graph`)

Directed graph modeling with nodes, edges, neighbor traversals, and reachability:

```typescript
// Nodes
await db.graph.setNode("social", "user_1", "User", { name: "Alice" });
await db.graph.setNode("social", "user_2", "User", { name: "Bob" });

// Edges
await db.graph.addEdge("social", "user_1", "user_2", "FOLLOWS");

// Traversal
const neighbors = await db.graph.neighbors("social", "user_1", { direction: "outgoing" });
const network = await db.graph.reachability("social", ["user_1"], { direction: "outgoing" });
```

---

### Full-Text Search (`db.fts`)

Integrated SQLite FTS5 with BM25 scoring and snippet generation:

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

### Declarative SQL Querying (`db.sql`)

LiliaDB exposes structured SQLite views with on-the-fly transparent decryption functions:
- `lilia_json`: `(space, id, value, version)`
- `lilia_kv`: `(namespace, key, value, version, expires_at_ms)`
- `lilia_graph_nodes`: `(graph, id, kind, metadata)`
- `lilia_graph_edges`: `(graph, source, target, kind)`

#### Typed Queries (`db.sql.query`)
```typescript
interface UserRow {
  id: string;
  name: string;
  role: string;
}

const users = await db.sql.query<UserRow>(
  `SELECT id, json_extract(value, '$.name') AS name, json_extract(value, '$.role') AS role
   FROM lilia_json
   WHERE space = ?1`,
  ["users"]
);
```

#### Raw Projections (`db.sql.raw`)
```typescript
const { columns, rows } = await db.sql.raw(
  "SELECT id, json_extract(value, '$.role') AS role FROM lilia_json WHERE space = ?1",
  ["users"]
);
```

#### Executing Mutations (`db.sql.execute`)
```typescript
const { rowsAffected } = await db.sql.execute(
  "UPDATE _lilia_json SET version = version + 1 WHERE space = ?1",
  ["users"]
);
```

---

### Keyset Streaming & Node.js Streams

Stream unbounded datasets without loading them entirely into memory:

#### Asynchronous Iteration (`for await`)
```typescript
// Stream JSON documents with automatic keyset pagination
for await (const doc of db.json.scanStream("events", { batchSize: 200, limit: 1000 })) {
  console.log(doc.id, doc.value);
}

// Stream Key/Value pairs
for await (const entry of db.kv.scanStream("analytics", { batchSize: 500 })) {
  console.log(entry.key, entry.value);
}
```

#### Node.js Standard Streams (`createReadStream`)
```typescript
import { pipeline } from "node:stream/promises";
import { Transform } from "node:stream";

const readStream = db.json.createReadStream("records", { batchSize: 100 });
const toCsv = new Transform({
  objectMode: true,
  transform(doc, _, callback) {
    callback(null, `${doc.id},${JSON.stringify(doc.value)}\n`);
  },
});

await pipeline(readStream, toCsv, process.stdout);
```

---

### Real-Time CDC Watch Subscriptions (`db.watch`)

Subscribe to reactive database mutations with namespace filtering:

```typescript
const sub = await db.watch("orders", (event) => {
  console.log(`[CDC] ${event.type}: ${event.id || event.key} (deleted: ${event.deleted})`);
});

// Mutate data -> triggers callback
await db.json.put("orders", "ord_100", { total: 49.99 });

// Cleanup
sub.unsubscribe();
```

---

### Atomic Multi-Model Transactions (`db.batch`)

Combine operations across KV, JSON, Graph, and FTS into a single atomic ACID commit:

```typescript
const results = await db.batch([
  {
    model: "json_put",
    space: "users",
    id: "ada",
    value: { name: "Ada", status: "verified" },
    ifVersion: 2n,
  },
  {
    model: "kv_set",
    namespace: "audit",
    key: new TextEncoder().encode("log:ada:verified"),
    value: new TextEncoder().encode(new Date().toISOString()),
  },
  {
    model: "kv_delete",
    namespace: "pending_verifications",
    key: new TextEncoder().encode("ada"),
  },
]);
```

---

## 6. Version Precision & OCC

- All versions returned by reads, scans, and mutations are `bigint`.
- Expected versions in conditional mutations (`ifVersion`) must be `bigint` (where `0n` signifies *key does not exist*).
- Out-of-bounds versions throw structured `LiliaError` with code `INVALID_INPUT`.

---

## 7. Error Handling

Operations throw typed `LiliaError` instances:

```typescript
import { LiliaError } from "@liliadb/node";

try {
  await db.json.put("users", "ada", { status: "updated" }, { ifVersion: 99n });
} catch (error) {
  if (error instanceof LiliaError) {
    console.error(`Code: ${error.code}`);         // e.g. "CONFLICT", "CLOSED", "TIMEOUT"
    console.error(`Message: ${error.message}`);
    console.error(`Retryable: ${error.retryable}`); // boolean
    console.error(`RequestId: ${error.requestId}`);
  }
}
```

---

## 8. Graceful Shutdown

```typescript
// Drain active in-flight queries before releasing file locks and worker threads
await db.close({ timeoutMs: 5_000 });
```

---

## 9. License

Apache-2.0

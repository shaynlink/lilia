# @liliadb/node

Official Node.js SDK for **LiliaDB**, a local-first multimodel database providing durable Key/Value and JSON Document storage backed by SQLite and native plugins.

## Features

- **Embedded & Daemon modes**: Run in-process with direct Node-API bindings or connect to a local daemon over Unix domain sockets / Windows named pipes.
- **Multimodel transactional consistency**: Mutate Key/Value pairs and JSON documents atomically within the exact same commit.
- **Asynchronous streaming**: Stream large datasets using standard `for await (const entry of ...)` and Node.js Streams (`Readable`).
- **Exact version precision**: 64-bit integer versions (`bigint`) for robust optimistic concurrency control (`ifVersion`).
- **TTL Support**: Built-in millisecond expiration on Key/Value entries.

---

## Installation

```sh
pnpm add @liliadb/node
# or npm install @liliadb/node
```

> **Requirements**: Node.js `>=22` and a platform-supported native addon (`@liliadb/node-darwin-arm64`, `@liliadb/node-darwin-x64`, `@liliadb/node-linux-x64-gnu`, `@liliadb/node-linux-arm64-gnu`, or `@liliadb/node-win32-x64-msvc`).

---

## Quick Start

```ts
import { Database } from "@liliadb/node";

const db = await Database.open({ path: "app.lilia" });

// 1. JSON Document
await db.json.put("users", "ada", { name: "Ada Lovelace", role: "admin" });
const user = await db.json.get("users", "ada");
console.log(user?.value);

// 2. Key/Value
const encoder = new TextEncoder();
await db.kv.set("cache", encoder.encode("token"), encoder.encode("xyz123"));
const token = await db.kv.get("cache", encoder.encode("token"));
console.log(new TextDecoder().decode(token?.value));

// 3. Clean shutdown
await db.close();
```

---

## Modes of Operation

### Embedded Mode (Default)
Runs in-process via high-performance Node-API bindings:
```ts
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
Connects over local IPC to an isolated `lilia-daemon` process:
```ts
const db = await Database.open({
  mode: "daemon",
  path: "app.lilia",
  endpoint: "app.lilia.sock", // or \\.\pipe\lilia on Windows
  tokenFile: "app.lilia.token",
});
```

---

## Key/Value Model

Keys and values are binary `Uint8Array` slices.

```ts
const encoder = new TextEncoder();
const decoder = new TextDecoder();

// Put with optional version check and TTL
const write = await db.kv.set(
  "sessions",
  encoder.encode("sess_01"),
  encoder.encode("active"),
  {
    ifVersion: 0n,                     // Only insert if key does not exist
    expiresAtMs: Date.now() + 3600_000, // 1 hour TTL
  }
);
console.log(`Version: ${write.version}`);

// Get
const entry = await db.kv.get("sessions", encoder.encode("sess_01"));
if (entry) {
  console.log(decoder.decode(entry.value), entry.version, entry.expiresAtMs);
}

// Delete
await db.kv.delete("sessions", encoder.encode("sess_01"), { ifVersion: write.version });

// Scan (bounded array in memory)
const entries = await db.kv.scan("sessions", { limit: 100 });
```

---

## JSON Document Model

Documents are stored as structured JSON objects, validated and indexed.

```ts
// Put document
const put = await db.json.put("customers", "cust_100", {
  name: "Grace Hopper",
  plan: "enterprise",
  credits: 5000,
  tags: ["vip", "early-adopter"],
}, { ifVersion: 0n });

// Get document
const doc = await db.json.get("customers", "cust_100");
console.log(doc?.value);

// Scan documents
const list = await db.json.scan("customers", { limit: 50 });

// --- Dynamic Secondary Indexing ---

// Create an index on a JSON field or path (backfills existing docs and auto-indexes multikey arrays)
await db.json.createIndex("customers", "plan");
await db.json.createIndex("customers", "email", { name: "idx_custom_email" });
await db.json.createIndex("customers", "tags");

// List indexes
const indexes = await db.json.listIndexes("customers");
console.log(indexes);

// Find documents matching query criteria (auto-accelerated by index if available, with scan fallback)
const enterpriseCustomers = await db.json.find("customers", { plan: "enterprise" }, { limit: 10 });

// Find documents matching multikey array elements
const vipCustomers = await db.json.find("customers", { tags: "vip" });

// Stream search results with automatic pagination
for await (const doc of db.json.findStream("customers", { plan: "enterprise" }, { batchSize: 50 })) {
  console.log(doc.id, doc.value);
}

// Low-level index lookup by exact index name
const results = await db.json.findByIndex("customers", "idx_custom_email", "grace@example.com");

// Drop an index
await db.json.dropIndex("customers", "idx_custom_email");

```

---

## Streaming & Large Scans

For large datasets, loading all records into an in-memory array with `.scan()` can be slow and memory-intensive. Use **`scanStream`** (or its alias **`stream`**) for streaming records with transparent keyset pagination.

### Asynchronous Iteration (`for await`)

```ts
// Iterate over JSON documents with automatic chunking
for await (const doc of db.json.scanStream("telemetry", { batchSize: 200 })) {
  console.log(doc.id, doc.value);
}

// Key/Value streaming with limit
for await (const entry of db.kv.scanStream("analytics", {
  batchSize: 500,
  limit: 2500, // stops after 2,500 entries
})) {
  console.log(entry.key, entry.value);
}
```

### Resume from a Cursor (`after`)

```ts
// Start scanning strictly after a previous document ID or key
for await (const doc of db.json.scanStream("events", { after: "event_0500", batchSize: 100 })) {
  console.log(doc.id);
}
```

### Node.js Streams (`createReadStream`)

Convert any scan directly into a Node.js `Readable` stream in `objectMode`:

```ts
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

## Atomic Multi-Model Transactions (`batch`)

Combine Key/Value operations and JSON Document operations into a single atomic transaction:

```ts
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

---

## Declarative SQL Querying (`db.sql`)

LiliaDB provides declarative SQL querying directly over its multi-model storage via predefined SQLite views. Custom scalar functions (`lilia_decode_kv`, `lilia_decode_json`) automatically handle transparent on-the-fly decryption if an encryption codec is enabled.

### Available Views
- **`lilia_json`**: `(space, id, value, version)` — `value` is decoded plaintext JSON, allowing native SQLite `json_extract()`, `json_tree()`, and JSON operators.
- **`lilia_kv`**: `(namespace, key, value, version, expires_at_ms)` — `value` is decoded plaintext bytes.
- **`lilia_graph_nodes`**: `(graph, id, kind, metadata)`
- **`lilia_graph_edges`**: `(graph, source, target, kind)`

### Typed SQL Queries (`db.sql.query`)
Returns an array of typed objects mapped directly from column names:

```ts
interface Developer {
  id: string;
  name: string;
  points: number;
}

const devs = await db.sql.query<Developer>(
  `SELECT
     id,
     json_extract(value, '$.name') AS name,
     json_extract(value, '$.points') AS points
   FROM lilia_json
   WHERE space = ?1 AND json_extract(value, '$.points') >= ?2
   ORDER BY points DESC`,
  ["developers", 900],
);

for (const dev of devs) {
  console.log(`${dev.name}: ${dev.points} pts`);
}
```

### Raw Projections (`db.sql.raw`)
Returns a `{ columns, rows, rowsAffected }` structure for raw projections or tabular displays:

```ts
const raw = await db.sql.raw(
  "SELECT id, json_extract(value, '$.role') AS role FROM lilia_json WHERE space = ?1",
  ["users"],
);
console.log(raw.columns); // ["id", "role"]
console.log(raw.rows);    // [["ada", "admin"], ...]
```

### Executing Mutations (`db.sql.execute`)
Executes mutating SQL statements via the write queue and returns `{ rowsAffected: number }`:

```ts
const { rowsAffected } = await db.sql.execute(
  "UPDATE _lilia_json SET version = version + 1 WHERE space = ?1",
  ["users"],
);
console.log(`Updated ${rowsAffected} rows`);
```

---

## Version Precision & Concurrency

- Use `bigint` for version comparisons (`0n` means *key does not exist*).
- Versions returned by reads, scans, and mutations are always `bigint`.
- Out-of-bounds versions throw an `INVALID_INPUT` error.
- The stored version limit is SQLite `i64::MAX`.

---

## Error Handling

All SDK operations throw structured `LiliaError` instances when operations fail:

```ts
import { LiliaError } from "@liliadb/node";

try {
  await db.json.put("users", "ada", { status: "updated" }, { ifVersion: 99n });
} catch (error) {
  if (error instanceof LiliaError) {
    console.error(`Code: ${error.code}`);         // e.g. "CONFLICT", "CLOSED", "TIMEOUT"
    console.error(`Message: ${error.message}`);
    console.error(`Retryable: ${error.retryable}`); // true if temporary / busy
    console.error(`RequestId: ${error.requestId}`);
  }
}
```

---

## Lifecycle & Graceful Shutdown

```ts
// Drains in-flight JavaScript operations up to timeoutMs before releasing locks
await db.close({ timeoutMs: 5_000 });
```

---

## License

Apache-2.0

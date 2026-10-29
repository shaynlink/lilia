# LiliaDB HTTP REST Gateway Specification

`lilia-daemon` includes a high-performance local loopback HTTP/REST gateway. It provides language-agnostic access to LiliaDB's Key/Value store, JSON document engine, embedded Graph relationships, Full-Text Search (FTS5), atomic batch transactions, SQL analytical views, and real-time Change Data Capture (CDC) Server-Sent Events (SSE).

---

## 1. Security Architecture & Network Model

The HTTP gateway is engineered strictly for **local IPC replacement** (e.g., Python scripts, AI agent tools, sidecars, and local containers):

1. **Loopback Only**: The server refuses to bind to non-loopback interfaces. Attempting to pass `--http-bind 0.0.0.0` or a public IP exits immediately with an error. Valid bindings are `127.0.0.1` (IPv4 loopback) or `::1` (IPv6 loopback).
2. **Bearer Token Authentication**: Every `/v1/*` request requires a valid `Authorization: Bearer <TOKEN>` header (or an optional `?token=<TOKEN>` query parameter for SSE event streams).
3. **Constant-Time Comparison**: Tokens are validated using constant-time equality verification (`subtle::ConstantTimeEq`) to prevent timing side-channel attacks.
4. **Public vs. Protected Routes**:
   - `GET /health`: Public (no token needed, for health checks and orchestrator probes).
   - All `/v1/*` routes: Protected by Bearer token.

---

## 2. Starting the Daemon with HTTP Gateway

To enable the HTTP gateway on `lilia-daemon`, pass `--http-port`:

```bash
lilia-daemon \
  --database /path/to/database.lilia \
  --endpoint /path/to/daemon.sock \
  --token-file /path/to/token \
  --http-port 4444
```

Or specify an explicit auth token and loopback bind address:

```bash
lilia-daemon \
  --database ./data.lilia \
  --http-port 4444 \
  --http-bind 127.0.0.1:4444 \
  --token-file ./data.lilia.token \
  --development
```

When started, the daemon logs:
```
[INFO] HTTP gateway listening on http://127.0.0.1:4444
```

---

## 3. Error Model & HTTP Status Codes

Errors return a JSON object with standard fields:
```json
{
  "ok": false,
  "error": {
    "code": "CONFLICT",
    "message": "version mismatch",
    "retryable": false,
    "details": {
      "expected": "1",
      "actual": "2"
    }
  }
}
```

### Status Code Mapping

| HTTP Status | Error Code | Description |
|---|---|---|
| **200 OK** | - | Operation succeeded. |
| **400 Bad Request** | `INVALID_INPUT` | Malformed JSON request body or invalid parameters. |
| **401 Unauthorized** | `AUTHENTICATION` | Missing or invalid `Authorization: Bearer <TOKEN>` header. |
| **404 Not Found** | `NOT_FOUND` | Document or Key/Value entry does not exist. |
| **409 Conflict** | `CONFLICT` | Optimistic concurrency conflict (`if_version` mismatch). |
| **500 Internal Error** | `STORAGE` / `INTERNAL` | Storage engine failure or unexpected internal error. |

---

## 4. Endpoints Reference

### Public Endpoints

#### `GET /health`
Returns daemon health and status. Does not require authentication.

**Request:**
```bash
curl -s http://127.0.0.1:4444/health
```

**Response (`200 OK`):**
```json
{
  "ok": true,
  "status": "running",
  "version": "0.1.0-alpha.4"
}
```

---

### Key / Value Endpoints

#### `POST /v1/kv/get`
Retrieve a Key/Value entry by namespace and key. Supports string encoding (`utf8` default, `base64`, `hex`).

**Request:**
```bash
curl -s -X POST http://127.0.0.1:4444/v1/kv/get \
  -H "Authorization: Bearer my-secret-token" \
  -H "Content-Type: application/json" \
  -d '{"namespace": "session", "key": "sess_123"}'
```

**Response (`200 OK`):**
```json
{
  "ok": true,
  "entry": {
    "namespace": "session",
    "key": "sess_123",
    "value": "user_42",
    "version": "1",
    "expires_at_ms": null
  }
}
```

#### `POST /v1/kv/set`
Insert or update a Key/Value entry with optional optimistic concurrency (`if_version`) and TTL (`expires_at_ms`).

**Request:**
```bash
curl -s -X POST http://127.0.0.1:4444/v1/kv/set \
  -H "Authorization: Bearer my-secret-token" \
  -H "Content-Type: application/json" \
  -d '{
    "namespace": "session",
    "key": "sess_123",
    "value": "user_42",
    "if_version": null,
    "expires_at_ms": 1780000000000
  }'
```

**Response (`200 OK`):**
```json
{
  "ok": true,
  "version": "1"
}
```

#### `POST /v1/kv/delete`
Delete a Key/Value entry with optional optimistic concurrency (`if_version`).

**Request:**
```bash
curl -s -X POST http://127.0.0.1:4444/v1/kv/delete \
  -H "Authorization: Bearer my-secret-token" \
  -H "Content-Type: application/json" \
  -d '{"namespace": "session", "key": "sess_123", "if_version": 1}'
```

**Response (`200 OK`):**
```json
{
  "ok": true,
  "deleted": true
}
```

#### `POST /v1/kv/scan`
Scan entries in a namespace using keyset pagination (`after` cursor).

**Request:**
```bash
curl -s -X POST http://127.0.0.1:4444/v1/kv/scan \
  -H "Authorization: Bearer my-secret-token" \
  -H "Content-Type: application/json" \
  -d '{"namespace": "session", "limit": 25, "after": null}'
```

**Response (`200 OK`):**
```json
{
  "ok": true,
  "entries": [
    {
      "namespace": "session",
      "key": "sess_001",
      "value": "active",
      "version": "1",
      "expires_at_ms": null
    }
  ],
  "count": 1
}
```

---

### JSON Document Endpoints

#### `GET /v1/json/:space/:id`
Fetch a single JSON document via path parameters.

**Request:**
```bash
curl -s -X GET http://127.0.0.1:4444/v1/json/users/alice \
  -H "Authorization: Bearer my-secret-token"
```

**Response (`200 OK` if found, `404 Not Found` if missing):**
```json
{
  "space": "users",
  "id": "alice",
  "value": {
    "name": "Alice",
    "email": "alice@example.com",
    "roles": ["admin"]
  },
  "version": "1"
}
```

#### `POST /v1/json/get`
Fetch a JSON document using JSON request body.

**Request:**
```bash
curl -s -X POST http://127.0.0.1:4444/v1/json/get \
  -H "Authorization: Bearer my-secret-token" \
  -H "Content-Type: application/json" \
  -d '{"space": "users", "id": "alice"}'
```

**Response (`200 OK`):**
```json
{
  "ok": true,
  "document": {
    "space": "users",
    "id": "alice",
    "value": {"name": "Alice", "roles": ["admin"]},
    "version": "1"
  }
}
```

#### `POST /v1/json/put`
Insert or update a JSON document with optional optimistic concurrency (`if_version`).

**Request:**
```bash
curl -s -X POST http://127.0.0.1:4444/v1/json/put \
  -H "Authorization: Bearer my-secret-token" \
  -H "Content-Type: application/json" \
  -d '{
    "space": "users",
    "id": "alice",
    "value": {"name": "Alice", "role": "admin"},
    "if_version": 0
  }'
```

**Response (`200 OK`):**
```json
{
  "ok": true,
  "version": "1"
}
```

#### `POST /v1/json/delete`
Delete a JSON document with optional version checking.

**Request:**
```bash
curl -s -X POST http://127.0.0.1:4444/v1/json/delete \
  -H "Authorization: Bearer my-secret-token" \
  -H "Content-Type: application/json" \
  -d '{"space": "users", "id": "alice", "if_version": 1}'
```

**Response (`200 OK`):**
```json
{
  "ok": true,
  "deleted": true
}
```

#### `POST /v1/json/scan`
Scan documents in a space using keyset cursor pagination (`after`).

**Request:**
```bash
curl -s -X POST http://127.0.0.1:4444/v1/json/scan \
  -H "Authorization: Bearer my-secret-token" \
  -H "Content-Type: application/json" \
  -d '{"space": "users", "limit": 10, "after": null}'
```

**Response (`200 OK`):**
```json
{
  "ok": true,
  "documents": [
    {
      "space": "users",
      "id": "alice",
      "value": {"name": "Alice", "role": "admin"},
      "version": "1"
    }
  ],
  "count": 1
}
```

#### Dynamic Secondary Indexes (`/v1/json/indexes/*`, `/v1/json/find`)

##### `POST /v1/json/indexes/create`
```bash
curl -s -X POST http://127.0.0.1:4444/v1/json/indexes/create \
  -H "Authorization: Bearer my-secret-token" \
  -H "Content-Type: application/json" \
  -d '{"space": "users", "field_path": "role", "name": "idx_user_role"}'
```

##### `POST /v1/json/indexes/list`
```bash
curl -s -X POST http://127.0.0.1:4444/v1/json/indexes/list \
  -H "Authorization: Bearer my-secret-token" \
  -H "Content-Type: application/json" \
  -d '{"space": "users"}'
```

##### `POST /v1/json/indexes/drop`
```bash
curl -s -X POST http://127.0.0.1:4444/v1/json/indexes/drop \
  -H "Authorization: Bearer my-secret-token" \
  -H "Content-Type: application/json" \
  -d '{"space": "users", "name": "idx_user_role"}'
```

##### `POST /v1/json/find`
Query documents matching criteria with automatic secondary index acceleration:
```bash
curl -s -X POST http://127.0.0.1:4444/v1/json/find \
  -H "Authorization: Bearer my-secret-token" \
  -H "Content-Type: application/json" \
  -d '{"space": "users", "query": {"role": "admin"}, "limit": 10}'
```

---

### Embedded Graph Endpoints

LiliaDB's embedded graph engine represents directed entities and relationships natively.

#### `POST /v1/graph/node`
Create, update, or delete a graph node with structured metadata.

**Request:**
```bash
curl -s -X POST http://127.0.0.1:4444/v1/graph/node \
  -H "Authorization: Bearer my-secret-token" \
  -H "Content-Type: application/json" \
  -d '{
    "graph": "social",
    "id": "u1",
    "kind": "User",
    "metadata": {"name": "Alice", "role": "admin"}
  }'
```

**Response (`200 OK`):**
```json
{
  "ok": true
}
```

#### `POST /v1/graph/edge`
Create or remove a directed edge between two nodes.

**Request:**
```bash
curl -s -X POST http://127.0.0.1:4444/v1/graph/edge \
  -H "Authorization: Bearer my-secret-token" \
  -H "Content-Type: application/json" \
  -d '{
    "graph": "social",
    "source": "u1",
    "target": "u2",
    "kind": "FOLLOWS"
  }'
```

**Response (`200 OK`):**
```json
{
  "ok": true
}
```

#### `POST /v1/graph/neighbors`
Find neighboring nodes connected to a node (`direction: "outgoing" | "incoming" | "both"`).

**Request:**
```bash
curl -s -X POST http://127.0.0.1:4444/v1/graph/neighbors \
  -H "Authorization: Bearer my-secret-token" \
  -H "Content-Type: application/json" \
  -d '{
    "graph": "social",
    "node": "u1",
    "direction": "outgoing",
    "kind": "FOLLOWS"
  }'
```

**Response (`200 OK`):**
```json
{
  "ok": true,
  "neighbors": ["u2"]
}
```

#### `POST /v1/graph/reachability`
Compute transitive reachability from a set of starting nodes.

**Request:**
```bash
curl -s -X POST http://127.0.0.1:4444/v1/graph/reachability \
  -H "Authorization: Bearer my-secret-token" \
  -H "Content-Type: application/json" \
  -d '{
    "graph": "social",
    "start_nodes": ["u1"],
    "direction": "outgoing"
  }'
```

**Response (`200 OK`):**
```json
{
  "ok": true,
  "reachable": ["u1", "u2", "u3"]
}
```

---

### Full-Text Search Endpoints (FTS5)

High-performance text search backed by SQLite's FTS5 extension with BM25 ranking.

#### `POST /v1/fts/index`
Index or update text content for a document in a search collection.

**Request:**
```bash
curl -s -X POST http://127.0.0.1:4444/v1/fts/index \
  -H "Authorization: Bearer my-secret-token" \
  -H "Content-Type: application/json" \
  -d '{
    "collection": "articles",
    "doc_id": "art_1",
    "content": "LiliaDB delivers local-first multimodel embedded database storage."
  }'
```

**Response (`200 OK`):**
```json
{
  "ok": true
}
```

#### `POST /v1/fts/search`
Search indexed documents using full-text queries, returning snippets and relevance rankings.

**Request:**
```bash
curl -s -X POST http://127.0.0.1:4444/v1/fts/search \
  -H "Authorization: Bearer my-secret-token" \
  -H "Content-Type: application/json" \
  -d '{
    "collection": "articles",
    "query": "multimodel embedded",
    "limit": 5
  }'
```

**Response (`200 OK`):**
```json
{
  "ok": true,
  "hits": [
    {
      "collection": "articles",
      "doc_id": "art_1",
      "snippet": "LiliaDB delivers local-first <b>multimodel</b> <b>embedded</b> database storage.",
      "rank": -0.875
    }
  ]
}
```

---

### Real-Time Change Data Capture (SSE Stream)

#### `GET /v1/watch`
Subscribe to live change events in real time via Server-Sent Events (SSE).

**Query Parameters:**
- `namespace`: Optional filter string to receive events for a specific KV namespace, JSON space, or collection.
- `token`: Optional Bearer token parameter if the HTTP `Authorization` header cannot be set (e.g. standard browser `EventSource`).

**Request:**
```bash
curl -N -s http://127.0.0.1:4444/v1/watch?namespace=users \
  -H "Authorization: Bearer my-secret-token" \
  -H "Accept: text/event-stream"
```

**Stream Output:**
```
event: mutation
data: {"type":"json_put","space":"users","id":"alice","version":"1","deleted":false}

event: mutation
data: {"type":"json_delete","space":"users","id":"bob","version":"2","deleted":true}
```

---

### Batch & Transactions

#### `POST /v1/batch`
Execute multiple Key/Value, JSON, Graph, and FTS mutations atomically in a single ACID commit.

**Supported Operations:**
- `kv_set` (`namespace`, `key`, `value`, `if_version`, `expires_at_ms`)
- `kv_delete` (`namespace`, `key`, `if_version`)
- `json_put` (`space`, `id`, `value`, `if_version`)
- `json_delete` (`space`, `id`, `if_version`)
- `graph_set_node` (`graph`, `id`, `kind`, `metadata`)
- `graph_delete_node` (`graph`, `id`)
- `graph_add_edge` (`graph`, `source`, `target`, `kind`)
- `graph_remove_edge` (`graph`, `source`, `target`, `kind`)
- `fts_index` (`collection`, `doc_id`, `content`)
- `fts_delete` (`collection`, `doc_id`)

**Request:**
```bash
curl -s -X POST http://127.0.0.1:4444/v1/batch \
  -H "Authorization: Bearer my-secret-token" \
  -H "Content-Type: application/json" \
  -d '{
    "operations": [
      {
        "type": "kv_set",
        "namespace": "audit",
        "key": "last_action",
        "value": "batch_update"
      },
      {
        "type": "json_put",
        "space": "users",
        "id": "charlie",
        "value": {"name": "Charlie", "role": "engineer"},
        "if_version": 0
      },
      {
        "type": "graph_set_node",
        "graph": "social",
        "id": "charlie",
        "kind": "User"
      }
    ]
  }'
```

**Response (`200 OK`):**
```json
{
  "results": [
    {"version": "1", "deleted": false},
    {"version": "1", "deleted": false},
    {"version": null, "deleted": false}
  ]
}
```

---

### SQL Analytics

#### `POST /v1/sql`
Execute SQLite queries across structured multi-model views (`lilia_json`, `lilia_kv`, `lilia_graph_nodes`, `lilia_graph_edges`).

**Request:**
```bash
curl -s -X POST http://127.0.0.1:4444/v1/sql \
  -H "Authorization: Bearer my-secret-token" \
  -H "Content-Type: application/json" \
  -d '{
    "query": "SELECT id, json_extract(value, '\''$.role'\'') AS role FROM lilia_json WHERE space = ?1",
    "params": ["users"]
  }'
```

**Response (`200 OK`):**
```json
{
  "columns": ["id", "role"],
  "rows": [
    ["alice", "admin"],
    ["charlie", "engineer"]
  ],
  "rows_affected": 0
}
```

---

### Maintenance & Diagnostics

#### `GET /v1/doctor`
Run SQLite database integrity verification.

**Request:**
```bash
curl -s -X GET http://127.0.0.1:4444/v1/doctor \
  -H "Authorization: Bearer my-secret-token"
```

**Response (`200 OK`):**
```json
{
  "ok": true,
  "integrity": true
}
```

#### `POST /v1/backup`
Trigger an online snapshot backup to destination path.

**Request:**
```bash
curl -s -X POST http://127.0.0.1:4444/v1/backup \
  -H "Authorization: Bearer my-secret-token" \
  -H "Content-Type: application/json" \
  -d '{"destination": "/tmp/lilia_snapshot.lilia"}'
```

**Response (`200 OK`):**
```json
{
  "ok": true
}
```

#### `POST /v1/shutdown`
Gracefully terminate the background daemon process.

**Request:**
```bash
curl -s -X POST http://127.0.0.1:4444/v1/shutdown \
  -H "Authorization: Bearer my-secret-token"
```

**Response (`200 OK`):**
```json
{
  "ok": true,
  "message": "daemon shutting down"
}
```

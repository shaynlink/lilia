# LiliaDB HTTP REST Gateway Specification

`lilia-daemon` includes a high-performance local loopback HTTP/REST gateway. It provides language-agnostic access to LiliaDB's Key/Value store, JSON document engine, atomic batch transactions, and SQL analytical views.

---

## 1. Security Architecture & Network Model

The HTTP gateway is engineered strictly for **local IPC replacement** (e.g., Python scripts, AI agent tools, sidecars, and local containers):

1. **Loopback Only**: The server refuses to bind to non-loopback interfaces. Attempting to pass `--http-bind 0.0.0.0` or a public IP exits immediately with an error. Valid bindings are `127.0.0.1` (IPv4 loopback) or `::1` (IPv6 loopback).
2. **Bearer Token Authentication**: Every `/v1/*` request requires a valid `Authorization: Bearer <TOKEN>` header.
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
  --http-bind 127.0.0.1 \
  --auth-token my-secret-token \
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
  "code": "CONFLICT",
  "message": "version mismatch",
  "details": {
    "expected": "1",
    "actual": "2"
  }
}
```

### Status Code Mapping

| HTTP Status | Error Code | Description |
|---|---|---|
| **200 OK** | - | Operation succeeded. |
| **400 Bad Request** | `INVALID_INPUT` | Malformed JSON request body or invalid parameters. |
| **401 Unauthorized** | `UNAUTHORIZED` | Missing or invalid `Authorization: Bearer <TOKEN>` header. |
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
  "status": "running"
}
```

---

### Key / Value Endpoints

#### `POST /v1/kv/get`
Retrieve a Key/Value entry.

**Request:**
```bash
curl -s -X POST http://127.0.0.1:4444/v1/kv/get \
  -H "Authorization: Bearer my-secret-token" \
  -H "Content-Type: application/json" \
  -d '{"space": "config", "key": "theme"}'
```

**Response (`200 OK` if found, `404 Not Found` if missing):**
```json
{
  "space": "config",
  "key": "theme",
  "value": "dark",
  "version": "1",
  "expires_at_ms": null
}
```

#### `POST /v1/kv/set`
Set a Key/Value entry with optional optimistic concurrency (`if_version`) and TTL (`expires_at_ms`).

**Request:**
```bash
curl -s -X POST http://127.0.0.1:4444/v1/kv/set \
  -H "Authorization: Bearer my-secret-token" \
  -H "Content-Type: application/json" \
  -d '{
    "space": "config",
    "key": "theme",
    "value": "dark",
    "if_version": "0",
    "expires_at_ms": null
  }'
```

**Response (`200 OK`):**
```json
{
  "version": "1"
}
```

#### `POST /v1/kv/delete`
Delete a Key/Value entry with optional conditional version matching.

**Request:**
```bash
curl -s -X POST http://127.0.0.1:4444/v1/kv/delete \
  -H "Authorization: Bearer my-secret-token" \
  -H "Content-Type: application/json" \
  -d '{"space": "config", "key": "theme", "if_version": "1"}'
```

**Response (`200 OK`):**
```json
{
  "deleted": true
}
```

#### `POST /v1/kv/scan`
Scan Key/Value entries within a space using keyset pagination.

**Request:**
```bash
curl -s -X POST http://127.0.0.1:4444/v1/kv/scan \
  -H "Authorization: Bearer my-secret-token" \
  -H "Content-Type: application/json" \
  -d '{"space": "config", "limit": 10, "after": null}'
```

**Response (`200 OK`):**
```json
[
  {
    "space": "config",
    "key": "app_name",
    "value": "LiliaDB",
    "version": "1",
    "expires_at_ms": null
  }
]
```

---

### JSON Document Endpoints

#### `GET /v1/json/:space/:id`
Fetch a single JSON document by path parameters.

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
    "value": {
      "name": "Alice",
      "email": "alice@example.com",
      "roles": ["admin"]
    },
    "if_version": null
  }'
```

**Response (`200 OK`):**
```json
{
  "version": "1"
}
```

#### `POST /v1/json/delete`
Delete a JSON document.

**Request:**
```bash
curl -s -X POST http://127.0.0.1:4444/v1/json/delete \
  -H "Authorization: Bearer my-secret-token" \
  -H "Content-Type: application/json" \
  -d '{"space": "users", "id": "alice", "if_version": "1"}'
```

**Response (`200 OK`):**
```json
{
  "deleted": true
}
```

#### `POST /v1/json/scan`
Scan JSON documents in a space with cursor pagination.

**Request:**
```bash
curl -s -X POST http://127.0.0.1:4444/v1/json/scan \
  -H "Authorization: Bearer my-secret-token" \
  -H "Content-Type: application/json" \
  -d '{"space": "users", "limit": 2, "after": null}'
```

**Response (`200 OK`):**
```json
[
  {
    "space": "users",
    "id": "alice",
    "value": {"name": "Alice"},
    "version": "1"
  },
  {
    "space": "users",
    "id": "bob",
    "value": {"name": "Bob"},
    "version": "1"
  }
]
```

#### `POST /v1/json/indexes/create`
Create a secondary index on a JSON field or path in a space. Backfills existing documents and maintains the index synchronously on future mutations.

**Request:**
```bash
curl -s -X POST http://127.0.0.1:4444/v1/json/indexes/create \
  -H "Authorization: Bearer my-secret-token" \
  -H "Content-Type: application/json" \
  -d '{
    "space": "users",
    "name": "idx_users_email",
    "path": "email"
  }'
```

**Response (`200 OK`):**
```json
{
  "ok": true
}
```

#### `POST /v1/json/indexes/drop`
Drop a secondary index.

**Request:**
```bash
curl -s -X POST http://127.0.0.1:4444/v1/json/indexes/drop \
  -H "Authorization: Bearer my-secret-token" \
  -H "Content-Type: application/json" \
  -d '{
    "space": "users",
    "name": "idx_users_email"
  }'
```

**Response (`200 OK`):**
```json
{
  "ok": true,
  "deleted": true
}
```

#### `POST /v1/json/indexes/list`
List all secondary indexes configured on a space.

**Request:**
```bash
curl -s -X POST http://127.0.0.1:4444/v1/json/indexes/list \
  -H "Authorization: Bearer my-secret-token" \
  -H "Content-Type: application/json" \
  -d '{"space": "users"}'
```

**Response (`200 OK`):**
```json
{
  "ok": true,
  "indexes": [
    {
      "space": "users",
      "index_name": "idx_users_email",
      "json_path": "$.email"
    }
  ]
}
```

#### `POST /v1/json/scan_by_index`
Scan JSON documents matching a specific index value with keyset pagination.

**Request:**
```bash
curl -s -X POST http://127.0.0.1:4444/v1/json/scan_by_index \
  -H "Authorization: Bearer my-secret-token" \
  -H "Content-Type: application/json" \
  -d '{
    "space": "users",
    "index_name": "idx_users_role",
    "value": "admin",
    "limit": 10,
    "after": null
  }'
```

**Response (`200 OK`):**
```json
{
  "ok": true,
  "documents": [
    {
      "space": "users",
      "id": "alice",
      "value": {
        "name": "Alice",
        "role": "admin"
      },
      "version": "1"
    }
  ],
  "count": 1
}
```

#### `POST /v1/json/find`
Find documents matching a query filter (e.g. `{"role": "admin"}`). If a secondary index exists matching one of the query fields, LiliaDB automatically accelerates the query using index lookups with cursor pagination and in-memory residual filtering; otherwise, it performs a document scan with filter matching.

**Request:**
```bash
curl -s -X POST http://127.0.0.1:4444/v1/json/find \
  -H "Authorization: Bearer my-secret-token" \
  -H "Content-Type: application/json" \
  -d '{
    "space": "users",
    "query": {
      "role": "admin"
    },
    "limit": 10,
    "after": null
  }'
```

**Response (`200 OK`):**
```json
{
  "ok": true,
  "documents": [
    {
      "space": "users",
      "id": "alice",
      "value": {
        "name": "Alice",
        "role": "admin"
      },
      "version": "1"
    }
  ],
  "count": 1
}
```

---

### Batch & Transactions

#### `POST /v1/batch`
Execute multiple Key/Value and JSON mutations in an atomic transaction.

**Request:**
```bash
curl -s -X POST http://127.0.0.1:4444/v1/batch \
  -H "Authorization: Bearer my-secret-token" \
  -H "Content-Type: application/json" \
  -d '{
    "operations": [
      {
        "type": "kv_set",
        "space": "audit",
        "key": "last_action",
        "value": "batch_update",
        "if_version": null,
        "expires_at_ms": null
      },
      {
        "type": "json_put",
        "space": "users",
        "id": "charlie",
        "value": {"name": "Charlie"},
        "if_version": 0
      }
    ]
  }'
```

**Response (`200 OK`):**
```json
{
  "results": [
    {"version": "1", "deleted": false},
    {"version": "1", "deleted": false}
  ]
}
```

---

### SQL Analytics

#### `POST /v1/sql`
Execute read queries or data transformations using SQLite SQL.

**Request:**
```bash
curl -s -X POST http://127.0.0.1:4444/v1/sql \
  -H "Authorization: Bearer my-secret-token" \
  -H "Content-Type: application/json" \
  -d '{"query": "SELECT id, json_extract(value, '\''$.email'\'') AS email FROM lilia_json WHERE space = '\''users'\''"}'
```

**Response (`200 OK`):**
```json
{
  "columns": ["id", "email"],
  "rows": [
    ["alice", "alice@example.com"],
    ["bob", "bob@example.com"]
  ]
}
```

---

### Maintenance & Diagnostics

#### `GET /v1/doctor`
Run database integrity checks.

**Request:**
```bash
curl -s -X GET http://127.0.0.1:4444/v1/doctor \
  -H "Authorization: Bearer my-secret-token"
```

**Response (`200 OK`):**
```json
{
  "ok": true
}
```

#### `POST /v1/backup`
Generate an online atomic backup snapshot of the database.

**Request:**
```bash
curl -s -X POST http://127.0.0.1:4444/v1/backup \
  -H "Authorization: Bearer my-secret-token" \
  -H "Content-Type: application/json" \
  -d '{"target_path": "/tmp/lilia_backup.lilia"}'
```

**Response (`200 OK`):**
```json
{
  "backup_path": "/tmp/lilia_backup.lilia"
}
```

#### `POST /v1/shutdown`
Gracefully terminate the daemon process.

**Request:**
```bash
curl -s -X POST http://127.0.0.1:4444/v1/shutdown \
  -H "Authorization: Bearer my-secret-token"
```

**Response (`200 OK`):**
```json
{
  "ok": true
}
```

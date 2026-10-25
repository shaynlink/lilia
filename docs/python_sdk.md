# LiliaDB Python SDK (`liliadb`)

Official zero-dependency Python SDK for LiliaDB, built for high performance, AI agent memory pipelines, sidecar tooling, and lightweight automation.

---

## 1. Key Features

- **Zero Dependencies**: Pure Python 3.9+ using standard library modules (`urllib.request`, `json`). No heavy external dependencies like `requests`, `aiohttp`, or `httpx` required.
- **Multi-Model**: Key/Value store with TTL, JSON document store, and SQL queries over documents in a single unified client.
- **Optimistic Concurrency Control (OCC)**: Enforce version checks (`if_version`) to eliminate race conditions without distributed locks.
- **Transparent Keyset Streaming**: Auto-paginating Python generators for KV and JSON scans (`for doc in client.json.scan(...)`).
- **Dict-Ready SQL Analytics**: Query structured views (`lilia_json`) and project rows directly into Python dictionaries with `result.to_dicts()`.
- **Atomic Multi-Model Transactions**: Apply KV mutations and JSON documents atomically in a single ACID batch.

---

## 2. Installation

Install from the local repository:

```bash
pip install -e packages/python
```

Or add `packages/python` directly to your `PYTHONPATH`:

```bash
export PYTHONPATH="/path/to/liliadb/packages/python:$PYTHONPATH"
```

---

## 3. Quick Start

Ensure `lilia-daemon` is running with HTTP enabled:

```bash
lilia-daemon \
  --database ./app.lilia \
  --http-port 4444 \
  --auth-token secret-token \
  --development
```

Connect and query with Python:

```python
from liliadb import LiliaClient

client = LiliaClient(endpoint="http://127.0.0.1:4444", token="secret-token")

# Check connectivity
print("Status:", client.health())

# Store and retrieve a JSON document
client.json.put("users", "usr_1", {"name": "Alice", "role": "admin"})
user = client.json.get("users", "usr_1")
print(f"Retrieved: {user.value['name']} (version {user.version})")

# Run a SQL query
result = client.sql("SELECT id, json_extract(value, '$.role') AS role FROM lilia_json WHERE space = 'users'")
for row in result.to_dicts():
    print(f"User {row['id']} has role {row['role']}")
```

---

## 4. API Reference

### Client Initialization

```python
from liliadb import LiliaClient

client = LiliaClient(
    endpoint="http://127.0.0.1:4444",
    token="your-bearer-token",
    timeout=10.0,  # Request timeout in seconds (default: 10.0)
)
```

---

### Key / Value Operations (`client.kv`)

#### `client.kv.set(space, key, value, if_version=None, expires_at_ms=None) -> str`
Inserts or updates a string value. Returns the new version as a string.

```python
# Unconditional write
v1 = client.kv.set("cache", "session_abc", "user_active")

# Create-only (fails if key already exists)
v_new = client.kv.set("cache", "unique_key", "data", if_version=0)

# Optimistic concurrency update
v2 = client.kv.set("cache", "session_abc", "user_idle", if_version=int(v1))

# Write with TTL (timestamp in milliseconds)
import time
ttl_time = int((time.time() + 300) * 1000)  # expires in 5 minutes
client.kv.set("cache", "temp_token", "xyz", expires_at_ms=ttl_time)
```

#### `client.kv.get(space, key) -> Optional[KvEntry]`
Fetches a Key/Value entry. Returns `None` if the key does not exist.

```python
entry = client.kv.get("cache", "session_abc")
if entry:
    print(entry.key, entry.value, entry.version, entry.expires_at_ms)
```

#### `client.kv.delete(space, key, if_version=None) -> bool`
Deletes a Key/Value entry. Returns `True` if deleted, `False` if not found.

```python
deleted = client.kv.delete("cache", "session_abc", if_version=2)
```

#### `client.kv.scan(space, limit=100, after=None) -> Generator[KvEntry, None, None]`
Returns a Python generator that lazily streams and auto-paginates across records.

```python
for entry in client.kv.scan("cache", limit=50):
    print(entry.key, "->", entry.value)
```

---

### JSON Document Operations (`client.json`)

#### `client.json.put(space, id, value, if_version=None) -> str`
Stores a dictionary/JSON-serializable object. Returns the new version string.

```python
doc = {
    "title": "Ergonomic Keyboard",
    "price": 99.90,
    "tags": ["hardware", "ergonomics"]
}
version = client.json.put("inventory", "item_42", doc)
```

#### `client.json.get(space, id) -> Optional[JsonDocument]`
Fetches a JSON document by ID. Returns `None` if not found.

```python
doc = client.json.get("inventory", "item_42")
if doc:
    print(doc.id, doc.value["title"], doc.version)
```

#### `client.json.delete(space, id, if_version=None) -> bool`
Deletes a document. Returns `True` if deleted, `False` if not found.

```python
client.json.delete("inventory", "item_42")
```

#### `client.json.scan(space, limit=100, after=None) -> Generator[JsonDocument, None, None]`
Auto-paginating generator streaming JSON documents using keyset pagination:

```python
for item in client.json.scan("inventory", limit=20):
    print(f"[{item.id}] {item.value['title']}")
```

---

### Multi-Model Batch Transactions (`client.batch`)

Executes multiple Key/Value and JSON mutations in an atomic transaction:

```python
results = client.batch([
    {
        "type": "kv_set",
        "space": "stats",
        "key": "total_items",
        "value": "100"
    },
    {
        "type": "json_put",
        "space": "inventory",
        "id": "item_101",
        "value": {"title": "Mouse", "price": 29.90}
    }
])

for res in results:
    print("Version:", res.version, "Deleted:", res.deleted)
```

---

### SQL Analytics (`client.sql`)

Executes SQLite SQL queries against LiliaDB's structured views (`lilia_json`, `lilia_kv`, etc.):

```python
query = """
    SELECT
        json_extract(value, '$.tags[0]') AS category,
        COUNT(*) AS count,
        AVG(CAST(json_extract(value, '$.price') AS REAL)) AS avg_price
    FROM lilia_json
    WHERE space = 'inventory'
    GROUP BY category
    ORDER BY avg_price DESC
"""

result = client.sql(query)

print("Columns:", result.columns)
print("Row count:", len(result))

# Iterate over rows as dictionaries
for row in result.to_dicts():
    print(f"Category: {row['category']} | Count: {row['count']} | Avg Price: {row['avg_price']:.2f}")

# Or iterate over raw tuples
for row in result:
    print(row)
```

---

### Maintenance & Diagnostics

```python
# Health check (unauthenticated route)
health = client.health()  # {'ok': True, 'status': 'running'}

# Database integrity verification
is_healthy = client.doctor()  # True

# Online atomic backup snapshot
backup_info = client.backup("/path/to/backup.lilia")

# Graceful daemon shutdown
client.shutdown()
```

---

## 5. Error Handling

The SDK raises typed exceptions inheriting from `LiliaError`:

```python
from liliadb import (
    AuthenticationError,
    ConflictError,
    InvalidInputError,
    LiliaError,
    NotFoundError,
    StorageError,
    TimeoutError,
)

try:
    client.kv.set("users", "alice", "data", if_version=1)
except ConflictError as exc:
    print(f"Concurrency conflict: {exc.message}")
    print(f"Expected version: {exc.details.get('expected')}, actual: {exc.details.get('actual')}")
except AuthenticationError:
    print("Invalid Bearer token")
except NotFoundError:
    print("Resource not found")
except LiliaError as exc:
    print(f"LiliaDB error [{exc.code}]: {exc.message}")
```

---

## 6. Runnable Examples

See the [`examples/python/`](file:///Users/shayn/Development/liliadb/examples/python) directory for complete scripts:
- `01_kv_basic.py`: KV storage and optimistic concurrency control.
- `02_json_documents.py`: JSON document store and keyset pagination.
- `03_sql_analytics.py`: SQL analytics and aggregations over JSON collections.
- `04_ai_agent_memory.py`: Conversational memory and session persistence for AI agents.

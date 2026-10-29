# LiliaDB Python SDK (`liliadb`)

Official zero-dependency Python SDK for LiliaDB, built for high performance, AI agent memory pipelines, sidecar tooling, and lightweight automation.

---

## 1. Key Features

- **Zero Dependencies**: Pure Python 3.9+ using standard library modules (`urllib.request`, `json`, `threading`). No external dependencies like `requests`, `aiohttp`, or `httpx` required.
- **Multimodel Engine**: Key/Value store with TTL, JSON document store with secondary indexing, embedded Graph traversal, Full-Text Search (FTS5), and SQL analytics in a single unified client.
- **Real-Time CDC**: Live change subscriptions via `client.watch()` with background thread callbacks or synchronous Python generators.
- **Optimistic Concurrency Control (OCC)**: Enforce version checks (`if_version`) to eliminate race conditions without distributed locks.
- **Transparent Keyset Streaming**: Auto-paginating Python generators for KV and JSON scans (`for doc in client.json.scan(...)`).
- **Dict-Ready SQL Analytics**: Query structured views (`lilia_json`, `lilia_kv`) and project rows directly into Python dictionaries with `result.to_dicts()`.
- **Atomic Multi-Model Transactions**: Apply KV mutations, JSON documents, graph edges, and search indexes atomically in a single ACID batch.

---

## 2. Installation

Install via PyPI:

```bash
pip install liliadb
```

Or install from the local repository for development:

```bash
pip install -e packages/python
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
    token_file="/path/to/token",  # Optional: read token from file
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

#### Dynamic Secondary Indexing (`client.json.create_index`, `find`, etc.)
Accelerate queries over JSON fields and nested paths with automatic index backfilling and multikey array support:

```python
# Create index on field or path
client.json.create_index("inventory", "tags")
client.json.create_index("inventory", "price", name="idx_inv_price")

# List existing indexes
indexes = client.json.list_indexes("inventory")
for idx in indexes:
    print(idx.name, idx.field_path)

# Query documents (accelerated by secondary index when available, fallback to scan)
matching = client.json.find("inventory", {"tags": "hardware"}, limit=10)
for doc in matching:
    print(doc.id, doc.value)

# Auto-paginating generator over query matches
for doc in client.json.find("inventory", {"tags": "hardware"}, auto_paginate=True):
    print(doc.id, doc.value)

# Drop an index
client.json.drop_index("inventory", "idx_inv_price")
```

---

### Embedded Graph Engine (`client.graph`)

LiliaDB provides a native directed graph engine embedded alongside your KV and JSON collections.

#### Nodes
```python
# Create or update nodes with labels and structured metadata
client.graph.set_node("org_chart", "u1", "Employee", {"name": "Alice", "title": "Staff Engineer"})
client.graph.set_node("org_chart", "u2", "Employee", {"name": "Bob", "title": "VP Engineering"})

# Delete a node and its incident edges
client.graph.delete_node("org_chart", "u1")
```

#### Edges
```python
# Create directed edge
client.graph.add_edge("org_chart", "u1", "u2", "REPORTS_TO")

# Remove directed edge
client.graph.remove_edge("org_chart", "u1", "u2", "REPORTS_TO")
```

#### Traversal & Reachability
```python
# Find immediate neighbors (direction: "outgoing" | "incoming" | "both")
managers = client.graph.neighbors("org_chart", "u1", direction="outgoing", kind="REPORTS_TO")

# Transitive reachability traversal
reachable = client.graph.reachability("org_chart", start_nodes=["u1"], direction="outgoing")
```

---

### Full-Text Search (`client.fts`)

Integrated SQLite FTS5 engine with BM25 ranking, highlighting, and snippet generation.

```python
# Index documents into a search collection
client.fts.index("kb", "doc_101", "LiliaDB delivers local-first multimodel embedded database storage.")
client.fts.index("kb", "doc_102", "SQLite WAL mode enables high concurrency and ACID guarantees.")

# Search collection
hits = client.fts.search("kb", "multimodel SQLite", limit=5)
for hit in hits:
    print(f"Doc: {hit.doc_id} | Score: {hit.rank:.3f} | Snippet: {hit.snippet}")

# Remove document from index
client.fts.delete("kb", "doc_101")
```

---

### Real-Time CDC & Live Event Subscriptions (`client.watch`)

Subscribe to live change events in real time with namespace filtering via Server-Sent Events (SSE).

#### Asynchronous Callback (Threaded)
```python
def on_event(event):
    print(f"CDC Event: {event.type} in {event.namespace or event.space} | ID: {event.id or event.key}")

# Use as a context manager for automatic cleanup
with client.watch(namespace="orders", callback=on_event) as sub:
    client.json.put("orders", "ord_100", {"customer": "Alice", "status": "paid"})
    # sub.stop() or sub.unsubscribe() called on exit
```

#### Synchronous Generator
```python
# Iterate over live mutations as a stream
for event in client.watch(namespace="orders"):
    print("Received event:", event.type, event.id, event.deleted)
    break
```

---

### Multi-Model Batch Transactions (`client.batch`)

Executes multiple Key/Value, JSON, Graph, and FTS operations atomically in a single ACID commit:

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
    },
    {
        "type": "graph_set_node",
        "graph": "inventory_graph",
        "id": "item_101",
        "kind": "Item",
        "metadata": {"price": 29.90}
    },
    {
        "type": "fts_index",
        "collection": "inventory_kb",
        "doc_id": "item_101",
        "content": "Optical USB-C Gaming Mouse"
    }
])

for res in results:
    print("Version:", res.version, "Deleted:", res.deleted)
```

---

### SQL Analytics (`client.sql`)

Executes SQLite SQL queries against LiliaDB's structured views (`lilia_json`, `lilia_kv`, `lilia_graph_nodes`, `lilia_graph_edges`):

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
backup_ok = client.backup("/path/to/backup.lilia")

# Graceful daemon shutdown
# (requires authentication)
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
- `05_cdc_live_events.py`: Real-time CDC change stream subscriptions and reactive cache sync.

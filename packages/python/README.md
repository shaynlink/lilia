# liliadb

Official Python SDK for **LiliaDB**, the local-first multimodel embedded & daemon database.

Zero external dependencies: uses Python's standard library (`urllib` and `socket`). Works on Python 3.9+ across macOS, Linux, and Windows.

## Quick Start

```python
from liliadb import LiliaClient

# Connect to local LiliaDB daemon HTTP gateway
client = LiliaClient(endpoint="http://127.0.0.1:4444", token="your-token")

# --- Key-Value ---
client.kv.set("settings", "theme", "dark")
entry = client.kv.get("settings", "theme")
print(entry.value)  # "dark"

# --- JSON Documents & Dynamic Secondary Indexing ---
client.json.put("users", "alice", {"name": "Alice", "role": "admin", "tags": ["lead", "core"]})
client.json.put("users", "bob", {"name": "Bob", "role": "engineer", "tags": ["core"]})

# Create secondary index on JSON field (auto-indexes multikey array elements as well)
client.json.create_index("users", "role")

# Query documents (accelerated by secondary index when available)
admins = client.json.find("users", {"role": "admin"})
for admin in admins:
    print(admin.id, admin.value)

# Auto-paginated generator over query results
for doc in client.json.find("users", {"tags": "core"}, auto_paginate=True):
    print(doc.id, doc.value)

# --- Graph Traversal ---
client.graph.set_node("social", "alice", "User", {"name": "Alice"})
client.graph.set_node("social", "bob", "User", {"name": "Bob"})
client.graph.add_edge("social", "alice", "bob", "FOLLOWS")
friends = client.graph.neighbors("social", "alice", direction="outgoing")
print("Friends:", friends)

# --- Full-Text Search (FTS5) ---
client.fts.index("kb", "doc1", "LiliaDB delivers unified SQLite multi-model storage.")
hits = client.fts.search("kb", "multi-model", limit=5)
for hit in hits:
    print(f"Hit {hit.doc_id} (rank {hit.rank}): {hit.snippet}")

# --- Real-Time CDC Watch ---
with client.watch(namespace="users", callback=lambda e: print("Event:", e.type, e.id)) as sub:
    client.json.put("users", "charlie", {"name": "Charlie"})

# --- Declarative SQL ---
result = client.sql("SELECT id, json_extract(value, '$.name') AS name FROM lilia_json WHERE space = 'users'")
for row in result.to_dicts():
    print(row["id"], row["name"])
```

## Features

- **Multi-model**: Key-Value, JSON Documents, Graph, Full-Text Search (FTS5), and SQL queries through the same database.
- **Secondary Indexing**: Dynamic indexes on JSON document fields with automatic acceleration and multikey array support.
- **Real-Time CDC**: Live change subscriptions via `client.watch()` with background thread callbacks or Python generators.
- **Zero dependencies**: No C extensions, no heavy third-party packages (uses standard library `urllib` and `json`).
- **Auto-pagination**: `client.kv.scan()`, `client.json.scan()`, and `client.json.find()` yield entries across pages automatically.
- **Optimistic Concurrency**: Safe versioned mutations with `if_version`.
- **Atomic Batches**: Execute multi-operation KV, JSON, Graph, and FTS transactions atomically.
- **Loopback Security**: Protected by Bearer tokens and constant-time validation.

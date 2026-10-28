# liliadb

Official Python SDK for **LiliaDB**, the local-first multimodel embedded & daemon database.

Zero external dependencies: uses Python's standard library (`urllib` and `socket`). Works on Python 3.9+ across macOS, Linux, and Windows.

## Quick Start

```python
from liliadb import LiliaClient

# Connect to local LiliaDB daemon HTTP gateway
client = LiliaClient(endpoint="http://127.0.0.1:4000", token="your-token")

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

# List & drop indexes
indexes = client.json.list_indexes("users")
client.json.drop_index("users", "idx_users_role")

# --- Declarative SQL ---
result = client.sql("SELECT id, json_extract(value, '$.name') AS name FROM lilia_json WHERE space = 'users'")
for row in result.to_dicts():
    print(row["id"], row["name"])
```

## Features

- **Multi-model**: Key-Value, JSON Documents, and SQL queries through the same database.
- **Secondary Indexing**: Dynamic indexes on JSON document fields with automatic acceleration and multikey array support.
- **Zero dependencies**: No C extensions, no heavy third-party packages.
- **Auto-pagination**: `client.kv.scan()`, `client.json.scan()`, and `client.json.find()` yield entries across pages automatically.
- **Optimistic Concurrency**: Safe versioned mutations with `if_version`.
- **Atomic Batches**: Execute multi-operation KV and JSON transactions atomically.
- **Loopback Security**: Protected by Bearer tokens and constant-time validation.

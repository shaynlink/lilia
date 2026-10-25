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

# --- JSON Documents ---
client.json.put("users", "alice", {"name": "Alice", "role": "admin"})
doc = client.json.get("users", "alice")
print(doc.value["name"])  # "Alice"

# --- Declarative SQL ---
result = client.sql("SELECT id, json_extract(value, '$.name') AS name FROM lilia_json WHERE space = 'users'")
for row in result.to_dicts():
    print(row["id"], row["name"])
```

## Features

- **Multi-model**: Key-Value, JSON Documents, and SQL queries through the same database.
- **Zero dependencies**: No C extensions, no heavy third-party packages.
- **Auto-pagination**: `client.kv.scan()` and `client.json.scan()` yield entries across pages automatically.
- **Optimistic Concurrency**: Safe versioned mutations with `if_version`.
- **Atomic Batches**: Execute multi-operation KV and JSON transactions atomically.
- **Loopback Security**: Protected by Bearer tokens and constant-time validation.

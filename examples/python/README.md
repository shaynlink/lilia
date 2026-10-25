# LiliaDB Python SDK Examples

This directory contains runnable examples for the official `liliadb` Python SDK.

## Prerequisites

1. Start `lilia-daemon` with the HTTP gateway enabled:
   ```bash
   cargo run -p lilia-daemon -- \
     --database /tmp/lilia_example.lilia \
     --http-port 4444 \
     --auth-token dev-secret
   ```

2. Make sure `liliadb` is available on your `PYTHONPATH` (or install in development mode `pip install -e packages/python`):
   ```bash
   export PYTHONPATH=packages/python
   ```

## Runnable Examples

| Example | Description |
|---|---|
| [`01_kv_basic.py`](file:///Users/shayn/Development/liliadb/examples/python/01_kv_basic.py) | Key/Value storage, TTL expirations, and Optimistic Concurrency Control (`if_version`). |
| [`02_json_documents.py`](file:///Users/shayn/Development/liliadb/examples/python/02_json_documents.py) | JSON document store, auto-paginating keyset scans, and atomic multi-model batch transactions. |
| [`03_sql_analytics.py`](file:///Users/shayn/Development/liliadb/examples/python/03_sql_analytics.py) | Declarative SQL queries over JSON document collections with `to_dicts()` mappings. |
| [`04_ai_agent_memory.py`](file:///Users/shayn/Development/liliadb/examples/python/04_ai_agent_memory.py) | AI Agent conversational state, working memory, and long-term user facts. |

## Running the Examples

```bash
PYTHONPATH=packages/python python3 examples/python/01_kv_basic.py
PYTHONPATH=packages/python python3 examples/python/02_json_documents.py
PYTHONPATH=packages/python python3 examples/python/03_sql_analytics.py
PYTHONPATH=packages/python python3 examples/python/04_ai_agent_memory.py
```

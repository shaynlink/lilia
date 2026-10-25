"""01_kv_basic.py - Basic Key/Value operations with optimistic concurrency in LiliaDB.

Requirements:
- Run lilia-daemon with HTTP enabled:
    cargo run -p lilia-daemon -- --database /tmp/lilia_example.lilia --http-port 4444 --auth-token dev-secret
- Run this script:
    python3 examples/python/01_kv_basic.py
"""

import os
import sys

from liliadb import ConflictError, LiliaClient

ENDPOINT = os.environ.get("LILIA_ENDPOINT", "http://127.0.0.1:4444")
TOKEN = os.environ.get("LILIA_TOKEN", "dev-secret")


def main() -> None:
    client = LiliaClient(endpoint=ENDPOINT, token=TOKEN)

    # 1. Check connectivity
    try:
        health = client.health()
        print(f"Connected to LiliaDB: {health}")
    except Exception as exc:
        print(f"Error connecting to daemon at {ENDPOINT}: {exc}")
        print("\nPlease start the daemon with:")
        print("  cargo run -p lilia-daemon -- --database /tmp/lilia_example.lilia --http-port 4444 --auth-token dev-secret\n")
        sys.exit(1)

    space = "config"
    key = "feature_flags"

    # 2. Set key with value
    print("\n--- 1. Set key ---")
    v1 = client.kv.set(space, key, '{"dark_mode": true, "beta": false}')
    print(f"Set '{key}' -> version: {v1}")

    # 3. Get key
    print("\n--- 2. Get key ---")
    entry = client.kv.get(space, key)
    if entry:
        print(f"Retrieved '{entry.key}': {entry.value} (version: {entry.version})")

    # 4. Optimistic Concurrency Control (OCC)
    print("\n--- 3. Optimistic Concurrency Control ---")
    # Successful update with matching version
    v2 = client.kv.set(space, key, '{"dark_mode": true, "beta": true}', if_version=int(v1))
    print(f"Updated with if_version={v1} -> new version: {v2}")

    # Conflict when using old version
    try:
        client.kv.set(space, key, '{"dark_mode": false}', if_version=int(v1))
    except ConflictError:
        print(f"Expected conflict caught: cannot update with stale version {v1}")

    # 5. Key with TTL expiration
    print("\n--- 4. Temporary Cache Key (TTL) ---")
    import time
    expires_at = int((time.time() + 3600) * 1000)  # expires in 1 hour
    client.kv.set("cache", "session_123", "user_active", expires_at_ms=expires_at)
    cache_entry = client.kv.get("cache", "session_123")
    print(f"Cache key stored with expires_at_ms={cache_entry.expires_at_ms}")

    # 6. Delete key
    print("\n--- 5. Delete key ---")
    deleted = client.kv.delete(space, key, if_version=int(v2))
    print(f"Deleted '{key}': {deleted}")
    print(f"Verify deleted: {client.kv.get(space, key)}")


if __name__ == "__main__":
    main()

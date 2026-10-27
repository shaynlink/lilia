"""05_cdc_live_events.py - Change Data Capture (CDC) & Live Event Streaming in LiliaDB.

Demonstrates:
- Subscribing to real-time database change events (CDC) over HTTP SSE
- Unified namespace filtering across KV, JSON, Graph, and FTS models
- Using client.watch() for live cache invalidation & reactive AI agent state
- Graceful subscription lifecycle (context manager and .unsubscribe())

Requirements:
- Run lilia-daemon with HTTP enabled:
    cargo run -p lilia-daemon -- --database /tmp/lilia_example.lilia --http-port 4444 --auth-token dev-secret
- Run this script:
    python3 examples/python/05_cdc_live_events.py
"""

import os
import sys
import time

from liliadb import DatabaseEvent, LiliaClient

ENDPOINT = os.environ.get("LILIA_ENDPOINT", "http://127.0.0.1:4444")
TOKEN = os.environ.get("LILIA_TOKEN", "dev-secret")


def main() -> None:
    client = LiliaClient(endpoint=ENDPOINT, token=TOKEN)

    # 1. Connectivity check
    try:
        health = client.health()
        print(f"Connected to LiliaDB: {health}")
    except Exception as exc:
        print(f"Error connecting to daemon at {ENDPOINT}: {exc}")
        print("\nPlease start the daemon with:")
        print("  cargo run -p lilia-daemon -- --database /tmp/lilia_example.lilia --http-port 4444 --auth-token dev-secret\n")
        sys.exit(1)

    print("\n=== LiliaDB Real-Time CDC & Live Streaming (client.watch()) ===\n")

    # -------------------------------------------------------------------------
    # Scenario: Autonomous AI Agent Memory & Reactive Cache
    # The agent monitors the 'agent_memory' namespace to update its local context
    # -------------------------------------------------------------------------
    active_memory = {}

    def on_memory_event(event: DatabaseEvent) -> None:
        key_or_id = event.id or event.key or "unknown"
        print(f"  [CDC Live Event] Type: {event.type} | ID/Key: {key_or_id} | Deleted: {event.deleted}")
        if event.deleted:
            active_memory.pop(key_or_id, None)
        else:
            active_memory[key_or_id] = {
                "version": event.version,
                "type": event.type,
                "timestamp": time.time(),
            }

    print("[1] Subscribing to 'agent_memory' namespace changes...")
    subscription = client.watch("agent_memory", callback=on_memory_event)
    time.sleep(0.2)  # Allow SSE stream to establish

    # -------------------------------------------------------------------------
    # 2. Mutate watched namespace (events should be received)
    # -------------------------------------------------------------------------
    print("\n[2] Agent receives user prompt, storing session state...")
    client.json.put(
        "agent_memory",
        "session_current",
        {
            "user_id": "usr_42",
            "topic": "Rust & Distributed Systems",
            "context_tokens": 1420,
        },
    )

    client.kv.set("agent_memory", "goal_status", "in_progress")

    # -------------------------------------------------------------------------
    # 3. Mutate unwatched namespace (filtered out by daemon)
    # -------------------------------------------------------------------------
    print("\n[3] Background telemetry mutation (namespace: 'system_metrics')...")
    client.kv.set("system_metrics", "cpu_pct", "14.5")

    # Wait briefly for SSE events to be processed by background listener
    time.sleep(0.3)

    print(f"\n[4] Reactive Agent Local Memory Snapshot ({len(active_memory)} items):")
    for k, v in active_memory.items():
        print(f"  - Key: '{k}', Type: {v['type']}, Version: {v['version']}")

    # -------------------------------------------------------------------------
    # 4. Deletion mutation
    # -------------------------------------------------------------------------
    print("\n[5] Session completed, deleting session memory...")
    client.json.delete("agent_memory", "session_current")
    time.sleep(0.3)

    print(f"[6] Active memory items after deletion: {len(active_memory)}")

    # -------------------------------------------------------------------------
    # 5. Unsubscribe cleanly
    # -------------------------------------------------------------------------
    print("\n[7] Unsubscribing from live CDC stream...")
    subscription.unsubscribe()
    print("  Subscription closed successfully.")
    print("\nCDC live streaming demo completed successfully!")


if __name__ == "__main__":
    main()

"""02_json_documents.py - Document store, keyset scan pagination, and batch in LiliaDB.

Requirements:
- Run lilia-daemon with HTTP enabled:
    cargo run -p lilia-daemon -- --database /tmp/lilia_example.lilia --http-port 4444 --auth-token dev-secret
- Run this script:
    python3 examples/python/02_json_documents.py
"""

import os
import sys

from liliadb import LiliaClient

ENDPOINT = os.environ.get("LILIA_ENDPOINT", "http://127.0.0.1:4444")
TOKEN = os.environ.get("LILIA_TOKEN", "dev-secret")


def main() -> None:
    client = LiliaClient(endpoint=ENDPOINT, token=TOKEN)

    try:
        client.health()
    except Exception as exc:
        print(f"Error connecting to daemon at {ENDPOINT}: {exc}")
        print("\nPlease start the daemon with:")
        print("  cargo run -p lilia-daemon -- --database /tmp/lilia_example.lilia --http-port 4444 --auth-token dev-secret\n")
        sys.exit(1)

    space = "devices"

    print("\n--- 1. Insert JSON Documents ---")
    devices = [
        {"id": "dev_01", "name": "Sensor North", "temperature": 21.4, "battery": 95, "status": "active"},
        {"id": "dev_02", "name": "Sensor South", "temperature": 24.1, "battery": 82, "status": "active"},
        {"id": "dev_03", "name": "Sensor East", "temperature": 19.8, "battery": 14, "status": "low_battery"},
        {"id": "dev_04", "name": "Sensor West", "temperature": 22.0, "battery": 90, "status": "active"},
    ]

    for d in devices:
        doc_id = d["id"]
        v = client.json.put(space, doc_id, d)
        print(f"Stored device {doc_id} -> version: {v}")

    print("\n--- 2. Fetch Single Document ---")
    doc = client.json.get(space, "dev_01")
    print(f"Device dev_01: name={doc.value.get('name')}, temp={doc.value.get('temperature')}°C")

    print("\n--- 3. Auto-Paginating Keyset Scan ---")
    # client.json.scan returns a Python generator that transparently fetches pages
    print("Scanning devices with limit=2 (forcing multi-page requests):")
    for doc in client.json.scan(space, limit=2):
        print(f"  [{doc.id}] {doc.value.get('name')}: battery={doc.value.get('battery')}%")

    print("\n--- 4. Multi-Model Atomic Batch ---")
    batch_res = client.batch([
        {
            "type": "kv_set",
            "space": "stats",
            "key": "last_scan_device",
            "value": "dev_04",
        },
        {
            "type": "json_put",
            "space": space,
            "id": "dev_05",
            "value": {"name": "Sensor Central", "temperature": 20.5, "battery": 99},
        },
    ])
    print(f"Batch applied atomically ({len(batch_res)} operations):")
    for op in batch_res:
        print(f"  Result version: {op.version}")

    print("\n--- 5. Cleanup ---")
    for d in devices:
        client.json.delete(space, d["id"])
    client.json.delete(space, "dev_05")
    print("Cleaned up test documents.")


if __name__ == "__main__":
    main()

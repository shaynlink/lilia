"""Live integration tests running LiliaClient against a real lilia-daemon instance."""

import json
import os
import shutil
import socket
import subprocess
import tempfile
import threading
import time
import unittest
import urllib.request
from pathlib import Path

from liliadb import ConflictError, LiliaClient


def get_free_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


class TestLiveIntegration(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        raw_temp = tempfile.mkdtemp()
        temp_path = Path(raw_temp).resolve()
        os.chmod(temp_path, 0o700)
        cls.temp_path = temp_path
        cls.db_path = temp_path / "test.lilia"
        cls.endpoint = temp_path / "test.sock"
        cls.token_file = temp_path / "token"
        cls.http_port = get_free_port()

        # Build daemon binary first if not already built
        workspace_root = Path(__file__).resolve().parents[3]
        target_bin = workspace_root / "target" / "debug" / "lilia-daemon"

        if not target_bin.exists():
            subprocess.run(["cargo", "build", "-p", "lilia-daemon"], cwd=workspace_root, check=True)

        cmd = [
            str(target_bin),
            "--database",
            str(cls.db_path),
            "--endpoint",
            str(cls.endpoint),
            "--token-file",
            str(cls.token_file),
            "--http-port",
            str(cls.http_port),
            "--development",
        ]
        cls.daemon_process = subprocess.Popen(cmd, cwd=workspace_root, stderr=subprocess.PIPE)

        # Wait for daemon to be ready on HTTP port
        cls.endpoint_url = f"http://127.0.0.1:{cls.http_port}"
        health_url = f"{cls.endpoint_url}/health"

        ready = False
        start_time = time.time()
        while time.time() - start_time < 10:
            if cls.daemon_process.poll() is not None:
                err = cls.daemon_process.stderr.read().decode("utf-8") if cls.daemon_process.stderr else ""
                raise RuntimeError(f"Daemon exited prematurely with code {cls.daemon_process.returncode}: {err}")
            try:
                with urllib.request.urlopen(health_url, timeout=1) as resp:
                    if resp.status == 200:
                        ready = True
                        break
            except Exception:
                time.sleep(0.1)

        if not ready:
            cls.daemon_process.kill()
            err = cls.daemon_process.stderr.read().decode("utf-8") if cls.daemon_process.stderr else ""
            raise RuntimeError(f"Daemon did not start within 10s on {health_url}: {err}")

        cls.token = cls.token_file.read_text(encoding="utf-8").strip()
        cls.client = LiliaClient(endpoint=cls.endpoint_url, token=cls.token)

    @classmethod
    def tearDownClass(cls):
        try:
            cls.client.health()
            # Send shutdown
            try:
                cls.client._transport.request("POST", "/v1/shutdown")
            except Exception:
                pass
            cls.daemon_process.wait(timeout=5)
        except Exception:
            cls.daemon_process.kill()
        finally:
            if hasattr(cls, "temp_path") and cls.temp_path.exists():
                shutil.rmtree(cls.temp_path, ignore_errors=True)

    def test_health_check(self):
        health = self.client.health()
        self.assertTrue(health["ok"])
        self.assertEqual(health["status"], "running")

    def test_kv_crud_and_optimistic_concurrency(self):
        # 1. Set key
        v1 = self.client.kv.set("users", "alice", "engineer")
        self.assertEqual(v1, "1")

        # 2. Get key
        entry = self.client.kv.get("users", "alice")
        self.assertIsNotNone(entry)
        self.assertEqual(entry.value, "engineer")
        self.assertEqual(entry.version, "1")

        # 3. Update key with matching if_version
        v2 = self.client.kv.set("users", "alice", "senior engineer", if_version=1)
        self.assertEqual(v2, "2")

        # 4. Conflict on stale if_version
        with self.assertRaises(ConflictError):
            self.client.kv.set("users", "alice", "lead engineer", if_version=1)

        # 5. Delete key
        deleted = self.client.kv.delete("users", "alice", if_version=2)
        self.assertTrue(deleted)

        # 6. Verify deleted
        self.assertIsNone(self.client.kv.get("users", "alice"))

    def test_json_crud_and_sql_query(self):
        # 1. Put JSON documents
        self.client.json.put("products", "p1", {"title": "Keyboard", "price": 89.99, "stock": 15})
        self.client.json.put("products", "p2", {"title": "Mouse", "price": 49.99, "stock": 42})
        self.client.json.put("products", "p3", {"title": "Monitor", "price": 299.99, "stock": 8})

        # 2. Get JSON document
        doc = self.client.json.get("products", "p1")
        self.assertIsNotNone(doc)
        self.assertEqual(doc.value["title"], "Keyboard")
        self.assertEqual(doc.value["stock"], 15)

        # 3. SQL query over JSON collection
        query = """
            SELECT id,
                   json_extract(value, '$.title') as title,
                   json_extract(value, '$.stock') as stock
            FROM lilia_json
            WHERE space = 'products'
            ORDER BY stock DESC
        """
        result = self.client.sql(query)
        self.assertEqual(result.columns, ["id", "title", "stock"])
        self.assertEqual(len(result), 3)

        items = result.to_dicts()
        self.assertEqual(items[0]["title"], "Mouse")
        self.assertEqual(items[0]["stock"], 42)
        self.assertEqual(items[1]["title"], "Keyboard")
        self.assertEqual(items[1]["stock"], 15)
        self.assertEqual(items[2]["title"], "Monitor")
        self.assertEqual(items[2]["stock"], 8)

    def test_scan_auto_pagination(self):
        # Populate 15 items in a space
        for i in range(15):
            self.client.json.put("bench", f"item_{i:02d}", {"num": i})

        # Scan with limit=4 to force multiple page requests
        scanned = list(self.client.json.scan("bench", limit=4))
        self.assertEqual(len(scanned), 15)
        self.assertEqual([d.id for d in scanned], [f"item_{i:02d}" for i in range(15)])

    def test_batch_atomic_operations(self):
        results = self.client.batch([
            {
                "type": "kv_set",
                "space": "metrics",
                "key": "cpu_load",
                "value": "42%",
            },
            {
                "type": "json_put",
                "space": "servers",
                "id": "srv_1",
                "value": {"host": "web-01", "active": True},
            },
        ])
        self.assertEqual(len(results), 2)
        self.assertEqual(results[0].version, "1")
        self.assertEqual(results[1].version, "1")

        # Verify through regular getters
        kv_entry = self.client.kv.get("metrics", "cpu_load")
        self.assertIsNotNone(kv_entry)
        self.assertEqual(kv_entry.value, "42%")

        srv_doc = self.client.json.get("servers", "srv_1")
        self.assertIsNotNone(srv_doc)
        self.assertEqual(srv_doc.value["host"], "web-01")

    def test_doctor_and_integrity(self):
        self.assertTrue(self.client.doctor())

    def test_live_watch_stream(self):
        events = []

        def on_event(evt):
            events.append(evt)

        # Watch only "orders" namespace
        sub = self.client.watch("orders", callback=on_event)
        time.sleep(0.15)  # allow connection to be established

        # Mutate in watched namespace
        self.client.kv.set("orders", "ord_1", "created")
        self.client.json.put("orders", "ord_doc_1", {"total": 99})

        # Mutate in another namespace (should be filtered out by daemon)
        self.client.kv.set("inventory", "item_1", "instock")

        # Give a moment for events to arrive via SSE
        for _ in range(50):
            if len(events) >= 2:
                break
            time.sleep(0.05)

        sub.unsubscribe()

        self.assertEqual(len(events), 2)
        self.assertEqual(events[0].type, "kv_mutated")
        self.assertEqual(events[0].namespace, "orders")
        self.assertEqual(events[0].key, "ord_1")

        self.assertEqual(events[1].type, "json_mutated")
        self.assertEqual(events[1].space, "orders")
        self.assertEqual(events[1].id, "ord_doc_1")


if __name__ == "__main__":
    unittest.main()

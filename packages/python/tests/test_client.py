"""Unit tests for LiliaClient with mock transport."""

import unittest
from typing import Any, Dict, List, Optional

from liliadb import LiliaClient
from liliadb.transport import Transport


class MockTransport(Transport):
    def __init__(self):
        self.calls: List[Dict[str, Any]] = []
        self.responses: Dict[str, Any] = {}

    def set_response(self, path: str, response: Dict[str, Any]):
        self.responses[path] = response

    def request(self, method: str, path: str, payload: Optional[Dict[str, Any]] = None) -> Dict[str, Any]:
        self.calls.append({"method": method, "path": path, "payload": payload})
        if path in self.responses:
            return self.responses[path]
        return {"ok": True}


class TestClient(unittest.TestCase):
    def setUp(self):
        self.mock = MockTransport()
        self.client = LiliaClient(transport=self.mock)

    def test_kv_get(self):
        self.mock.set_response(
            "/v1/kv/get",
            {
                "ok": True,
                "entry": {
                    "namespace": "cfg",
                    "key": "theme",
                    "value": "dark",
                    "version": "1",
                    "expires_at_ms": None,
                },
            },
        )
        entry = self.client.kv.get("cfg", "theme")
        self.assertIsNotNone(entry)
        self.assertEqual(entry.key, "theme")
        self.assertEqual(entry.value, "dark")
        self.assertEqual(entry.version, "1")

    def test_kv_set(self):
        self.mock.set_response("/v1/kv/set", {"ok": True, "version": "42"})
        ver = self.client.kv.set("cfg", "theme", "light", if_version=41)
        self.assertEqual(ver, "42")
        call = self.mock.calls[-1]
        self.assertEqual(call["payload"]["key"], "theme")
        self.assertEqual(call["payload"]["value"], "light")
        self.assertEqual(call["payload"]["if_version"], 41)

    def test_kv_delete(self):
        self.mock.set_response("/v1/kv/delete", {"ok": True, "deleted": True})
        deleted = self.client.kv.delete("cfg", "theme")
        self.assertTrue(deleted)

    def test_kv_scan_pagination(self):
        # 2 pages: page 1 has 2 items, page 2 has 1 item (less than limit 2)
        call_count = 0

        def dynamic_request(method, path, payload):
            nonlocal call_count
            call_count += 1
            if payload.get("after") is None:
                return {
                    "ok": True,
                    "entries": [
                        {"namespace": "ns", "key": "k1", "value": "v1", "version": "1"},
                        {"namespace": "ns", "key": "k2", "value": "v2", "version": "1"},
                    ],
                }
            elif payload.get("after") == "k2":
                return {
                    "ok": True,
                    "entries": [
                        {"namespace": "ns", "key": "k3", "value": "v3", "version": "1"},
                    ],
                }
            return {"ok": True, "entries": []}

        self.mock.request = dynamic_request
        entries = list(self.client.kv.scan("ns", limit=2))
        self.assertEqual(len(entries), 3)
        self.assertEqual([e.key for e in entries], ["k1", "k2", "k3"])
        self.assertEqual(call_count, 2)

    def test_json_get(self):
        self.mock.set_response(
            "/v1/json/get",
            {
                "ok": True,
                "document": {
                    "space": "users",
                    "id": "u1",
                    "value": {"name": "Alice"},
                    "version": "1",
                },
            },
        )
        doc = self.client.json.get("users", "u1")
        self.assertIsNotNone(doc)
        self.assertEqual(doc.id, "u1")
        self.assertEqual(doc.value["name"], "Alice")

    def test_json_put(self):
        self.mock.set_response("/v1/json/put", {"ok": True, "version": "5"})
        ver = self.client.json.put("users", "u1", {"name": "Alice Updated"}, if_version=4)
        self.assertEqual(ver, "5")

    def test_json_delete(self):
        self.mock.set_response("/v1/json/delete", {"ok": True, "deleted": True})
        self.assertTrue(self.client.json.delete("users", "u1"))

    def test_sql_query(self):
        self.mock.set_response(
            "/v1/sql",
            {
                "ok": True,
                "columns": ["id", "count"],
                "rows": [["u1", 5], ["u2", 12]],
                "rows_affected": 2,
            },
        )
        res = self.client.sql("SELECT id, count FROM stats")
        self.assertEqual(res.columns, ["id", "count"])
        self.assertEqual(len(res), 2)
        dicts = res.to_dicts()
        self.assertEqual(dicts[0]["id"], "u1")
        self.assertEqual(dicts[0]["count"], 5)

    def test_batch(self):
        self.mock.set_response(
            "/v1/batch",
            {
                "ok": True,
                "results": [{"version": "1", "deleted": False}, {"deleted": True}],
            },
        )
        ops = [
            {"model": "kv_set", "namespace": "n", "key": [1], "value": [2]},
            {"model": "kv_delete", "namespace": "n", "key": [1]},
        ]
        results = self.client.batch(ops)
        self.assertEqual(len(results), 2)
        self.assertEqual(results[0].version, "1")
        self.assertTrue(results[1].deleted)

    def test_health_doctor_backup(self):
        self.mock.set_response("/health", {"ok": True, "status": "running"})
        self.assertEqual(self.client.health()["status"], "running")

        self.mock.set_response("/v1/doctor", {"ok": True, "integrity": True})
        self.assertTrue(self.client.doctor())

        self.mock.set_response("/v1/backup", {"ok": True})
        self.assertTrue(self.client.backup("/tmp/bak.lilia"))


if __name__ == "__main__":
    unittest.main()

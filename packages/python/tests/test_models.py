"""Tests for LiliaDB Python SDK models and error mappings."""

import unittest

from liliadb import (
    AuthenticationError,
    BatchResult,
    ConflictError,
    InvalidInputError,
    JsonDocument,
    KvEntry,
    LiliaError,
    NotFoundError,
    SqlResult,
    TimeoutError,
)
from liliadb.errors import raise_for_status


class TestModels(unittest.TestCase):
    def test_kv_entry(self):
        entry = KvEntry(namespace="users", key="k1", value="v1", version="1", expires_at_ms=1000)
        self.assertEqual(entry.namespace, "users")
        self.assertEqual(entry.key, "k1")
        self.assertEqual(entry.value, "v1")
        self.assertEqual(entry.version, "1")
        self.assertEqual(entry.expires_at_ms, 1000)

    def test_json_document(self):
        doc = JsonDocument(space="items", id="i1", value={"name": "widget", "price": 9.99}, version="2")
        self.assertEqual(doc.space, "items")
        self.assertEqual(doc.id, "i1")
        self.assertEqual(doc.value["name"], "widget")
        self.assertEqual(doc.version, "2")

    def test_sql_result_to_dicts(self):
        res = SqlResult(
            columns=["id", "name", "age"],
            rows=[["u1", "Alice", 30], ["u2", "Bob", 25]],
            rows_affected=2,
        )
        self.assertEqual(len(res), 2)
        dicts = res.to_dicts()
        self.assertEqual(len(dicts), 2)
        self.assertEqual(dicts[0], {"id": "u1", "name": "Alice", "age": 30})
        self.assertEqual(dicts[1], {"id": "u2", "name": "Bob", "age": 25})

        # Test direct iteration
        iter_items = list(res)
        self.assertEqual(iter_items, dicts)

    def test_batch_result(self):
        b1 = BatchResult(version="10", deleted=False)
        self.assertEqual(b1.version, "10")
        self.assertFalse(b1.deleted)

        b2 = BatchResult(version=None, deleted=True)
        self.assertTrue(b2.deleted)


class TestErrors(unittest.TestCase):
    def test_raise_for_status_ok(self):
        # Should not raise
        raise_for_status({"ok": True, "value": 42})

    def test_raise_for_status_mappings(self):
        with self.assertRaises(AuthenticationError):
            raise_for_status({"ok": False, "error": {"code": "AUTHENTICATION", "message": "bad token"}})

        with self.assertRaises(NotFoundError):
            raise_for_status({"ok": False, "error": {"code": "NOT_FOUND", "message": "doc missing"}})

        with self.assertRaises(ConflictError):
            raise_for_status({"ok": False, "error": {"code": "CONFLICT", "message": "version mismatch"}})

        with self.assertRaises(InvalidInputError):
            raise_for_status({"ok": False, "error": {"code": "INVALID_INPUT", "message": "bad json"}})

        with self.assertRaises(TimeoutError):
            raise_for_status({"ok": False, "error": {"code": "TIMEOUT", "message": "timed out"}})

        with self.assertRaises(LiliaError):
            raise_for_status({"ok": False, "error": {"code": "STORAGE", "message": "io failure"}})


if __name__ == "__main__":
    unittest.main()

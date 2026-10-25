"""High-level client API for LiliaDB."""

from pathlib import Path
from typing import Any, Dict, Generator, List, Optional, Union

from .models import BatchResult, JsonDocument, KvEntry, SqlResult
from .transport import HttpTransport, Transport


class KvClient:
    """Client for Key-Value model operations."""

    def __init__(self, transport: Transport):
        self._transport = transport

    def get(self, namespace: str, key: str, encoding: str = "text") -> Optional[KvEntry]:
        """Retrieves a key from the given namespace."""
        payload = {"namespace": namespace, "key": key, "encoding": encoding}
        res = self._transport.request("POST", "/v1/kv/get", payload)
        entry_data = res.get("entry")
        if not entry_data:
            return None
        return KvEntry(
            namespace=entry_data["namespace"],
            key=entry_data["key"],
            value=entry_data["value"],
            version=entry_data["version"],
            expires_at_ms=entry_data.get("expires_at_ms"),
        )

    def set(
        self,
        namespace: str,
        key: str,
        value: str,
        if_version: Optional[int] = None,
        expires_at_ms: Optional[int] = None,
        encoding: str = "text",
    ) -> str:
        """Sets a key/value pair with optional version check and expiry."""
        payload: Dict[str, Any] = {
            "namespace": namespace,
            "key": key,
            "value": value,
            "encoding": encoding,
        }
        if if_version is not None:
            payload["if_version"] = if_version
        if expires_at_ms is not None:
            payload["expires_at_ms"] = expires_at_ms

        res = self._transport.request("POST", "/v1/kv/set", payload)
        return str(res.get("version", ""))

    def delete(
        self,
        namespace: str,
        key: str,
        if_version: Optional[int] = None,
        encoding: str = "text",
    ) -> bool:
        """Deletes a key from the given namespace."""
        payload: Dict[str, Any] = {"namespace": namespace, "key": key, "encoding": encoding}
        if if_version is not None:
            payload["if_version"] = if_version

        res = self._transport.request("POST", "/v1/kv/delete", payload)
        return bool(res.get("deleted", False))

    def scan(
        self,
        namespace: str,
        after: Optional[str] = None,
        limit: int = 100,
        encoding: str = "text",
    ) -> Generator[KvEntry, None, None]:
        """Scans keys in a namespace, yielding entries and automatically paginating."""
        cursor = after
        while True:
            payload: Dict[str, Any] = {
                "namespace": namespace,
                "limit": limit,
                "encoding": encoding,
            }
            if cursor is not None:
                payload["after"] = cursor

            res = self._transport.request("POST", "/v1/kv/scan", payload)
            entries = res.get("entries", [])
            if not entries:
                break

            for item in entries:
                entry = KvEntry(
                    namespace=item["namespace"],
                    key=item["key"],
                    value=item["value"],
                    version=item["version"],
                    expires_at_ms=item.get("expires_at_ms"),
                )
                yield entry
                cursor = entry.key

            if len(entries) < limit:
                break


class JsonClient:
    """Client for JSON Document model operations."""

    def __init__(self, transport: Transport):
        self._transport = transport

    def get(self, space: str, id: str) -> Optional[JsonDocument]:
        """Retrieves a document by space and ID."""
        payload = {"space": space, "id": id}
        res = self._transport.request("POST", "/v1/json/get", payload)
        doc_data = res.get("document")
        if not doc_data:
            return None
        return JsonDocument(
            space=doc_data["space"],
            id=doc_data["id"],
            value=doc_data["value"],
            version=doc_data["version"],
        )

    def put(
        self,
        space: str,
        id: str,
        value: Any,
        if_version: Optional[int] = None,
    ) -> str:
        """Puts a JSON document with optional version check."""
        payload: Dict[str, Any] = {"space": space, "id": id, "value": value}
        if if_version is not None:
            payload["if_version"] = if_version

        res = self._transport.request("POST", "/v1/json/put", payload)
        return str(res.get("version", ""))

    def delete(
        self,
        space: str,
        id: str,
        if_version: Optional[int] = None,
    ) -> bool:
        """Deletes a JSON document by space and ID."""
        payload: Dict[str, Any] = {"space": space, "id": id}
        if if_version is not None:
            payload["if_version"] = if_version

        res = self._transport.request("POST", "/v1/json/delete", payload)
        return bool(res.get("deleted", False))

    def scan(
        self,
        space: str,
        after: Optional[str] = None,
        limit: int = 100,
    ) -> Generator[JsonDocument, None, None]:
        """Scans documents in a space, yielding documents and automatically paginating."""
        cursor = after
        while True:
            payload: Dict[str, Any] = {"space": space, "limit": limit}
            if cursor is not None:
                payload["after"] = cursor

            res = self._transport.request("POST", "/v1/json/scan", payload)
            docs = res.get("documents", [])
            if not docs:
                break

            for item in docs:
                doc = JsonDocument(
                    space=item["space"],
                    id=item["id"],
                    value=item["value"],
                    version=item["version"],
                )
                yield doc
                cursor = doc.id

            if len(docs) < limit:
                break


class LiliaClient:
    """Official LiliaDB client providing access to all models and capabilities."""

    def __init__(
        self,
        endpoint: str = "http://127.0.0.1:4000",
        token: Optional[str] = None,
        token_file: Optional[Union[str, Path]] = None,
        transport: Optional[Transport] = None,
    ):
        resolved_token = token
        if resolved_token is None and token_file is not None:
            resolved_token = Path(token_file).read_text(encoding="utf-8").strip()

        if transport is not None:
            self._transport = transport
        else:
            self._transport = HttpTransport(endpoint=endpoint, token=resolved_token)

        self.kv = KvClient(self._transport)
        self.json = JsonClient(self._transport)

    def sql(self, query: str, params: Optional[List[Any]] = None) -> SqlResult:
        """Executes a declarative SQL query over LiliaDB collections."""
        payload: Dict[str, Any] = {"query": query, "params": params or []}
        res = self._transport.request("POST", "/v1/sql", payload)
        return SqlResult(
            columns=res.get("columns", []),
            rows=res.get("rows", []),
            rows_affected=res.get("rows_affected", 0),
        )

    def batch(self, operations: List[Dict[str, Any]]) -> List[BatchResult]:
        """Executes a list of operations atomically within a single transaction."""
        payload = {"operations": operations}
        res = self._transport.request("POST", "/v1/batch", payload)
        results = res.get("results", [])
        return [
            BatchResult(
                version=item.get("version"),
                deleted=item.get("deleted", False),
            )
            for item in results
        ]

    def health(self) -> Dict[str, Any]:
        """Checks the daemon status."""
        return self._transport.request("GET", "/health")

    def doctor(self) -> bool:
        """Runs an SQLite integrity check on the active database."""
        res = self._transport.request("GET", "/v1/doctor")
        return bool(res.get("integrity", False))

    def backup(self, destination: str) -> bool:
        """Triggers an online backup to the destination path."""
        res = self._transport.request("POST", "/v1/backup", {"destination": destination})
        return bool(res.get("ok", False))

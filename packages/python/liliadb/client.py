"""High-level client API for LiliaDB."""

import json
from pathlib import Path
import threading
from typing import Any, Callable, Dict, Generator, List, Optional, Union
import urllib.parse
import urllib.request

from .models import BatchResult, DatabaseEvent, FtsHit, JsonDocument, JsonIndexInfo, KvEntry, SqlResult
from .transport import HttpTransport, Transport


class WatchSubscription:
    """Handle to an active live CDC event subscription."""

    def __init__(self, stop_fn: Callable[[], None], thread: Optional[threading.Thread] = None):
        self._stop_fn = stop_fn
        self._thread = thread
        self._closed = False

    def stop(self) -> None:
        """Stops the watch subscription and releases resources."""
        if not self._closed:
            self._closed = True
            self._stop_fn()

    def close(self) -> None:
        """Alias for stop()."""
        self.stop()

    def unsubscribe(self) -> None:
        """Alias for stop()."""
        self.stop()

    def __enter__(self) -> "WatchSubscription":
        return self

    def __exit__(self, exc_type: Any, exc_val: Any, exc_tb: Any) -> None:
        self.stop()


def _parse_database_event(raw: Dict[str, Any]) -> DatabaseEvent:
    event_type = raw.get("type", "")
    key_val = raw.get("key")
    if isinstance(key_val, list):
        key_str = bytes(key_val).decode("utf-8", errors="replace")
    elif isinstance(key_val, bytes):
        key_str = key_val.decode("utf-8", errors="replace")
    elif key_val is not None:
        key_str = str(key_val)
    else:
        key_str = None

    version_val = raw.get("version")
    version_str = str(version_val) if version_val is not None else None

    return DatabaseEvent(
        type=event_type,
        namespace=raw.get("namespace"),
        space=raw.get("space"),
        graph=raw.get("graph"),
        collection=raw.get("collection"),
        key=key_str,
        id=raw.get("id"),
        doc_id=raw.get("doc_id"),
        source=raw.get("source"),
        target=raw.get("target"),
        kind=raw.get("kind"),
        version=version_str,
        deleted=bool(raw.get("deleted", False)),
        raw=raw,
    )


def _watch_stream(
    transport: Transport,
    namespace: Optional[str] = None,
    callback: Optional[Callable[[DatabaseEvent], None]] = None,
) -> Union[WatchSubscription, Generator[DatabaseEvent, None, None]]:
    endpoint = getattr(transport, "endpoint", "http://127.0.0.1:4000")
    token = getattr(transport, "token", None)
    params = []
    if namespace:
        params.append(f"namespace={urllib.parse.quote(namespace)}")
    if token:
        params.append(f"token={urllib.parse.quote(token)}")
    query_str = f"?{'&'.join(params)}" if params else ""
    url = f"{endpoint}/v1/watch{query_str}"

    headers = {
        "Accept": "text/event-stream",
        "User-Agent": "liliadb-python/0.1.0",
    }
    if token:
        headers["Authorization"] = f"Bearer {token}"

    if callback is not None:
        stopped = threading.Event()
        response_holder: List[Any] = []

        def worker() -> None:
            req = urllib.request.Request(url, headers=headers, method="GET")
            try:
                resp = urllib.request.urlopen(req)
                response_holder.append(resp)
                with resp:
                    for line in resp:
                        if stopped.is_set():
                            break
                        text = line.decode("utf-8").strip()
                        if text.startswith("data:"):
                            data_str = text[5:].strip()
                            if data_str:
                                try:
                                    data_json = json.loads(data_str)
                                    event = _parse_database_event(data_json)
                                    callback(event)
                                except Exception:
                                    pass
            except Exception:
                pass

        thread = threading.Thread(target=worker, daemon=True)
        thread.start()

        def stop() -> None:
            stopped.set()
            if response_holder:
                try:
                    response_holder[0].close()
                except Exception:
                    pass

        return WatchSubscription(stop_fn=stop, thread=thread)

    def stream_generator() -> Generator[DatabaseEvent, None, None]:
        req = urllib.request.Request(url, headers=headers, method="GET")
        with urllib.request.urlopen(req) as resp:
            for line in resp:
                text = line.decode("utf-8").strip()
                if text.startswith("data:"):
                    data_str = text[5:].strip()
                    if data_str:
                        try:
                            data_json = json.loads(data_str)
                            yield _parse_database_event(data_json)
                        except Exception:
                            pass

    return stream_generator()


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

    def watch(
        self,
        namespace: str,
        callback: Optional[Callable[[DatabaseEvent], None]] = None,
    ) -> Union[WatchSubscription, Generator[DatabaseEvent, None, None]]:
        """Watches for live CDC events in this key-value namespace."""
        return _watch_stream(self._transport, namespace=namespace, callback=callback)


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

    def watch(
        self,
        space: str,
        callback: Optional[Callable[[DatabaseEvent], None]] = None,
    ) -> Union[WatchSubscription, Generator[DatabaseEvent, None, None]]:
        """Watches for live CDC events in this document space."""
        return _watch_stream(self._transport, namespace=space, callback=callback)

    def create_index(
        self,
        space: str,
        field_or_path: str,
        name: Optional[str] = None,
        path: Optional[str] = None,
    ) -> None:
        """Creates a secondary index on a document space."""
        idx_name = name or field_or_path
        if path is not None:
            idx_path = path
        elif field_or_path.startswith("$") or field_or_path.startswith("/"):
            idx_path = field_or_path
        else:
            idx_path = f"$.{field_or_path}"

        payload = {"space": space, "name": idx_name, "path": idx_path}
        self._transport.request("POST", "/v1/json/indexes/create", payload)

    def drop_index(self, space: str, name: str) -> bool:
        """Drops a secondary index by name."""
        payload = {"space": space, "name": name}
        res = self._transport.request("POST", "/v1/json/indexes/drop", payload)
        return bool(res.get("deleted", False))

    def list_indexes(self, space: str) -> List[JsonIndexInfo]:
        """Lists all secondary indexes defined on a document space."""
        payload = {"space": space}
        res = self._transport.request("POST", "/v1/json/indexes/list", payload)
        indexes = res.get("indexes", [])
        return [
            JsonIndexInfo(
                space=item["space"],
                index_name=item["index_name"],
                json_path=item["json_path"],
            )
            for item in indexes
        ]

    def find_by_index(
        self,
        space: str,
        index_name: str,
        value: Any,
        after: Optional[str] = None,
        limit: int = 100,
    ) -> Generator[JsonDocument, None, None]:
        """Scans documents matching an index value, automatically paginating."""
        cursor = after
        val_str = str(value) if not isinstance(value, bool) else ("true" if value else "false")
        while True:
            payload: Dict[str, Any] = {
                "space": space,
                "index_name": index_name,
                "value": val_str,
                "limit": limit,
            }
            if cursor is not None:
                payload["after"] = cursor

            res = self._transport.request("POST", "/v1/json/scan_by_index", payload)
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

    def find(
        self,
        space: str,
        query: Dict[str, Any],
        after: Optional[str] = None,
        limit: int = 100,
    ) -> Generator[JsonDocument, None, None]:
        """Queries documents matching a filter predicate with index acceleration and auto-pagination."""
        cursor = after
        while True:
            payload: Dict[str, Any] = {
                "space": space,
                "query": query,
                "limit": limit,
            }
            if cursor is not None:
                payload["after"] = cursor

            res = self._transport.request("POST", "/v1/json/find", payload)
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



class GraphClient:
    """Client for Graph model operations."""

    def __init__(self, transport: Transport):
        self._transport = transport

    def set_node(
        self,
        graph: str,
        id: str,
        kind: str = "default",
        metadata: Optional[Dict[str, Any]] = None,
    ) -> bool:
        """Sets or updates a node in the graph."""
        payload: Dict[str, Any] = {
            "graph": graph,
            "id": id,
            "kind": kind,
            "metadata": metadata or {},
        }
        res = self._transport.request("POST", "/v1/graph/node", payload)
        return bool(res.get("ok", False))

    def add_edge(
        self,
        graph: str,
        source: str,
        target: str,
        kind: str = "edge",
    ) -> bool:
        """Adds a directed edge between two nodes in the graph."""
        payload: Dict[str, Any] = {
            "graph": graph,
            "source": source,
            "target": target,
            "kind": kind,
        }
        res = self._transport.request("POST", "/v1/graph/edge", payload)
        return bool(res.get("ok", False))

    def neighbors(
        self,
        graph: str,
        node: str,
        direction: str = "outgoing",
        kind: Optional[str] = None,
    ) -> List[str]:
        """Finds neighboring nodes connected to a given node in the graph."""
        payload: Dict[str, Any] = {
            "graph": graph,
            "node": node,
            "direction": direction,
        }
        if kind is not None:
            payload["kind"] = kind
        res = self._transport.request("POST", "/v1/graph/neighbors", payload)
        return res.get("neighbors", [])

    def reachability(
        self,
        graph: str,
        start_nodes: List[str],
        direction: str = "outgoing",
    ) -> List[str]:
        """Finds all nodes reachable from the specified starting nodes."""
        payload: Dict[str, Any] = {
            "graph": graph,
            "start_nodes": start_nodes,
            "direction": direction,
        }
        res = self._transport.request("POST", "/v1/graph/reachability", payload)
        return res.get("reachable", [])


class FtsClient:
    """Client for Full-Text Search (FTS5) operations."""

    def __init__(self, transport: Transport):
        self._transport = transport

    def index(self, collection: str, doc_id: str, content: str) -> bool:
        """Indexes or updates a document in a full-text search collection."""
        payload: Dict[str, Any] = {
            "collection": collection,
            "doc_id": doc_id,
            "content": content,
        }
        res = self._transport.request("POST", "/v1/fts/index", payload)
        return bool(res.get("ok", False))

    def search(
        self,
        collection: str,
        query: str,
        limit: int = 10,
    ) -> List[FtsHit]:
        """Performs full-text search over indexed documents in a collection."""
        payload: Dict[str, Any] = {
            "collection": collection,
            "query": query,
            "limit": limit,
        }
        res = self._transport.request("POST", "/v1/fts/search", payload)
        hits_data = res.get("hits", [])
        return [
            FtsHit(
                collection=item["collection"],
                doc_id=item["doc_id"],
                snippet=item["snippet"],
                rank=float(item["rank"]),
            )
            for item in hits_data
        ]


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
        self.graph = GraphClient(self._transport)
        self.fts = FtsClient(self._transport)

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

    def watch(
        self,
        namespace: Optional[str] = None,
        callback: Optional[Callable[[DatabaseEvent], None]] = None,
    ) -> Union[WatchSubscription, Generator[DatabaseEvent, None, None]]:
        """Watches for live CDC database events.

        If a callback is provided, events are delivered asynchronously to the callback
        in a background thread, returning a WatchSubscription handle.
        If no callback is provided, returns a real-time generator yielding DatabaseEvent instances.
        """
        return _watch_stream(self._transport, namespace=namespace, callback=callback)

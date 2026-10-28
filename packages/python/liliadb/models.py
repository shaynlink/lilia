"""Data models and result containers for LiliaDB Python SDK."""

from dataclasses import dataclass
from typing import Any, Dict, List, Optional


@dataclass(frozen=True)
class KvEntry:
    """Represents an entry stored in a Key-Value namespace."""

    namespace: str
    key: str
    value: str
    version: str
    expires_at_ms: Optional[int] = None


@dataclass(frozen=True)
class JsonDocument:
    """Represents a JSON document stored in a document space."""

    space: str
    id: str
    value: Any
    version: str


@dataclass(frozen=True)
class JsonIndexInfo:
    """Represents a secondary index registered on a JSON document space."""

    space: str
    index_name: str
    json_path: str



@dataclass(frozen=True)
class SqlResult:
    """Represents the outcome of a declarative SQL query."""

    columns: List[str]
    rows: List[List[Any]]
    rows_affected: int = 0

    def to_dicts(self) -> List[Dict[str, Any]]:
        """Converts query rows into a list of dictionaries mapping column name to value."""
        return [dict(zip(self.columns, row)) for row in self.rows]

    def __iter__(self):
        """Allows direct iteration over rows as dictionaries."""
        return iter(self.to_dicts())

    def __len__(self) -> int:
        return len(self.rows)


@dataclass(frozen=True)
class BatchResult:
    """Outcome of a single operation in an atomic batch."""

    version: Optional[str] = None
    deleted: bool = False


@dataclass(frozen=True)
class FtsHit:
    """Represents a search match in a full-text search collection."""

    collection: str
    doc_id: str
    snippet: str
    rank: float


@dataclass(frozen=True)
class DatabaseEvent:
    """Represents a change event emitted on data mutation (CDC)."""

    type: str
    namespace: Optional[str] = None
    space: Optional[str] = None
    graph: Optional[str] = None
    collection: Optional[str] = None
    key: Optional[str] = None
    id: Optional[str] = None
    doc_id: Optional[str] = None
    source: Optional[str] = None
    target: Optional[str] = None
    kind: Optional[str] = None
    version: Optional[str] = None
    deleted: bool = False
    raw: Optional[Dict[str, Any]] = None


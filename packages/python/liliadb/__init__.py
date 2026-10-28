"""LiliaDB Python SDK - Local-first multimodel embedded & daemon database."""

from .client import FtsClient, GraphClient, JsonClient, KvClient, LiliaClient, WatchSubscription
from .errors import (
    AuthenticationError,
    ConflictError,
    InvalidInputError,
    LiliaError,
    NotFoundError,
    StorageError,
    TimeoutError,
)
from .models import BatchResult, DatabaseEvent, FtsHit, JsonDocument, JsonIndexInfo, KvEntry, SqlResult

__version__ = "0.1.0a3"

__all__ = [
    "LiliaClient",
    "KvClient",
    "JsonClient",
    "GraphClient",
    "FtsClient",
    "KvEntry",
    "JsonDocument",
    "JsonIndexInfo",
    "SqlResult",
    "BatchResult",
    "FtsHit",
    "DatabaseEvent",
    "WatchSubscription",
    "LiliaError",
    "AuthenticationError",
    "ConflictError",
    "NotFoundError",
    "InvalidInputError",
    "TimeoutError",
    "StorageError",
]

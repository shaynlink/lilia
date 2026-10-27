"""LiliaDB Python SDK - Local-first multimodel embedded & daemon database."""

from .client import JsonClient, KvClient, LiliaClient
from .errors import (
    AuthenticationError,
    ConflictError,
    InvalidInputError,
    LiliaError,
    NotFoundError,
    StorageError,
    TimeoutError,
)
from .models import BatchResult, JsonDocument, KvEntry, SqlResult

__version__ = "0.1.0a2"

__all__ = [
    "LiliaClient",
    "KvClient",
    "JsonClient",
    "KvEntry",
    "JsonDocument",
    "SqlResult",
    "BatchResult",
    "LiliaError",
    "AuthenticationError",
    "ConflictError",
    "NotFoundError",
    "InvalidInputError",
    "TimeoutError",
    "StorageError",
]

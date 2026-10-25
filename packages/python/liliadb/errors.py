"""Exception types for LiliaDB Python SDK."""

from typing import Optional


class LiliaError(Exception):
    """Base exception for all LiliaDB errors."""

    def __init__(self, message: str, code: str = "STORAGE", retryable: bool = False):
        super().__init__(message)
        self.message = message
        self.code = code
        self.retryable = retryable

    def __repr__(self) -> str:
        return f"{self.__class__.__name__}(code={self.code!r}, message={self.message!r}, retryable={self.retryable})"


class AuthenticationError(LiliaError):
    """Raised when authentication fails or the Bearer token is invalid."""

    def __init__(self, message: str = "invalid or missing authentication token"):
        super().__init__(message, code="AUTHENTICATION", retryable=False)


class NotFoundError(LiliaError):
    """Raised when a requested resource (document or key) is not found."""

    def __init__(self, message: str = "resource not found"):
        super().__init__(message, code="NOT_FOUND", retryable=False)


class ConflictError(LiliaError):
    """Raised when a mutation fails due to a version conflict (optimistic concurrency)."""

    def __init__(self, message: str = "version conflict"):
        super().__init__(message, code="CONFLICT", retryable=True)


class InvalidInputError(LiliaError):
    """Raised when arguments, parameters, or payloads are malformed."""

    def __init__(self, message: str = "invalid input"):
        super().__init__(message, code="INVALID_INPUT", retryable=False)


class TimeoutError(LiliaError):
    """Raised when a request deadline or lock acquisition times out."""

    def __init__(self, message: str = "request deadline exceeded"):
        super().__init__(message, code="TIMEOUT", retryable=True)


class StorageError(LiliaError):
    """Raised when an internal database or storage error occurs."""

    def __init__(self, message: str = "storage error"):
        super().__init__(message, code="STORAGE", retryable=False)


def raise_for_status(payload: dict) -> None:
    """Inspects a LiliaDB API response dictionary and raises an error if ok is false."""
    if payload.get("ok") is True:
        return

    error = payload.get("error", {})
    code = error.get("code", "STORAGE")
    message = error.get("message", "command failed")
    retryable = error.get("retryable", False)

    if code == "AUTHENTICATION":
        raise AuthenticationError(message)
    elif code == "NOT_FOUND":
        raise NotFoundError(message)
    elif code == "CONFLICT":
        raise ConflictError(message)
    elif code == "INVALID_INPUT":
        raise InvalidInputError(message)
    elif code == "TIMEOUT":
        raise TimeoutError(message)
    else:
        raise LiliaError(message, code=code, retryable=retryable)

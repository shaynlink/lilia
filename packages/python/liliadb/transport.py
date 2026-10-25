"""Transports for communicating with LiliaDB daemon."""

import json
import urllib.error
import urllib.request
from typing import Any, Dict, Optional

from .errors import AuthenticationError, LiliaError, raise_for_status


class Transport:
    """Base transport interface."""

    def request(self, method: str, path: str, payload: Optional[Dict[str, Any]] = None) -> Dict[str, Any]:
        raise NotImplementedError


class HttpTransport(Transport):
    """Communicates with LiliaDB via its loopback HTTP REST gateway."""

    def __init__(self, endpoint: str = "http://127.0.0.1:4000", token: Optional[str] = None):
        self.endpoint = endpoint.rstrip("/")
        self.token = token

    def request(self, method: str, path: str, payload: Optional[Dict[str, Any]] = None) -> Dict[str, Any]:
        url = f"{self.endpoint}/{path.lstrip('/')}"
        headers = {
            "Accept": "application/json",
            "User-Agent": "liliadb-python/0.1.0",
        }
        if self.token:
            headers["Authorization"] = f"Bearer {self.token}"

        data = None
        if payload is not None:
            data = json.dumps(payload).encode("utf-8")
            headers["Content-Type"] = "application/json"

        req = urllib.request.Request(url, data=data, headers=headers, method=method)

        try:
            with urllib.request.urlopen(req, timeout=30) as response:
                content = response.read().decode("utf-8")
                result = json.loads(content)
                raise_for_status(result)
                return result
        except urllib.error.HTTPError as error:
            try:
                error_body = error.read().decode("utf-8")
                error_json = json.loads(error_body)
                raise_for_status(error_json)
            except (json.JSONDecodeError, UnicodeDecodeError):
                pass
            if error.code == 401:
                raise AuthenticationError("invalid or missing authentication token")
            raise LiliaError(f"HTTP request failed with status {error.code}: {error.reason}")
        except urllib.error.URLError as error:
            raise LiliaError(f"Failed to connect to LiliaDB at {url}: {error.reason}", code="IO")

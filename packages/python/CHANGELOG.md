# liliadb (Python SDK) Changelog

All notable changes to the `liliadb` Python SDK will be documented in this file.

---

## [0.1.0a2] - 2026-10-27

### Added
- **Official PyPI Distribution**:
  - Package published to PyPI (`pip install liliadb`) with automated GitHub Actions Trusted Publishing (OIDC).
  - PEP 517/621 build configuration with pure Python wheel and sdist distributions.
  - Multi-version testing matrix across Python 3.9, 3.10, 3.11, 3.12, and 3.13.
- **Real-Time CDC & Live Event Subscriptions**:
  - `client.watch(namespace=None, callback=None)` supporting both threaded background callbacks and synchronous generator iterations.
  - Integration with `lilia-daemon` Server-Sent Events (`/v1/watch` SSE).
  - Subscription lifecycle management using Python context manager (`with client.watch(...) as sub:`).
- **Multi-Model Storage & SQL Engine**:
  - KV operations with TTL (`expires_at_ms`) and optimistic concurrency (`if_version`).
  - JSON document store with cursor pagination.
  - Dict-ready SQL analytics via `result.to_dicts()`.
  - Atomic multi-model batch transactions (`client.batch(...)`).
- **Zero Dependencies**:
  - Standard library only (`urllib.request`, `json`, `threading`).

---

## [0.1.0a1] - 2026-09-10

### Added
- Initial prototype Python client for `lilia-daemon`.

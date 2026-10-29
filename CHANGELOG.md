# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [0.1.0-alpha.4] - 2026-10-29

### Fixed
- **Homebrew Formula URL Alignment**:
  - Corrected archive download URLs in `Formula/lilia.rb` to point to official repository `https://github.com/shaynlink/lilia`.
  - Harmonized tap instructions to `brew tap liliadb/tap && brew install lilia`.

### Added
- **Exhaustive Documentation**:
  - Completed HTTP REST API specification (`docs/http_api.md`) with full endpoints documentation for Graph (`/v1/graph/*`), FTS5 (`/v1/fts/*`), and real-time Change Data Capture SSE streams (`/v1/watch`).
  - Completed Python SDK reference (`docs/python_sdk.md`) with Graph (`client.graph`), FTS5 (`client.fts`), and dynamic JSON secondary indexing (`client.json.create_index`).
  - Added dedicated comprehensive Node.js SDK documentation (`docs/node_sdk.md`) covering embedded and daemon modes, streaming, OCC, and cross-runtime compatibility.

### Changed
- **Workspace & CI Hygiene**:
  - Disabled telemetry in Nx workspace configuration to eliminate interactive CLI prompts in CI.
  - Purged stale alpha.2 build artifacts and verified 100% test pass rate across Rust, Node, and Python test suites.

---

## [0.1.0-alpha.3] - 2026-10-28

### Added
- **Public Homebrew Tap**:
  - Official public tap repository at `shaynlink/homebrew-tap` supporting `brew tap shaynlink/tap && brew install lilia` (and custom alias `brew tap liliadb/tap`).
  - Automated formula synchronization with SHA256 checksums on release dispatch.
- **Packaging & CI Hardening**:
  - Direct npm publication of multi-platform native tarball prebuilds for macOS (arm64/x64), Linux (arm64/x64), and Windows MSVC.
  - Dual PyPI authentication support (OIDC Trusted Publishing and API token fallback).
  - Native Deno 2 auto node_modules resolution via `deno.json`.
  - Windows chaos recovery permission handling.
- **Daemon HTTP & Storage Enhancements**:
  - Dedicated REST endpoints for batch, transactions, doctor, and live SSE event streams.

## [0.1.0-alpha.2] - 2026-10-27

### Added
- **Global Packaging & Package Managers**:
  - Official Homebrew formula (`Formula/liliadb.rb`, `Formula/lilia.rb`) for macOS (Apple Silicon / Intel) and Linux (x86_64 / arm64) with background daemon management via `brew services start liliadb`.
  - Automated PyPI publishing workflow (`.github/workflows/publish-pypi.yml`) with PyPA Trusted Publishing (OIDC).
  - Multi-runtime server compatibility in `@liliadb/node` with automated CI matrix for Node.js (22, 24, 26), Bun 1.x, and Deno 2.x.
  - Automated checksum calculator tool (`tools/update-homebrew-formula.mjs`) synchronizing release assets with Homebrew formula.
- **Real-Time CDC & Live Watch Subscriptions**:
  - Live reactive change event bus across all storage mutations (KV, JSON, Graph, FTS5).
  - SDK subscription API: `db.watch(namespace, callback)` in `@liliadb/node` and `client.watch(namespace, callback)` in Python SDK.
  - HTTP Server-Sent Events endpoint `GET /v1/watch` on `lilia-daemon` for cross-platform event streaming.
  - Binary protocol streaming request framing (`RequestPayload::SubscribeWatch`, `ResponsePayload::WatchEvent`).
- **Graph Engine & Full-Text Search (FTS5)**:
  - Directed graph representation (`lilia-graph`), adjacency storage, neighborhood traversal (`graphNeighbors`), and transitive reachability analysis (`graphReachability`).
  - SQLite FTS5 integration with BM25 ranking, snippet generation, and highlighted query matching across CLI, Node SDK, and MCP server.
- **Encryption at Rest (AES-256-GCM AEAD)**:
  - Hardware-accelerated native encryption via `plugins/lilia-plugin-codec` protecting KV and JSON values on disk and in WAL frames with zero plaintext leakage.
- **Declarative SQL Engine**:
  - Integrated SQL analytical engine via `plugins/lilia-plugin-sql` exposing views (`lilia_kv`, `lilia_json`, `lilia_graph_nodes`, `lilia_graph_edges`) and transparent decryption functions (`lilia_decode_kv`, `lilia_decode_json`).
- **Keyset Streaming & Async Iteration**:
  - Native async generator `scanStream()` and Node.js `createReadStream()` in `@liliadb/node` for unbounded, memory-constant key scans.
- **Multi-Platform Prebuilds**:
  - Cross-compilation builder script (`tools/build-prebuilds.mjs`) and release packaging for `darwin-arm64`, `darwin-x64`, `linux-arm64-gnu`, `linux-x64-gnu`, and `win32-x64-msvc`.

### Security
- **Windows Access Control Hardening**:
  - Enforced restrictive DACLs (`O:{sid}D:P(...)`) on IPC named pipes, authentication tokens, database files, and backup staging, establishing strict multi-user access denial parity with Unix (`0700`/`0600`).
- **Crash & Chaos Resilience**:
  - Formal tolerance verification against power loss, torn WAL frames, bit-flips in checksums, abrupt `SIGKILL` termination, and `ENOSPC` disk exhaustion with automatic atomic self-healing.
- **Constant-Time Authentication**:
  - Constant-time Bearer token comparison (`subtle::ConstantTimeEq`) on `lilia-daemon` HTTP gateway to prevent timing side-channel attacks.

### Performance
- **Calibrated Benchmarks**:
  - Dedicated benchmark tool `lilia-bench` in `crates/lilia-storage-sqlite` and TypeScript runner in `packages/node/src/benchmarks/`.
  - Comprehensive performance metrics documented in `docs/benchmarks.md` demonstrating up to 281,016 ops/sec in native Rust and 171,051 ops/sec over HTTP loopback.

---

## [0.1.0-alpha.1] - 2026-09-10

### Added
- Initial project release with SQLite WAL multi-model engine (KV, JSON).
- Node-API native bindings in `@liliadb/node`.
- Standalone CLI (`lilia`), Daemon (`lilia-daemon`), and MCP server (`lilia-mcp`).
- Conformance test suite with shared 28-fixture oracle.

# @liliadb/node Changelog

All notable changes to the `@liliadb/node` SDK will be documented in this file.

---

## [0.1.0-alpha.4] - 2026-10-29

### Changed
- Comprehensive documentation synchronization with dedicated `docs/node_sdk.md`.
- Workspace cleanup, telemetry disabling, and Homebrew tap formula path alignment.

---

## [0.1.0-alpha.3] - 2026-10-28

### Added
- **Multi-Platform Prebuilds**:
  - Direct npm publication of multi-platform native tarball prebuilds for macOS (arm64/x64), Linux (arm64/x64), and Windows MSVC.
  - Native Deno 2 auto node_modules resolution via `deno.json`.
- **Packaging Hardening**:
  - Automatic prebuild artifact packaging via `tools/build-prebuilds.mjs`.

---

## [0.1.0-alpha.2] - 2026-10-27

### Added
- **Multi-Runtime Compatibility**:
  - Full native support for **Node.js** (>= 22), **Bun** (>= 1.0), and **Deno** (>= 2.0).
  - Automated cross-runtime test verification in CI (`compat-runtimes.yml`).
- **Real-Time CDC & Live Event Streaming**:
  - New `db.watch(namespace?, callback)` method to subscribe to live mutations across KV, JSON, Graph, and FTS collections.
  - Streaming MessagePack framing over local Unix domain sockets and Windows named pipes.
  - Graceful unsubscription via `subscription.unsubscribe()`.
- **Embedded Graph Operations**:
  - New `db.graph` namespace exposing `setNode`, `addEdge`, `neighbors(graph, node, options)`, and `reachability(graph, startNodes, options)`.
- **Full-Text Search (FTS5)**:
  - New `db.fts` namespace providing BM25-ranked full-text indexing and querying (`db.fts.index`, `db.fts.search`) with highlights and snippet extraction.
- **Keyset Streaming & Async Iteration**:
  - `db.kv.scanStream()` and `db.json.scanStream()` async generator iteration.
  - `db.kv.createReadStream()` and `db.json.createReadStream()` for standard Node.js object-mode streams.
- **Declarative SQL Queries**:
  - `db.sql.query<T>()`, `db.sql.execute()`, and `db.sql.raw()` querying preconfigured views (`lilia_kv`, `lilia_json`, `lilia_graph_nodes`, `lilia_graph_edges`) with transparent decryption.
- **Precompiled Native Addons**:
  - Prebuilt Node-API binaries for `darwin-arm64`, `darwin-x64`, `linux-arm64-gnu`, `linux-x64-gnu`, and `win32-x64-msvc`.

---

## [0.1.0-alpha.1] - 2026-09-10

### Added
- Initial release of `@liliadb/node` featuring embedded SQLite storage and daemon IPC modes.
- Key/Value and JSON document operations with optimistic concurrency control (`ifVersion`).

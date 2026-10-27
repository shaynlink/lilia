# LiliaDB

LiliaDB is a local-first multimodel embedded & daemon database written in Rust. It provides
durable Key/Value, JSON documents, Graph, and Full-Text Search through the same SQLite ACID
transaction boundary, with embedded Rust, Node.js, Python, CLI, and MCP entry points.

## Status: v0.1.0-alpha.2

This repository contains the `0.1.0-alpha.2` release featuring:
- **Encryption at Rest**: Native authenticated encryption (AES-256-GCM AEAD) via the official codec plugin (`plugins/lilia-plugin-codec`), ensuring zero plaintext is stored on disk or in WAL frames, with transparent decryption across KV, JSON, and SQL views.
- **Declarative SQL Engine**: Integrated via `plugins/lilia-plugin-sql`, providing read/write SQL queries, preconfigured views (`lilia_kv`, `lilia_json`, `lilia_graph_nodes`, `lilia_graph_edges`), and transparent decryption functions (`lilia_decode_kv`, `lilia_decode_json`) across CLI (`lilia sql`), Node.js SDK (`db.sql`), and MCP server (`lilia_sql_query`).
- **Physical Crash & Chaos Resilience**: Proven tolerance against physical power loss (torn WAL frame recovery), bit-flips in WAL frame checksums, abrupt process termination (`SIGKILL`) during multi-model transactions, and disk exhaustion (`ENOSPC`) with atomic rollback, lock release, and automatic self-healing.
- **Windows Security Hardening**: Restrictive, protected DACLs (`O:{sid}D:P(...)`) applied to IPC directories, authentication tokens, database files, and backup staging, guaranteeing strict multi-user access denial parity with Unix (`0700`/`0600`).
- **Keyset Streaming & Async Iteration**: Native async generator `scanStream()` and Node.js `createReadStream()` in `@liliadb/node` for unbounded, memory-efficient traversals without offset drift.
- **Official Python SDK & HTTP Gateway**: Zero-dependency `liliadb` Python package (`packages/python`) interfacing with the daemon's local loopback HTTP REST gateway (`--http-port`).
- **Multi-Platform Prebuilds**: Precompiled Node-API binaries for `aarch64-apple-darwin`, `x86_64-apple-darwin`, `x86_64-unknown-linux-gnu`, `x86_64-pc-windows-msvc`, and `aarch64-unknown-linux-gnu`.

## Quick start

Requirements: Rust 1.91.1, Node.js 22 or newer, and pnpm 11.5.2.

```sh
pnpm install
pnpm test
pnpm lilia -- --database ./demo.lilia init --output json
pnpm lilia -- --database ./demo.lilia json put users ada '{"name":"Ada"}' --output json
pnpm lilia -- --database ./demo.lilia json get users ada --output json
pnpm lilia -- --database ./demo.lilia sql "SELECT id, json_extract(value, '$.name') AS name FROM lilia_json WHERE space = 'users'"
pnpm lilia -- --database ./demo.lilia backup ./snapshots/demo.lilia --output json
pnpm lilia -- plugin list
```

### Encryption at Rest (AES-256-GCM Codec Plugin)

Start the CLI or Daemon with native encryption at rest:

```sh
# Set 256-bit encryption key (hex or passphrase)
export LILIA_CODEC_KEY="0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"

# Run CLI with codec plugin: all stored KV and JSON values are encrypted at rest
cargo run -p lilia-cli -- \
  --database ./secure.lilia \
  --plugin ./target/release/liblilia_plugin_codec.dylib \
  kv set vault secret_key "confidential_token"

# Decrypt transparently via SQL
cargo run -p lilia-cli -- \
  --database ./secure.lilia \
  --plugin ./target/release/liblilia_plugin_codec.dylib \
  sql "SELECT key, lilia_decode_kv(value) FROM lilia_kv"
```

### Local Daemon & Python SDK

Run the local daemon on Unix in a private directory with optional loopback HTTP REST gateway:

```sh
cargo run -p lilia-daemon -- \
  --database ./lilia-local/demo.lilia \
  --endpoint ./lilia-local/demo.lilia.sock \
  --token-file ./lilia-local/demo.lilia.token \
  --http-port 4444
```

Connect using the zero-dependency official Python SDK (`packages/python`):

```python
from liliadb import LiliaClient

client = LiliaClient(endpoint="http://127.0.0.1:4444", token="...")
client.json.put("users", "ada", {"name": "Ada Lovelace"})
doc = client.json.get("users", "ada")
print(doc.value)
```

The daemon creates missing IPC directories with mode `0700`; an existing directory
must already be private. CLI default socket/token paths are next to the database,
so CLI daemon commands also require a private database directory unless explicit
IPC paths are supplied. Use physical paths without symlinks (resolve macOS temporary
directory aliases first). Existing socket paths are never removed automatically.
See [IPC filesystem policy](docs/architecture.md#unix-ipc-filesystem-policy).

On Windows, use a local pipe name and a new token subdirectory so the daemon can
create its current-user-only DACL (existing broader ACLs are refused):

```powershell
cargo run -p lilia-daemon -- --database .\lilia-local\demo.lilia --endpoint '\\.\pipe\liliadb-demo' --token-file .\lilia-local\ipc\token
```

### MCP Stdio Server

Run the Model Context Protocol (MCP) server with explicitly allowed database aliases:

```sh
cargo run -p lilia-mcp -- --database demo=./demo.lilia
```

Tools exposed: `lilia_kv_get`, `lilia_kv_set`, `lilia_json_get`, `lilia_json_put`, `lilia_sql_query`, `lilia_backup`, `lilia_integrity_check`, `lilia_checkpoint_stats`.

## Workspace Structure

- `apps/` contains the CLI (`lilia-cli`), local daemon (`lilia-daemon`), and MCP server (`lilia-mcp`).
- `crates/` contains the core engine (`lilia-core`), SQLite storage (`lilia-storage-sqlite`), IPC protocol (`lilia-protocol`), plugin C-ABI & packaging (`lilia-plugin-api`), Graph engine (`lilia-graph`), and Node native addon (`lilia-node-native`).
- `plugins/` contains native model & codec plugins:
  - `lilia-plugin-codec` : AES-256-GCM AEAD encryption at rest.
  - `lilia-plugin-sql` : Declarative SQL engine and custom SQLite decoding scalar functions.
  - `lilia-plugin-kv` & `lilia-plugin-json` : Standalone KV and JSON plugins.
- `packages/` contains published language SDKs:
  - `@liliadb/node` : Node.js SDK with keyset streaming (`packages/node`) and prebuilds (`packages/node-*`).
  - `liliadb` : Python SDK (`packages/python`).
- `examples/` contains runnable examples for Node.js, Python, and native Rust.

See [Architecture](docs/architecture.md), [HTTP REST API](docs/http_api.md), [Python SDK](docs/python_sdk.md), [plugin format](docs/plugins.md), and [security policy](SECURITY.md).

## License

Apache-2.0.

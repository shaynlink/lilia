# Architecture

## Boundaries

`lilia-core` owns database lifecycle, storage migrations, transactions, stable errors, KV, and JSON
contracts. It does not depend on the CLI, Node, daemon, MCP, or plugin loader.

The standard storage adapter is bundled SQLite in WAL mode with `synchronous=FULL`. A process may
run concurrent reads while writes are serialized by SQLite and the in-process writer mutex. LiliaDB
is not supported on network filesystems.

All internal tables use the `_lilia_` prefix. Format version `1` provides:

- `_lilia_metadata` for database format metadata;
- `_lilia_kv` for binary namespaces, keys, values, versions, and optional expiry;
- `_lilia_json` for named JSON spaces, identifiers, values, and versions.

`batch` is the common atomic mutation primitive. An expected version of `0` means create-only;
other expected versions implement optimistic concurrency. A missing expectation is unconditional.

## Runtime modes

- Embedded Rust calls `lilia-core` directly.
- Embedded Node calls `lilia-node-native`; blocking SQLite work runs outside the JavaScript loop.
- Daemon clients use size-prefixed MessagePack over a Unix socket or Windows named pipe.
- CLI uses the embedded core and emits human, JSON, or JSONL output.
- MCP uses stdio and opens only aliases declared by the operator at startup.

The daemon handshake has protocol major/minor `1.0`, a 16 MiB frame limit, per-request UUIDs, and a
random local token stored with user-only permissions. No TCP listener exists in version `0.1`.

## Roadmap boundaries

SQL, Document, Graph, replication, networking, and encryption are not part of the first format.
Their future plugins must use the same transaction boundary and protect `_lilia_*` tables.

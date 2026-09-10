# Architecture

## Boundaries

`lilia-core` owns stable contracts and errors. `lilia-storage-sqlite` owns the standard SQLite
adapter, lifecycle, migrations, transactions, read pool, and writer admission. Neither depends on
the CLI, Node, daemon, MCP, or plugin loader.

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

The daemon handshake has protocol major/minor `1.0`, a 16 MiB frame limit, per-request UUIDs, a
random local token stored with user-only permissions, bounded connections, and idle/deadline
timeouts. No TCP listener exists in version `0.1`.

Daemon requests are authenticated before dispatch, including shutdown. A rejected
shutdown never signals daemon termination. Equal-length tokens are compared in
constant time. Blocking storage operations run outside the async executor.

`deadline_ms` is an optional relative admission budget starting after frame decode.
Zero rejects immediately; the budget is checked again when a blocking worker becomes
available. An expired queued request returns retryable `TIMEOUT` without dispatching
the operation. This is not an execution or response-time limit: it does not interrupt
SQLite work or its internal lock waits after dispatch, and a queued response may wait
for a worker. Once work starts, the daemon returns its actual outcome rather than
claiming that a potentially committed write timed out. Omitting the field imposes no
admission deadline. The existing frame-read idle timeout remains separate.

## Roadmap boundaries

SQL, Document, Graph, replication, networking, and encryption are not part of the first format.
Their future plugins must use the same transaction boundary and protect `_lilia_*` tables.

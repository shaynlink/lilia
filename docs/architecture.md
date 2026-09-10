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

## Unix IPC filesystem policy

The daemon requires an owned, user-only parent directory for each socket and token.
Missing directories are created with mode `0700`; existing directory permissions are
never changed by the IPC setup. Ancestors must be owned by root or the current user
and not writable by others, except root-owned sticky temporary directories. Paths
containing parent traversal or symlinks are rejected, including directory symlinks.
Use physical paths (for example, resolve macOS `/var` or `/tmp` aliases with `realpath`
before passing a generated temporary directory). Relative paths without traversal
are supported. This intentionally rejects older configurations using a shared IPC
directory; choose a new private directory rather than broadening its permissions.
On macOS, extended ACL grants on ancestors, IPC directories, existing tokens or
staging files are refused even when mode bits are private. Deny-only ACLs, such as
the usual home-directory delete restriction, remain supported. This deliberately
rejects even grants to the current user rather than reinterpreting ACL inheritance.

The socket is claimed before credential rotation. Every existing endpoint is refused,
even a stale socket: inspect and remove stale endpoints explicitly after confirming
that no daemon is using them. Cleanup only removes the socket inode created by this
instance. Accepted connections must report the current effective UID via OS peer
credentials; credential lookup failures are refused.

Tokens are written into a mode-`0600` staging file, synced, then atomically published.
Rotation accepts only an owned private regular file with one hard link; symlinks,
hardlinked files, public files and other entry types are rejected without modification.
Normal shutdown retains the token for subsequent atomic replacement on restart.
These checks isolate different unprivileged users, not root or hostile processes
already running as the same user. They are not a general-purpose filesystem sandbox.

## Windows IPC security

The daemon creates local named pipes with an explicit protected DACL granting full
access only to the current process user SID. The first instance claims the name
before token rotation; replacement listeners are created while the connected
instance remains alive. Remote pipe names and remote clients are rejected.

Windows token paths must use a local drive, without traversal, alternate data streams
or reparse points. The final parent must have a protected current-user-only DACL;
missing directories receive that DACL at creation. Existing files with broader or
inherited ACLs are refused, not silently repaired. Directory handles deny delete
sharing and remain open for the daemon lifetime to prevent ancestor replacement.
Ancestors must be owned by the current user, SYSTEM, Administrators or the exact
Windows Modules Installer (TrustedInstaller) service SID and must not
grant other principals rights to modify data, attributes, ACLs, ownership or child
deletion. This also prevents in-place junction conversion; delete sharing alone
does not. Uninterpreted ACL entry types are refused.
Tokens are staged with the private DACL before writing, synced, then renamed over
an existing validated token. Hardlinks and nonregular entries are refused.

Use a new private IPC subdirectory when migrating from earlier Windows builds.
These guarantees exclude administrators with privilege overrides and hostile
same-user processes. Native Windows runtime tests and second-account denial tests
are separate from cross-compilation; neither should be inferred from a macOS test run.

## Backups

`backup(destination)` creates a new snapshot using SQLite's online backup API,
checks its integrity, closes and syncs it, then publishes it atomically without
overwriting any existing entry. Use a fresh destination name for every backup;
existing files, directories, hard links and symlinks are rejected. Staging is in a
private sibling directory and is cleaned on normal success or failure. A process
kill can leave an unpublished `.lilia-backup-*` directory for manual cleanup.
Publication requires a local filesystem supporting hard links; unsupported
filesystems return an error without deleting the destination. Unix publication
also syncs the destination directory. A directory-sync failure can be reported
after the complete snapshot has become visible.

Existing parent-directory permissions are preserved; newly created directories
are user-only on Unix. Destination parent directories must be trusted and must
not be concurrently replaced by an untrusted process; this is not a sandboxed
path API. Windows directory durability and ACL hardening remain separate work.

## Roadmap boundaries

SQL, Document, Graph, replication, networking, and encryption are not part of the first format.
Their future plugins must use the same transaction boundary and protect `_lilia_*` tables.

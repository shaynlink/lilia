# LiliaDB

LiliaDB is a local-first multimodel database written in Rust. The first vertical slice provides
durable key/value and JSON storage through the same SQLite transaction boundary, with embedded,
daemon, Node.js, CLI, and MCP entry points.

## Status

This repository contains the `0.1.0-alpha.1` foundation. SQL, Document, Graph, and encryption are planned
as separately installed native plugins; they are not claimed as implemented yet.

## Quick start

Requirements: Rust 1.91.1, Node.js 22 or newer, and pnpm 11.5.2.

```sh
pnpm install
pnpm test
pnpm lilia -- --database ./demo.lilia init --output json
pnpm lilia -- --database ./demo.lilia json put users ada '{"name":"Ada"}' --output json
pnpm lilia -- --database ./demo.lilia json get users ada --output json
pnpm lilia -- --database ./demo.lilia backup ./snapshots/demo.lilia --output json
```

Run the local daemon on Unix in a new private directory:

```sh
cargo run -p lilia-daemon -- \
  --database ./lilia-local/demo.lilia \
  --endpoint ./lilia-local/demo.lilia.sock \
  --token-file ./lilia-local/demo.lilia.token
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

Run the MCP stdio server with explicitly allowed database aliases:

```sh
cargo run -p lilia-mcp -- --database demo=./demo.lilia
```

## Workspace

- `apps/` contains the CLI, local daemon, and MCP server.
- `crates/` contains the engine, IPC protocol, plugin ABI, and Node native binding.
- `plugins/` contains independently built native model plugins.
- `packages/` contains published language SDKs.

See [Architecture](docs/architecture.md), [plugin format](docs/plugins.md), and
[security policy](SECURITY.md).

The native SDK selects one optional Node-API package for the current platform. Configure
`busyTimeoutMs`, `writerQueueCapacity`, `readPoolSize`, and `checkpointPolicy` when tuning a local
deployment; release performance budgets are calibrated from the alpha baseline.

## License

Apache-2.0.

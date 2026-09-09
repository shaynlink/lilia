# LiliaDB

LiliaDB is a local-first multimodel database written in Rust. The first vertical slice provides
durable key/value and JSON storage through the same SQLite transaction boundary, with embedded,
daemon, Node.js, CLI, and MCP entry points.

## Status

This repository contains the `0.1.0` foundation. SQL, Document, Graph, and encryption are planned
as separately installed native plugins; they are not claimed as implemented yet.

## Quick start

Requirements: Rust 1.91.1, Node.js 22 or newer, and pnpm 11.5.2.

```sh
pnpm install
pnpm test
pnpm lilia -- --database ./demo.lilia init --output json
pnpm lilia -- --database ./demo.lilia json put users ada '{"name":"Ada"}' --output json
pnpm lilia -- --database ./demo.lilia json get users ada --output json
```

Run the local daemon:

```sh
cargo run -p lilia-daemon -- \
  --database ./demo.lilia \
  --endpoint ./demo.lilia.sock \
  --token-file ./demo.lilia.token
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

## License

Apache-2.0.

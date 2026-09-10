# Shared conformance

Run `pnpm nx run lilia-conformance:test` from the workspace root. This builds the
Rust binaries/addon and TypeScript SDK, stages the local addon, runs the Rust
fixture test, then runs the same fixtures through Node embedded, the daemon via
the Node SDK, CLI JSON subprocesses, and an initialized MCP stdio session.
Missing binaries are failures, never skips. Tests use fresh temporary databases
per surface; servers are terminated and awaited before temporary files are removed.

`fixtures.json` is the single expected-result oracle. Its ordered steps cover
missing records, recursive JSON, binary KV values, optimistic updates and conflicts,
multimodel rollback, past expiration, exclusive-cursor pagination, namespace/space
isolation, conditional deletion and invalid names/batches. Both runners validate
error codes, retryability and request IDs. JS adapters additionally check public
`Uint8Array`/`bigint` types before normalizing them to the JSON oracle. Error text,
random request IDs and transport envelopes are not compared verbatim.

CLI invalid JSON and MCP unknown aliases/malformed tool arguments have additional
transport-specific assertions. This is functional model conformance, not a complete
MCP specification validator or a substitute for storage fault-injection tests.

The fixtures deliberately use text-compatible KV keys (the CLI currently accepts
text keys), small exact versions, and an already-expired timestamp rather than
timing-dependent sleeps. Arbitrary binary CLI keys, full-range u64 transport
precision, malformed JSON on every transport, scan mutation interleavings and
all release prebuild installations remain separate coverage.

CI executes this target on five native platforms with Node 24, and Linux x64 with
Node 22/24/26. Nx caching is disabled for this end-to-end target so native artifacts
and child processes are exercised on every invocation. The SQLite test target
explicitly includes the shared fixture file in its cache inputs.

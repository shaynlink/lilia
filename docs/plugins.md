# Native plugin format

Native plugins are trusted, privileged code. A signature establishes provenance and integrity; it
does not sandbox the library.

## Package

A package directory contains:

- `manifest.json`, serialized canonically without unknown fields;
- `manifest.sig`, an Ed25519 signature over the manifest bytes;
- one platform library whose SHA-256 digest is recorded by the manifest.

The manifest identifies name, semantic version, ABI `1.0`, engine requirement, target, library,
digest, and capabilities. It must be the exact compact serialization produced by
`PluginManifest::canonical_bytes`; whitespace or field-order variants are rejected before signature
verification. The engine requirement is a SemVer range which must include the running engine.

Target identifiers are exact Rust target triples. Alpha 1 accepts only:

- `aarch64-apple-darwin`;
- `x86_64-apple-darwin`;
- `aarch64-unknown-linux-gnu`;
- `x86_64-unknown-linux-gnu`;
- `x86_64-pc-windows-msvc`.

The installer rejects absolute and nested paths, traversal, symlinks, unexpected or oversized files,
mismatched digests, unknown signing keys, incompatible targets, duplicate capabilities, and
incompatible ABI versions. The plugin root must be a real private directory (`0700` on Unix).
Installation copies into a random private staging directory, re-verifies the staged bytes, serializes
competing installers with an exclusive destination reservation, and publishes without overwriting an
existing version. Installed packages live at `<root>/<name>-<version>-<target>/package`. Loading
occurs only at process startup, and the exported descriptor must declare exactly the capabilities
present in the signed manifest.

## ABI & Capabilities

Each library exports `lilia_plugin_v1` using the C ABI and returns `PluginDescriptorV1`. No Rust ABI,
panic, owned Rust value, or allocator boundary may cross the interface. Capabilities are bitflags:

- `1 << 0`: `kv`
- `1 << 1`: `json`
- `1 << 2`: `sql`
- `1 << 3`: `document`
- `1 << 4`: `graph`
- `1 << 5`: `codec` (authenticated storage encryption at rest)

## Trust & Signatures

Official release packages are signed with the embedded LiliaDB official release key, which is
trusted by default out of the box. Third-party keys live in the per-user trust store shared by
the CLI and daemon:

- macOS: `~/Library/Application Support/LiliaDB/plugins/.trust`;
- Linux: `$XDG_DATA_HOME/liliadb/plugins/.trust`, or `~/.local/share/liliadb/plugins/.trust`;
- Windows: `%LOCALAPPDATA%\\LiliaDB\\plugins\\.trust`.

`lilia plugin trust add <base64-public-key>` displays its SHA-256 fingerprint and requires an
interactive confirmation; structured and non-interactive callers must pass `--yes`. Listing returns
fingerprints only. Each key is staged privately and atomically published as `<fingerprint>.pub`, so
an interrupted addition is ignored. Symlinked, malformed, public-on-Unix, and fingerprint-mismatched
stores are rejected.

Production always installs and loads from this OS-specific root. Alternate roots and inline
`--trusted-key` values require `--development`. Unsigned packages fail closed by default; the CLI
requires both `--allow-unsigned --development`, and the daemon equivalently requires
`--allow-unsigned-plugins --development`. Production launchers must never enable these pairs.

## CLI Packaging & Lifecycle Tooling

LiliaDB includes built-in commands for the entire plugin packaging and distribution lifecycle:

```sh
# 1. Generate an Ed25519 signing keypair
lilia plugin keygen --out-dir ./keys --name my-author

# 2. Package a compiled dynamic library with canonical manifest.json
lilia plugin pack ./target/release/libmy_plugin.dylib \
  --name lilia-plugin-custom \
  --version 0.1.0 \
  --capability codec \
  --out-dir ./dist/lilia-plugin-custom-package

# 3. Cryptographically sign the package with your private key (produces manifest.sig)
lilia plugin sign ./dist/lilia-plugin-custom-package --key ./keys/my-author.key

# 4. Verify package integrity and signature against trusted keys
lilia plugin verify ./dist/lilia-plugin-custom-package

# 5. Install into the local plugin store (use --force to overwrite an existing version)
lilia plugin install ./dist/lilia-plugin-custom-package [--force]

# 6. List installed plugins (supports human ASCII table, JSON, and JSONL)
lilia plugin list

# 7. Remove an installed plugin
lilia plugin remove lilia-plugin-custom-0.1.0-aarch64-apple-darwin
```

## Plugin Resolution Priority

When launching the CLI or Daemon with `--plugin <ARG>`, LiliaDB resolves the plugin using a
strict 3-step priority order:

1. **Direct filesystem path**: If `<ARG>` exists on disk (as a `.dylib`/`.so`/`.dll` library file or
   a directory containing `manifest.json` / `package/manifest.json`), it is loaded directly.
2. **Exact plugin name**: If not found on disk, the installed plugin store is searched for an
   exact match on package ID or plugin name (e.g. `lilia-plugin-codec`).
3. **Short name or capability**: If still unresolved, the store is searched for a matching short
   name (e.g. `codec`) or capability (e.g. `sql`).


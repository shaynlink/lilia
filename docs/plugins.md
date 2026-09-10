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

## ABI

Each library exports `lilia_plugin_v1` using the C ABI and returns `PluginDescriptorV1`. No Rust ABI,
panic, owned Rust value, or allocator boundary may cross the interface. Capability bits `1` and `2`
currently identify KV and JSON.

Official packages will use an embedded release trust key. Operators can pass additional verifying
keys explicitly. Unsigned packages fail closed by default. The CLI requires both
`--allow-unsigned --development`; the daemon equivalently requires
`--allow-unsigned-plugins --development`. Production launchers must never enable these pairs.

# Native plugin format

Native plugins are trusted, privileged code. A signature establishes provenance and integrity; it
does not sandbox the library.

## Package

A package directory contains:

- `manifest.json`, serialized canonically without unknown fields;
- `manifest.sig`, an Ed25519 signature over the manifest bytes;
- one platform library whose SHA-256 digest is recorded by the manifest.

The manifest identifies name, semantic version, ABI `1.0`, engine requirement, target, library,
digest, and capabilities. Target identifiers use `<architecture>-<operating-system>` in `0.1`.

The installer rejects absolute paths, traversal, symlink escapes, oversized files, mismatched
digests, unknown signing keys, incompatible targets, and incompatible ABI versions. Installation
uses a staging directory followed by an atomic rename. Loading occurs only at process startup.

## ABI

Each library exports `lilia_plugin_v1` using the C ABI and returns `PluginDescriptorV1`. No Rust ABI,
panic, owned Rust value, or allocator boundary may cross the interface. Capability bits `1` and `2`
currently identify KV and JSON.

Official packages will use an embedded release trust key. Operators can pass additional verifying
keys explicitly. `--allow-unsigned` is a development-only escape hatch and must not be enabled in a
production launcher.

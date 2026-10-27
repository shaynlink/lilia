# Security policy

## Supported versions

Only the latest `0.1.x` release is supported while LiliaDB is pre-1.0.

## Reporting

Do not open a public issue for a suspected vulnerability. Use GitHub private vulnerability
reporting for `shaynlink/lilia` and include affected version, reproduction, impact, and suggested
mitigation when available.

## Security model

### 1. Encryption at Rest (Authenticated AEAD)
- **Algorithm**: Authenticated Encryption with Associated Data (AEAD) using AES-256-GCM via `plugins/lilia-plugin-codec`.
- **Nonces & Authentication Tags**: Every mutation generates a cryptographically secure 96-bit random nonce. A 128-bit MAC authentication tag verifies data integrity upon read; any bit-flip or ciphertext tampering is immediately detected, rejecting the read with `ErrorCode::Corrupt`.
- **Zero Plaintext Storage**: Raw values are encrypted before writing to SQLite pages. Neither the primary database file nor the SQLite write-ahead log (`-wal`) contains unencrypted plaintext.
- **Key Management**: Keys are provided out-of-band via the `LILIA_CODEC_KEY` environment variable (as a 64-character hexadecimal 256-bit key or a high-entropy secret derived via SHA-256) or initialized directly via the C-ABI `codec_init` function. Keys are held in memory only and never written to disk or logs.

### 2. Filesystem & DACL Hardening (Unix & Windows Parity)
- **Unix**: All database files, IPC socket paths, and authentication token files are created with mode `0600`; directories receive mode `0700`.
- **Windows**: All database files, IPC named pipes, token paths, and backup staging directories receive a protected, restrictive DACL (`O:{sid}D:P(A;OICI;FA;;;{sid})` for directories, `O:{sid}D:P(A;;FA;;;{sid})` for files) granting access exclusively to the current process user SID. Inheritance from parent volumes is explicitly disabled (`SE_DACL_PROTECTED`), and existing broader ACLs or unauthorized account permissions are rejected on startup.

### 3. IPC & Daemon Authentication
- **Local Loopback Only**: The daemon binds strictly to local Unix domain sockets, local Windows named pipes (`\\.\pipe\...`), or local loopback HTTP (`127.0.0.1`). Remote pipes and network-exposed IPC endpoints are strictly rejected.
- **Per-Session Bearer Tokens**: Daemon interactions require a cryptographically random token generated at daemon initialization. Token files are created with private DACLs and verified on every IPC and HTTP request.
- **MCP Surface Restrictions**: The Model Context Protocol (MCP) server accesses only database aliases explicitly declared via `--database <alias>=<path>` at process launch.

### 4. Native Plugin Signature & Capability Verification
- **Cryptographic Signatures**: Native dynamic plugins (`.dylib`, `.so`, `.dll`) packaged with `lilia plugin package` must be cryptographically signed using Ed25519.
- **Signature Verification**: `lilia plugin install` and `lilia plugin verify` compute the SHA-256 digest of both the manifest and the shared library, verifying the Ed25519 signature against trusted public keys before permitting execution.
- **Capability Isolation**: Plugins explicitly declare their capabilities (e.g. `Capability::Codec`, `Capability::Sql`, `Capability::ModelKv`). Descriptors are validated against their manifest declarations.

### 5. Memory, Log, and Crash Hygiene
- **Zero Sensitive Data Logging**: Secret keys, authentication tokens, and user database payloads are strictly excluded from debug logs, error messages, and metrics.
- **Request Tracing**: All errors are assigned random request UUIDs (`request_id`) for diagnostics without exposing sensitive contextual payloads.
- **Physical Crash Resilience**: Incomplete writes (torn WAL frames) and bit-flipped storage sectors are discarded during WAL recovery, guaranteeing that corrupt physical data never silently enters active memory or overwrites the database.

Before production use, keep dependencies current, verify plugin signatures, back up using `backup(destination)` (which uses SQLite's online backup API and verifies snapshot integrity), and never place a WAL database on an untrusted network filesystem.

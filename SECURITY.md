# Security policy

## Supported versions

Only the latest `0.1.x` release is supported while LiliaDB is pre-1.0.

## Reporting

Do not open a public issue for a suspected vulnerability. Use GitHub private vulnerability
reporting for `shaynlink/lilia` and include affected version, reproduction, impact, and suggested
mitigation when available.

## Security model

- Database directories and daemon credentials are restricted to the current OS user.
- The daemon exposes local IPC only and authenticates every request with a random token.
- MCP can access only database aliases explicitly declared at startup.
- Native plugins execute with full process privileges and must be signed by a trusted key.
- Logs and errors must not include stored values, daemon tokens, or signing private keys.
- LiliaDB does not provide encryption at rest in `0.1`; use encrypted storage until the codec plugin
  is available.

Before production use, keep dependencies current, verify plugin signatures, back up with a
checkpoint-aware SQLite backup, and never place a WAL database on a network filesystem.

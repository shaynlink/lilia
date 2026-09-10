use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use clap::Parser;
use lilia_core::{ErrorCode, LiliaError};
use lilia_protocol::{
    read_frame, write_frame, Operation, Request, Response, ResponseValue, PROTOCOL_MAJOR,
    PROTOCOL_MINOR,
};
use lilia_storage_sqlite::{Database, DatabaseOptions};
use subtle::ConstantTimeEq;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{Notify, Semaphore};
use tracing::info;
use uuid::Uuid;

#[cfg(test)]
mod tests;

#[cfg(unix)]
mod ipc_unix;

#[cfg(windows)]
mod ipc_windows;

#[derive(Debug, Parser)]
#[command(version, about = "LiliaDB local daemon")]
struct Arguments {
    #[arg(long)]
    database: PathBuf,
    #[arg(long)]
    endpoint: PathBuf,
    #[arg(long)]
    token_file: PathBuf,
    #[arg(long = "plugin")]
    plugins: Vec<PathBuf>,
    #[arg(long = "trusted-key")]
    trusted_keys: Vec<String>,
    #[arg(long)]
    plugin_root: Option<PathBuf>,
    #[arg(long)]
    allow_unsigned_plugins: bool,
    /// Explicitly enable development-only plugin policy exceptions.
    #[arg(long)]
    development: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("lilia_daemon=info")
        .with_writer(std::io::stderr)
        .init();
    let arguments = Arguments::parse();
    let _plugins = load_plugins(&arguments)?;
    // Claim the endpoint before rotating credentials; duplicate startup must not
    // invalidate the token of an already running daemon.
    #[cfg(unix)]
    let listener = ipc_unix::ListenerGuard::bind(&arguments.endpoint)?;
    #[cfg(windows)]
    let security = ipc_windows::UserSecurity::new()?;
    #[cfg(windows)]
    let listener = security.pipe(&arguments.endpoint, true)?;
    #[cfg(unix)]
    let token = create_token(&arguments.token_file)?;
    #[cfg(windows)]
    let credentials = security.credentials(&arguments.token_file)?;
    #[cfg(windows)]
    let token = credentials.token.clone();
    let database = Arc::new(Database::open(DatabaseOptions::durable(
        &arguments.database,
    ))?);
    let shutdown = Arc::new(Notify::new());
    info!(database = %arguments.database.display(), "daemon started");
    #[cfg(unix)]
    return serve(listener, database, token, shutdown).await;
    #[cfg(windows)]
    serve(
        &arguments.endpoint,
        listener,
        &security,
        database,
        token,
        shutdown,
    )
    .await
}

fn load_plugins(arguments: &Arguments) -> anyhow::Result<Vec<lilia_plugin_api::LoadedPlugin>> {
    use base64::Engine;

    if arguments.allow_unsigned_plugins && !arguments.development {
        anyhow::bail!("--allow-unsigned-plugins requires explicit --development mode");
    }

    let mut encoded_keys = arguments.trusted_keys.clone();
    if let Some(root) = &arguments.plugin_root {
        let path = root.join("trust-keys.json");
        if path.exists() {
            encoded_keys.extend(serde_json::from_slice::<Vec<String>>(&fs::read(path)?)?);
        }
    }
    let trusted_keys = encoded_keys
        .iter()
        .map(|encoded| {
            let bytes = base64::engine::general_purpose::STANDARD.decode(encoded)?;
            let bytes: [u8; 32] = bytes
                .try_into()
                .map_err(|_| anyhow::anyhow!("trusted keys must be 32 bytes"))?;
            ed25519_dalek::VerifyingKey::from_bytes(&bytes).map_err(Into::into)
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    arguments
        .plugins
        .iter()
        .map(|path| {
            if let Some(root) = &arguments.plugin_root {
                let canonical_root = root.canonicalize()?;
                let canonical_plugin = path.canonicalize()?;
                if !canonical_plugin.starts_with(canonical_root) {
                    anyhow::bail!("plugin path is outside the authorized plugin root");
                }
            }
            let manifest = lilia_plugin_api::verify_package(
                path,
                &trusted_keys,
                arguments.allow_unsigned_plugins,
            )?;
            load_trusted_descriptor(path, &manifest)
        })
        .collect()
}

#[allow(unsafe_code)]
fn load_trusted_descriptor(
    path: &Path,
    manifest: &lilia_plugin_api::PluginManifest,
) -> anyhow::Result<lilia_plugin_api::LoadedPlugin> {
    // Signature verification happens immediately before this call. Native plugins remain
    // privileged code, as documented in SECURITY.md and docs/plugins.md.
    unsafe { lilia_plugin_api::load_descriptor(path, manifest).map_err(Into::into) }
}

#[cfg(unix)]
fn create_token(path: &Path) -> std::io::Result<String> {
    ipc_unix::create_token(path)
}

#[cfg(unix)]
async fn serve(
    listener: ipc_unix::ListenerGuard,
    database: Arc<Database>,
    token: String,
    shutdown: Arc<Notify>,
) -> anyhow::Result<()> {
    let connections = Arc::new(Semaphore::new(64));
    loop {
        tokio::select! {
            accepted = listener.listener.accept() => {
                let (stream, _) = accepted?;
                if !stream.peer_cred().is_ok_and(|peer| ipc_unix::same_user(peer.uid())) {
                    continue;
                }
                let database = Arc::clone(&database);
                let token = token.clone();
                let shutdown = Arc::clone(&shutdown);
                let Ok(permit) = connections.clone().try_acquire_owned() else { continue };
                tokio::spawn(async move {
                    let _permit = permit;
                    if let Err(error) = handle_stream(stream, database, &token, shutdown).await {
                        tracing::warn!(%error, "client disconnected with error");
                    }
                });
            }
            () = shutdown.notified() => break,
        }
    }
    close_database(database).await
}

#[cfg(windows)]
async fn serve(
    endpoint: &Path,
    mut server: tokio::net::windows::named_pipe::NamedPipeServer,
    security: &ipc_windows::UserSecurity,
    database: Arc<Database>,
    token: String,
    shutdown: Arc<Notify>,
) -> anyhow::Result<()> {
    let connections = Arc::new(Semaphore::new(64));
    loop {
        tokio::select! {
            connected = server.connect() => {
                connected?;
                // Keep an instance alive continuously, even when rejecting a
                // connection under load. Never release the pipe namespace.
                let next = security.pipe(endpoint, false)?;
                let accepted = std::mem::replace(&mut server, next);
                let database = Arc::clone(&database);
                let token = token.clone();
                let shutdown = Arc::clone(&shutdown);
                let Ok(permit) = connections.clone().try_acquire_owned() else { continue };
                tokio::spawn(async move {
                    let _permit = permit;
                    handle_stream(accepted, database, &token, shutdown).await.ok();
                });
            }
            () = shutdown.notified() => break,
        }
    }
    close_database(database).await
}

async fn close_database(database: Arc<Database>) -> anyhow::Result<()> {
    tokio::task::spawn_blocking(move || database.close(std::time::Duration::from_secs(5)))
        .await??;
    Ok(())
}

async fn handle_stream<S>(
    mut stream: S,
    database: Arc<Database>,
    token: &str,
    shutdown: Arc<Notify>,
) -> anyhow::Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    loop {
        let request: Request =
            match tokio::time::timeout(Duration::from_secs(30), read_frame(&mut stream)).await {
                Err(_) => return Err(anyhow::anyhow!("client idle timeout")),
                Ok(Ok(request)) => request,
                Ok(Err(error)) if error.kind() == std::io::ErrorKind::UnexpectedEof => {
                    return Ok(())
                }
                Ok(Err(error)) => return Err(error.into()),
            };
        let operation_name = operation_name(&request.operation);
        let request_id = request.id.clone();
        let should_shutdown = matches!(request.operation, Operation::Shutdown);
        let started = Instant::now();
        let request_uuid = Uuid::parse_str(&request.id).unwrap_or_else(|_| Uuid::new_v4());
        let result = if bool::from(request.token.as_bytes().ct_eq(token.as_bytes())) {
            execute(
                Arc::clone(&database),
                request.operation,
                started,
                request.deadline_ms,
            )
            .await
        } else {
            Err(LiliaError::new(
                ErrorCode::Unauthorized,
                "invalid daemon token",
                false,
            ))
        };
        let result = result.map_err(|error| error.with_request_id(request_uuid));
        let success = result.is_ok();
        write_frame(
            &mut stream,
            &Response {
                id: request.id,
                result,
            },
        )
        .await?;
        info!(
            request_id,
            operation = operation_name,
            success,
            elapsed_us = started.elapsed().as_micros(),
            "request completed"
        );
        if should_shutdown && success {
            shutdown.notify_one();
            return Ok(());
        }
    }
}

fn check_deadline(started: Instant, deadline_ms: Option<u64>) -> lilia_core::Result<()> {
    if deadline_ms.is_some_and(|ms| started.elapsed() >= Duration::from_millis(ms)) {
        return Err(LiliaError::new(
            ErrorCode::Timeout,
            "request deadline exceeded",
            true,
        ));
    }
    Ok(())
}

async fn execute(
    database: Arc<Database>,
    operation: Operation,
    started: Instant,
    deadline_ms: Option<u64>,
) -> lilia_core::Result<ResponseValue> {
    check_deadline(started, deadline_ms)?;
    tokio::task::spawn_blocking(move || {
        // Include worker and FIFO waiting time. Once the transaction starts, return
        // its actual outcome: timing out a running write could hide a committed mutation.
        check_deadline(started, deadline_ms)?;
        if let Operation::Batch { operations } = operation {
            let deadline = deadline_ms
                .and_then(|ms| started.checked_add(std::time::Duration::from_millis(ms)));
            return database
                .batch_with_deadline(&operations, deadline)
                .map(ResponseValue::Mutations);
        }
        dispatch(&database, operation)
    })
    .await
    .map_err(|_| LiliaError::new(ErrorCode::Storage, "request worker failed", false))?
}

fn operation_name(operation: &Operation) -> &'static str {
    match operation {
        Operation::Handshake { .. } => "handshake",
        Operation::Shutdown => "shutdown",
        Operation::IntegrityCheck => "integrity_check",
        Operation::Backup { .. } => "backup",
        Operation::KvGet { .. } => "kv_get",
        Operation::KvScan { .. } => "kv_scan",
        Operation::JsonGet { .. } => "json_get",
        Operation::JsonScan { .. } => "json_scan",
        Operation::Batch { .. } => "batch",
    }
}

fn dispatch(database: &Database, operation: Operation) -> lilia_core::Result<ResponseValue> {
    Ok(match operation {
        Operation::Handshake { major, minor: _ } if major == PROTOCOL_MAJOR => {
            ResponseValue::Handshake {
                major: PROTOCOL_MAJOR,
                minor: PROTOCOL_MINOR,
                capabilities: vec!["kv".into(), "json".into(), "batch".into()],
            }
        }
        Operation::Handshake { .. } => {
            return Err(LiliaError::new(
                ErrorCode::Unsupported,
                "incompatible protocol major",
                false,
            ));
        }
        Operation::Shutdown => ResponseValue::Shutdown,
        Operation::IntegrityCheck => ResponseValue::Integrity(database.integrity_check()?),
        Operation::Backup { destination } => {
            database.backup(destination)?;
            ResponseValue::Ack
        }
        Operation::KvGet { namespace, key } => {
            ResponseValue::Kv(database.kv_get(&namespace, &key)?)
        }
        Operation::KvScan {
            namespace,
            after,
            limit,
        } => ResponseValue::KvPage(database.kv_scan(&namespace, after.as_deref(), limit)?),
        Operation::JsonGet { space, id } => ResponseValue::Json(database.json_get(&space, &id)?),
        Operation::JsonScan {
            space,
            after,
            limit,
        } => ResponseValue::JsonPage(database.json_scan(&space, after.as_deref(), limit)?),
        Operation::Batch { operations } => ResponseValue::Mutations(database.batch(&operations)?),
    })
}

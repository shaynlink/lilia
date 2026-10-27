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
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite};
use tokio::sync::{Notify, Semaphore};
use tracing::info;
use uuid::Uuid;

#[cfg(test)]
mod tests;

#[cfg(unix)]
mod ipc_unix;

#[cfg(windows)]
mod ipc_windows;

mod http;

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
    /// Optional port for local loopback HTTP REST gateway (127.0.0.1:<PORT>).
    #[arg(long = "http-port")]
    http_port: Option<u16>,
    /// Optional custom socket address for local loopback HTTP REST gateway.
    #[arg(long = "http-bind")]
    http_bind: Option<std::net::SocketAddr>,
    /// Optional passphrase or key to enable AES-256-GCM encryption at rest.
    #[arg(long = "encryption-key")]
    encryption_key: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("lilia_daemon=info")
        .with_writer(std::io::stderr)
        .init();
    let arguments = Arguments::parse();
    let plugins = load_plugins(&arguments)?;
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
    let mut options = DatabaseOptions::durable(&arguments.database);
    for plugin in &plugins {
        if let Some(codec) = plugin.codec() {
            info!(codec = %codec.name(), "enabled plugin storage codec");
            options = options.with_codec(Arc::new(codec));
            break;
        }
    }
    let encryption_key = arguments
        .encryption_key
        .clone()
        .or_else(|| std::env::var("LILIA_ENCRYPTION_KEY").ok());
    if let Some(ref key) = encryption_key {
        info!("enabled AES-256-GCM at-rest encryption codec");
        options = options.with_codec(Arc::new(lilia_core::Aes256GcmCodec::from_passphrase(key)));
    }
    let database = Arc::new(Database::open(options)?);
    let shutdown = Arc::new(Notify::new());
    info!(database = %arguments.database.display(), "daemon started");

    let http_addr = resolve_http_bind(&arguments)?;
    if let Some(addr) = http_addr {
        let http_listener = tokio::net::TcpListener::bind(addr).await?;
        info!(addr = %addr, "http gateway started");
        let http_database = database.clone();
        let http_token = token.clone();
        let http_shutdown = shutdown.clone();
        tokio::spawn(async move {
            if let Err(e) =
                http::serve_http(http_listener, http_database, http_token, http_shutdown).await
            {
                tracing::error!(error = %e, "http gateway error");
            }
        });
    }

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

fn resolve_http_bind(arguments: &Arguments) -> anyhow::Result<Option<std::net::SocketAddr>> {
    if let Some(addr) = arguments.http_bind {
        if !addr.ip().is_loopback() && !arguments.development {
            anyhow::bail!(
                "HTTP gateway can only bind to loopback addresses (127.0.0.1 or ::1) unless in development mode"
            );
        }
        return Ok(Some(addr));
    }
    if let Some(port) = arguments.http_port {
        return Ok(Some(std::net::SocketAddr::from(([127, 0, 0, 1], port))));
    }
    Ok(None)
}

fn load_plugins(arguments: &Arguments) -> anyhow::Result<Vec<lilia_plugin_api::LoadedPlugin>> {
    use base64::Engine;

    if arguments.allow_unsigned_plugins && !arguments.development {
        anyhow::bail!("--allow-unsigned-plugins requires explicit --development mode");
    }
    if arguments.plugin_root.is_some() && !arguments.development {
        anyhow::bail!("--plugin-root requires explicit --development mode");
    }
    if !arguments.trusted_keys.is_empty() && !arguments.development {
        anyhow::bail!("--trusted-key requires explicit --development mode");
    }
    if arguments.plugins.is_empty() {
        return Ok(Vec::new());
    }

    let root = arguments
        .plugin_root
        .clone()
        .map_or_else(lilia_plugin_api::default_plugin_root, Ok)?;
    let mut trusted_keys = vec![lilia_plugin_api::official_release_key()];
    if let Ok(store) = lilia_plugin_api::TrustStore::open(&root) {
        if let Ok(keys) = store.keys() {
            trusted_keys.extend(keys);
        }
    }
    trusted_keys.extend(
        arguments
            .trusted_keys
            .iter()
            .map(|encoded| {
                let bytes = base64::engine::general_purpose::STANDARD.decode(encoded)?;
                let bytes: [u8; 32] = bytes
                    .try_into()
                    .map_err(|_| anyhow::anyhow!("trusted keys must be 32 bytes"))?;
                ed25519_dalek::VerifyingKey::from_bytes(&bytes).map_err(Into::into)
            })
            .collect::<anyhow::Result<Vec<_>>>()?,
    );
    let canonical_root = root.canonicalize()?;
    arguments
        .plugins
        .iter()
        .map(|path| {
            let resolved_path = if path.exists() {
                if path.join("package").join("manifest.json").is_file() {
                    path.join("package")
                } else {
                    path.clone()
                }
            } else if let Some(installed) =
                lilia_plugin_api::find_installed(&root, path.to_str().unwrap_or_default())?
            {
                installed.package_path
            } else {
                anyhow::bail!(
                    "plugin `{}` not found as a file or directory, and not installed in `{}`",
                    path.display(),
                    root.display()
                );
            };

            let canonical_plugin = resolved_path.canonicalize()?;
            if !canonical_plugin.starts_with(&canonical_root) {
                anyhow::bail!("plugin path is outside the authorized plugin root");
            }
            let manifest = lilia_plugin_api::verify_package(
                &canonical_plugin,
                &trusted_keys,
                arguments.allow_unsigned_plugins,
            )?;
            load_trusted_descriptor(&canonical_plugin, &manifest)
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

#[allow(clippy::too_many_lines)]
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

        if let Operation::Subscribe { namespace } = request.operation {
            let authorized = bool::from(request.token.as_bytes().ct_eq(token.as_bytes()));
            if !authorized {
                let err = LiliaError::new(ErrorCode::Unauthorized, "invalid daemon token", false)
                    .with_request_id(request_uuid);
                write_frame(
                    &mut stream,
                    &Response {
                        id: request.id,
                        result: Err(err),
                    },
                )
                .await?;
                return Ok(());
            }
            write_frame(
                &mut stream,
                &Response {
                    id: request.id.clone(),
                    result: Ok(ResponseValue::Ack),
                },
            )
            .await?;
            info!(
                request_id,
                operation = "subscribe",
                namespace = namespace.as_deref().unwrap_or("*"),
                "client subscription started"
            );
            let mut rx = database.subscribe();
            let mut read_buf = [0u8; 1];
            loop {
                tokio::select! {
                    evt = rx.recv() => {
                        match evt {
                            Ok(event) => {
                                if event.matches_filter(namespace.as_deref())
                                    && write_frame(
                                        &mut stream,
                                        &Response {
                                            id: request.id.clone(),
                                            result: Ok(ResponseValue::Event(event)),
                                        },
                                    )
                                    .await
                                    .is_err()
                                {
                                    break;
                                }
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                                tracing::warn!(skipped, "ipc subscriber lagged behind events");
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                        }
                    }
                    res = stream.read(&mut read_buf) => {
                        match res {
                            Ok(0) | Err(_) => break,
                            _ => {}
                        }
                    }
                    () = shutdown.notified() => break,
                }
            }
            info!(request_id, "client subscription closed");
            return Ok(());
        }

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
        Operation::SqlQuery { .. } => "sql_query",
        Operation::GraphNeighbors { .. } => "graph_neighbors",
        Operation::GraphReachability { .. } => "graph_reachability",
        Operation::FtsSearch { .. } => "fts_search",
        Operation::Subscribe { .. } => "subscribe",
    }
}

fn dispatch(database: &Database, operation: Operation) -> lilia_core::Result<ResponseValue> {
    Ok(match operation {
        Operation::Handshake { major, minor: _ } if major == PROTOCOL_MAJOR => {
            ResponseValue::Handshake {
                major: PROTOCOL_MAJOR,
                minor: PROTOCOL_MINOR,
                capabilities: vec![
                    "kv".into(),
                    "json".into(),
                    "batch".into(),
                    "sql".into(),
                    "graph".into(),
                    "fts".into(),
                    "watch".into(),
                ],
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
        Operation::Subscribe { .. } => ResponseValue::Ack,
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
        Operation::SqlQuery { sql, params } => {
            ResponseValue::Sql(database.sql_query(&sql, &params)?)
        }
        Operation::GraphNeighbors {
            graph,
            node,
            direction,
            kind,
        } => ResponseValue::GraphNeighbors(database.graph_neighbors(
            &graph,
            &node,
            direction,
            kind.as_deref(),
        )?),
        Operation::GraphReachability {
            graph,
            start_nodes,
            direction,
        } => ResponseValue::GraphReachability(database.graph_reachability(
            &graph,
            &start_nodes,
            direction,
        )?),
        Operation::FtsSearch {
            collection,
            query,
            limit,
        } => ResponseValue::FtsHits(database.fts_search(&collection, &query, limit)?),
    })
}

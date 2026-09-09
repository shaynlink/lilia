use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use clap::Parser;
use lilia_core::{ErrorCode, LiliaError};
use lilia_protocol::{
    read_frame, write_frame, Operation, Request, Response, ResponseValue, PROTOCOL_MAJOR,
    PROTOCOL_MINOR,
};
use lilia_storage_sqlite::{Database, DatabaseOptions};
use rand::RngCore;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::Notify;
use tracing::info;

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
    allow_unsigned_plugins: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("lilia_daemon=info")
        .with_writer(std::io::stderr)
        .init();
    let arguments = Arguments::parse();
    let _plugins = load_plugins(&arguments)?;
    let token = create_token(&arguments.token_file)?;
    let database = Arc::new(Database::open(DatabaseOptions::durable(
        &arguments.database,
    ))?);
    let shutdown = Arc::new(Notify::new());
    info!(database = %arguments.database.display(), "daemon started");
    serve(&arguments.endpoint, database, token, shutdown).await
}

fn load_plugins(arguments: &Arguments) -> anyhow::Result<Vec<lilia_plugin_api::LoadedPlugin>> {
    use base64::Engine;

    let trusted_keys = arguments
        .trusted_keys
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

fn create_token(path: &Path) -> std::io::Result<String> {
    let mut bytes = [0_u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    let token = hex::encode(bytes);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, &token)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(token)
}

#[cfg(unix)]
async fn serve(
    endpoint: &Path,
    database: Arc<Database>,
    token: String,
    shutdown: Arc<Notify>,
) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    use tokio::net::UnixListener;

    if endpoint.exists() {
        fs::remove_file(endpoint)?;
    }
    if let Some(parent) = endpoint.parent() {
        fs::create_dir_all(parent)?;
    }
    let listener = UnixListener::bind(endpoint)?;
    fs::set_permissions(endpoint, fs::Permissions::from_mode(0o600))?;
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (stream, _) = accepted?;
                let database = Arc::clone(&database);
                let token = token.clone();
                let shutdown = Arc::clone(&shutdown);
                tokio::spawn(async move {
                    if let Err(error) = handle_stream(stream, database, &token, shutdown).await {
                        tracing::warn!(%error, "client disconnected with error");
                    }
                });
            }
            () = shutdown.notified() => break,
        }
    }
    fs::remove_file(endpoint).ok();
    Ok(())
}

#[cfg(windows)]
async fn serve(
    endpoint: &Path,
    database: Arc<Database>,
    token: String,
    shutdown: Arc<Notify>,
) -> anyhow::Result<()> {
    use tokio::net::windows::named_pipe::ServerOptions;

    let pipe_name = endpoint.to_string_lossy().into_owned();
    loop {
        let server = ServerOptions::new().create(&pipe_name)?;
        tokio::select! {
            connected = server.connect() => {
                connected?;
                let database = Arc::clone(&database);
                let token = token.clone();
                let shutdown = Arc::clone(&shutdown);
                tokio::spawn(async move {
                    handle_stream(server, database, &token, shutdown).await.ok();
                });
            }
            () = shutdown.notified() => break,
        }
    }
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
        let request: Request = match read_frame(&mut stream).await {
            Ok(request) => request,
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        let operation_name = operation_name(&request.operation);
        let request_id = request.id.clone();
        let should_shutdown = matches!(request.operation, Operation::Shutdown);
        let started = Instant::now();
        let result = if request.token == token {
            dispatch(&database, request.operation)
        } else {
            Err(LiliaError::new(
                ErrorCode::Unauthorized,
                "invalid daemon token",
                false,
            ))
        };
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
        if should_shutdown {
            shutdown.notify_one();
            return Ok(());
        }
    }
}

fn operation_name(operation: &Operation) -> &'static str {
    match operation {
        Operation::Handshake { .. } => "handshake",
        Operation::Shutdown => "shutdown",
        Operation::IntegrityCheck => "integrity_check",
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

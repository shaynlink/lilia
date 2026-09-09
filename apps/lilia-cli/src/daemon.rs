#[cfg(windows)]
use std::collections::hash_map::DefaultHasher;
use std::fs;
#[cfg(windows)]
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use lilia_protocol::{
    read_frame, write_frame, Operation, Request, Response, ResponseValue, PROTOCOL_MAJOR,
    PROTOCOL_MINOR,
};
use serde_json::{json, Value};
use tokio::io::{AsyncRead, AsyncWrite};
use uuid::Uuid;

use crate::args::DaemonCommand;

pub(crate) fn execute(command: DaemonCommand, database: &Path) -> anyhow::Result<Value> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    match command {
        DaemonCommand::Start {
            endpoint,
            token_file,
            foreground,
        } => {
            let endpoint = endpoint.unwrap_or_else(|| default_endpoint(database));
            let token_file = token_file.unwrap_or_else(|| default_token(database));
            start(database, &endpoint, &token_file, foreground)?;
            if foreground {
                return Ok(json!({"started": true, "foreground": true}));
            }
            wait_for_token(&token_file)?;
            let response = runtime.block_on(call(
                &endpoint,
                &token_file,
                Operation::Handshake {
                    major: PROTOCOL_MAJOR,
                    minor: PROTOCOL_MINOR,
                },
            ))?;
            Ok(json!({"started": true, "endpoint": endpoint, "response": response}))
        }
        DaemonCommand::Status {
            endpoint,
            token_file,
        } => {
            let response = runtime.block_on(call(
                &endpoint.unwrap_or_else(|| default_endpoint(database)),
                &token_file.unwrap_or_else(|| default_token(database)),
                Operation::Handshake {
                    major: PROTOCOL_MAJOR,
                    minor: PROTOCOL_MINOR,
                },
            ))?;
            Ok(json!({"running": true, "response": response}))
        }
        DaemonCommand::Stop {
            endpoint,
            token_file,
        } => {
            let response = runtime.block_on(call(
                &endpoint.unwrap_or_else(|| default_endpoint(database)),
                &token_file.unwrap_or_else(|| default_token(database)),
                Operation::Shutdown,
            ))?;
            Ok(json!({"stopped": true, "response": response}))
        }
    }
}

fn start(
    database: &Path,
    endpoint: &Path,
    token_file: &Path,
    foreground: bool,
) -> anyhow::Result<()> {
    let executable = std::env::current_exe()?.with_file_name(if cfg!(windows) {
        "lilia-daemon.exe"
    } else {
        "lilia-daemon"
    });
    if !executable.exists() {
        anyhow::bail!(
            "{} is missing; build lilia-daemon first",
            executable.display()
        );
    }
    let mut command = Command::new(executable);
    command
        .args(["--database"])
        .arg(database)
        .args(["--endpoint"])
        .arg(endpoint)
        .args(["--token-file"])
        .arg(token_file)
        .stdin(Stdio::null());
    if foreground {
        let status = command.status()?;
        if !status.success() {
            anyhow::bail!("daemon exited with {status}");
        }
    } else {
        command
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
    }
    Ok(())
}

fn wait_for_token(path: &Path) -> anyhow::Result<()> {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !path.exists() {
        if Instant::now() >= deadline {
            anyhow::bail!("daemon startup timed out");
        }
        thread::sleep(Duration::from_millis(25));
    }
    Ok(())
}

async fn call(
    endpoint: &Path,
    token_file: &Path,
    operation: Operation,
) -> anyhow::Result<ResponseValue> {
    let token = fs::read_to_string(token_file)?;
    #[cfg(unix)]
    let stream = tokio::net::UnixStream::connect(endpoint).await?;
    #[cfg(windows)]
    let stream = tokio::net::windows::named_pipe::ClientOptions::new().open(endpoint)?;
    exchange(stream, token.trim().into(), operation).await
}

async fn exchange<S>(
    mut stream: S,
    token: String,
    operation: Operation,
) -> anyhow::Result<ResponseValue>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let id = Uuid::new_v4().to_string();
    write_frame(
        &mut stream,
        &Request {
            id: id.clone(),
            token,
            deadline_ms: Some(5_000),
            operation,
        },
    )
    .await?;
    let response: Response = read_frame(&mut stream).await?;
    if response.id != id {
        anyhow::bail!("daemon response id mismatch");
    }
    response.result.map_err(Into::into)
}

#[cfg(unix)]
fn default_endpoint(database: &Path) -> PathBuf {
    PathBuf::from(format!("{}.sock", database.display()))
}

#[cfg(windows)]
fn default_endpoint(database: &Path) -> PathBuf {
    let mut hasher = DefaultHasher::new();
    database.hash(&mut hasher);
    PathBuf::from(format!(r"\\.\pipe\liliadb-{:016x}", hasher.finish()))
}

fn default_token(database: &Path) -> PathBuf {
    PathBuf::from(format!("{}.token", database.display()))
}

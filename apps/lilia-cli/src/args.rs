use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

#[derive(Debug, Parser)]
#[command(name = "lilia", version, about = "Local-first multimodel database")]
pub(crate) struct Arguments {
    #[arg(long, global = true, default_value = "human")]
    pub(crate) output: Output,
    #[arg(long, global = true, default_value = "lilia.lilia")]
    pub(crate) database: PathBuf,
    #[command(subcommand)]
    pub(crate) command: Command,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum Output {
    Human,
    Json,
    Jsonl,
}

#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    Init,
    Doctor,
    Backup {
        destination: PathBuf,
    },
    Daemon {
        #[command(subcommand)]
        command: DaemonCommand,
    },
    Kv {
        #[command(subcommand)]
        command: KvCommand,
    },
    Json {
        #[command(subcommand)]
        command: JsonCommand,
    },
    Batch {
        file: PathBuf,
    },
    Plugin {
        #[command(subcommand)]
        command: PluginCommand,
    },
}

#[derive(Debug, Subcommand)]
pub(crate) enum DaemonCommand {
    Start {
        #[arg(long)]
        endpoint: Option<PathBuf>,
        #[arg(long)]
        token_file: Option<PathBuf>,
        #[arg(long)]
        foreground: bool,
    },
    Status {
        #[arg(long)]
        endpoint: Option<PathBuf>,
        #[arg(long)]
        token_file: Option<PathBuf>,
    },
    Stop {
        #[arg(long)]
        endpoint: Option<PathBuf>,
        #[arg(long)]
        token_file: Option<PathBuf>,
    },
}

#[derive(Debug, Subcommand)]
pub(crate) enum KvCommand {
    Get {
        namespace: String,
        key: String,
    },
    Set {
        namespace: String,
        key: String,
        value: String,
        #[arg(long)]
        if_version: Option<u64>,
        #[arg(long)]
        expires_at_ms: Option<i64>,
    },
    Delete {
        namespace: String,
        key: String,
        #[arg(long)]
        if_version: Option<u64>,
    },
    Scan {
        namespace: String,
        #[arg(long)]
        after: Option<String>,
        #[arg(long, default_value_t = 100)]
        limit: u32,
    },
}

#[derive(Debug, Subcommand)]
pub(crate) enum JsonCommand {
    Get {
        space: String,
        id: String,
    },
    Put {
        space: String,
        id: String,
        value: String,
        #[arg(long)]
        if_version: Option<u64>,
    },
    Delete {
        space: String,
        id: String,
        #[arg(long)]
        if_version: Option<u64>,
    },
    Scan {
        space: String,
        #[arg(long)]
        after: Option<String>,
        #[arg(long, default_value_t = 100)]
        limit: u32,
    },
}

#[derive(Debug, Subcommand)]
pub(crate) enum PluginCommand {
    Verify {
        package: PathBuf,
        #[arg(long = "trusted-key")]
        trusted_keys: Vec<String>,
        #[arg(long)]
        allow_unsigned: bool,
    },
    Install {
        package: PathBuf,
        root: PathBuf,
        #[arg(long = "trusted-key")]
        trusted_keys: Vec<String>,
        #[arg(long)]
        allow_unsigned: bool,
    },
    List {
        root: PathBuf,
    },
    Remove {
        root: PathBuf,
        installed_name: String,
    },
}

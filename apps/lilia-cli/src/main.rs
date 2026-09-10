mod args;
mod daemon;
mod models;
mod plugins;

use clap::Parser;
use serde::Serialize;
use serde_json::json;

use args::{Arguments, Command, Output};

fn main() {
    if let Err(error) = run() {
        let structured = error
            .downcast_ref::<lilia_core::LiliaError>()
            .cloned()
            .unwrap_or_else(|| {
                let code = if error.downcast_ref::<serde_json::Error>().is_some() {
                    lilia_core::ErrorCode::InvalidInput
                } else if error.downcast_ref::<std::io::Error>().is_some() {
                    lilia_core::ErrorCode::Io
                } else {
                    lilia_core::ErrorCode::Storage
                };
                lilia_core::LiliaError::new(code, "command failed", false)
            });
        let payload = serde_json::to_string(&json!({"ok": false, "error": structured}))
            .unwrap_or_else(|_| "{\"ok\":false}".into());
        eprintln!("{payload}");
        std::process::exit(1);
    }
}

fn run() -> anyhow::Result<()> {
    let arguments = Arguments::parse();
    let value = match arguments.command {
        Command::Plugin { command } => plugins::execute(command, arguments.output)?,
        Command::Daemon { command } => daemon::execute(command, &arguments.database)?,
        command => models::execute(command, &arguments.database)?,
    };
    print_value(arguments.output, &value)?;
    Ok(())
}

fn print_value(output: Output, value: &impl Serialize) -> anyhow::Result<()> {
    match output {
        Output::Human => println!("{}", serde_json::to_string_pretty(value)?),
        Output::Json | Output::Jsonl => println!("{}", serde_json::to_string(value)?),
    }
    Ok(())
}

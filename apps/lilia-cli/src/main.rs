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
        let payload = serde_json::to_string(&json!({"ok": false, "error": error.to_string()}))
            .unwrap_or_else(|_| "{\"ok\":false}".into());
        eprintln!("{payload}");
        std::process::exit(1);
    }
}

fn run() -> anyhow::Result<()> {
    let arguments = Arguments::parse();
    let value = match arguments.command {
        Command::Plugin { command } => plugins::execute(command)?,
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

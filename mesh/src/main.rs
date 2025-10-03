use anyhow::Result;
use clap::{Parser, Subcommand};
use std::io::{self, Write};
use tracing::info;

mod client;
mod commands;
mod common;
mod server;

use commands::{ClientCommand, ServerCommand};

#[derive(Parser)]
#[command(
    name = "mesh",
    about = "Anywhere Mesh - Service mesh for ECS Anywhere",
    version = env!("CARGO_PKG_VERSION"),
    author = "ktruck"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Enable verbose logging
    #[arg(short, long, global = true)]
    verbose: bool,

    /// Log level (trace, debug, info, warn, error)
    #[arg(long, global = true, default_value = "info")]
    log_level: String,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the ingress server
    Server(ServerCommand),
    /// Start the Anywhere Mesh client
    Client(ClientCommand),
}

#[tokio::main]
async fn main() -> Result<()> {
    // Force stdout to be line buffered
    let _ = io::stdout().flush();

    let cli = Cli::parse();

    // Initialize logging
    let log_level = if cli.verbose { "debug" } else { &cli.log_level };
    std::env::set_var("RUST_LOG", log_level);
    tracing_subscriber::fmt().init();

    // Print banner
    print_banner();
    let _ = io::stdout().flush();

    match cli.command {
        Commands::Server(server_cmd) => {
            info!("Starting Anywhere Mesh Server");
            server::run(server_cmd).await?;
        }
        Commands::Client(client_cmd) => {
            info!("Starting Anywhere Mesh Client");
            client::run(client_cmd).await?;
        }
    }

    Ok(())
}

fn print_banner() {
    println!();
    println!("🔗 Anywhere Mesh");
    println!();
    io::stdout().flush().unwrap();
}

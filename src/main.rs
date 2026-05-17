//! `byt` — terminal-based VPN switcher.
//!
//! See `README.md` for an overview. This file wires up logging, error reporting,
//! single-instance locking, and dispatches to either the TUI or a subcommand.

#![doc(html_root_url = "https://docs.rs/byt")]

mod app;
mod cli;
mod error;
mod event;
mod lock;
mod tui;
mod ui;
mod vpn;

use clap::Parser;
use color_eyre::eyre::WrapErr;
use tracing_subscriber::EnvFilter;

use crate::cli::{Cli, Command};

#[tokio::main]
async fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    init_tracing();

    let cli = Cli::parse();

    match cli.command.unwrap_or(Command::Run) {
        Command::Run => run_tui().await,
        Command::Status => run_status().await,
        Command::Import { path, name } => run_import(path, name).await,
    }
}

async fn run_tui() -> color_eyre::Result<()> {
    // Hold the lock for the lifetime of the TUI session.
    let _guard = lock::acquire().wrap_err("could not acquire single-instance lock")?;

    let mut terminal = tui::init().wrap_err("could not initialise terminal")?;
    let result = app::App::new().run(&mut terminal).await;

    // Always try to restore the terminal even on error, then propagate.
    tui::restore().wrap_err("could not restore terminal")?;
    result
}

async fn run_status() -> color_eyre::Result<()> {
    let snapshot = vpn::snapshot().await?;
    for conn in &snapshot.connections {
        println!("{conn}");
    }
    Ok(())
}

async fn run_import(path: std::path::PathBuf, name: Option<String>) -> color_eyre::Result<()> {
    let preview = vpn::wireguard::parse_conf(&path)
        .wrap_err_with(|| format!("could not parse {}", path.display()))?;
    println!("{preview}");

    let connection_name = name.unwrap_or_else(|| preview.suggested_name());
    vpn::nmcli::import_wireguard(&path, &connection_name).await?;
    println!("✓ imported as `{connection_name}`");
    Ok(())
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("byt=info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .compact()
        .init();
}

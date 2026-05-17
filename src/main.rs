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

use std::path::PathBuf;

use clap::Parser;
use color_eyre::eyre::WrapErr;
use tracing_subscriber::EnvFilter;

use crate::cli::{Cli, Command};
use crate::vpn::ConfigKind;

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
    let _guard = lock::acquire().wrap_err("could not acquire single-instance lock")?;

    let mut terminal = tui::init().wrap_err("could not initialise terminal")?;
    let result = app::App::new().run(&mut terminal).await;

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

async fn run_import(path: PathBuf, name: Option<String>) -> color_eyre::Result<()> {
    let kind = vpn::detect_config_kind(&path)
        .wrap_err_with(|| format!("could not identify {}", path.display()))?;

    match kind {
        ConfigKind::WireGuard => {
            let preview = vpn::wireguard::parse_conf(&path)
                .wrap_err_with(|| format!("could not parse {}", path.display()))?;
            print!("{preview}");
            let connection_name = name.unwrap_or_else(|| preview.suggested_name());
            vpn::nmcli::import_wireguard(&path, &connection_name).await?;
            println!("✓ imported `{connection_name}` (wireguard)");
        }
        ConfigKind::OpenVpn => {
            let preview = vpn::openvpn::parse_conf(&path)
                .wrap_err_with(|| format!("could not parse {}", path.display()))?;
            print!("{preview}");
            let connection_name = name.unwrap_or_else(|| preview.suggested_name());
            vpn::nmcli::import_openvpn(&path, &connection_name).await?;
            println!("✓ imported `{connection_name}` (openvpn)");

            if preview.auth_user_pass {
                println!();
                println!("This config uses `auth-user-pass`. Set credentials before activating:");
                println!("  nmcli connection modify {connection_name} vpn.user-name '<username>'");
                println!("  nmcli connection modify {connection_name} +vpn.data password-flags=0");
                println!(
                    "  nmcli connection modify {connection_name} vpn.secrets password='<password>'"
                );
            }
        }
    }
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

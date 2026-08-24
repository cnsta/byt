//! `byt` — keyboard-driven VPN switcher for Linux.
//!
//! `byt` (no args) opens the GUI. `byt status`/`byt import` run as CLI tools
//! without spinning up iced. Both paths share the same `vpn::*` library code.

#![doc(html_root_url = "https://docs.rs/byt")]

mod app;
mod cli;
mod config;
mod error;
mod lock;
mod vpn;

use std::future::Future;
use std::path::{Path, PathBuf};

use clap::Parser;
use color_eyre::eyre::WrapErr;
use tracing_subscriber::EnvFilter;

use crate::cli::{Cli, Command};
use crate::vpn::ConfigKind;

fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    init_tracing();

    let cli = Cli::parse();
    match cli.command.unwrap_or(Command::Run) {
        Command::Run => run_gui(),
        Command::Status => block_on(run_status()),
        Command::Import { paths, name } => block_on(run_import(paths, name)),
    }
}

fn block_on<F>(fut: F) -> color_eyre::Result<()>
where
    F: Future<Output = color_eyre::Result<()>>,
{
    tokio::runtime::Runtime::new()?.block_on(fut)
}

fn run_gui() -> color_eyre::Result<()> {
    let _guard = lock::acquire().wrap_err("could not acquire single-instance lock")?;

    iced::application(app::App::new, app::App::update, app::App::view)
        .title(app::App::title)
        .subscription(app::App::subscription)
        .theme(app::App::theme)
        .window(iced::window::Settings {
            size: iced::Size::new(600.0, 400.0),
            platform_specific: iced::window::settings::PlatformSpecific {
                application_id: "dev.cnst.byt".to_owned(),
                ..Default::default()
            },
            ..Default::default()
        })
        .run()
        .wrap_err("iced failed to start")?;

    drop(_guard);
    Ok(())
}

async fn run_status() -> color_eyre::Result<()> {
    let snapshot = vpn::snapshot().await?;
    for conn in &snapshot.connections {
        println!("{conn}");
    }
    Ok(())
}

async fn run_import(paths: Vec<PathBuf>, name: Option<String>) -> color_eyre::Result<()> {
    if name.is_some() && paths.len() > 1 {
        color_eyre::eyre::bail!("--name only makes sense when importing a single file");
    }

    let total = paths.len();
    let mut failed = 0_usize;
    for path in &paths {
        if let Err(err) = import_single(path, name.clone()).await {
            failed += 1;
            eprintln!("✗ {}: {err:#}", path.display());
        }
    }
    if failed > 0 {
        color_eyre::eyre::bail!("{failed} of {total} imports failed");
    }
    Ok(())
}

async fn import_single(path: &Path, name: Option<String>) -> color_eyre::Result<()> {
    let kind = vpn::detect_config_kind(path)
        .wrap_err_with(|| format!("could not identify {}", path.display()))?;

    let (preview_str, suggested, auth_user_pass) = match kind {
        ConfigKind::WireGuard => {
            let p = vpn::wireguard::parse_conf(path)?;
            (p.to_string(), p.suggested_name(), false)
        }
        ConfigKind::OpenVpn => {
            let p = vpn::openvpn::parse_conf(path)?;
            let auth = p.auth_user_pass;
            (p.to_string(), p.suggested_name(), auth)
        }
    };

    print!("{preview_str}");
    let connection_name = name.unwrap_or(suggested);
    vpn::import::import(kind, path, &connection_name).await?;
    println!("✓ imported `{connection_name}` ({})", kind.as_str());

    if auth_user_pass {
        println!();
        println!("This config uses `auth-user-pass`. Set credentials before activating:");
        println!("  nmcli connection modify {connection_name} vpn.user-name '<username>'");
        println!("  nmcli connection modify {connection_name} +vpn.data password-flags=0");
        println!("  nmcli connection modify {connection_name} vpn.secrets password='<password>'");
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

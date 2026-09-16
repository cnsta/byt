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

struct ImportPreview {
    path: PathBuf,
    kind: ConfigKind,
    connection_name: String,
    auth_user_pass: bool,
}

fn preview(path: &Path, name: Option<String>) -> color_eyre::Result<ImportPreview> {
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
    Ok(ImportPreview {
        path: path.to_path_buf(),
        kind,
        connection_name: name.unwrap_or(suggested),
        auth_user_pass,
    })
}

async fn run_import(paths: Vec<PathBuf>, name: Option<String>) -> color_eyre::Result<()> {
    if name.is_some() && paths.len() > 1 {
        color_eyre::eyre::bail!("--name only makes sense when importing a single file");
    }
    let total = paths.len();

    let mut previews: Vec<Option<ImportPreview>> = Vec::new();
    for path in &paths {
        match preview(path, name.clone()) {
            Ok(p) => previews.push(Some(p)),
            Err(err) => {
                eprintln!("✗ {}: {err:#}", path.display());
                previews.push(None);
            }
        }
    }

    let outcomes = futures::future::join_all(previews.into_iter().map(|entry| async move {
        match entry {
            None => None,
            Some(p) => {
                let result = vpn::import::import(p.kind, &p.path, &p.connection_name).await;
                Some((p, result))
            }
        }
    }))
    .await;

    let mut failed = 0_usize;
    for outcome in outcomes {
        match outcome {
            None => failed += 1,
            Some((p, Ok(()))) => {
                println!("✓ imported `{}` ({})", p.connection_name, p.kind.as_str());
                if p.auth_user_pass {
                    println!();
                    println!(
                        "This config uses `auth-user-pass`. Set credentials before activating:"
                    );
                    println!("{}", error::secrets_hint(&p.connection_name));
                }
            }
            Some((p, Err(err))) => {
                failed += 1;
                eprintln!("✗ {}: {err:#}", p.path.display());
            }
        }
    }

    if failed > 0 {
        color_eyre::eyre::bail!("{failed} of {total} imports failed");
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

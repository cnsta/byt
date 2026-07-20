//! Command-line interface definitions.

use std::path::PathBuf;

use clap::{Parser, Subcommand};

/// A terminal-based VPN switcher for Linux.
#[derive(Debug, Parser)]
#[command(version, about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Open the interactive TUI (default).
    Run,

    /// Print current VPN state and exit.
    Status,

    /// Import WireGuard/OpenVPN config files as NetworkManager connections.
    Import {
        /// Paths to the config files (one or more).
        #[arg(required = true)]
        paths: Vec<PathBuf>,

        /// Connection name. Defaults to the file stem. Only valid when
        /// importing a single file.
        #[arg(short, long)]
        name: Option<String>,
    },
}

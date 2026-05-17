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

    /// Import a WireGuard `.conf` file as a NetworkManager connection.
    Import {
        /// Path to the `.conf` file.
        path: PathBuf,

        /// Connection name. Defaults to the file stem.
        #[arg(short, long)]
        name: Option<String>,
    },
}

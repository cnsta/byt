//! Crate-wide error type and `Result` alias

use std::path::PathBuf;

use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("D-Bus error: {0}")]
    DBus(#[from] zbus::Error),

    #[error("D-Bus value conversion failed: {0}")]
    DBusValue(String),

    #[error("required executable not found: `{0}`. Is it installed and on PATH?")]
    MissingExecutable(&'static str),

    #[error("`{cmd}` exited with status {status}: {stderr}")]
    CommandFailed {
        cmd: String,
        status: i32,
        stderr: String,
    },

    #[error("invalid config at {path}: {context}")]
    InvalidConfig { path: PathBuf, context: String },

    #[error("connection `{0}` not found")]
    NotFound(String),

    #[error(
        "activation needs secrets that aren't stored. Set them with:\n  \
         nmcli connection modify '{name}' vpn.user-name '<username>'\n  \
         nmcli connection modify '{name}' +vpn.data password-flags=0\n  \
         nmcli connection modify '{name}' vpn.secrets password='<password>'"
    )]
    SecretsRequired { name: String },

    #[error("{kind} backend is not available on this system")]
    Unavailable { kind: &'static str },

    #[error("another instance is already running")]
    AlreadyRunning,

    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("{kind} connections cannot be deleted from byt")]
    CannotDelete { kind: &'static str },

    #[error("tailscale is logged out; run `tailscale up` in a terminal to authenticate")]
    TailscaleNeedsLogin,
}

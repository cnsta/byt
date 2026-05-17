//! Crate-wide error type and `Result` alias.
//!
//! Library-style code returns [`Error`]. The binary entrypoint converts these
//! into `color_eyre::Report` for pretty terminal output.

use std::path::PathBuf;

use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("required executable not found: `{0}`. Is it installed and on PATH?")]
    MissingExecutable(&'static str),

    #[error("`{cmd}` exited with status {status}: {stderr}")]
    CommandFailed {
        cmd: String,
        status: i32,
        stderr: String,
    },

    #[error("could not parse output of `{cmd}`: {context}")]
    ParseOutput { cmd: &'static str, context: String },

    #[error("invalid config at {path}: {context}")]
    InvalidConfig { path: PathBuf, context: String },

    #[error("{kind} backend is not available on this system")]
    Unavailable { kind: &'static str },

    #[error("another instance is already running")]
    AlreadyRunning,

    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}

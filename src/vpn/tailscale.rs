//! Tailscale wrapper.
//!
//! Activation is via `systemctl start/stop tailscaled` (matches the original
//! script). State and metadata come from `tailscale status --json`.

use serde::Deserialize;
use tokio::process::Command;

use crate::error::{Error, Result};
use crate::vpn::{Connection, ConnectionState, VpnKind};

const SERVICE: &str = "tailscaled";
const TAILSCALE: &str = "tailscale";
const SYSTEMCTL: &str = "systemctl";

/// Slice of `tailscale status --json` we care about.
#[derive(Debug, Deserialize)]
struct StatusJson {
    #[serde(rename = "BackendState")]
    backend_state: String,
    #[serde(rename = "MagicDNSSuffix")]
    magic_dns_suffix: Option<String>,
}

pub async fn status() -> Result<Connection> {
    let active = systemctl_is_active().await;

    let detail = if active {
        match tailscale_status_json().await {
            Ok(json) => json.magic_dns_suffix,
            Err(err) => {
                tracing::warn!(?err, "tailscale status --json failed");
                None
            }
        }
    } else {
        None
    };

    Ok(Connection {
        name: "Tailscale".to_owned(),
        kind: VpnKind::Tailscale,
        state: if active { ConnectionState::Active } else { ConnectionState::Inactive },
        detail,
    })
}

pub async fn start() -> Result<()> {
    sudo_systemctl(&["start", SERVICE]).await
}

pub async fn stop() -> Result<()> {
    sudo_systemctl(&["stop", SERVICE]).await
}

async fn systemctl_is_active() -> bool {
    let result = Command::new(SYSTEMCTL)
        .args(["is-active", "--quiet", SERVICE])
        .status()
        .await;
    matches!(result, Ok(s) if s.success())
}

async fn tailscale_status_json() -> Result<StatusJson> {
    let output = Command::new(TAILSCALE)
        .args(["status", "--json"])
        .output()
        .await
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => Error::MissingExecutable(TAILSCALE),
            _ => Error::Io(e),
        })?;

    if !output.status.success() {
        // Backend may simply be "Stopped" — treat as no detail rather than a
        // hard error. Surface non-zero only as a parse failure.
        return Err(Error::ParseOutput {
            cmd: "tailscale status --json",
            context: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }

    let parsed: StatusJson = serde_json::from_slice(&output.stdout)?;
    if parsed.backend_state != "Running" {
        // Not really an error; just no detail to show.
        return Ok(StatusJson {
            backend_state: parsed.backend_state,
            magic_dns_suffix: None,
        });
    }
    Ok(parsed)
}

/// `systemctl` operations that mutate state require root. We call it via sudo
/// and let the user authenticate (matches the original script).
async fn sudo_systemctl(args: &[&str]) -> Result<()> {
    let mut cmd_args = vec!["systemctl"];
    cmd_args.extend_from_slice(args);

    let output = Command::new("sudo")
        .args(&cmd_args)
        .output()
        .await
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => Error::MissingExecutable("sudo"),
            _ => Error::Io(e),
        })?;

    if !output.status.success() {
        return Err(Error::CommandFailed {
            cmd: format!("sudo {}", cmd_args.join(" ")),
            status: output.status.code().unwrap_or(-1),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    Ok(())
}

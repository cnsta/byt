//! Tailscale wrapper.
//!
//! `start` / `stop` go through systemd's dbus API (polkit-mediated, no sudo
//! prompt). `tailscale status --json` stays a subprocess call, it doesn't
//! need elevation and the local API socket isn't a stable third-party interface.

use std::path::PathBuf;

use serde::Deserialize;
use tokio::process::Command;
use zbus::proxy;
use zbus::zvariant::{ObjectPath, OwnedObjectPath};

use crate::error::{Error, Result};
use crate::vpn::{Connection, ConnectionState, VpnKind};

const SERVICE: &str = "tailscaled.service";
const TAILSCALE: &str = "tailscale";

#[proxy(
    interface = "org.freedesktop.systemd1.Manager",
    default_service = "org.freedesktop.systemd1",
    default_path = "/org/freedesktop/systemd1"
)]
trait SystemdManager {
    fn start_unit(&self, name: &str, mode: &str) -> zbus::Result<OwnedObjectPath>;
    fn stop_unit(&self, name: &str, mode: &str) -> zbus::Result<OwnedObjectPath>;
    fn get_unit(&self, name: &str) -> zbus::Result<OwnedObjectPath>;
}

#[proxy(
    interface = "org.freedesktop.systemd1.Unit",
    default_service = "org.freedesktop.systemd1"
)]
trait SystemdUnit {
    #[zbus(property)]
    fn active_state(&self) -> zbus::Result<String>;
}

#[derive(Debug, Deserialize)]
struct StatusJson {
    #[serde(rename = "BackendState")]
    backend_state: String,
    #[serde(rename = "MagicDNSSuffix")]
    magic_dns_suffix: Option<String>,
}

pub async fn status() -> Result<Connection> {
    if !is_installed() {
        return Ok(Connection {
            name: "Tailscale".to_owned(),
            kind: VpnKind::Tailscale,
            state: ConnectionState::Unavailable,
            detail: Some("not installed".to_owned()),
        });
    }

    let active = is_unit_active().await.unwrap_or(false);

    let detail = if active {
        match tailscale_status_json().await {
            Ok(json) if json.backend_state == "Running" => json.magic_dns_suffix,
            Ok(_) => None,
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
        state: if active {
            ConnectionState::Active
        } else {
            ConnectionState::Inactive
        },
        detail,
    })
}

pub async fn start() -> Result<()> {
    let bus = zbus::Connection::system().await?;
    let mgr = SystemdManagerProxy::new(&bus).await?;
    // "replace" cancels any queued jobs for this unit and queues ours.
    mgr.start_unit(SERVICE, "replace").await?;
    Ok(())
}

pub async fn stop() -> Result<()> {
    let bus = zbus::Connection::system().await?;
    let mgr = SystemdManagerProxy::new(&bus).await?;
    mgr.stop_unit(SERVICE, "replace").await?;
    Ok(())
}

async fn is_unit_active() -> Result<bool> {
    let bus = zbus::Connection::system().await?;
    let mgr = SystemdManagerProxy::new(&bus).await?;

    // GetUnit returns the loaded unit's object path, or a dbus error if the
    // unit isn't loaded at all.
    let Ok(unit_path) = mgr.get_unit(SERVICE).await else {
        return Ok(false);
    };

    let unit = SystemdUnitProxy::builder(&bus)
        .path(unit_path)?
        .build()
        .await?;
    let state = unit.active_state().await?;
    Ok(state == "active")
}

fn is_installed() -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir: PathBuf| dir.join(TAILSCALE).is_file())
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
        return Err(Error::CommandFailed {
            cmd: format!("{TAILSCALE} status --json"),
            status: output.status.code().unwrap_or(-1),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }

    Ok(serde_json::from_slice(&output.stdout)?)
}

#[allow(dead_code)]
fn _force_unused_object_path_use(_p: ObjectPath<'_>) {}

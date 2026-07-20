//! Tailscale wrapper.
//!
//! `start` / `stop` go through systemd's dbus API (polkit-mediated, no sudo
//! prompt). `tailscale status --json` stays a subprocess call, it doesn't
//! need elevation and the local API socket isn't a stable third-party interface.

use std::path::PathBuf;
use std::time::Duration;

use serde::Deserialize;
use tokio::process::Command;
use zbus::proxy;
use zbus::proxy::MethodFlags;
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

    let unit_active = is_unit_active().await.unwrap_or(false);

    // `tailscaled.service` being active only means the daemon is running —
    // on most installs it runs from boot, always. Whether the VPN is up is
    // the backend state: `tailscale down`, a fresh install, and a logged-out
    // node all leave the daemon active with a state other than "Running".
    let (state, detail) = if unit_active {
        match tailscale_status_json().await {
            Ok(json) => match json.backend_state.as_str() {
                "Running" => (ConnectionState::Active, json.magic_dns_suffix),
                "NeedsLogin" | "NeedsMachineAuth" => (
                    ConnectionState::Inactive,
                    Some("logged out — run `tailscale up`".to_owned()),
                ),
                _ => (ConnectionState::Inactive, None),
            },
            Err(err) => {
                tracing::warn!(?err, "tailscale status --json failed");
                (ConnectionState::Inactive, None)
            }
        }
    } else {
        (ConnectionState::Inactive, None)
    };

    Ok(Connection {
        name: "Tailscale".to_owned(),
        kind: VpnKind::Tailscale,
        state,
        detail,
    })
}

pub async fn start() -> Result<()> {
    if !is_unit_active().await.unwrap_or(false) {
        // Daemon is down — this is the state byt's own `stop` leaves behind.
        // Starting the unit restores the saved prefs (incl. WantRunning), so
        // the node reconnects on its own.
        return start_unit().await;
    }

    // Daemon already running: `StartUnit` on an active unit is a no-op, so
    // the connection has to come up through the local API instead.
    match tailscale_status_json().await.map(|j| j.backend_state) {
        Ok(state) if state == "Running" => Ok(()),
        Ok(state) if state == "NeedsLogin" || state == "NeedsMachineAuth" => {
            Err(Error::TailscaleNeedsLogin)
        }
        // "Stopped" (i.e. after `tailscale down`), or status query failed:
        // try `tailscale up` and let its stderr explain any failure.
        _ => tailscale_up().await,
    }
}

async fn start_unit() -> Result<()> {
    let bus = zbus::Connection::system().await?;
    let mgr = SystemdManagerProxy::new(&bus).await?;
    let _: Option<OwnedObjectPath> = mgr
        .inner()
        .call_with_flags(
            "StartUnit",
            MethodFlags::AllowInteractiveAuth.into(),
            &(SERVICE, "replace"),
        )
        .await?;
    Ok(())
}

/// How long to let `tailscale up` run before giving up on it.
const UP_TIMEOUT: Duration = Duration::from_secs(15);

/// `tailscale up` with no flags at all. The CLI only takes its "simple up"
/// path — resume with saved prefs, don't touch settings — when *zero* flags
/// are given. Passing any flag, even a non-pref one like `--timeout`, routes
/// it through the settings-diff check, which then errors unless every
/// non-default pref (`--accept-routes`, `--login-server`, …) is restated.
/// So the timeout lives out here instead, and `kill_on_drop` reaps the child
/// if we bail. Requires root or operator rights on the socket.
async fn tailscale_up() -> Result<()> {
    let run = Command::new(TAILSCALE)
        .arg("up")
        .kill_on_drop(true)
        .output();

    let output = tokio::time::timeout(UP_TIMEOUT, run)
        .await
        .map_err(|_| Error::CommandFailed {
            cmd: format!("{TAILSCALE} up"),
            status: -1,
            stderr: format!("timed out after {}s", UP_TIMEOUT.as_secs()),
        })?
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => Error::MissingExecutable(TAILSCALE),
            _ => Error::Io(e),
        })?;

    if output.status.success() {
        return Ok(());
    }

    let mut stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    if stderr.contains("Access denied") {
        stderr.push_str("\nhint: grant yourself operator rights: sudo tailscale set --operator=$USER");
    }
    Err(Error::CommandFailed {
        cmd: format!("{TAILSCALE} up"),
        status: output.status.code().unwrap_or(-1),
        stderr,
    })
}

pub async fn stop() -> Result<()> {
    let bus = zbus::Connection::system().await?;
    let mgr = SystemdManagerProxy::new(&bus).await?;
    let _: Option<OwnedObjectPath> = mgr
        .inner()
        .call_with_flags(
            "StopUnit",
            MethodFlags::AllowInteractiveAuth.into(),
            &(SERVICE, "replace"),
        )
        .await?;
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

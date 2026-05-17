//! VPN abstraction.
//!
//! Public surface:
//! - [`Connection`], [`ConnectionState`], [`VpnKind`] — data the UI renders.
//! - [`snapshot`] — query current state across all backends.
//! - [`activate_exclusive`] — bring one connection up, ensuring others are down.
//! - [`disconnect_all`] — tear everything down.
//! - [`detect_config_kind`] — sniff WireGuard vs OpenVPN from a config file.
//!
//! Backends (`nmcli`, `tailscale`, `wireguard`, `openvpn`) are crate-internal;
//! the UI never imports them directly so we can swap implementations later.

pub mod nmcli;
pub mod openvpn;
pub mod tailscale;
pub mod wireguard;

use std::fmt;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

#[derive(Debug, Default, Clone)]
pub struct Snapshot {
    pub connections: Vec<Connection>,
}

#[derive(Debug, Clone)]
pub struct Connection {
    pub name: String,
    pub kind: VpnKind,
    pub state: ConnectionState,
    /// Free-form detail shown in the UI (e.g. tailnet name, endpoint).
    pub detail: Option<String>,
}

impl fmt::Display for Connection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mark = match self.state {
            ConnectionState::Active => "active",
            ConnectionState::Inactive => "inactive",
            ConnectionState::Unavailable => "unavailable",
        };
        write!(f, "{:10} {:12} {}", self.kind.as_str(), mark, self.name)?;
        if let Some(d) = &self.detail {
            write!(f, "  ({d})")?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VpnKind {
    Tailscale,
    WireGuard,
    OpenVpn,
}

impl VpnKind {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            VpnKind::Tailscale => "tailscale",
            VpnKind::WireGuard => "wireguard",
            VpnKind::OpenVpn => "openvpn",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    Active,
    Inactive,
    Unavailable,
}

/// Which config format an imported file uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigKind {
    WireGuard,
    OpenVpn,
}

/// Sniff a config file to determine whether it's wireguqard or openvpn.
pub fn detect_config_kind(path: &Path) -> Result<ConfigKind> {
    let raw = std::fs::read_to_string(path)?;
    for line in raw.lines().take(100) {
        let line = strip_comment(line).trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with("[Interface]") || line.starts_with("[Peer]") {
            return Ok(ConfigKind::WireGuard);
        }
        if line == "client"
            || line.starts_with("remote ")
            || line.starts_with("dev ")
            || line.starts_with("proto ")
            || line.starts_with("<ca>")
        {
            return Ok(ConfigKind::OpenVpn);
        }
    }
    Err(Error::InvalidConfig {
        path: path.to_path_buf(),
        context: "could not determine config format (neither WireGuard nor OpenVPN markers found)"
            .into(),
    })
}

fn strip_comment(line: &str) -> &str {
    if let Some(idx) = line.find(['#', ';']) {
        &line[..idx]
    } else {
        line
    }
}

/// Query every backend and return a unified snapshot.
///
/// Backends are queried concurrently; ordering in the result is fixed.
/// Tailscale > wg > openvpn
pub async fn snapshot() -> Result<Snapshot> {
    let (ts, wg, ovpn) = tokio::join!(
        tailscale::status(),
        nmcli::wireguard_connections(),
        nmcli::openvpn_connections(),
    );

    let mut connections = Vec::new();
    connections.push(ts?);

    let mut wg = wg?;
    wg.sort_by(|a, b| a.name.cmp(&b.name));
    connections.extend(wg);

    let mut ovpn = ovpn?;
    ovpn.sort_by(|a, b| a.name.cmp(&b.name));
    connections.extend(ovpn);

    Ok(Snapshot { connections })
}

/// Bring `target` up and bring everything else down. Idempotent.
pub async fn activate_exclusive(target: &Connection) -> Result<()> {
    if target.state == ConnectionState::Unavailable {
        return Err(Error::Unavailable {
            kind: target.kind.as_str(),
        });
    }

    let snap = snapshot().await?;

    let exclusive = snap.connections.iter().all(|c| {
        if c.name == target.name && c.kind == target.kind {
            c.state == ConnectionState::Active
        } else {
            c.state != ConnectionState::Active
        }
    });
    if exclusive {
        return Ok(());
    }

    bring_others_down(&snap, target).await?;
    bring_up(target).await
}

pub async fn disconnect_all() -> Result<()> {
    let snap = snapshot().await?;
    for c in &snap.connections {
        if c.state == ConnectionState::Active {
            bring_down(c).await?;
        }
    }
    Ok(())
}

async fn bring_others_down(snap: &Snapshot, target: &Connection) -> Result<()> {
    for c in &snap.connections {
        if (c.name == target.name && c.kind == target.kind) || c.state != ConnectionState::Active {
            continue;
        }
        bring_down(c).await?;
    }
    Ok(())
}

async fn bring_up(c: &Connection) -> Result<()> {
    match c.kind {
        VpnKind::Tailscale => tailscale::start().await,
        // Openvpn and wg connections are both managed by networkmanager
        // and brought up identically by name.
        VpnKind::WireGuard | VpnKind::OpenVpn => nmcli::connection_up(&c.name).await,
    }
}

async fn bring_down(c: &Connection) -> Result<()> {
    match c.kind {
        VpnKind::Tailscale => tailscale::stop().await,
        VpnKind::WireGuard | VpnKind::OpenVpn => nmcli::connection_down(&c.name).await,
    }
}

//! VPN abstraction.
//!
//! Public surface:
//! - [`Connection`], [`ConnectionState`], [`VpnKind`], [`ConfigKind`] —
//!   data the UI renders.
//! - [`snapshot`] — query current state across all backends.
//! - [`activate_exclusive`] — bring one connection up, ensuring others are down.
//! - [`disconnect_all`] — tear everything down.
//! - [`detect_config_kind`] — sniff WireGuard vs OpenVPN from a config file.

pub mod import;
pub mod nm;
pub mod openvpn;
pub mod tailscale;
pub mod wireguard;

use std::fmt;
use std::path::Path;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigKind {
    WireGuard,
    OpenVpn,
}

impl ConfigKind {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            ConfigKind::WireGuard => "wireguard",
            ConfigKind::OpenVpn => "openvpn",
        }
    }
}

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
    line.find(['#', ';']).map_or(line, |idx| &line[..idx])
}

pub async fn snapshot() -> Result<Snapshot> {
    let (ts, nm_list) = tokio::join!(tailscale::status(), nm::list());

    let mut connections = Vec::new();
    connections.push(ts?);

    let mut nm_list = nm_list?;
    nm_list.sort_by(|a, b| (a.kind.as_str(), &a.name).cmp(&(b.kind.as_str(), &b.name)));
    connections.extend(nm_list);

    Ok(Snapshot { connections })
}

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

    for c in &snap.connections {
        if (c.name == target.name && c.kind == target.kind) || c.state != ConnectionState::Active {
            continue;
        }
        bring_down(c).await?;
    }
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

async fn bring_up(c: &Connection) -> Result<()> {
    match c.kind {
        VpnKind::Tailscale => tailscale::start().await,
        VpnKind::WireGuard | VpnKind::OpenVpn => nm::activate(&c.name).await,
    }
}

async fn bring_down(c: &Connection) -> Result<()> {
    match c.kind {
        VpnKind::Tailscale => tailscale::stop().await,
        VpnKind::WireGuard | VpnKind::OpenVpn => nm::deactivate(&c.name).await,
    }
}

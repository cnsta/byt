//! VPN abstraction.
//!
//! Public surface:
//! - [`Connection`], [`ConnectionState`], [`VpnKind`] — data the UI renders.
//! - [`snapshot`] — query current state across all backends.
//! - [`activate_exclusive`] — bring one connection up, ensuring others are down.
//! - [`disconnect_all`] — tear everything down.
//!
//! Backends (`nmcli`, `tailscale`, `wireguard`) are crate-internal modules; the
//! UI never imports them directly so we can swap implementations later.

pub mod nmcli;
pub mod tailscale;
pub mod wireguard;

use std::fmt;

use crate::error::Result;

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
        write!(f, "{:10} {:10} {}", self.kind.as_str(), mark, self.name)?;
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
}

impl VpnKind {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            VpnKind::Tailscale => "tailscale",
            VpnKind::WireGuard => "wireguard",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    Active,
    Inactive,
    Unavailable,
}

/// Query every backend and return a unified snapshot. Order: Tailscale first,
/// then WireGuard profiles sorted by name.
pub async fn snapshot() -> Result<Snapshot> {
    // Run backends concurrently — they don't depend on each other.
    let (ts, wg) = tokio::join!(tailscale::status(), nmcli::wireguard_connections());
    let mut connections = Vec::new();
    connections.push(ts?);
    let mut wg = wg?;
    wg.sort_by(|a, b| a.name.cmp(&b.name));
    connections.extend(wg);
    Ok(Snapshot { connections })
}

/// Bring `target` up and bring everything else down. Idempotent: activating an
/// already-active connection (with nothing else running) is a no-op.
pub async fn activate_exclusive(target: &Connection) -> Result<()> {
    let snap = snapshot().await?;

    // Already exclusively active? Nothing to do.
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

    match target.kind {
        VpnKind::Tailscale => tailscale::start().await,
        VpnKind::WireGuard => nmcli::connection_up(&target.name).await,
    }
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
        if (c.name == target.name && c.kind == target.kind)
            || c.state != ConnectionState::Active
        {
            continue;
        }
        bring_down(c).await?;
    }
    Ok(())
}

async fn bring_down(c: &Connection) -> Result<()> {
    match c.kind {
        VpnKind::Tailscale => tailscale::stop().await,
        VpnKind::WireGuard => nmcli::connection_down(&c.name).await,
    }
}

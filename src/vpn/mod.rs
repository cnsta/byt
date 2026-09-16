//! VPN abstraction.
//!
//! Public surface:
//! - [`Connection`], [`ConnectionState`], [`VpnKind`], [`ConfigKind`] —
//!   data the UI renders.
//! - [`snapshot`] — query current state across all backends.
//! - [`toggle_exclusive`] — toggle one connection, ensuring others are down.
//! - [`disconnect`] — tear everything down.
//! - [`detect_config_kind`] — sniff WireGuard vs OpenVPN from a config file.

mod dbus;
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

/// Outcome of [`toggle_exclusive`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Toggled {
    /// The target connection was brought up.
    Up,
    /// The target connection was brought down.
    Down,
}

/// Toggle one connection, ensuring mutual exclusion.
///
/// If the target is currently down, every other active connection is brought
/// down and the target is brought up. If the target is already up, it is
/// brought down instead (along with any strays), so the bound key acts as a
/// connect/disconnect toggle.
///
/// The decision is based on a fresh [`snapshot`], not on `target.state`,
/// which may be stale UI state.
pub async fn toggle_exclusive(target: &Connection) -> Result<Toggled> {
    if target.state == ConnectionState::Unavailable {
        return Err(Error::Unavailable {
            kind: target.kind.as_str(),
        });
    }

    let snap = snapshot().await?;
    let target_active = snap.connections.iter().any(|c| {
        c.name == target.name && c.kind == target.kind && c.state == ConnectionState::Active
    });

    let others = snap.connections.iter().filter(|c| {
        !(c.name == target.name && c.kind == target.kind) && c.state == ConnectionState::Active
    });
    bring_down_all(others).await?;

    if target_active {
        bring_down(target).await?;
        Ok(Toggled::Down)
    } else {
        bring_up(target).await?;
        Ok(Toggled::Up)
    }
}

pub async fn disconnect() -> Result<()> {
    let snap = snapshot().await?;
    bring_down_all(
        snap.connections
            .iter()
            .filter(|c| c.state == ConnectionState::Active),
    )
    .await
}

async fn bring_down_all<'a, I>(targets: I) -> Result<()>
where
    I: IntoIterator<Item = &'a Connection>,
{
    let results = futures::future::join_all(
        targets
            .into_iter()
            .map(|c| async move { (c.name.clone(), bring_down(c).await) }),
    )
    .await;
    aggregate_bring_down_errors(results)
}

fn aggregate_bring_down_errors(results: Vec<(String, Result<()>)>) -> Result<()> {
    let failures: Vec<(String, Error)> = results
        .into_iter()
        .filter_map(|(name, r)| r.err().map(|e| (name, e)))
        .collect();
    if failures.is_empty() {
        return Ok(());
    }
    let details = failures
        .iter()
        .map(|(name, err)| format!("  {name}: {err}"))
        .collect::<Vec<_>>()
        .join("\n");
    Err(Error::BringDownFailed(details))
}

pub async fn delete(target: &Connection) -> Result<()> {
    match target.kind {
        VpnKind::Tailscale => Err(Error::CannotDelete {
            kind: target.kind.as_str(),
        }),
        VpnKind::WireGuard | VpnKind::OpenVpn => nm::delete(&target.name).await,
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aggregates_all_ok_as_ok() {
        let results = vec![("a".to_owned(), Ok(())), ("b".to_owned(), Ok(()))];
        assert!(aggregate_bring_down_errors(results).is_ok());
    }

    #[test]
    fn aggregates_mixed_results_into_one_error_mentioning_all_failures() {
        let results = vec![
            ("a".to_owned(), Ok(())),
            ("b".to_owned(), Err(Error::NotFound("b".to_owned()))),
            (
                "c".to_owned(),
                Err(Error::CannotDelete { kind: "tailscale" }),
            ),
        ];
        let err = aggregate_bring_down_errors(results).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("b:") && msg.contains("c:") && !msg.contains("a:"));
    }
}

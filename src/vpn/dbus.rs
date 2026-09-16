//! Shared system bus connection, cached after first use.
//!
//! `nm.rs` and `tailscale.rs` both talk to the system bus; opening a fresh
//! connection (a full handshake) on every call adds latency to every VPN
//! action for no benefit, since `zbus::Connection` is cheap to clone and
//! share.

use tokio::sync::OnceCell;

use crate::error::Result;

static SYSTEM: OnceCell<zbus::Connection> = OnceCell::const_new();

/// The shared system bus connection, connecting lazily on first use.
pub(super) async fn system() -> Result<zbus::Connection> {
    let conn = SYSTEM.get_or_try_init(zbus::Connection::system).await?;
    Ok(conn.clone())
}

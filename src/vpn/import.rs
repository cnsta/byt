//! Config file import via `nmcli connection import`.
//!
//! Use nmcli for importing configuration files.
//! Everything else (listing, activating, deactivating, watching) goes through
//! [`crate::vpn::nm`] over dbus.

use std::path::Path;

use tokio::process::Command;

use crate::error::{Error, Result};
use crate::vpn::ConfigKind;

const NMCLI: &str = "nmcli";

/// Detect the config type of `path` and derive the connection name it would
/// import as (the file stem, for both formats). Cheap: reads the file, runs
/// nothing.
pub fn preview_name(path: &Path) -> Result<(ConfigKind, String)> {
    let kind = crate::vpn::detect_config_kind(path)?;
    let name = match kind {
        ConfigKind::WireGuard => crate::vpn::wireguard::parse_conf(path)?.suggested_name(),
        ConfigKind::OpenVpn => crate::vpn::openvpn::parse_conf(path)?.suggested_name(),
    };
    Ok((kind, name))
}

pub async fn import(kind: ConfigKind, path: &Path, desired_name: &str) -> Result<()> {
    let plugin = match kind {
        ConfigKind::WireGuard => "wireguard",
        ConfigKind::OpenVpn => "openvpn",
    };
    let path_str = path.to_string_lossy();

    nmcli(&["connection", "import", "type", plugin, "file", &path_str]).await?;

    let imported = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(desired_name);
    if imported != desired_name {
        nmcli(&[
            "connection",
            "modify",
            imported,
            "connection.id",
            desired_name,
        ])
        .await?;
    }

    // disable autoconnect
    nmcli(&[
        "connection",
        "modify",
        desired_name,
        "connection.autoconnect",
        "no",
    ])
    .await?;

    // disconnect from imported config, this is my personal desired behavior
    let _ = nmcli(&["connection", "down", desired_name]).await;

    Ok(())
}

async fn nmcli(args: &[&str]) -> Result<String> {
    let output = Command::new(NMCLI)
        .args(args)
        .output()
        .await
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => Error::MissingExecutable(NMCLI),
            _ => Error::Io(e),
        })?;

    if !output.status.success() {
        return Err(Error::CommandFailed {
            cmd: format!("{NMCLI} {}", args.join(" ")),
            status: output.status.code().unwrap_or(-1),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

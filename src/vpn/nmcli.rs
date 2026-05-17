//! `nmcli` wrapper. We invoke it with `--terse --fields ...` so the output is
//! machine-parseable: colon-separated, no headers, escaping `:` and `\`.

use std::collections::HashSet;
use std::path::Path;

use tokio::process::Command;

use crate::error::{Error, Result};
use crate::vpn::{Connection, ConnectionState, VpnKind};

const NMCLI: &str = "nmcli";

// listing

pub async fn wireguard_connections() -> Result<Vec<Connection>> {
    let all = run(&["-t", "-f", "NAME,TYPE", "connection", "show"]).await?;
    let active = run(&["-t", "-f", "NAME,TYPE", "connection", "show", "--active"]).await?;

    let active_names: HashSet<&str> = active
        .lines()
        .filter_map(|line| {
            parse_name_type(line)
                .filter(|(_, t)| *t == "wireguard")
                .map(|(n, _)| n)
        })
        .collect();

    Ok(all
        .lines()
        .filter_map(|line| parse_name_type(line).filter(|(_, t)| *t == "wireguard"))
        .map(|(name, _)| Connection {
            name: name.to_owned(),
            kind: VpnKind::WireGuard,
            state: if active_names.contains(name) {
                ConnectionState::Active
            } else {
                ConnectionState::Inactive
            },
            detail: None,
        })
        .collect())
}

/// List OpenVPN connections.
///
/// nmcli reports all VPN-plugin connections with `TYPE=vpn`.
pub async fn openvpn_connections() -> Result<Vec<Connection>> {
    let all = run(&["-t", "-f", "NAME,TYPE", "connection", "show"]).await?;
    let active = run(&["-t", "-f", "NAME,TYPE", "connection", "show", "--active"]).await?;

    let active_vpn_names: HashSet<String> = active
        .lines()
        .filter_map(|line| {
            parse_name_type(line)
                .filter(|(_, t)| *t == "vpn")
                .map(|(n, _)| n.to_owned())
        })
        .collect();

    let candidates: Vec<String> = all
        .lines()
        .filter_map(|line| {
            parse_name_type(line)
                .filter(|(_, t)| *t == "vpn")
                .map(|(n, _)| n.to_owned())
        })
        .collect();

    let mut out = Vec::new();
    for name in candidates {
        if !is_openvpn(&name).await? {
            continue;
        }
        let state = if active_vpn_names.contains(&name) {
            ConnectionState::Active
        } else {
            ConnectionState::Inactive
        };
        out.push(Connection {
            name,
            kind: VpnKind::OpenVpn,
            state,
            detail: None,
        });
    }
    Ok(out)
}

async fn is_openvpn(name: &str) -> Result<bool> {
    let out = run(&["-t", "-f", "vpn.service-type", "connection", "show", name]).await?;
    // Output looks like: "vpn.service-type:org.freedesktop.NetworkManager.openvpn"
    Ok(out.trim().ends_with(".openvpn"))
}

// mutation

pub async fn connection_up(name: &str) -> Result<()> {
    run(&["connection", "up", name]).await?;
    Ok(())
}

pub async fn connection_down(name: &str) -> Result<()> {
    run(&["connection", "down", name]).await?;
    Ok(())
}

pub async fn import_wireguard(path: &Path, desired_name: &str) -> Result<()> {
    import("wireguard", path, desired_name).await
}

pub async fn import_openvpn(path: &Path, desired_name: &str) -> Result<()> {
    import("openvpn", path, desired_name).await
}

/// Shared import path. nmcli derives the connection name from the file stem
/// and we rename via `connection modify` if a different name was requested.
async fn import(plugin: &str, path: &Path, desired_name: &str) -> Result<()> {
    let path_str = path.to_string_lossy();
    run(&["connection", "import", "type", plugin, "file", &path_str]).await?;

    let imported = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(desired_name);

    if imported != desired_name {
        run(&[
            "connection",
            "modify",
            imported,
            "connection.id",
            desired_name,
        ])
        .await?;
    }
    Ok(())
}

// helpers

async fn run(args: &[&str]) -> Result<String> {
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

fn parse_name_type(line: &str) -> Option<(&str, &str)> {
    if line.is_empty() {
        return None;
    }
    let idx = line.rfind(':')?;
    Some((&line[..idx], &line[idx + 1..]))
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn parses_terse_line() {
        assert_eq!(
            parse_name_type("home-vpn:wireguard"),
            Some(("home-vpn", "wireguard"))
        );
        assert_eq!(parse_name_type("my-ovpn:vpn"), Some(("my-ovpn", "vpn")));
        assert_eq!(
            parse_name_type("wired:802-3-ethernet"),
            Some(("wired", "802-3-ethernet"))
        );
        assert_eq!(parse_name_type(""), None);
    }
}

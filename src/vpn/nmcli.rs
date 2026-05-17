//! `nmcli` wrapper. We invoke it with `--terse --fields …` so the output is
//! machine-parseable: colon-separated, no headers, escaping `:` and `\`.

use std::path::Path;

use tokio::process::Command;

use crate::error::{Error, Result};
use crate::vpn::{Connection, ConnectionState, VpnKind};

const NMCLI: &str = "nmcli";

/// List all WireGuard connections known to NetworkManager, marking each as
/// active or inactive.
pub async fn wireguard_connections() -> Result<Vec<Connection>> {
    let all = run(&["-t", "-f", "NAME,TYPE", "connection", "show"]).await?;
    let active = run(&["-t", "-f", "NAME,TYPE", "connection", "show", "--active"]).await?;

    let active_names: Vec<&str> = active
        .lines()
        .filter_map(|line| parse_name_type(line).filter(|(_, t)| *t == "wireguard").map(|(n, _)| n))
        .collect();

    let mut out = Vec::new();
    for line in all.lines() {
        let Some((name, kind)) = parse_name_type(line) else { continue };
        if kind != "wireguard" {
            continue;
        }
        let state = if active_names.contains(&name) {
            ConnectionState::Active
        } else {
            ConnectionState::Inactive
        };
        out.push(Connection {
            name: name.to_owned(),
            kind: VpnKind::WireGuard,
            state,
            detail: None,
        });
    }
    Ok(out)
}

pub async fn connection_up(name: &str) -> Result<()> {
    run(&["connection", "up", name]).await?;
    Ok(())
}

pub async fn connection_down(name: &str) -> Result<()> {
    run(&["connection", "down", name]).await?;
    Ok(())
}

/// Import a WireGuard `.conf` as a NetworkManager connection.
///
/// nmcli derives the connection name from the file stem. If `desired_name`
/// differs, we rename it with a follow-up call.
pub async fn import_wireguard(path: &Path, desired_name: &str) -> Result<()> {
    let path_str = path.to_string_lossy();
    run(&["connection", "import", "type", "wireguard", "file", &path_str]).await?;

    let imported_name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(desired_name);

    if imported_name != desired_name {
        run(&[
            "connection",
            "modify",
            imported_name,
            "connection.id",
            desired_name,
        ])
        .await?;
    }
    Ok(())
}

/// Run `nmcli` with the given args, returning stdout on success.
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

/// Parse a `NAME:TYPE` line from `nmcli -t`. Returns `None` for blanks.
///
/// nmcli's terse format escapes `:` as `\:` and `\` as `\\`. We split on the
/// last *unescaped* colon. For our purposes (TYPE is always one of a small
/// fixed set with no `:` in it), splitting on the final `:` works.
fn parse_name_type(line: &str) -> Option<(&str, &str)> {
    if line.is_empty() {
        return None;
    }
    let idx = line.rfind(':')?;
    Some((&line[..idx], &line[idx + 1..]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn parses_terse_line() {
        assert_eq!(parse_name_type("home-vpn:wireguard"), Some(("home-vpn", "wireguard")));
        assert_eq!(parse_name_type("wired:802-3-ethernet"), Some(("wired", "802-3-ethernet")));
        assert_eq!(parse_name_type(""), None);
    }
}

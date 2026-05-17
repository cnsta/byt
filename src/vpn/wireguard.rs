//! WireGuard `.conf` parsing.
//!
//! We do *not* try to be a full parser — `nmcli connection import` does that
//! and handles every quirk providers throw at it. We extract a handful of
//! fields purely so the user can preview what they're about to import.

use std::fmt;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

#[derive(Debug, Default, Clone)]
pub struct WgPreview {
    pub source: PathBuf,
    pub addresses: Vec<String>,
    pub dns: Vec<String>,
    pub peer_public_key: Option<String>,
    pub endpoint: Option<String>,
    pub allowed_ips: Vec<String>,
}

impl WgPreview {
    /// Sensible default name derived from the file stem.
    #[must_use]
    pub fn suggested_name(&self) -> String {
        self.source
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("wireguard")
            .to_owned()
    }
}

impl fmt::Display for WgPreview {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "WireGuard config preview: {}", self.source.display())?;
        if !self.addresses.is_empty() {
            writeln!(f, "  address     {}", self.addresses.join(", "))?;
        }
        if !self.dns.is_empty() {
            writeln!(f, "  dns         {}", self.dns.join(", "))?;
        }
        if let Some(ep) = &self.endpoint {
            writeln!(f, "  endpoint    {ep}")?;
        }
        if let Some(pk) = &self.peer_public_key {
            writeln!(f, "  peer key    {pk}")?;
        }
        if !self.allowed_ips.is_empty() {
            writeln!(f, "  allowed ips {}", self.allowed_ips.join(", "))?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Section {
    None,
    Interface,
    Peer,
}

/// Parse a `.conf` file for preview purposes.
///
/// Tolerant: ignores unknown keys, blanks, and `#` / `;` comments. Returns an
/// error only if the file can't be read or has no recognisable sections — the
/// caller can still hand it to nmcli if they want.
pub fn parse_conf(path: &Path) -> Result<WgPreview> {
    let raw = std::fs::read_to_string(path)?;
    let mut preview = WgPreview {
        source: path.to_path_buf(),
        ..Default::default()
    };
    let mut section = Section::None;
    let mut saw_known_section = false;

    for line in raw.lines() {
        let line = strip_comment(line).trim();
        if line.is_empty() {
            continue;
        }

        if let Some(name) = section_header(line) {
            section = match name.to_ascii_lowercase().as_str() {
                "interface" => {
                    saw_known_section = true;
                    Section::Interface
                }
                "peer" => {
                    saw_known_section = true;
                    Section::Peer
                }
                _ => Section::None,
            };
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();

        match (section, key.to_ascii_lowercase().as_str()) {
            (Section::Interface, "address") => {
                preview.addresses.extend(split_csv(value));
            }
            (Section::Interface, "dns") => {
                preview.dns.extend(split_csv(value));
            }
            (Section::Peer, "publickey") => {
                preview.peer_public_key = Some(value.to_owned());
            }
            (Section::Peer, "endpoint") => {
                preview.endpoint = Some(value.to_owned());
            }
            (Section::Peer, "allowedips") => {
                preview.allowed_ips.extend(split_csv(value));
            }
            _ => {}
        }
    }

    if !saw_known_section {
        return Err(Error::InvalidConfig {
            path: path.to_path_buf(),
            context: "no [Interface] or [Peer] section found".into(),
        });
    }
    Ok(preview)
}

fn section_header(line: &str) -> Option<&str> {
    line.strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
}

fn strip_comment(line: &str) -> &str {
    if let Some(idx) = line.find(['#', ';']) {
        &line[..idx]
    } else {
        line
    }
}

fn split_csv(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use std::io::Write;

    fn tmp_conf(body: &str) -> tempfile::NamedTempFile {
        let mut f = tempfile::Builder::new().suffix(".conf").tempfile().unwrap();
        f.write_all(body.as_bytes()).unwrap();
        f
    }

    #[test]
    fn parses_typical_provider_conf() {
        let f = tmp_conf(
            r#"
            [Interface]
            PrivateKey = abc=
            Address = 10.2.0.2/32, fd00::2/128
            DNS = 10.2.0.1, 1.1.1.1

            [Peer]
            PublicKey = xyz=
            AllowedIPs = 0.0.0.0/0, ::/0
            Endpoint = vpn.example.com:51820
            "#,
        );
        let p = parse_conf(f.path()).unwrap();
        assert_eq!(p.addresses, vec!["10.2.0.2/32", "fd00::2/128"]);
        assert_eq!(p.dns, vec!["10.2.0.1", "1.1.1.1"]);
        assert_eq!(p.peer_public_key.as_deref(), Some("xyz="));
        assert_eq!(p.endpoint.as_deref(), Some("vpn.example.com:51820"));
        assert_eq!(p.allowed_ips, vec!["0.0.0.0/0", "::/0"]);
    }

    #[test]
    fn ignores_comments_and_blanks() {
        let f = tmp_conf("# header\n\n[Interface]\n; aside\nAddress = 10.0.0.2/32\n");
        let p = parse_conf(f.path()).unwrap();
        assert_eq!(p.addresses, vec!["10.0.0.2/32"]);
    }

    #[test]
    fn rejects_file_with_no_sections() {
        let f = tmp_conf("Address = 10.0.0.2/32\n");
        assert!(matches!(
            parse_conf(f.path()),
            Err(Error::InvalidConfig { .. })
        ));
    }
}

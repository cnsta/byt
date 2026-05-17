//! OpenVPN `.ovpn` / `.conf` preview parser.
//!
//! As with [`crate::vpn::wireguard`], this is intentionally not a full parser
//! — `nmcli connection import type openvpn` does the real work. We only
//! extract the handful of fields needed for the import preview.

use std::fmt;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

#[derive(Debug, Default, Clone)]
pub struct OvpnPreview {
    pub source: PathBuf,
    pub remotes: Vec<Remote>,
    pub proto: Option<String>,
    pub cipher: Option<String>,
    pub data_ciphers: Vec<String>,
    pub auth_user_pass: bool,
    pub has_inline_ca: bool,
    pub has_inline_cert: bool,
    pub has_inline_key: bool,
    pub has_inline_tls_auth: bool,
    pub has_inline_tls_crypt: bool,
}

#[derive(Debug, Clone)]
pub struct Remote {
    pub host: String,
    pub port: Option<u16>,
}

impl OvpnPreview {
    #[must_use]
    pub fn suggested_name(&self) -> String {
        self.source
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("openvpn")
            .to_owned()
    }
}

impl fmt::Display for OvpnPreview {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "OpenVPN config preview: {}", self.source.display())?;
        for r in &self.remotes {
            match r.port {
                Some(p) => writeln!(f, "  remote      {}:{p}", r.host)?,
                None => writeln!(f, "  remote      {}", r.host)?,
            }
        }
        if let Some(p) = &self.proto {
            writeln!(f, "  proto       {p}")?;
        }
        if !self.data_ciphers.is_empty() {
            writeln!(f, "  ciphers     {}", self.data_ciphers.join(", "))?;
        } else if let Some(c) = &self.cipher {
            writeln!(f, "  cipher      {c}")?;
        }
        let auth = if self.auth_user_pass {
            "username/password"
        } else {
            "certificate"
        };
        writeln!(f, "  auth        {auth}")?;
        let inline = [
            ("ca", self.has_inline_ca),
            ("cert", self.has_inline_cert),
            ("key", self.has_inline_key),
            ("tls-auth", self.has_inline_tls_auth),
            ("tls-crypt", self.has_inline_tls_crypt),
        ];
        let present: Vec<&str> = inline.iter().filter_map(|(n, p)| p.then_some(*n)).collect();
        if !present.is_empty() {
            writeln!(f, "  inline      {}", present.join(", "))?;
        }
        Ok(())
    }
}

pub fn parse_conf(path: &Path) -> Result<OvpnPreview> {
    let raw = std::fs::read_to_string(path)?;
    let mut p = OvpnPreview {
        source: path.to_path_buf(),
        ..Default::default()
    };

    let mut inside: Option<String> = None;
    let mut saw_any = false;

    for line in raw.lines() {
        let trimmed = line.trim();

        // Inside an inline block — only watch for the closing tag.
        if let Some(tag) = &inside {
            if trimmed == format!("</{tag}>") {
                inside = None;
            }
            continue;
        }

        // Opening inline tag like `<ca>`.
        if let Some(rest) = trimmed.strip_prefix('<') {
            if let Some(tag) = rest.strip_suffix('>') {
                if !tag.starts_with('/') {
                    match tag {
                        "ca" => p.has_inline_ca = true,
                        "cert" => p.has_inline_cert = true,
                        "key" => p.has_inline_key = true,
                        "tls-auth" => p.has_inline_tls_auth = true,
                        "tls-crypt" => p.has_inline_tls_crypt = true,
                        _ => {}
                    }
                    inside = Some(tag.to_owned());
                    saw_any = true;
                    continue;
                }
            }
        }

        let clean = strip_comment(trimmed);
        if clean.is_empty() {
            continue;
        }
        saw_any = true;

        let mut parts = clean.split_whitespace();
        let Some(keyword) = parts.next() else {
            continue;
        };

        match keyword {
            "remote" => {
                if let Some(host) = parts.next() {
                    let port = parts.next().and_then(|s| s.parse().ok());
                    p.remotes.push(Remote {
                        host: host.to_owned(),
                        port,
                    });
                }
            }
            "proto" => p.proto = parts.next().map(str::to_owned),
            "cipher" => p.cipher = parts.next().map(str::to_owned),
            "data-ciphers" => {
                if let Some(list) = parts.next() {
                    p.data_ciphers = list.split(':').map(str::to_owned).collect();
                }
            }
            "auth-user-pass" => p.auth_user_pass = true,
            _ => {}
        }
    }

    if !saw_any {
        return Err(Error::InvalidConfig {
            path: path.to_path_buf(),
            context: "no OpenVPN directives found".into(),
        });
    }
    Ok(p)
}

fn strip_comment(line: &str) -> &str {
    if let Some(idx) = line.find(['#', ';']) {
        &line[..idx]
    } else {
        line
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use pretty_assertions::assert_eq;

    use super::*;

    fn tmp_conf(body: &str) -> tempfile::NamedTempFile {
        let mut f = tempfile::Builder::new().suffix(".ovpn").tempfile().unwrap();
        f.write_all(body.as_bytes()).unwrap();
        f
    }

    #[test]
    fn parses_userpass_with_inline_blocks() {
        let f = tmp_conf(
            r#"
            client
            dev tun
            remote vpn-1.example.com 1194
            remote vpn-2.example.com 1194
            proto udp
            cipher CHACHA20-POLY1305
            data-ciphers CHACHA20-POLY1305:AES-256-GCM
            auth-user-pass

            <ca>
            -----BEGIN CERTIFICATE-----
            PLACEHOLDER
            -----END CERTIFICATE-----
            </ca>

            <tls-auth>
            -----BEGIN OpenVPN Static key V1-----
            PLACEHOLDER
            -----END OpenVPN Static key V1-----
            </tls-auth>
            "#,
        );
        let p = parse_conf(f.path()).unwrap();
        assert_eq!(p.remotes.len(), 2);
        assert_eq!(p.remotes[0].host, "vpn-1.example.com");
        assert_eq!(p.remotes[0].port, Some(1194));
        assert_eq!(p.proto.as_deref(), Some("udp"));
        assert_eq!(p.cipher.as_deref(), Some("CHACHA20-POLY1305"));
        assert_eq!(p.data_ciphers, vec!["CHACHA20-POLY1305", "AES-256-GCM"]);
        assert!(p.auth_user_pass);
        assert!(p.has_inline_ca);
        assert!(p.has_inline_tls_auth);
        assert!(!p.has_inline_cert);
    }

    #[test]
    fn parses_cert_auth() {
        let f = tmp_conf(
            r"
            client
            dev tun
            remote vpn.example.com 1194
            proto udp

            <ca>
            PLACEHOLDER
            </ca>
            <cert>
            PLACEHOLDER
            </cert>
            <key>
            PLACEHOLDER
            </key>
            ",
        );
        let p = parse_conf(f.path()).unwrap();
        assert!(!p.auth_user_pass);
        assert!(p.has_inline_ca && p.has_inline_cert && p.has_inline_key);
    }

    #[test]
    fn ignores_comments() {
        let f = tmp_conf("# header\nclient\n; aside\nproto tcp\n");
        let p = parse_conf(f.path()).unwrap();
        assert_eq!(p.proto.as_deref(), Some("tcp"));
    }
}

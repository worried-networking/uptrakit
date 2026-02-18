//! SSH target address parsing.
//!
//! Parses connection targets in the standard SSH address format:
//! - `[user@]host[:port]`
//! - `ssh://[user@]host[:port]`
//! - IPv6 bracket notation: `[::1]`, `user@[::1]:22`

use std::fmt;
use std::str::FromStr;

use thiserror::Error;

/// A parsed SSH connection target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshTarget {
    pub username: Option<String>,
    pub hostname: String,
    pub port: Option<u16>,
}

/// Errors produced while parsing an SSH target string.
#[derive(Debug, Error)]
pub enum ParseSshTargetError {
    #[error("empty target string")]
    Empty,

    #[error("empty hostname")]
    EmptyHostname,

    #[error("invalid port: {0}")]
    InvalidPort(String),

    #[error("invalid SSH URL: {0}")]
    InvalidUrl(String),
}

impl fmt::Display for SshTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(ref user) = self.username {
            write!(f, "{user}@")?;
        }
        // Use brackets for IPv6 addresses.
        if self.hostname.contains(':') {
            write!(f, "[{}]", self.hostname)?;
        } else {
            write!(f, "{}", self.hostname)?;
        }
        if let Some(port) = self.port {
            write!(f, ":{port}")?;
        }
        Ok(())
    }
}

impl FromStr for SshTarget {
    type Err = ParseSshTargetError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        if s.is_empty() {
            return Err(ParseSshTargetError::Empty);
        }

        if s.starts_with("ssh://") {
            parse_ssh_url(s)
        } else {
            parse_plain(s)
        }
    }
}

/// Parse `ssh://[user@]host[:port]` using the `url` crate.
fn parse_ssh_url(s: &str) -> Result<SshTarget, ParseSshTargetError> {
    let parsed = url::Url::parse(s).map_err(|e| ParseSshTargetError::InvalidUrl(e.to_string()))?;

    if parsed.scheme() != "ssh" {
        return Err(ParseSshTargetError::InvalidUrl(format!(
            "expected 'ssh' scheme, got '{}'",
            parsed.scheme()
        )));
    }

    let raw_host = parsed
        .host_str()
        .ok_or(ParseSshTargetError::EmptyHostname)?;
    if raw_host.is_empty() {
        return Err(ParseSshTargetError::EmptyHostname);
    }
    // The `url` crate includes brackets for IPv6 addresses; strip them.
    let hostname = raw_host
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(raw_host);

    let username = if parsed.username().is_empty() {
        None
    } else {
        Some(parsed.username().to_string())
    };

    let port = parsed.port();

    Ok(SshTarget {
        username,
        hostname: hostname.to_string(),
        port,
    })
}

/// Parse `[user@]host[:port]` with support for IPv6 bracket notation.
fn parse_plain(s: &str) -> Result<SshTarget, ParseSshTargetError> {
    // Split optional `user@` prefix.
    let (username, host_port) = if let Some(at_pos) = find_user_at(s) {
        let user = &s[..at_pos];
        if user.is_empty() {
            (None, &s[at_pos + 1..])
        } else {
            (Some(user.to_string()), &s[at_pos + 1..])
        }
    } else {
        (None, s)
    };

    let (hostname, port) = parse_host_port(host_port)?;

    if hostname.is_empty() {
        return Err(ParseSshTargetError::EmptyHostname);
    }

    Ok(SshTarget {
        username,
        hostname,
        port,
    })
}

/// Find the `@` that separates user from host.
///
/// Returns `None` if there's no `@`, or if `@` only appears inside brackets
/// (e.g. IPv6 addresses should not be split on `@`).
fn find_user_at(s: &str) -> Option<usize> {
    // If the string starts with `[`, the entire thing is a bracketed host
    // (possibly with port), so there's no user part.
    if s.starts_with('[') {
        return None;
    }
    s.find('@')
}

/// Parse `host[:port]` where host may be a bracketed IPv6 address.
fn parse_host_port(s: &str) -> Result<(String, Option<u16>), ParseSshTargetError> {
    if s.starts_with('[') {
        // IPv6 bracket notation: `[::1]` or `[::1]:22`
        let close = s.find(']').ok_or_else(|| {
            ParseSshTargetError::InvalidUrl("missing closing bracket for IPv6 address".to_string())
        })?;
        let host = &s[1..close];
        let rest = &s[close + 1..];

        if rest.is_empty() {
            Ok((host.to_string(), None))
        } else if let Some(port_str) = rest.strip_prefix(':') {
            let port = parse_port(port_str)?;
            Ok((host.to_string(), Some(port)))
        } else {
            Err(ParseSshTargetError::InvalidUrl(format!(
                "unexpected characters after closing bracket: '{rest}'"
            )))
        }
    } else {
        // Plain hostname or IPv4: `host` or `host:port`
        // We need to be careful: if the host part itself contains colons
        // (bare IPv6 without brackets), we treat the entire string as a
        // hostname when there are multiple colons.
        let colon_count = s.chars().filter(|&c| c == ':').count();
        if colon_count == 0 {
            Ok((s.to_string(), None))
        } else if colon_count == 1 {
            let Some((host, port_str)) = s.rsplit_once(':') else {
                return Ok((s.to_string(), None));
            };
            let port = parse_port(port_str)?;
            Ok((host.to_string(), Some(port)))
        } else {
            // Multiple colons: bare IPv6 address (no port).
            Ok((s.to_string(), None))
        }
    }
}

fn parse_port(s: &str) -> Result<u16, ParseSshTargetError> {
    if s.is_empty() {
        return Err(ParseSshTargetError::InvalidPort("empty port".to_string()));
    }
    s.parse::<u16>()
        .map_err(|e| ParseSshTargetError::InvalidPort(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Plain format ────────────────────────────────────────────────

    #[test]
    fn plain_host_only() {
        let target: SshTarget = "example.com".parse().expect("should parse");
        assert_eq!(target.hostname, "example.com");
        assert!(target.username.is_none());
        assert!(target.port.is_none());
    }

    #[test]
    fn plain_host_with_port() {
        let target: SshTarget = "example.com:2222".parse().expect("should parse");
        assert_eq!(target.hostname, "example.com");
        assert!(target.username.is_none());
        assert_eq!(target.port, Some(2222));
    }

    #[test]
    fn plain_user_and_host() {
        let target: SshTarget = "root@example.com".parse().expect("should parse");
        assert_eq!(target.hostname, "example.com");
        assert_eq!(target.username.as_deref(), Some("root"));
        assert!(target.port.is_none());
    }

    #[test]
    fn plain_user_host_and_port() {
        let target: SshTarget = "admin@192.168.1.1:22".parse().expect("should parse");
        assert_eq!(target.hostname, "192.168.1.1");
        assert_eq!(target.username.as_deref(), Some("admin"));
        assert_eq!(target.port, Some(22));
    }

    #[test]
    fn plain_ipv4() {
        let target: SshTarget = "10.0.0.1".parse().expect("should parse");
        assert_eq!(target.hostname, "10.0.0.1");
        assert!(target.username.is_none());
        assert!(target.port.is_none());
    }

    // ── IPv6 ────────────────────────────────────────────────────────

    #[test]
    fn ipv6_bracketed() {
        let target: SshTarget = "[::1]".parse().expect("should parse");
        assert_eq!(target.hostname, "::1");
        assert!(target.username.is_none());
        assert!(target.port.is_none());
    }

    #[test]
    fn ipv6_bracketed_with_port() {
        let target: SshTarget = "[::1]:2222".parse().expect("should parse");
        assert_eq!(target.hostname, "::1");
        assert!(target.username.is_none());
        assert_eq!(target.port, Some(2222));
    }

    #[test]
    fn ipv6_bracketed_with_user() {
        let target: SshTarget = "root@[::1]".parse().expect("should parse");
        assert_eq!(target.hostname, "::1");
        assert_eq!(target.username.as_deref(), Some("root"));
        assert!(target.port.is_none());
    }

    #[test]
    fn ipv6_bracketed_with_user_and_port() {
        let target: SshTarget = "root@[::1]:22".parse().expect("should parse");
        assert_eq!(target.hostname, "::1");
        assert_eq!(target.username.as_deref(), Some("root"));
        assert_eq!(target.port, Some(22));
    }

    #[test]
    fn ipv6_full_address_bracketed() {
        let target: SshTarget = "[2001:db8::1]".parse().expect("should parse");
        assert_eq!(target.hostname, "2001:db8::1");
    }

    #[test]
    fn ipv6_bare_no_port() {
        // Bare IPv6 without brackets — multiple colons, treated as hostname.
        let target: SshTarget = "::1".parse().expect("should parse");
        assert_eq!(target.hostname, "::1");
        assert!(target.port.is_none());
    }

    // ── SSH URL format ──────────────────────────────────────────────

    #[test]
    fn ssh_url_host_only() {
        let target: SshTarget = "ssh://example.com".parse().expect("should parse");
        assert_eq!(target.hostname, "example.com");
        assert!(target.username.is_none());
        assert!(target.port.is_none());
    }

    #[test]
    fn ssh_url_with_port() {
        let target: SshTarget = "ssh://example.com:2222".parse().expect("should parse");
        assert_eq!(target.hostname, "example.com");
        assert_eq!(target.port, Some(2222));
    }

    #[test]
    fn ssh_url_with_user() {
        let target: SshTarget = "ssh://root@example.com".parse().expect("should parse");
        assert_eq!(target.hostname, "example.com");
        assert_eq!(target.username.as_deref(), Some("root"));
    }

    #[test]
    fn ssh_url_full() {
        let target: SshTarget = "ssh://admin@example.com:22".parse().expect("should parse");
        assert_eq!(target.hostname, "example.com");
        assert_eq!(target.username.as_deref(), Some("admin"));
        assert_eq!(target.port, Some(22));
    }

    #[test]
    fn ssh_url_ipv6() {
        let target: SshTarget = "ssh://[::1]:22".parse().expect("should parse");
        assert_eq!(target.hostname, "::1");
        assert_eq!(target.port, Some(22));
    }

    #[test]
    fn ssh_url_ipv6_with_user() {
        let target: SshTarget = "ssh://root@[::1]".parse().expect("should parse");
        assert_eq!(target.hostname, "::1");
        assert_eq!(target.username.as_deref(), Some("root"));
    }

    // ── Edge cases ──────────────────────────────────────────────────

    #[test]
    fn whitespace_is_trimmed() {
        let target: SshTarget = "  example.com  ".parse().expect("should parse");
        assert_eq!(target.hostname, "example.com");
    }

    #[test]
    fn empty_string_fails() {
        let err = "".parse::<SshTarget>().expect_err("should fail");
        assert!(matches!(err, ParseSshTargetError::Empty));
    }

    #[test]
    fn empty_hostname_fails() {
        let err = "root@".parse::<SshTarget>().expect_err("should fail");
        assert!(matches!(err, ParseSshTargetError::EmptyHostname));
    }

    #[test]
    fn invalid_port_fails() {
        let err = "example.com:abc"
            .parse::<SshTarget>()
            .expect_err("should fail");
        assert!(matches!(err, ParseSshTargetError::InvalidPort(_)));
    }

    #[test]
    fn port_overflow_fails() {
        let err = "example.com:99999"
            .parse::<SshTarget>()
            .expect_err("should fail");
        assert!(matches!(err, ParseSshTargetError::InvalidPort(_)));
    }

    #[test]
    fn empty_port_fails() {
        let err = "example.com:"
            .parse::<SshTarget>()
            .expect_err("should fail");
        assert!(matches!(err, ParseSshTargetError::InvalidPort(_)));
    }

    #[test]
    fn ssh_url_empty_host_fails() {
        let err = "ssh://".parse::<SshTarget>().expect_err("should fail");
        assert!(matches!(
            err,
            ParseSshTargetError::EmptyHostname | ParseSshTargetError::InvalidUrl(_)
        ));
    }

    // ── Display round-trip ──────────────────────────────────────────

    #[test]
    fn display_plain_host() {
        let target = SshTarget {
            username: None,
            hostname: "example.com".to_string(),
            port: None,
        };
        assert_eq!(target.to_string(), "example.com");
    }

    #[test]
    fn display_full() {
        let target = SshTarget {
            username: Some("root".to_string()),
            hostname: "example.com".to_string(),
            port: Some(22),
        };
        assert_eq!(target.to_string(), "root@example.com:22");
    }

    #[test]
    fn display_ipv6() {
        let target = SshTarget {
            username: Some("root".to_string()),
            hostname: "::1".to_string(),
            port: Some(22),
        };
        assert_eq!(target.to_string(), "root@[::1]:22");
    }

    // ── SSH alias (short hostname) ──────────────────────────────────

    #[test]
    fn short_hostname_alias() {
        let target: SshTarget = "myserver".parse().expect("should parse");
        assert_eq!(target.hostname, "myserver");
        assert!(target.username.is_none());
        assert!(target.port.is_none());
    }

    #[test]
    fn alias_with_user() {
        let target: SshTarget = "deploy@prod".parse().expect("should parse");
        assert_eq!(target.hostname, "prod");
        assert_eq!(target.username.as_deref(), Some("deploy"));
    }
}

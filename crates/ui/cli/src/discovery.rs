//! CLI-side controller discovery: browse the LAN for an advertised controller
//! during `auth login` when no server is configured.
//!
//! The interactive confirmation here is UX only — the security gate is the
//! TOFU ceremony in `commands::auth` (see
//! docs/security/zeroconf-discovery.md).

use std::time::Duration;

use uptrakit_zeroconf::DiscoveredController;

use crate::commands::auth::prompt;
use crate::error::Result;

/// Full browse window when nothing has answered yet.
const BROWSE_WINDOW: Duration = Duration::from_secs(10);
/// Grace period after the first response to catch near-simultaneous responders.
const BROWSE_SETTLE: Duration = Duration::from_secs(2);

/// Which path `login()` takes to obtain the server URL.
#[derive(Debug, PartialEq, Eq)]
pub enum ServerSource {
    /// `--server`/`UPTRAKIT_SERVER` was given — use it verbatim.
    Explicit(String),
    /// A stored `config.server` exists — prompt with it as the default.
    PromptWithDefault(String),
    /// Nothing configured — attempt zeroconf discovery.
    Discover,
}

/// Decide how `login()` obtains the server URL (precedence:
/// explicit flag/env > stored config > discovery).
pub fn resolve_server_source(
    server_override: Option<&str>,
    config_server: Option<&str>,
) -> ServerSource {
    if let Some(server) = server_override {
        ServerSource::Explicit(server.to_string())
    } else if let Some(stored) = config_server {
        ServerSource::PromptWithDefault(stored.to_string())
    } else {
        ServerSource::Discover
    }
}

/// Outcome of comparing the mDNS-advertised CA fingerprint with the fetched one.
#[derive(Debug, PartialEq, Eq)]
pub enum CrossCheck {
    /// Advertisement matches (or none was advertised) — proceed silently.
    Ok,
    /// Mismatch with an explicit `--tofu=<fp>` supplied: advisory only — the
    /// operator's out-of-band fingerprint outranks the untrusted advertisement.
    Warn(String),
    /// Mismatch without an explicit fingerprint: abort the login.
    Fail(String),
}

/// Consistency cross-check of the advertised CA fingerprint against the CA
/// the server actually serves. NOT an MITM defense: an attacker controlling
/// both mDNS and the endpoint passes it trivially. It catches split control
/// and stale/misconfigured advertisements.
pub fn cross_check_advertised(
    advertised: Option<&str>,
    fetched: &str,
    has_explicit_fp: bool,
) -> CrossCheck {
    let Some(advertised) = advertised else {
        return CrossCheck::Ok;
    };
    if advertised.eq_ignore_ascii_case(fetched) {
        return CrossCheck::Ok;
    }
    let msg = format!(
        "mDNS-advertised CA fingerprint ({advertised}) does not match the \
         fingerprint of the CA the controller serves ({fetched})"
    );
    if has_explicit_fp {
        CrossCheck::Warn(msg)
    } else {
        CrossCheck::Fail(msg)
    }
}

/// Parse a 1-based menu selection against a menu of `count` entries.
///
/// Bounds-dependent menu-input handling (`usize::from_str` + range check),
/// not a string-to-domain-type conversion — the `FromStr` rule does not
/// apply here.
pub fn parse_selection(input: &str, count: usize) -> Option<usize> {
    let index: usize = input.trim().parse().ok()?;
    if (1..=count).contains(&index) {
        Some(index - 1)
    } else {
        None
    }
}

fn fp_display(fp: Option<&str>) -> String {
    fp.map_or_else(|| "none advertised".to_string(), |f| format!("SHA256:{f}"))
}

/// Browse the LAN and let the operator confirm or pick a controller.
///
/// Returns `Ok(Some(controller))` on acceptance, `Ok(None)` to fall back to
/// the manual server prompt (nothing found after the full window, browse
/// unavailable, declined confirmation, or invalid selection).
///
/// # Errors
///
/// Returns an error only when reading operator input fails.
pub async fn discover_server_interactive() -> Result<Option<DiscoveredController>> {
    eprintln!("Searching for a controller on the local network (up to 10 s)…");
    let found = match uptrakit_zeroconf::browse_all(BROWSE_WINDOW, BROWSE_SETTLE).await {
        Ok(found) => found,
        Err(e) => {
            // A browse failure (no multicast, sandboxed socket) must not kill
            // login — fall back to manual entry.
            eprintln!("Warning: mDNS browse failed ({e}); falling back to manual entry.");
            return Ok(None);
        }
    };

    if found.len() == 1 {
        let Some(controller) = found.into_iter().next() else {
            return Ok(None);
        };
        eprintln!("Discovered controller: {}", controller.url);
        eprintln!(
            "  Advertised CA fingerprint: {}",
            fp_display(controller.ca_fingerprint.as_deref())
        );
        let answer = prompt("Use this controller? [y/N]: ")?;
        if matches!(answer.to_ascii_lowercase().as_str(), "y" | "yes") {
            return Ok(Some(controller));
        }
        return Ok(None);
    }

    if found.is_empty() {
        return Ok(None);
    }

    eprintln!("Discovered multiple controllers:");
    for (i, controller) in found.iter().enumerate() {
        eprintln!(
            "  {}. {} (CA fingerprint: {})",
            i + 1,
            controller.url,
            fp_display(controller.ca_fingerprint.as_deref())
        );
    }
    let answer = prompt("Select a controller by number (empty to enter manually): ")?;
    match parse_selection(&answer, found.len()) {
        Some(index) => Ok(found.into_iter().nth(index)),
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_prefers_explicit_over_config() {
        assert_eq!(
            resolve_server_source(Some("https://a"), Some("https://b")),
            ServerSource::Explicit("https://a".to_string())
        );
    }

    #[test]
    fn resolve_uses_config_when_no_override() {
        assert_eq!(
            resolve_server_source(None, Some("https://b")),
            ServerSource::PromptWithDefault("https://b".to_string())
        );
    }

    #[test]
    fn resolve_discovers_when_nothing_configured() {
        assert_eq!(resolve_server_source(None, None), ServerSource::Discover);
    }

    #[test]
    fn cross_check_match_is_ok() {
        assert_eq!(
            cross_check_advertised(Some("abcd12"), "abcd12", false),
            CrossCheck::Ok
        );
    }

    #[test]
    fn cross_check_is_case_insensitive() {
        assert_eq!(
            cross_check_advertised(Some("ABCD12"), "abcd12", false),
            CrossCheck::Ok
        );
    }

    #[test]
    fn cross_check_mismatch_without_explicit_fp_fails() {
        assert!(matches!(
            cross_check_advertised(Some("abcd12"), "ffff00", false),
            CrossCheck::Fail(_)
        ));
    }

    #[test]
    fn cross_check_mismatch_with_explicit_fp_warns_not_fails() {
        assert!(matches!(
            cross_check_advertised(Some("abcd12"), "ffff00", true),
            CrossCheck::Warn(_)
        ));
    }

    #[test]
    fn cross_check_absent_advertisement_is_ok() {
        assert_eq!(cross_check_advertised(None, "abcd12", true), CrossCheck::Ok);
        assert_eq!(
            cross_check_advertised(None, "abcd12", false),
            CrossCheck::Ok
        );
    }

    #[test]
    fn parse_selection_accepts_in_range() {
        assert_eq!(parse_selection("1", 3), Some(0));
        assert_eq!(parse_selection(" 3 ", 3), Some(2));
    }

    #[test]
    fn parse_selection_rejects_out_of_range_garbage_and_empty() {
        assert_eq!(parse_selection("0", 3), None);
        assert_eq!(parse_selection("4", 3), None);
        assert_eq!(parse_selection("abc", 3), None);
        assert_eq!(parse_selection("", 3), None);
    }
}

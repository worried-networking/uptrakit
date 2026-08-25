//! Typed redirect policy with per-hop security guards.

use std::collections::HashSet;

use rootcause::prelude::*;

use crate::SsrfMode;

/// Redirect policy for plugin HTTP clients.
///
/// Closed enum by owner decision — do NOT add `#[non_exhaustive]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RedirectMode {
    /// Never follow redirects.
    None,
    /// Follow up to `hops` redirects; every hop passes [`check_hop`]:
    /// no https-to-http downgrade, and no private-IP-literal target
    /// under [`SsrfMode::Strict`].
    Limited {
        /// Maximum number of redirect hops to follow.
        hops: usize,
    },
}

/// A redirect hop violated a guard.
#[derive(Debug, Clone, thiserror::Error)]
pub enum HopGuardError {
    /// A hop left https for http.
    #[error("redirect downgrades {previous} (https) to {target} (http)")]
    SchemeDowngrade {
        /// The URL the redirect chain was following before this hop.
        previous: String,
        /// The URL this hop redirects to.
        target: String,
    },
    /// A hop targets a private/loopback IP literal.
    #[error("redirect targets private address {addr} via {target}")]
    PrivateTarget {
        /// The private/loopback address the hop targets.
        addr: std::net::IpAddr,
        /// The URL this hop redirects to.
        target: String,
    },
}

/// Classify one redirect hop.
///
/// `Ok(None)`: the hop is clean. `Ok(Some(err))`: the hop trips a guard
/// that [`SsrfMode::Permissive`] follows anyway — the caller must warn.
/// `Err(report)`: the hop dies (both modes for scheme downgrade; Strict
/// for private targets).
pub fn check_hop(
    previous: &reqwest::Url,
    target: &reqwest::Url,
    mode: SsrfMode,
) -> Result<Option<HopGuardError>, Report<HopGuardError>> {
    if previous.scheme() == "https" && target.scheme() == "http" {
        bail!(HopGuardError::SchemeDowngrade {
            previous: previous.to_string(),
            target: target.to_string(),
        });
    }
    let ip = match target.host() {
        Some(url::Host::Ipv4(v4)) => Some(std::net::IpAddr::V4(v4)),
        Some(url::Host::Ipv6(v6)) => Some(std::net::IpAddr::V6(v6)),
        _ => None,
    };
    if let Some(addr) = ip
        && uptrakit_shared_types::network::is_private_ip(addr)
    {
        let err = HopGuardError::PrivateTarget {
            addr,
            target: target.to_string(),
        };
        match mode {
            SsrfMode::Strict => bail!(err),
            SsrfMode::Permissive => return Ok(Some(err)),
        }
    }
    Ok(None)
}

/// Cap on distinct warned hosts per client.
/// ponytail: when full, warn once then treat unseen hosts as seen —
/// bounded memory wins over warn fidelity for a per-client set.
const MAX_WARNED_HOSTS: usize = 128;

struct WarnDedupe {
    hosts: HashSet<String>,
    full_warned: bool,
}

enum DedupeOutcome {
    /// First sighting: warn at WARN level.
    Warn,
    /// Repeat host (or set full and already announced): DEBUG only.
    Debug,
    /// Set just filled: emit the one-time "set full" WARN.
    FullWarn,
}

fn dedupe_decision(dedupe: &mut WarnDedupe, host: &str) -> DedupeOutcome {
    if dedupe.hosts.contains(host) {
        return DedupeOutcome::Debug;
    }
    if dedupe.hosts.len() >= MAX_WARNED_HOSTS {
        if dedupe.full_warned {
            return DedupeOutcome::Debug;
        }
        dedupe.full_warned = true;
        return DedupeOutcome::FullWarn;
    }
    dedupe.hosts.insert(host.to_string());
    DedupeOutcome::Warn
}

impl RedirectMode {
    /// Build the reqwest policy for this mode.
    #[must_use]
    pub(crate) fn into_policy(self, mode: SsrfMode) -> reqwest::redirect::Policy {
        match self {
            Self::None => reqwest::redirect::Policy::none(),
            Self::Limited { hops } => limited_policy(hops, mode),
        }
    }
}

fn limited_policy(hops: usize, mode: SsrfMode) -> reqwest::redirect::Policy {
    let counting = reqwest::redirect::Policy::limited(hops);
    let warned = parking_lot::Mutex::new(WarnDedupe {
        hosts: HashSet::new(),
        full_warned: false,
    });
    reqwest::redirect::Policy::custom(move |attempt| {
        let target = attempt.url().clone();
        let Some(previous) = attempt.previous().last().cloned() else {
            // A redirect attempt always carries at least the original URL;
            // if reqwest ever hands us none, fall through to pure counting.
            return counting.redirect(attempt);
        };
        match check_hop(&previous, &target, mode) {
            Ok(None) => counting.redirect(attempt),
            Ok(Some(guard)) => {
                let host = target.host_str().unwrap_or("<no-host>").to_string();
                let outcome = {
                    let mut dedupe = warned.lock();
                    dedupe_decision(&mut dedupe, &host)
                };
                match outcome {
                    DedupeOutcome::Warn => tracing::warn!(
                        previous = %previous, target = %target, guard = %guard,
                        "permissive mode followed a guarded redirect hop"
                    ),
                    DedupeOutcome::FullWarn => tracing::warn!(
                        previous = %previous, target = %target, guard = %guard,
                        "permissive redirect warn-dedupe set full; further hosts logged at debug"
                    ),
                    DedupeOutcome::Debug => tracing::debug!(
                        previous = %previous, target = %target, guard = %guard,
                        "permissive mode followed a guarded redirect hop (repeat host)"
                    ),
                }
                counting.redirect(attempt)
            }
            Err(report) => {
                tracing::warn!(
                    previous = %previous, target = %target, guard = %report.current_context(),
                    "redirect hop rejected"
                );
                // Fixed foreign signature: attempt.error wants a boxed
                // std error; clone the context out of the report.
                let err: HopGuardError = report.current_context().clone();
                attempt.error(err)
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(s: &str) -> reqwest::Url {
        s.parse::<reqwest::Url>().expect("test url")
    }

    #[test]
    fn downgrade_dies_in_both_modes() {
        for mode in [SsrfMode::Strict, SsrfMode::Permissive] {
            let result = check_hop(&url("https://a.example/"), &url("http://a.example/"), mode);
            let err = result.expect_err("downgrade must be rejected");
            assert!(matches!(
                err.current_context(),
                HopGuardError::SchemeDowngrade { .. }
            ));
        }
    }

    #[test]
    fn https_to_https_and_http_to_https_pass() {
        for mode in [SsrfMode::Strict, SsrfMode::Permissive] {
            let result = check_hop(&url("https://a.example/"), &url("https://b.example/"), mode);
            assert!(matches!(result, Ok(None)));

            let result = check_hop(&url("http://a.example/"), &url("https://b.example/"), mode);
            assert!(matches!(result, Ok(None)));
        }
    }

    #[test]
    fn private_ip_literal_strict_dies() {
        for target in ["http://10.0.0.8/x", "http://127.0.0.1/", "http://[::1]/"] {
            let result = check_hop(
                &url("http://origin.example/"),
                &url(target),
                SsrfMode::Strict,
            );
            let err = result.expect_err("private target must be rejected under Strict");
            assert!(matches!(
                err.current_context(),
                HopGuardError::PrivateTarget { .. }
            ));
        }
    }

    #[test]
    fn private_ip_literal_permissive_classifies() {
        for target in ["http://10.0.0.8/x", "http://127.0.0.1/", "http://[::1]/"] {
            let result = check_hop(
                &url("http://origin.example/"),
                &url(target),
                SsrfMode::Permissive,
            );
            match result {
                Ok(Some(HopGuardError::PrivateTarget { .. })) => {}
                other => panic!("expected Ok(Some(PrivateTarget)), got {other:?}"),
            }
        }
    }

    #[test]
    fn public_ip_literal_passes_in_strict() {
        for target in ["http://93.184.216.34/", "http://[2001:4860:4860::8888]/"] {
            let result = check_hop(
                &url("http://origin.example/"),
                &url(target),
                SsrfMode::Strict,
            );
            assert!(matches!(result, Ok(None)));
        }
    }

    #[test]
    fn hostname_target_passes() {
        let result = check_hop(
            &url("https://origin.example/"),
            &url("https://cdn.example/asset"),
            SsrfMode::Strict,
        );
        assert!(matches!(result, Ok(None)));
    }

    #[test]
    fn first_host_warns_then_debugs() {
        let mut dedupe = WarnDedupe {
            hosts: HashSet::new(),
            full_warned: false,
        };
        assert!(matches!(
            dedupe_decision(&mut dedupe, "host-a"),
            DedupeOutcome::Warn
        ));
        assert!(matches!(
            dedupe_decision(&mut dedupe, "host-a"),
            DedupeOutcome::Debug
        ));
    }

    #[test]
    fn full_set_warns_once() {
        let mut dedupe = WarnDedupe {
            hosts: HashSet::new(),
            full_warned: false,
        };
        for i in 0..MAX_WARNED_HOSTS {
            let host = format!("host-{i}");
            assert!(matches!(
                dedupe_decision(&mut dedupe, &host),
                DedupeOutcome::Warn
            ));
        }
        assert!(matches!(
            dedupe_decision(&mut dedupe, "overflow-host-1"),
            DedupeOutcome::FullWarn
        ));
        assert!(matches!(
            dedupe_decision(&mut dedupe, "overflow-host-2"),
            DedupeOutcome::Debug
        ));
    }
}

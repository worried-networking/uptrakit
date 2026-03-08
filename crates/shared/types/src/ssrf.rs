//! SSRF-safe DNS resolver for `reqwest` HTTP clients.
//!
//! Provides [`SsrfSafeResolver`], a [`reqwest::dns::Resolve`] implementation
//! that filters resolved addresses through [`is_private_ip`] to prevent DNS
//! rebinding attacks.  Even if a hostname passes the static
//! [`is_private_host`](crate::network::is_private_host) check at config-save
//! time, a DNS rebinding attack can cause the hostname to resolve to a private
//! IP at request time.  By rejecting private IPs at the resolver level, this
//! module provides defence-in-depth against SSRF.
//!
//! # Usage
//!
//! ```rust,ignore
//! use std::sync::Arc;
//! use uptrakit_shared_types::ssrf::SsrfSafeResolver;
//!
//! let client = reqwest::Client::builder()
//!     .dns_resolver(Arc::new(SsrfSafeResolver::new()))
//!     .build()?;
//! ```
//!
//! For self-hosted deployments where private URLs are intentionally allowed,
//! use [`SsrfSafeResolver::permissive()`] which resolves all addresses without
//! filtering.

use std::net::SocketAddr;

use reqwest::dns::{Addrs, Name, Resolve, Resolving};

use crate::network::is_private_ip;

/// A [`reqwest::dns::Resolve`] implementation that blocks private IP addresses.
///
/// When `allow_private` is `false` (the default via [`SsrfSafeResolver::new`]),
/// any DNS lookup that resolves exclusively to private/loopback/link-local
/// addresses will fail with an error.  If the lookup returns a mix of public
/// and private addresses, only the public addresses are returned.
///
/// When `allow_private` is `true` (via [`SsrfSafeResolver::permissive`]), all
/// resolved addresses are returned unchanged — this is a pass-through mode for
/// deployments that intentionally allow private URLs.
pub struct SsrfSafeResolver {
    allow_private: bool,
}

impl SsrfSafeResolver {
    /// Create a resolver that blocks private IP addresses.
    pub fn new() -> Self {
        Self {
            allow_private: false,
        }
    }

    /// Create a resolver that allows all addresses (pass-through mode).
    pub fn permissive() -> Self {
        Self {
            allow_private: true,
        }
    }
}

impl Default for SsrfSafeResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl Resolve for SsrfSafeResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let allow_private = self.allow_private;
        Box::pin(async move {
            let host = format!("{}:0", name.as_str());
            let addrs: Vec<SocketAddr> = tokio::net::lookup_host(&host)
                .await
                .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { Box::new(e) })?
                .collect();

            if addrs.is_empty() {
                return Err("DNS lookup returned no addresses".into());
            }

            if allow_private {
                let addrs: Addrs = Box::new(addrs.into_iter());
                return Ok(addrs);
            }

            let public: Vec<SocketAddr> = addrs
                .into_iter()
                .filter(|addr| !is_private_ip(addr.ip()))
                .collect();

            if public.is_empty() {
                return Err(
                    "DNS lookup resolved to private/loopback addresses (blocked by SSRF protection)"
                        .into(),
                );
            }

            let addrs: Addrs = Box::new(public.into_iter());
            Ok(addrs)
        })
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;

    #[tokio::test]
    async fn resolves_public_hostname() {
        let resolver = SsrfSafeResolver::new();
        // example.com is an IANA-reserved domain that resolves to a public IP.
        let result = resolver.resolve(Name::from_str("example.com").unwrap());
        // This test requires network access; it verifies the happy path.
        // If DNS fails (CI without network), we just skip the assertion.
        if let Ok(addrs) = result.await {
            let addrs: Vec<_> = addrs.collect();
            assert!(!addrs.is_empty());
            for addr in &addrs {
                assert!(
                    !is_private_ip(addr.ip()),
                    "public hostname should not resolve to private IP: {addr}"
                );
            }
        }
    }

    #[tokio::test]
    async fn blocks_localhost_resolution() {
        let resolver = SsrfSafeResolver::new();
        let result = resolver.resolve(Name::from_str("localhost").unwrap()).await;
        assert!(
            result.is_err(),
            "localhost should be blocked by SSRF resolver"
        );
    }

    #[tokio::test]
    async fn permissive_allows_localhost() {
        let resolver = SsrfSafeResolver::permissive();
        let result = resolver.resolve(Name::from_str("localhost").unwrap()).await;
        assert!(result.is_ok(), "permissive resolver should allow localhost");
    }

    #[test]
    fn default_is_restrictive() {
        let resolver = SsrfSafeResolver::default();
        assert!(!resolver.allow_private);
    }
}

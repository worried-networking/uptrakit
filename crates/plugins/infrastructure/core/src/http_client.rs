//! Shared HTTP client builder for plugins that make outbound HTTP requests.
//!
//! All plugins that fetch data from external APIs (GitHub, GitLab, Forgejo,
//! Docker registries, npm, Cargo crates.io) share identical security requirements:
//! WebPKI certificate verification, SSRF-safe DNS resolution, standard timeouts.
//! This module centralises those requirements so they cannot drift per-plugin.

use std::sync::Arc;

use uptrakit_shared_types::ssrf::{
    SsrfSafeResolver, danger_accept_any_cert_client_config, webpki_client_config,
};

use crate::RedirectMode;

/// Typed error returned when building a plugin HTTP client fails.
#[derive(Debug, thiserror::Error)]
pub enum PluginHttpClientBuildError {
    /// `reqwest::Client::builder().build()` failed.
    #[error("failed to build HTTP client: {source}")]
    Build {
        /// Source error from reqwest.
        #[source]
        source: reqwest::Error,
    },
}

impl From<PluginHttpClientBuildError> for String {
    fn from(value: PluginHttpClientBuildError) -> Self {
        value.to_string()
    }
}

/// Controls whether the SSRF-safe resolver blocks connections to private/loopback addresses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SsrfMode {
    /// Blocks connections to private IP ranges.
    ///
    /// Use this for plugins that only ever talk to public SaaS APIs
    /// (GitHub, GitLab.com, npm registry, crates.io).
    Strict,
    /// Allows connections to private/loopback IP ranges.
    ///
    /// Use this when the user can configure a self-hosted registry URL
    /// that may resolve to a private address (e.g. a private Forgejo instance,
    /// a private Docker registry, or an internal Cargo registry).
    Permissive,
}

/// Configuration for building a plugin HTTP client with standard security defaults.
pub struct PluginHttpClientConfig<'a> {
    /// Value for the `User-Agent` request header.
    pub user_agent: &'a str,
    /// SSRF resolver mode.
    pub ssrf_mode: SsrfMode,
    /// Per-request timeout in seconds (default: 60).
    pub request_timeout_secs: u64,
    /// Redirect-following policy (default: never follow).
    pub redirect: RedirectMode,
    /// Optional default headers to include on every request.
    pub default_headers: Option<reqwest::header::HeaderMap>,
    /// Accept invalid/self-signed TLS certificates.
    ///
    /// Only for operator-owned infrastructure endpoints that default to
    /// self-signed certs (Proxmox VE). Everything else keeps webpki
    /// verification.
    pub danger_accept_invalid_certs: bool,
}

impl Default for PluginHttpClientConfig<'_> {
    fn default() -> Self {
        Self {
            user_agent: "uptrakit-plugin",
            ssrf_mode: SsrfMode::Strict,
            request_timeout_secs: 60,
            redirect: RedirectMode::None,
            default_headers: None,
            danger_accept_invalid_certs: false,
        }
    }
}

/// Build a [`reqwest::Client`] with standard plugin security defaults.
///
/// Every client produced by this function:
///
/// - Uses WebPKI certificate verification via [`webpki_client_config()`].
/// - Resolves hostnames through an SSRF-safe DNS resolver (strict or permissive,
///   controlled by [`PluginHttpClientConfig::ssrf_mode`]).
/// - Enforces a 10-second connection timeout.
/// - Enforces a per-request timeout (default 60 seconds).
/// - Follows a configurable redirect policy (default: no redirects).
/// - Sends optional default headers with every request.
///
/// Returns [`PluginHttpClientBuildError`] if the underlying
/// `reqwest::Client::builder()` fails, which is only possible when TLS
/// initialisation fails (i.e. essentially never in a correctly linked binary).
pub fn build_plugin_http_client(
    cfg: PluginHttpClientConfig<'_>,
) -> Result<reqwest::Client, PluginHttpClientBuildError> {
    let resolved_redirect = cfg.redirect.into_policy(cfg.ssrf_mode);
    let tls = if cfg.danger_accept_invalid_certs {
        danger_accept_any_cert_client_config()
    } else {
        webpki_client_config()
    };
    #[expect(
        clippy::disallowed_methods,
        reason = "the one sanctioned Client::builder call — this fn IS the enforcement point"
    )]
    let mut builder = reqwest::Client::builder()
        .user_agent(cfg.user_agent)
        .redirect(resolved_redirect)
        .use_preconfigured_tls(tls)
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(cfg.request_timeout_secs));

    builder = match cfg.ssrf_mode {
        SsrfMode::Strict => builder.dns_resolver(Arc::new(SsrfSafeResolver::new())),
        SsrfMode::Permissive => builder.dns_resolver(Arc::new(SsrfSafeResolver::permissive())),
    };

    if let Some(headers) = cfg.default_headers {
        builder = builder.default_headers(headers);
    }

    builder
        .build()
        .map_err(|source| PluginHttpClientBuildError::Build { source })
}

/// Rebase `candidate` onto `origin`'s scheme/host/port, keeping only
/// `candidate`'s path and query.
///
/// Pagination `Link` headers come from the untrusted upstream response;
/// honoring a cross-origin `next` URL would send our authenticated
/// pagination requests wherever the server says. A mismatch is logged
/// and the path is rebased onto the trusted origin regardless.
#[must_use]
pub fn rebase_to_origin(origin: &reqwest::Url, candidate: &reqwest::Url) -> reqwest::Url {
    if candidate.origin() != origin.origin() {
        tracing::warn!(
            origin = %origin,
            candidate = %candidate,
            "cross-origin pagination link rebased onto the request origin"
        );
    }
    let mut rebased = origin.clone();
    rebased.set_path(candidate.path());
    rebased.set_query(candidate.query());
    rebased
}

/// Build a shared base [`reqwest::Client`] with SSRF protection for controller-side use.
///
/// This creates a reusable HTTP client intended to be stored in [`CatalogConfig`] and
/// cloned by controller-side singletons (transports, enhancements) and
/// [`ControllerRuntime`]. It provides:
///
/// - WebPKI certificate verification.
/// - SSRF-safe DNS resolution (strict or permissive based on `allow_private_urls`).
/// - 10-second connection timeout and 60-second per-request timeout.
/// - Redirect following up to 10 hops (individual plugins can override per-request).
/// - No default authentication headers — auth is applied per-request by each plugin.
///
/// [`CatalogConfig`]: crate::descriptor::CatalogConfig
/// [`ControllerRuntime`]: crate::descriptor::ControllerRuntime
#[cfg(feature = "catalog")]
pub fn build_base_http_client(
    allow_private_urls: bool,
) -> Result<reqwest::Client, PluginHttpClientBuildError> {
    build_plugin_http_client(PluginHttpClientConfig {
        user_agent: "uptrakit",
        ssrf_mode: if allow_private_urls {
            SsrfMode::Permissive
        } else {
            SsrfMode::Strict
        },
        redirect: RedirectMode::Limited { hops: 10 },
        ..PluginHttpClientConfig::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(s: &str) -> reqwest::Url {
        s.parse::<reqwest::Url>().expect("test url")
    }

    /// Canary: proves the `reqwest::ClientBuilder::new` ban in clippy.toml
    /// still resolves against the real symbol. Never called.
    #[expect(dead_code, reason = "canary is never called")]
    fn clientbuilder_new_canary() -> reqwest::ClientBuilder {
        #[expect(
            clippy::disallowed_methods,
            reason = "canary: proves the ClientBuilder::new ban still resolves"
        )]
        reqwest::ClientBuilder::new()
    }

    #[test]
    fn default_config_verifies_tls() {
        assert!(!PluginHttpClientConfig::default().danger_accept_invalid_certs);
    }

    #[test]
    fn rebase_to_origin_is_a_no_op_for_same_origin() {
        let origin = url("https://api.github.com/repos/foo/bar/releases");
        let candidate = url("https://api.github.com/repos/foo/bar/releases?page=2");

        let rebased = rebase_to_origin(&origin, &candidate);

        assert_eq!(rebased.as_str(), candidate.as_str());
    }

    #[test]
    fn rebase_to_origin_pins_cross_origin_candidate_to_the_origin_host() {
        let origin = url("https://api.github.com/repos/foo/bar/releases");
        let candidate = url("http://evil.example/api/steal?page=2&token=x");

        let rebased = rebase_to_origin(&origin, &candidate);

        assert_eq!(rebased.scheme(), "https");
        assert_eq!(rebased.host_str(), Some("api.github.com"));
        assert_eq!(rebased.path(), "/api/steal");
        assert_eq!(rebased.query(), Some("page=2&token=x"));
    }

    #[test]
    fn rebase_to_origin_treats_a_port_mismatch_as_cross_origin() {
        let origin = url("https://api.example.com:8443/releases");
        let candidate = url("https://api.example.com/releases?page=2");

        let rebased = rebase_to_origin(&origin, &candidate);

        assert_eq!(rebased.port(), Some(8443));
        assert_eq!(rebased.path(), "/releases");
        assert_eq!(rebased.query(), Some("page=2"));
    }
}

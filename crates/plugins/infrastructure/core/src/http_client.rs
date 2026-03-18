//! Shared HTTP client builder for plugins that make outbound HTTP requests.
//!
//! All plugins that fetch data from external APIs (GitHub, GitLab, Forgejo,
//! Docker registries, npm, Cargo crates.io) share identical security requirements:
//! WebPKI certificate verification, SSRF-safe DNS resolution, standard timeouts.
//! This module centralises those requirements so they cannot drift per-plugin.

use std::sync::Arc;

use uptrakit_shared_types::ssrf::{SsrfSafeResolver, webpki_client_config};

/// Controls whether the SSRF-safe resolver blocks connections to private/loopback addresses.
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
    /// Redirect-following policy (default: `Policy::none()`).
    pub redirect_policy: reqwest::redirect::Policy,
    /// Optional default headers to include on every request.
    pub default_headers: Option<reqwest::header::HeaderMap>,
}

impl Default for PluginHttpClientConfig<'_> {
    fn default() -> Self {
        Self {
            user_agent: "uptrakit-plugin",
            ssrf_mode: SsrfMode::Strict,
            request_timeout_secs: 60,
            redirect_policy: reqwest::redirect::Policy::none(),
            default_headers: None,
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
/// Returns `Err(String)` if the underlying `reqwest::Client::builder()` fails,
/// which is only possible when TLS initialisation fails (i.e. essentially never
/// in a correctly linked binary).
pub fn build_plugin_http_client(
    cfg: PluginHttpClientConfig<'_>,
) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder()
        .user_agent(cfg.user_agent)
        .redirect(cfg.redirect_policy)
        .use_preconfigured_tls(webpki_client_config())
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
        .map_err(|e| format!("failed to build HTTP client: {e}"))
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
pub fn build_base_http_client(allow_private_urls: bool) -> Result<reqwest::Client, String> {
    build_plugin_http_client(PluginHttpClientConfig {
        user_agent: "uptrakit",
        ssrf_mode: if allow_private_urls {
            SsrfMode::Permissive
        } else {
            SsrfMode::Strict
        },
        redirect_policy: reqwest::redirect::Policy::limited(10),
        ..PluginHttpClientConfig::default()
    })
}

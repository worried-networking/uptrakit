//! SSRF-safe DNS resolver and TLS helpers for `reqwest` HTTP clients.
//!
//! Provides:
//! - [`SsrfSafeResolver`]: a [`reqwest::dns::Resolve`] implementation that
//!   filters resolved addresses through [`is_private_ip`] to prevent DNS
//!   rebinding attacks.  Even if a hostname passes the static
//!   [`is_private_host`](crate::network::is_private_host) check at config-save
//!   time, a DNS rebinding attack can cause the hostname to resolve to a private
//!   IP at request time.  By rejecting private IPs at the resolver level, this
//!   module provides defence-in-depth against SSRF.
//! - [`webpki_client_config`]: a [`rustls::ClientConfig`] using bundled Mozilla
//!   CA roots.  Avoids `rustls-platform-verifier` which calls `SecPolicyCreateSSL`
//!   on macOS and can panic when the system Security framework returns `NULL`.
//! - [`danger_accept_any_cert_client_config`]: a [`rustls::ClientConfig`] that
//!   skips certificate chain verification (for `verify_tls = false` plugin
//!   configurations).  Handshake signatures are still verified.
//!
//! # Usage
//!
//! ```rust,ignore
//! use std::sync::Arc;
//! use uptrakit_shared_types::ssrf::{SsrfSafeResolver, webpki_client_config};
//!
//! let client = reqwest::Client::builder()
//!     .use_preconfigured_tls(webpki_client_config())
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

// ── TLS helpers ───────────────────────────────────────────────────────────────

/// Build a [`rustls::ClientConfig`] using bundled Mozilla CA roots.
///
/// Uses the [`webpki-roots`] certificate bundle instead of
/// `rustls-platform-verifier`.  The platform verifier calls `SecPolicyCreateSSL`
/// on macOS which can return `NULL` in certain contexts, causing an unrecoverable
/// panic inside `security-framework`.  The bundled Mozilla roots are a safe
/// alternative for all public-internet HTTPS endpoints.
///
/// Pass the returned config to [`reqwest::ClientBuilder::use_preconfigured_tls`]
/// when building HTTP clients in plugins.
///
/// The function constructs a fresh `CryptoProvider` via
/// `rustls::crypto::aws_lc_rs::default_provider()` so it does not depend on a
/// process-wide default provider being installed.
#[expect(
    clippy::expect_used,
    reason = "infallible: with_safe_default_protocol_versions() only fails for unknown protocol versions, which cannot happen with the bundled provider"
)]
pub fn webpki_client_config() -> rustls::ClientConfig {
    let mut root_store = rustls::RootCertStore::empty();
    root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    rustls::ClientConfig::builder_with_provider(std::sync::Arc::new(
        rustls::crypto::aws_lc_rs::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .expect("safe default TLS versions are always valid")
    .with_root_certificates(root_store)
    .with_no_client_auth()
}

/// Build a [`rustls::ClientConfig`] that accepts any server certificate without
/// verifying the certificate chain.
///
/// **Security warning**: only use when the user has explicitly opted out of TLS
/// certificate verification (e.g. `verify_tls = false` in a plugin config).
/// The TLS handshake signature is still verified via the installed
/// [`rustls::crypto::CryptoProvider`] to prevent trivial MITM attacks that forge
/// invalid signatures.
///
/// Pass the returned config to [`reqwest::ClientBuilder::use_preconfigured_tls`]
/// in place of [`reqwest::ClientBuilder::danger_accept_invalid_certs`], which is
/// silently ignored when `use_preconfigured_tls` is also set.
#[expect(
    clippy::expect_used,
    reason = "infallible: with_safe_default_protocol_versions() only fails for unknown protocol versions, which cannot happen with the bundled provider"
)]
pub fn danger_accept_any_cert_client_config() -> rustls::ClientConfig {
    #[derive(Debug)]
    struct AcceptAnyServerCert;

    impl rustls::client::danger::ServerCertVerifier for AcceptAnyServerCert {
        fn verify_server_cert(
            &self,
            _end_entity: &rustls::pki_types::CertificateDer<'_>,
            _intermediates: &[rustls::pki_types::CertificateDer<'_>],
            _server_name: &rustls::pki_types::ServerName<'_>,
            _ocsp_response: &[u8],
            _now: rustls::pki_types::UnixTime,
        ) -> std::result::Result<rustls::client::danger::ServerCertVerified, rustls::Error>
        {
            // Cert chain validation intentionally skipped: the caller has
            // explicitly opted out of TLS certificate verification.
            Ok(rustls::client::danger::ServerCertVerified::assertion())
        }

        fn verify_tls12_signature(
            &self,
            message: &[u8],
            cert: &rustls::pki_types::CertificateDer<'_>,
            dss: &rustls::DigitallySignedStruct,
        ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error>
        {
            let provider = rustls::crypto::CryptoProvider::get_default().ok_or(
                rustls::Error::General("no crypto provider installed".into()),
            )?;
            rustls::crypto::verify_tls12_signature(
                message,
                cert,
                dss,
                &provider.signature_verification_algorithms,
            )
        }

        fn verify_tls13_signature(
            &self,
            message: &[u8],
            cert: &rustls::pki_types::CertificateDer<'_>,
            dss: &rustls::DigitallySignedStruct,
        ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error>
        {
            let provider = rustls::crypto::CryptoProvider::get_default().ok_or(
                rustls::Error::General("no crypto provider installed".into()),
            )?;
            rustls::crypto::verify_tls13_signature(
                message,
                cert,
                dss,
                &provider.signature_verification_algorithms,
            )
        }

        fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
            rustls::crypto::CryptoProvider::get_default()
                .map(|p| p.signature_verification_algorithms.supported_schemes())
                .unwrap_or_default()
        }
    }

    rustls::ClientConfig::builder_with_provider(std::sync::Arc::new(
        rustls::crypto::aws_lc_rs::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .expect("safe default TLS versions are always valid")
    .dangerous()
    .with_custom_certificate_verifier(std::sync::Arc::new(AcceptAnyServerCert))
    .with_no_client_auth()
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

    // ── TLS helper tests ─────────────────────────────────────────────

    #[test]
    fn webpki_client_config_builds_successfully() {
        // builder_with_provider does not require a global default — this must
        // succeed without calling install_default() first.
        let config = webpki_client_config();
        // No client auth configured.
        assert!(!config.client_auth_cert_resolver.has_certs());
    }

    #[test]
    fn danger_accept_any_cert_client_config_builds_successfully() {
        // Must succeed without a global default provider installed.
        let _config = danger_accept_any_cert_client_config();
    }

    #[test]
    fn webpki_reqwest_client_builds_successfully() {
        use std::sync::Arc;
        // Verifies the full path: config → use_preconfigured_tls → build.
        let result = reqwest::Client::builder()
            .use_preconfigured_tls(webpki_client_config())
            .dns_resolver(Arc::new(SsrfSafeResolver::new()))
            .build();
        assert!(result.is_ok(), "reqwest client with webpki TLS must build");
    }
}

//! Custom HTTP client adapter for OpenID Connect.
//!
//! Implements [`openidconnect::AsyncHttpClient`] for our workspace `reqwest` (v0.13)
//! so that the `openidconnect` crate can be used with `default-features = false`,
//! eliminating its bundled `reqwest 0.12` (and transitively, `ring`).

use std::future::Future;
use std::pin::Pin;

use openidconnect::{AsyncHttpClient, HttpClientError, HttpRequest, HttpResponse};
use uptrakit_plugin_infrastructure_registry::{
    PluginHttpClientBuildError, PluginHttpClientConfig, RedirectMode, SsrfMode,
    build_plugin_http_client,
};

/// Wrapper around [`reqwest::Client`] (workspace v0.13) that implements
/// [`openidconnect::AsyncHttpClient`].
///
/// Constructed with project-standard timeouts (10 s connect, 60 s total).
pub(crate) struct OidcHttpClient {
    inner: reqwest::Client,
}

impl OidcHttpClient {
    /// Create a new OIDC HTTP client with project-standard timeouts.
    ///
    /// # Errors
    ///
    /// Returns a [`PluginHttpClientBuildError`] if the underlying client
    /// cannot be built (e.g. TLS initialisation failure).
    pub(crate) fn new(
        allow_private_network_issuers: bool,
    ) -> Result<Self, PluginHttpClientBuildError> {
        let ssrf_mode = if allow_private_network_issuers {
            SsrfMode::Permissive
        } else {
            SsrfMode::Strict
        };
        let inner = build_plugin_http_client(PluginHttpClientConfig {
            user_agent: "uptrakit-controller",
            ssrf_mode,
            redirect: RedirectMode::Limited { hops: 10 },
            ..Default::default()
        })?;
        Ok(Self { inner })
    }
}

impl<'c> AsyncHttpClient<'c> for OidcHttpClient {
    type Error = HttpClientError<reqwest::Error>;
    type Future =
        Pin<Box<dyn Future<Output = Result<HttpResponse, Self::Error>> + Send + Sync + 'c>>;

    fn call(&'c self, request: HttpRequest) -> Self::Future {
        Box::pin(async move {
            let response = self
                .inner
                .execute(request.try_into().map_err(Box::new)?)
                .await
                .map_err(Box::new)?;

            let mut builder = http::Response::builder()
                .status(response.status())
                .version(response.version());

            for (name, value) in response.headers().iter() {
                builder = builder.header(name, value);
            }

            builder
                .body(response.bytes().await.map_err(Box::new)?.to_vec())
                .map_err(HttpClientError::Http)
        })
    }
}

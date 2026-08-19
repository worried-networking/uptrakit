//! Canonical resource URL newtype. Single source of truth for RFC 8707 audience binding.
//!
//! [`CanonicalResourceUrl`] enforces the canonicalisation rules required by the
//! MCP OAuth spec (§7): https scheme only, no fragment, no query string, no
//! trailing slash on non-root paths, lowercase host. [`CanonicalUrlConfig`]
//! wraps operator-supplied hostnames into the issuer + primary resource +
//! accepted-alias set used by the audience binding check.
//!
//! Lives in `uptrakit-web-api-types` (the shared crate) so `uptrakit-mcp`
//! can consume it without a reverse dependency on `uptrakit-web-api`.

use thiserror::Error;
use url::Url;

/// A canonical resource URL suitable for use as an RFC 8707 `resource`
/// parameter or audience claim.
///
/// The canonical string form omits the trailing `/` that [`url::Url`] adds
/// for bare-host inputs, so `parse("https://example.com")` round-trips as
/// `"https://example.com"` rather than `"https://example.com/"`. This matches
/// the issuer-identifier convention in RFC 8414 §2.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CanonicalResourceUrl {
    url: Url,
    canonical: String,
}

/// Parse errors for [`CanonicalResourceUrl`].
#[non_exhaustive]
#[derive(Debug, Error, PartialEq, Eq)]
pub enum CanonicalUrlError {
    /// The input failed RFC 3986 URL parsing.
    #[error("url is malformed: {0}")]
    Malformed(#[from] url::ParseError),
    /// The URL did not use the `https` scheme.
    #[error("url must use https scheme")]
    InsecureScheme,
    /// The URL contained a fragment (`#...`).
    #[error("url must not contain a fragment")]
    Fragment,
    /// The URL contained a query string (`?...`).
    #[error("url must not contain a query string")]
    QueryString,
    /// The URL had a trailing slash on a non-root path.
    #[error("url must not have a trailing slash (use bare-root form)")]
    TrailingSlash,
}

impl CanonicalResourceUrl {
    /// Parse and canonicalise a URL string.
    ///
    /// # Errors
    ///
    /// Returns [`CanonicalUrlError::Malformed`] if the input fails RFC 3986 parsing.
    /// Returns [`CanonicalUrlError::InsecureScheme`] if the scheme is not `https`.
    /// Returns [`CanonicalUrlError::Fragment`] if the URL contains a fragment.
    /// Returns [`CanonicalUrlError::QueryString`] if the URL contains a query.
    /// Returns [`CanonicalUrlError::TrailingSlash`] if the URL has a trailing
    /// slash on a non-root path.
    #[must_use = "parsing returns a canonicalised URL; callers must persist or compare it"]
    pub fn parse(s: &str) -> Result<Self, CanonicalUrlError> {
        let url = Url::parse(s)?;
        if url.scheme() != "https" {
            return Err(CanonicalUrlError::InsecureScheme);
        }
        if url.fragment().is_some() {
            return Err(CanonicalUrlError::Fragment);
        }
        if url.query().is_some() {
            return Err(CanonicalUrlError::QueryString);
        }
        let path = url.path();
        if path.len() > 1 && path.ends_with('/') {
            return Err(CanonicalUrlError::TrailingSlash);
        }
        // `url::Url` already lowercases the host during parsing. It also
        // forces a `/` path for bare-host URLs; strip it so the canonical
        // string matches the issuer-identifier convention.
        let raw = url.as_str();
        let canonical = if path == "/" && raw.ends_with('/') {
            raw.trim_end_matches('/').to_owned()
        } else {
            raw.to_owned()
        };
        Ok(Self { url, canonical })
    }

    /// Returns the canonical string representation.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.canonical
    }

    /// Consumes the wrapper and returns the underlying [`Url`].
    ///
    /// Note: [`Url`] always retains the normalised `/` root path for
    /// bare-host inputs. The canonical string form (without the trailing
    /// slash) is only exposed via [`Self::as_str`].
    #[must_use]
    pub fn into_url(self) -> Url {
        self.url
    }
}

/// Write-time shape gate for the `oauth.canonical_host` setting: a bare host,
/// optionally with a port — no scheme, userinfo, path, query, fragment, or
/// whitespace.
///
/// Intentionally stricter than [`CanonicalResourceUrl::parse`], which stays
/// lenient so boot never fails on a legacy stored value; this shape is
/// enforced at write time only, in `UpdateOAuthSettingsRequest::validate`.
///
/// Case and IDN forms deliberately pass through un-normalized
/// (`Auth.Example.COM`, `exämple.com` are stored as typed) — this gate
/// checks shape only; any host/origin comparison logic must normalize or
/// compare case-insensitively on its side.
#[must_use]
pub fn is_bare_host(host: &str) -> bool {
    // `\` is banned alongside `/`: for special schemes the WHATWG URL parser
    // treats a backslash as a path separator, so `evil.com\path` would parse
    // to host `evil.com` and silently drop the rest of the operator's input.
    !host.contains(['/', '\\', '@', '?', '#'])
        && !host.contains(char::is_whitespace)
        && url::Url::parse(&format!("https://{host}")).is_ok_and(|u| u.host_str().is_some())
}

/// Maximum number of operator-supplied audience aliases.
///
/// Caps the accepted-audience set so a misconfigured deployment cannot turn
/// every request into a per-host lookup.
pub const MAX_ACCEPTED_AUDIENCE_HOSTS: usize = 5;

/// Resolved canonical-URL configuration: issuer, primary resource, and the
/// set of accepted audience values.
#[non_exhaustive]
#[derive(Clone, Debug)]
pub struct CanonicalUrlConfig {
    issuer: CanonicalResourceUrl,
    primary_resource: CanonicalResourceUrl,
    accepted_resources: Vec<CanonicalResourceUrl>,
}

/// Construction errors for [`CanonicalUrlConfig`].
#[non_exhaustive]
#[derive(Debug, Error)]
pub enum CanonicalUrlConfigError {
    /// `canonical_host` was empty.
    #[error("oauth.canonical_host is required when oauth.mcp_enabled is true")]
    Missing,
    /// More aliases were supplied than [`MAX_ACCEPTED_AUDIENCE_HOSTS`] permits.
    #[error("oauth.accepted_audience_hosts exceeds cap of {MAX_ACCEPTED_AUDIENCE_HOSTS}")]
    TooManyAliases,
    /// One of the supplied hostnames failed canonicalisation.
    #[error("canonical host invalid: {0}")]
    InvalidHost(#[from] CanonicalUrlError),
}

impl CanonicalUrlConfig {
    /// Build a [`CanonicalUrlConfig`] from operator-supplied hostnames.
    ///
    /// The primary `canonical_host` is used for the issuer (`https://<host>`)
    /// and the primary resource (`https://<host>/mcp`). Each alias produces an
    /// additional accepted audience (`https://<alias>/mcp`). The primary
    /// resource is always present in the accepted set; duplicates among the
    /// aliases are silently dropped.
    ///
    /// # Errors
    ///
    /// Returns [`CanonicalUrlConfigError::Missing`] if `canonical_host` is empty.
    /// Returns [`CanonicalUrlConfigError::TooManyAliases`] if more than
    /// [`MAX_ACCEPTED_AUDIENCE_HOSTS`] aliases were supplied.
    /// Returns [`CanonicalUrlConfigError::InvalidHost`] for any host that
    /// fails [`CanonicalResourceUrl::parse`] (e.g. wrong scheme, fragment,
    /// query, or trailing slash after composition).
    pub fn new(
        canonical_host: String,
        accepted_aliases: Vec<String>,
    ) -> Result<Self, CanonicalUrlConfigError> {
        if canonical_host.is_empty() {
            return Err(CanonicalUrlConfigError::Missing);
        }
        if accepted_aliases.len() > MAX_ACCEPTED_AUDIENCE_HOSTS {
            return Err(CanonicalUrlConfigError::TooManyAliases);
        }
        let issuer = CanonicalResourceUrl::parse(&format!("https://{canonical_host}"))?;
        let primary_resource =
            CanonicalResourceUrl::parse(&format!("https://{canonical_host}/mcp"))?;
        let mut accepted_resources = vec![primary_resource.clone()];
        for alias in accepted_aliases {
            let r = CanonicalResourceUrl::parse(&format!("https://{alias}/mcp"))?;
            if accepted_resources.iter().any(|p| p == &r) {
                continue;
            }
            accepted_resources.push(r);
        }
        Ok(Self {
            issuer,
            primary_resource,
            accepted_resources,
        })
    }

    /// The OAuth issuer identifier (`https://<canonical_host>`).
    #[must_use]
    pub fn issuer(&self) -> &CanonicalResourceUrl {
        &self.issuer
    }

    /// The primary MCP resource URL (`https://<canonical_host>/mcp`).
    #[must_use]
    pub fn primary_resource(&self) -> &CanonicalResourceUrl {
        &self.primary_resource
    }

    /// Returns `true` if `aud` is one of the accepted audience values.
    ///
    /// Compares against the canonicalised string form of each accepted
    /// resource. Callers SHOULD canonicalise `aud` before calling this method
    /// (e.g. via [`CanonicalResourceUrl::parse`]) to avoid false negatives
    /// from purely cosmetic differences.
    #[must_use]
    pub fn accepts_audience(&self, aud: &str) -> bool {
        self.accepted_resources.iter().any(|r| r.as_str() == aud)
    }

    /// Returns the accepted audience URL strings (primary resource + all aliases).
    ///
    /// Pass directly to [`McpOAuthJwtVerifier::new`] as `accepted_audiences`.
    #[must_use]
    pub fn accepted_resources_strings(&self) -> Vec<String> {
        self.accepted_resources
            .iter()
            .map(|r| r.as_str().to_owned())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_simple_host() {
        let u = CanonicalResourceUrl::parse("https://controller.example.com/mcp").unwrap();
        assert_eq!(u.as_str(), "https://controller.example.com/mcp");
    }

    #[test]
    fn rejects_fragment() {
        let err = CanonicalResourceUrl::parse("https://controller.example.com/mcp#x").unwrap_err();
        assert!(matches!(err, CanonicalUrlError::Fragment));
    }

    #[test]
    fn rejects_query() {
        let err =
            CanonicalResourceUrl::parse("https://controller.example.com/mcp?x=1").unwrap_err();
        assert!(matches!(err, CanonicalUrlError::QueryString));
    }

    #[test]
    fn rejects_http_scheme() {
        let err = CanonicalResourceUrl::parse("http://controller.example.com/mcp").unwrap_err();
        assert!(matches!(err, CanonicalUrlError::InsecureScheme));
    }

    #[test]
    fn rejects_trailing_slash() {
        let err = CanonicalResourceUrl::parse("https://controller.example.com/mcp/").unwrap_err();
        assert!(matches!(err, CanonicalUrlError::TrailingSlash));
    }

    #[test]
    fn lowercases_host() {
        let u = CanonicalResourceUrl::parse("https://Controller.Example.Com/mcp").unwrap();
        assert_eq!(u.as_str(), "https://controller.example.com/mcp");
    }

    // Tests for CanonicalUrlConfig
    #[test]
    fn config_derives_issuer_and_resource_from_host() {
        let cfg = CanonicalUrlConfig::new("controller.example.com".into(), vec![]).unwrap();
        assert_eq!(cfg.issuer().as_str(), "https://controller.example.com");
        assert_eq!(
            cfg.primary_resource().as_str(),
            "https://controller.example.com/mcp"
        );
        assert!(cfg.accepts_audience("https://controller.example.com/mcp"));
    }

    #[test]
    fn config_rejects_too_many_aliases() {
        let aliases: Vec<String> = (0..=MAX_ACCEPTED_AUDIENCE_HOSTS)
            .map(|i| format!("alias{i}.example.com"))
            .collect();
        let err = CanonicalUrlConfig::new("controller.example.com".into(), aliases).unwrap_err();
        assert!(matches!(err, CanonicalUrlConfigError::TooManyAliases));
    }

    #[test]
    fn config_accepts_alias_audience() {
        let cfg = CanonicalUrlConfig::new(
            "controller.example.com".into(),
            vec!["legacy.example.com".into()],
        )
        .unwrap();
        assert!(cfg.accepts_audience("https://controller.example.com/mcp"));
        assert!(cfg.accepts_audience("https://legacy.example.com/mcp"));
        assert!(!cfg.accepts_audience("https://intruder.example.com/mcp"));
    }

    #[test]
    fn config_missing_host_errors() {
        let err = CanonicalUrlConfig::new(String::new(), vec![]).unwrap_err();
        assert!(matches!(err, CanonicalUrlConfigError::Missing));
    }

    #[test]
    fn accepted_resources_strings_includes_primary_and_aliases() {
        let cfg = CanonicalUrlConfig::new(
            "controller.example.com".into(),
            vec!["legacy.example.com".into()],
        )
        .unwrap();
        let strings = cfg.accepted_resources_strings();
        assert!(strings.contains(&"https://controller.example.com/mcp".to_owned()));
        assert!(strings.contains(&"https://legacy.example.com/mcp".to_owned()));
        assert_eq!(strings.len(), 2);
    }

    #[test]
    fn accepted_resources_strings_no_aliases() {
        let cfg = CanonicalUrlConfig::new("controller.example.com".into(), vec![]).unwrap();
        let strings = cfg.accepted_resources_strings();
        assert_eq!(
            strings,
            vec!["https://controller.example.com/mcp".to_owned()]
        );
    }
}

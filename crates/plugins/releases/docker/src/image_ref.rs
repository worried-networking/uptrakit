//! Centralised Docker image reference parsing.
//!
//! An [`ImageRef`] represents a fully-parsed Docker/OCI image reference,
//! breaking it into registry, repository, tag, and convenience accessors.
//!
//! ## Container-qualified identifiers
//!
//! When used as a `package_identifier`, an image reference may carry an
//! optional **container name suffix** separated by `#`:
//!
//! ```text
//! nginx:latest#web-server
//! ```
//!
//! The `#container_name` suffix identifies a specific named container on the
//! host. It is used by [`crate::plugin::DockerPlugin`] to target individual
//! containers during updates while leaving the image-level registry operations
//! (detect version, fetch releases) unchanged.
//!
//! All image-level fields (`registry`, `repository`, `tag`, `image`, `full_ref`)
//! are derived exclusively from the part before `#`. The container name is
//! stored separately in [`ImageRef::container_name`].

use std::str::FromStr;

use thiserror::Error;

/// A parsed Docker/OCI image reference, optionally carrying a container name.
///
/// # Parsing rules
///
/// | Input | Registry | Repository | Tag | Container |
/// |---|---|---|---|---|
/// | `nginx` | `registry-1.docker.io` | `library/nginx` | `latest` | `None` |
/// | `myuser/app:v2` | `registry-1.docker.io` | `myuser/app` | `v2` | `None` |
/// | `ghcr.io/owner/app:main` | `ghcr.io` | `owner/app` | `main` | `None` |
/// | `host:5000/app:latest` | `host:5000` | `app` | `latest` | `None` |
/// | `nginx:latest#web-server` | `registry-1.docker.io` | `library/nginx` | `latest` | `Some("web-server")` |
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageRef {
    /// Resolved registry hostname (e.g. `"registry-1.docker.io"`, `"ghcr.io"`, `"myhost:5000"`).
    pub registry: String,
    /// Repository path (e.g. `"library/nginx"`, `"owner/app"`).
    pub repository: String,
    /// Tag (defaults to `"latest"`).
    pub tag: String,
    /// Image name without tag (e.g. `"nginx"`, `"ghcr.io/owner/app"`).
    pub image: String,
    /// Full `"image:tag"` string for Bollard / docker pull calls.
    ///
    /// Does **not** include the `#container_name` suffix.
    pub full_ref: String,
    /// Container name parsed from a `#container_name` suffix in the input string.
    ///
    /// `None` when the identifier has no container qualifier, in which case
    /// image-level operations apply to all containers using the image.
    pub container_name: Option<String>,
}

/// Error from parsing an image reference.
#[derive(Debug, Error)]
pub enum ParseImageRefError {
    #[error("invalid image reference: {reason}")]
    Invalid { reason: String },
}

impl FromStr for ImageRef {
    type Err = ParseImageRefError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();

        if s.is_empty() {
            return Err(ParseImageRefError::Invalid {
                reason: "image reference must not be empty".to_string(),
            });
        }

        // Split on the first `#` to separate the image ref from an optional
        // container name qualifier.  Docker image refs never contain `#`, so
        // this split is unambiguous.
        let (image_part, container_name) = match s.find('#') {
            Some(pos) => {
                let name = &s[pos + 1..];
                validate_container_name(name)?;
                (&s[..pos], Some(name.to_string()))
            }
            None => (s, None),
        };

        if image_part.is_empty() {
            return Err(ParseImageRefError::Invalid {
                reason: "image reference part must not be empty".to_string(),
            });
        }
        if image_part.contains("//") {
            return Err(ParseImageRefError::Invalid {
                reason: "image reference must not contain '//'".to_string(),
            });
        }
        if image_part.split('/').any(|seg| seg == "..") {
            return Err(ParseImageRefError::Invalid {
                reason: "image reference segments must not be '..'".to_string(),
            });
        }
        if image_part.contains(char::is_whitespace) {
            return Err(ParseImageRefError::Invalid {
                reason: "image reference must not contain whitespace".to_string(),
            });
        }

        // Split off the tag (`:tag`) — but only from the last path segment
        // because registry hostnames may include a port (e.g. `host:5000`).
        let (image_no_tag, tag) = split_image_tag(image_part);

        let registry = infer_registry(image_no_tag).to_string();
        let repository = resolve_repository(image_no_tag);
        let full_ref = format!("{image_no_tag}:{tag}");

        Ok(Self {
            registry,
            repository,
            tag,
            image: image_no_tag.to_string(),
            full_ref,
            container_name,
        })
    }
}

impl ImageRef {
    /// Return the full package identifier string, including the `#container_name`
    /// suffix when present.
    ///
    /// This is the canonical form that round-trips through [`FromStr`].
    pub fn full_pkg_id(&self) -> String {
        match &self.container_name {
            Some(name) => format!("{}#{name}", self.full_ref),
            None => self.full_ref.clone(),
        }
    }

    /// Best-effort browsable web URL for this image at the given tag or digest.
    pub fn web_url(&self, tag_or_digest: &str) -> String {
        if self.registry == "registry-1.docker.io" {
            let hub_path = self
                .repository
                .strip_prefix("library/")
                .unwrap_or(&self.repository);
            format!("https://hub.docker.com/_/{hub_path}/tags?name={tag_or_digest}")
        } else if self.registry == "ghcr.io" {
            format!("https://ghcr.io/{}:{tag_or_digest}", self.repository)
        } else {
            format!(
                "https://{}/{}:{tag_or_digest}",
                self.registry, self.repository
            )
        }
    }

    /// Registry server address used for Docker credential association.
    ///
    /// Docker Hub returns the legacy `https://index.docker.io/v1/` address
    /// because that is what the Docker daemon expects in its credential store.
    pub fn server_address(&self) -> String {
        if self.registry == "registry-1.docker.io" {
            "https://index.docker.io/v1/".to_string()
        } else {
            format!("https://{}/", self.registry)
        }
    }
}

/// Validate an image identifier string.
///
/// Accepts both plain image references (`nginx:latest`) and container-qualified
/// references (`nginx:latest#web-server`).
///
/// Returns `Ok(())` when the string is a valid Docker image reference
/// whose registry hostname does not point to a private/internal network,
/// or an error message string on failure.
///
/// Used by the plugin registry to validate `package_identifier` values
/// for the Docker plugin.
pub fn validate_identifier(id: &str) -> std::result::Result<(), String> {
    // Split on the first '#' to isolate the image ref part for SSRF checking.
    // Container name validation is handled by ImageRef::from_str via
    // validate_container_name.
    let image_ref = id.parse::<ImageRef>().map_err(|e| e.to_string())?;

    // Strip port from registry hostname before checking (e.g. "host:5000" → "host").
    let registry_host = image_ref
        .registry
        .split(':')
        .next()
        .unwrap_or(&image_ref.registry);

    if uptrakit_shared_types::network::is_private_host(registry_host) {
        return Err(format!(
            "registry hostname '{}' resolves to a private/internal address",
            registry_host
        ));
    }

    Ok(())
}

// ── Private helpers ───────────────────────────────────────────────────────────

/// Validate a Docker container name.
///
/// Docker container names must satisfy `[a-zA-Z0-9][a-zA-Z0-9_.-]*`.
/// We also enforce a reasonable length limit (253 characters).
fn validate_container_name(name: &str) -> Result<(), ParseImageRefError> {
    if name.is_empty() {
        return Err(ParseImageRefError::Invalid {
            reason: "container name after '#' must not be empty".to_string(),
        });
    }
    if name.len() > 253 {
        return Err(ParseImageRefError::Invalid {
            reason: format!(
                "container name exceeds maximum length of 253 characters (got {})",
                name.len()
            ),
        });
    }
    if name.contains(char::is_whitespace) {
        return Err(ParseImageRefError::Invalid {
            reason: "container name must not contain whitespace".to_string(),
        });
    }
    if name.contains('/') || name.contains('\\') {
        return Err(ParseImageRefError::Invalid {
            reason: "container name must not contain path separators".to_string(),
        });
    }

    // Docker container names follow [a-zA-Z0-9][a-zA-Z0-9_.-]*
    let mut chars = name.chars();
    // Safe: non-empty already checked above.
    let first = chars.next().expect("non-empty");
    if !first.is_ascii_alphanumeric() {
        return Err(ParseImageRefError::Invalid {
            reason: format!("container name must start with a letter or digit, got '{first}'"),
        });
    }
    for c in chars {
        if !c.is_ascii_alphanumeric() && c != '_' && c != '.' && c != '-' {
            return Err(ParseImageRefError::Invalid {
                reason: format!(
                    "container name contains invalid character '{c}': \
                     only letters, digits, '_', '.', and '-' are allowed"
                ),
            });
        }
    }

    Ok(())
}

/// Split `"image:tag"` into `("image", "tag")`.
///
/// The colon is only treated as a tag separator when it appears in the
/// *last path segment* (i.e., after the final `/`). A colon in the first
/// component before a slash is treated as a port number in a registry hostname
/// (`host:5000/repo:tag`).
fn split_image_tag(s: &str) -> (&str, String) {
    // Find the last `/` to isolate the final path segment
    let last_segment_start = s.rfind('/').map(|i| i + 1).unwrap_or(0);
    let last_segment = &s[last_segment_start..];

    if let Some(colon_pos) = last_segment.find(':') {
        let split = last_segment_start + colon_pos;
        let tag = s[split + 1..].to_string();
        let image = &s[..split];
        (image, tag)
    } else {
        (s, "latest".to_string())
    }
}

/// Infer the registry hostname from an image reference (without tag).
fn infer_registry(image: &str) -> &str {
    if let Some(first_slash) = image.find('/') {
        let first_component = &image[..first_slash];
        if first_component.contains('.')
            || first_component.contains(':')
            || first_component == "localhost"
        {
            return first_component;
        }
    }
    "registry-1.docker.io"
}

/// Resolve the repository path from an image reference (without tag).
fn resolve_repository(image: &str) -> String {
    if let Some(first_slash) = image.find('/') {
        let first_component = &image[..first_slash];
        if first_component.contains('.')
            || first_component.contains(':')
            || first_component == "localhost"
        {
            // Strip the registry prefix
            return image[first_slash + 1..].to_string();
        }
        // Not a registry hostname — the whole thing is the repository path
        return image.to_string();
    }
    // Single name like "nginx" → Docker Hub official: "library/nginx"
    format!("library/{image}")
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> ImageRef {
        s.parse().expect("valid image ref")
    }

    // ── Parsing — plain image refs ────────────────────────────────────────────

    #[test]
    fn docker_hub_official_image() {
        let r = parse("nginx");
        assert_eq!(r.registry, "registry-1.docker.io");
        assert_eq!(r.repository, "library/nginx");
        assert_eq!(r.tag, "latest");
        assert_eq!(r.image, "nginx");
        assert_eq!(r.full_ref, "nginx:latest");
        assert!(r.container_name.is_none());
    }

    #[test]
    fn docker_hub_user_image_with_tag() {
        let r = parse("myuser/app:v2");
        assert_eq!(r.registry, "registry-1.docker.io");
        assert_eq!(r.repository, "myuser/app");
        assert_eq!(r.tag, "v2");
        assert_eq!(r.image, "myuser/app");
        assert_eq!(r.full_ref, "myuser/app:v2");
        assert!(r.container_name.is_none());
    }

    #[test]
    fn ghcr_image_with_tag() {
        let r = parse("ghcr.io/owner/app:main");
        assert_eq!(r.registry, "ghcr.io");
        assert_eq!(r.repository, "owner/app");
        assert_eq!(r.tag, "main");
        assert_eq!(r.image, "ghcr.io/owner/app");
        assert_eq!(r.full_ref, "ghcr.io/owner/app:main");
        assert!(r.container_name.is_none());
    }

    #[test]
    fn private_registry_with_port() {
        let r = parse("host:5000/app:latest");
        assert_eq!(r.registry, "host:5000");
        assert_eq!(r.repository, "app");
        assert_eq!(r.tag, "latest");
        assert_eq!(r.image, "host:5000/app");
        assert_eq!(r.full_ref, "host:5000/app:latest");
    }

    #[test]
    fn missing_tag_defaults_to_latest() {
        let r = parse("ghcr.io/owner/app");
        assert_eq!(r.tag, "latest");
        assert_eq!(r.full_ref, "ghcr.io/owner/app:latest");
    }

    #[test]
    fn private_registry_nested_path() {
        let r = parse("registry.example.com/org/team/app:stable");
        assert_eq!(r.registry, "registry.example.com");
        assert_eq!(r.repository, "org/team/app");
        assert_eq!(r.tag, "stable");
    }

    #[test]
    fn localhost_registry() {
        let r = parse("localhost/myapp:dev");
        assert_eq!(r.registry, "localhost");
        assert_eq!(r.repository, "myapp");
        assert_eq!(r.tag, "dev");
    }

    // ── Parsing — container-qualified identifiers ─────────────────────────────

    #[test]
    fn container_qualified_nginx() {
        let r = parse("nginx:latest#web-server");
        assert_eq!(r.registry, "registry-1.docker.io");
        assert_eq!(r.repository, "library/nginx");
        assert_eq!(r.tag, "latest");
        assert_eq!(r.image, "nginx");
        assert_eq!(r.full_ref, "nginx:latest");
        assert_eq!(r.container_name.as_deref(), Some("web-server"));
    }

    #[test]
    fn container_qualified_no_tag_defaults_to_latest() {
        let r = parse("nginx#my-proxy");
        assert_eq!(r.tag, "latest");
        assert_eq!(r.full_ref, "nginx:latest");
        assert_eq!(r.container_name.as_deref(), Some("my-proxy"));
    }

    #[test]
    fn container_qualified_ghcr() {
        let r = parse("ghcr.io/owner/app:main#my-container");
        assert_eq!(r.registry, "ghcr.io");
        assert_eq!(r.full_ref, "ghcr.io/owner/app:main");
        assert_eq!(r.container_name.as_deref(), Some("my-container"));
    }

    #[test]
    fn container_qualified_with_underscores_and_dots() {
        let r = parse("nginx:latest#my_app.v2");
        assert_eq!(r.container_name.as_deref(), Some("my_app.v2"));
    }

    #[test]
    fn full_pkg_id_without_container() {
        let r = parse("nginx:latest");
        assert_eq!(r.full_pkg_id(), "nginx:latest");
    }

    #[test]
    fn full_pkg_id_with_container() {
        let r = parse("nginx:latest#web-server");
        assert_eq!(r.full_pkg_id(), "nginx:latest#web-server");
    }

    // ── Container name validation errors ─────────────────────────────────────

    #[test]
    fn empty_container_name_fails() {
        assert!("nginx:latest#".parse::<ImageRef>().is_err());
    }

    #[test]
    fn container_name_with_whitespace_fails() {
        assert!("nginx:latest#web server".parse::<ImageRef>().is_err());
    }

    #[test]
    fn container_name_with_slash_fails() {
        assert!("nginx:latest#web/server".parse::<ImageRef>().is_err());
    }

    #[test]
    fn container_name_starting_with_underscore_fails() {
        assert!("nginx:latest#_bad".parse::<ImageRef>().is_err());
    }

    #[test]
    fn container_name_starting_with_dash_fails() {
        assert!("nginx:latest#-bad".parse::<ImageRef>().is_err());
    }

    #[test]
    fn container_name_with_invalid_char_fails() {
        assert!("nginx:latest#bad@name".parse::<ImageRef>().is_err());
    }

    #[test]
    fn container_name_too_long_fails() {
        let long = "a".repeat(254);
        assert!(format!("nginx:latest#{long}").parse::<ImageRef>().is_err());
    }

    #[test]
    fn container_name_at_max_length_succeeds() {
        let long = format!("a{}", "b".repeat(252)); // 253 chars total
        assert!(format!("nginx:latest#{long}").parse::<ImageRef>().is_ok());
    }

    // ── Error cases — plain image refs ────────────────────────────────────────

    #[test]
    fn empty_string_fails() {
        assert!("".parse::<ImageRef>().is_err());
    }

    #[test]
    fn whitespace_string_fails() {
        assert!("  ".parse::<ImageRef>().is_err());
    }

    #[test]
    fn double_slash_fails() {
        assert!("ghcr.io//owner/app".parse::<ImageRef>().is_err());
    }

    #[test]
    fn dotdot_segment_fails() {
        assert!("ghcr.io/../app".parse::<ImageRef>().is_err());
    }

    #[test]
    fn whitespace_in_ref_fails() {
        assert!("nginx latest".parse::<ImageRef>().is_err());
    }

    // ── web_url ───────────────────────────────────────────────────────────────

    #[test]
    fn web_url_docker_hub_official() {
        let r = parse("nginx");
        assert_eq!(
            r.web_url("1.25.0"),
            "https://hub.docker.com/_/nginx/tags?name=1.25.0"
        );
    }

    #[test]
    fn web_url_docker_hub_user() {
        let r = parse("myuser/myrepo");
        assert_eq!(
            r.web_url("latest"),
            "https://hub.docker.com/_/myuser/myrepo/tags?name=latest"
        );
    }

    #[test]
    fn web_url_ghcr() {
        let r = parse("ghcr.io/owner/repo");
        assert_eq!(r.web_url("v1.0.0"), "https://ghcr.io/owner/repo:v1.0.0");
    }

    #[test]
    fn web_url_private_registry() {
        let r = parse("registry.example.com/myapp");
        assert_eq!(
            r.web_url("latest"),
            "https://registry.example.com/myapp:latest"
        );
    }

    // ── server_address ────────────────────────────────────────────────────────

    #[test]
    fn server_address_docker_hub() {
        let r = parse("nginx");
        assert_eq!(r.server_address(), "https://index.docker.io/v1/");
    }

    #[test]
    fn server_address_ghcr() {
        let r = parse("ghcr.io/owner/app");
        assert_eq!(r.server_address(), "https://ghcr.io/");
    }

    #[test]
    fn server_address_private_with_port() {
        let r = parse("myhost:5000/app");
        assert_eq!(r.server_address(), "https://myhost:5000/");
    }

    // ── validate_identifier ───────────────────────────────────────────────────

    #[test]
    fn validate_identifier_valid_plain() {
        assert!(validate_identifier("nginx").is_ok());
        assert!(validate_identifier("ghcr.io/owner/app:latest").is_ok());
        assert!(validate_identifier("myuser/app:v2").is_ok());
    }

    #[test]
    fn validate_identifier_valid_container_qualified() {
        assert!(validate_identifier("nginx:latest#web-server").is_ok());
        assert!(validate_identifier("ghcr.io/owner/app:main#my-app").is_ok());
        assert!(validate_identifier("nginx#api-proxy").is_ok());
    }

    #[test]
    fn validate_identifier_invalid_plain() {
        assert!(validate_identifier("").is_err());
        assert!(validate_identifier("nginx latest").is_err());
        assert!(validate_identifier("ghcr.io//app").is_err());
    }

    #[test]
    fn validate_identifier_invalid_container_name() {
        assert!(validate_identifier("nginx:latest#").is_err());
        assert!(validate_identifier("nginx:latest#bad name").is_err());
        assert!(validate_identifier("nginx:latest#bad/name").is_err());
        assert!(validate_identifier("nginx:latest#_bad").is_err());
    }

    // ── SSRF: private registry hostname rejection ────────────────────────

    #[test]
    fn validate_identifier_rejects_localhost_registry() {
        let err = validate_identifier("localhost/myapp:latest").unwrap_err();
        assert!(err.contains("private"), "error: {err}");
    }

    #[test]
    fn validate_identifier_rejects_private_ip_registry() {
        let err = validate_identifier("10.0.0.1/myapp:latest").unwrap_err();
        assert!(err.contains("private"), "error: {err}");

        let err = validate_identifier("192.168.1.1/myapp:latest").unwrap_err();
        assert!(err.contains("private"), "error: {err}");

        let err = validate_identifier("172.16.0.1/myapp:latest").unwrap_err();
        assert!(err.contains("private"), "error: {err}");
    }

    #[test]
    fn validate_identifier_rejects_link_local_registry() {
        let err = validate_identifier("169.254.169.254/latest/meta-data:v1").unwrap_err();
        assert!(err.contains("private"), "error: {err}");
    }

    #[test]
    fn validate_identifier_rejects_loopback_registry() {
        let err = validate_identifier("127.0.0.1/myapp:latest").unwrap_err();
        assert!(err.contains("private"), "error: {err}");
    }

    #[test]
    fn validate_identifier_rejects_private_ip_with_port() {
        let err = validate_identifier("192.168.1.1:5000/myapp:latest").unwrap_err();
        assert!(err.contains("private"), "error: {err}");
    }

    #[test]
    fn validate_identifier_allows_public_registry() {
        assert!(validate_identifier("registry.example.com/myapp:latest").is_ok());
        assert!(validate_identifier("ghcr.io/owner/app:v1").is_ok());
    }

    #[test]
    fn validate_identifier_allows_docker_hub() {
        // Docker Hub default registry is not private
        assert!(validate_identifier("nginx").is_ok());
        assert!(validate_identifier("myuser/myapp:latest").is_ok());
    }

    #[test]
    fn validate_identifier_rejects_container_qualified_private_registry() {
        // SSRF check still applies when a container name suffix is present
        let err = validate_identifier("192.168.1.1/myapp:latest#my-container").unwrap_err();
        assert!(err.contains("private"), "error: {err}");
    }
}

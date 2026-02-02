use std::net::SocketAddr;
use std::path::PathBuf;

use clap::Parser;
use ipnet::IpNet;

/// Uptrakit Controller — central server for the Uptrakit update tracking toolkit.
#[derive(Parser, Debug)]
#[command(name = "uptrakit-controller")]
pub struct Args {
    /// Data directory (CA keys, certs, future DB).
    /// Supports `~` for home directory expansion.
    #[arg(long, default_value = "~/.uptrakit-controller")]
    pub data_dir: String,

    /// Database URL. If not provided, defaults to SQLite in data directory.
    /// Supported schemes depend on enabled features:
    ///   SQLite (default): sqlite://path/to/db.sqlite
    ///   PostgreSQL: postgresql://user:pass@host:5432/dbname
    ///   MySQL: mysql://user:pass@host:3306/dbname
    #[arg(long)]
    pub db_url: Option<String>,

    /// HTTPS listen address (dual-stack by default).
    /// Stored in DB as `network.https_addr`. CLI value used only on first run
    /// or with `--force-settings-override`.
    #[arg(long)]
    pub https_addr: Option<SocketAddr>,

    /// Override: path to PEM-encoded server certificate.
    /// If not provided, a certificate is auto-generated using the internal CA.
    #[arg(long)]
    pub tls_cert: Option<PathBuf>,

    /// Override: path to PEM-encoded server private key.
    /// Must be provided together with --tls-cert.
    #[arg(long)]
    pub tls_key: Option<PathBuf>,

    /// Trusted reverse proxy IP or CIDR (repeatable).
    /// Bare IPs are treated as /32 (IPv4) or /128 (IPv6).
    /// Both IPv4 and IPv6 are supported.
    /// Stored in DB as `network.trusted_proxies`. CLI value used only on first
    /// run or with `--force-settings-override`.
    #[arg(long = "trusted-proxy", value_parser = parse_proxy)]
    pub trusted_proxies: Vec<IpNet>,

    /// Header to extract the real client IP from when behind a trusted proxy.
    /// Supported: X-Forwarded-For, Forwarded (RFC 7239), X-Real-Ip,
    /// or any custom header name (parsed as comma-separated IPs).
    /// Stored in DB as `network.real_ip_header`. CLI value used only on first
    /// run or with `--force-settings-override`.
    #[arg(long)]
    pub real_ip_header: Option<String>,

    /// Override: path to PEM-encoded CA certificate.
    /// If not provided, a CA is auto-generated and managed internally.
    /// Must be provided together with --ca-key.
    #[arg(long)]
    pub ca_cert: Option<PathBuf>,

    /// Override: path to PEM-encoded CA private key.
    /// Must be provided together with --ca-cert.
    #[arg(long)]
    pub ca_key: Option<PathBuf>,

    /// Additional SAN for the generated server certificate (IP or DNS name, repeatable).
    /// Stored in DB as `network.extra_sans`. CLI value used only on first run
    /// or with `--force-settings-override`.
    #[arg(long = "san")]
    pub sans: Vec<String>,

    /// Header name for structured forwarded client certificate info
    /// (e.g. `X-Forwarded-Tls-Client-Cert-Info` for Traefik).
    /// Requires `--trusted-proxy` to take effect.
    /// Stored in DB as `network.forwarded_client_cert_info_header`.
    #[arg(long)]
    pub forwarded_client_cert_info_header: Option<String>,

    /// Header name for PEM-encoded forwarded client certificate
    /// (e.g. `X-Forwarded-Tls-Client-Cert` for Traefik/Caddy).
    /// Used as fallback when the info header is absent.
    /// Requires `--trusted-proxy` to take effect.
    /// Stored in DB as `network.forwarded_client_cert_pem_header`.
    #[arg(long)]
    pub forwarded_client_cert_pem_header: Option<String>,

    /// URL for PKI endpoints (OCSP, CRL, CA cert) embedded in certificate extensions.
    /// Supports both http:// and https:// schemes. http:// is recommended because
    /// Nginx only supports http:// OCSP responder URLs — https:// AIA URLs are
    /// silently ignored by Nginx's ssl_ocsp directive.
    /// Example: http://controller:8080 (recommended) or https://controller.internal:8443
    /// Stored in DB as `network.pki_addr`. CLI value used only on first run
    /// or with `--force-settings-override`.
    #[arg(long, value_parser = parse_pki_addr)]
    pub pki_addr: Option<String>,

    /// How to serve PKI endpoints over plain HTTP.
    /// Use `listener` to start a built-in HTTP server on the port from --pki-addr,
    /// or `external` if PKI HTTP is handled by a reverse proxy (suppresses the
    /// http:// scheme warning when --pki-addr uses http://).
    #[arg(long, value_enum)]
    pub pki_http: Option<PkiHttpMode>,

    /// Path to the built frontend directory. Enables SPA serving.
    #[arg(long)]
    pub static_dir: Option<PathBuf>,

    /// When set, CLI values for DB-managed settings (network, OIDC) overwrite
    /// any existing values in the database. Without this flag, DB values take
    /// priority and a warning is logged when CLI values differ.
    #[arg(long)]
    pub force_settings_override: bool,

    #[command(flatten)]
    pub oidc_bootstrap: OidcBootstrapArgs,
}

/// OIDC provider bootstrap options.
///
/// When `--oidc-issuer-url` is provided, the controller ensures an OIDC provider
/// exists in the database at startup. This solves the chicken-and-egg problem
/// where OIDC configuration requires ManageSettings permission, but the first
/// user must log in via OIDC.
#[derive(Parser, Debug)]
pub struct OidcBootstrapArgs {
    /// OIDC issuer URL. When set, bootstraps an OIDC provider at startup.
    /// Requires --oidc-client-id and --oidc-client-secret.
    #[arg(long)]
    pub oidc_issuer_url: Option<String>,

    /// OIDC client ID. Required when --oidc-issuer-url is set.
    #[arg(long)]
    pub oidc_client_id: Option<String>,

    /// OIDC client secret. Required when --oidc-issuer-url is set.
    #[arg(long)]
    pub oidc_client_secret: Option<String>,

    /// Display name for the bootstrapped OIDC provider.
    #[arg(long, default_value = "SSO")]
    pub oidc_provider_name: Option<String>,

    /// URL-safe slug for the bootstrapped OIDC provider.
    #[arg(long, default_value = "sso")]
    pub oidc_provider_slug: Option<String>,

    /// Space-separated OIDC scopes.
    #[arg(long, default_value = "openid email profile groups")]
    pub oidc_scopes: Option<String>,
}

/// How to serve PKI endpoints over plain HTTP.
#[derive(Clone, Copy, Debug, clap::ValueEnum)]
pub enum PkiHttpMode {
    /// Start a built-in plain HTTP listener for PKI endpoints.
    Listener,
    /// PKI HTTP is handled externally (e.g., by a reverse proxy).
    /// Suppresses the warning about http:// pki-addr without a listener.
    External,
}

/// Validate a PKI address URL argument.
/// Must be a valid URL with http or https scheme, and no trailing slash.
fn parse_pki_addr(s: &str) -> Result<String, String> {
    let url: url::Url = s.parse().map_err(|e| format!("invalid URL: {e}"))?;
    match url.scheme() {
        "http" | "https" => {}
        other => {
            return Err(format!(
                "unsupported URL scheme: {other} (expected http or https)"
            ));
        }
    }
    let result = s.trim_end_matches('/').to_string();
    Ok(result)
}

/// Parse a trusted proxy argument. Accepts:
/// - Bare IP: `192.168.1.1` -> `192.168.1.1/32`
/// - Bare IPv6: `::1` -> `::1/128`
/// - CIDR: `10.0.0.0/8`, `fd00::/8`
fn parse_proxy(s: &str) -> Result<IpNet, String> {
    // Try CIDR first
    if let Ok(net) = s.parse::<IpNet>() {
        return Ok(net);
    }
    // Try bare IP -> host network
    if let Ok(ip) = s.parse::<std::net::IpAddr>() {
        return Ok(IpNet::from(ip));
    }
    Err(format!("invalid IP or CIDR: {s}"))
}

impl Args {
    /// Resolve `data_dir` by expanding `~` to the user's home directory.
    pub fn resolve_data_dir(&self) -> Result<PathBuf, String> {
        let path = if self.data_dir.starts_with("~/") {
            let home = home_dir().ok_or("could not determine home directory")?;
            home.join(&self.data_dir[2..])
        } else if self.data_dir == "~" {
            home_dir().ok_or("could not determine home directory")?
        } else {
            PathBuf::from(&self.data_dir)
        };
        Ok(path)
    }
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    use clap::Parser;
    use ipnet::IpNet;

    use super::parse_proxy;

    #[test]
    fn bare_ipv4_parsed_as_host() {
        let net = parse_proxy("192.168.1.1").unwrap();
        let expected: IpNet = "192.168.1.1/32".parse().unwrap();
        assert_eq!(net, expected);
    }

    #[test]
    fn cidr_notation_parsed() {
        let net = parse_proxy("10.0.0.0/8").unwrap();
        let expected: IpNet = "10.0.0.0/8".parse().unwrap();
        assert_eq!(net, expected);
    }

    #[test]
    fn invalid_proxy_string() {
        assert!(parse_proxy("not-an-ip").is_err());
    }

    #[test]
    fn ipv6_bare_address() {
        let net = parse_proxy("::1").unwrap();
        assert_eq!(net, IpNet::from(IpAddr::V6(Ipv6Addr::LOCALHOST)));
    }

    #[test]
    fn ipv6_cidr() {
        let net = parse_proxy("fd00::/8").unwrap();
        let expected: IpNet = "fd00::/8".parse().unwrap();
        assert_eq!(net, expected);
    }

    #[test]
    fn empty_proxy_list() {
        // Just verifying parse_proxy isn't called with empty strings accidentally
        assert!(parse_proxy("").is_err());
    }

    #[test]
    fn defaults_have_no_addresses() {
        let args =
            super::Args::try_parse_from(["uptrakit-controller"]).expect("should parse defaults");
        assert!(args.https_addr.is_none());
        assert!(args.real_ip_header.is_none());
        assert!(args.trusted_proxies.is_empty());
        assert!(args.sans.is_empty());
        assert!(!args.force_settings_override);
    }

    #[test]
    fn explicit_addresses_parsed() {
        let args = super::Args::try_parse_from([
            "uptrakit-controller",
            "--https-addr",
            "0.0.0.0:9443",
            "--real-ip-header",
            "X-Real-Ip",
        ])
        .expect("should parse explicit addresses");
        assert_eq!(
            args.https_addr.unwrap(),
            "0.0.0.0:9443".parse::<std::net::SocketAddr>().unwrap()
        );
        assert_eq!(args.real_ip_header.as_deref(), Some("X-Real-Ip"));
    }

    #[test]
    fn force_settings_override_flag() {
        let args =
            super::Args::try_parse_from(["uptrakit-controller", "--force-settings-override"])
                .expect("should parse force flag");
        assert!(args.force_settings_override);
    }

    #[test]
    fn forwarded_cert_headers_not_set_by_default() {
        let args =
            super::Args::try_parse_from(["uptrakit-controller"]).expect("should parse defaults");
        assert!(args.forwarded_client_cert_info_header.is_none());
        assert!(args.forwarded_client_cert_pem_header.is_none());
    }

    #[test]
    fn forwarded_cert_headers_custom_values() {
        let args = super::Args::try_parse_from([
            "uptrakit-controller",
            "--forwarded-client-cert-info-header",
            "X-Forwarded-Tls-Client-Cert-Info",
            "--forwarded-client-cert-pem-header",
            "X-Forwarded-Tls-Client-Cert",
        ])
        .expect("should parse custom values");
        assert_eq!(
            args.forwarded_client_cert_info_header.as_deref(),
            Some("X-Forwarded-Tls-Client-Cert-Info")
        );
        assert_eq!(
            args.forwarded_client_cert_pem_header.as_deref(),
            Some("X-Forwarded-Tls-Client-Cert")
        );
    }

    #[test]
    fn bare_ipv4_contains_only_self() {
        let net = parse_proxy("192.168.1.1").unwrap();
        assert!(net.contains(&IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1))));
        assert!(!net.contains(&IpAddr::V4(Ipv4Addr::new(192, 168, 1, 2))));
    }

    #[test]
    fn oidc_bootstrap_not_set_by_default() {
        let args =
            super::Args::try_parse_from(["uptrakit-controller"]).expect("should parse defaults");
        assert!(args.oidc_bootstrap.oidc_issuer_url.is_none());
        assert!(args.oidc_bootstrap.oidc_client_id.is_none());
        assert!(args.oidc_bootstrap.oidc_client_secret.is_none());
        assert_eq!(
            args.oidc_bootstrap.oidc_provider_name.as_deref(),
            Some("SSO")
        );
        assert_eq!(
            args.oidc_bootstrap.oidc_provider_slug.as_deref(),
            Some("sso")
        );
        assert_eq!(
            args.oidc_bootstrap.oidc_scopes.as_deref(),
            Some("openid email profile groups")
        );
    }

    #[test]
    fn oidc_bootstrap_custom_values() {
        let args = super::Args::try_parse_from([
            "uptrakit-controller",
            "--oidc-issuer-url",
            "https://auth.example.com",
            "--oidc-client-id",
            "my-client",
            "--oidc-client-secret",
            "my-secret",
            "--oidc-provider-name",
            "My IdP",
            "--oidc-provider-slug",
            "my-idp",
            "--oidc-scopes",
            "openid email",
        ])
        .expect("should parse custom values");

        assert_eq!(
            args.oidc_bootstrap.oidc_issuer_url.as_deref(),
            Some("https://auth.example.com")
        );
        assert_eq!(
            args.oidc_bootstrap.oidc_client_id.as_deref(),
            Some("my-client")
        );
        assert_eq!(
            args.oidc_bootstrap.oidc_client_secret.as_deref(),
            Some("my-secret")
        );
        assert_eq!(
            args.oidc_bootstrap.oidc_provider_name.as_deref(),
            Some("My IdP")
        );
        assert_eq!(
            args.oidc_bootstrap.oidc_provider_slug.as_deref(),
            Some("my-idp")
        );
        assert_eq!(
            args.oidc_bootstrap.oidc_scopes.as_deref(),
            Some("openid email")
        );
    }

    #[test]
    fn oidc_bootstrap_partial_requires_all_three() {
        // Only issuer URL without client ID and secret should still parse
        // (validation happens at runtime, not at CLI parse time)
        let args = super::Args::try_parse_from([
            "uptrakit-controller",
            "--oidc-issuer-url",
            "https://auth.example.com",
        ])
        .expect("should parse with only issuer URL");
        assert!(args.oidc_bootstrap.oidc_issuer_url.is_some());
        assert!(args.oidc_bootstrap.oidc_client_id.is_none());
        assert!(args.oidc_bootstrap.oidc_client_secret.is_none());
    }
}

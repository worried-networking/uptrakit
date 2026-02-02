use std::path::PathBuf;

use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "uptrakit-agent")]
#[command(about = "Uptrakit agent that connects to the controller")]
pub struct Args {
    /// Controller URL in the format https://host:port.
    /// Port defaults to 443 if omitted.
    #[arg(long)]
    pub url: String,

    /// Trust the controller's TLS certificate on first connection (TOFU).
    /// Only effective when no CA certificate is cached locally.
    #[arg(long, conflicts_with = "ca_cert")]
    pub tofu: bool,

    /// Path to a PEM-encoded CA certificate file.
    #[arg(long)]
    pub ca_cert: Option<PathBuf>,

    /// Data directory for persistent state (CA cert, agent.json).
    /// Supports `~` for home directory expansion.
    #[arg(long, default_value = "~/.uptrakit-agent")]
    pub data_dir: String,

    /// Friendly name for this agent (defaults to system hostname)
    #[arg(long)]
    pub friendly_name: Option<String>,

    /// Pre-shared enrollment token for auto-approval
    #[arg(long)]
    pub enrollment_token: Option<String>,

    /// Force fresh enrollment, discarding any existing state.
    /// Use when the agent's certificate has been revoked.
    #[arg(long)]
    pub force_enroll: bool,

    /// Optional URL for PKI endpoints (CA certificate, OCSP).
    /// When set, the agent fetches the CA certificate from this address
    /// instead of from the main --url.
    /// Supports both http:// and https:// schemes.
    #[arg(long, value_parser = parse_pki_addr)]
    pub pki_addr: Option<String>,
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

    /// Parse `--url` into `(host, port)`.
    pub fn parsed_url(&self) -> Result<(String, u16), String> {
        let url_str = self.url.trim_end_matches('/');
        let parsed = url::Url::parse(url_str).map_err(|e| format!("invalid URL: {e}"))?;
        if parsed.scheme() != "https" {
            return Err("URL scheme must be https".to_string());
        }
        let host = parsed
            .host_str()
            .ok_or("URL must contain a host")?
            .to_string();
        let port = parsed.port().unwrap_or(443);
        Ok((host, port))
    }
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// Validate `--pki-addr`: must be http:// or https://, must have a host,
/// trailing slashes are stripped.
fn parse_pki_addr(s: &str) -> std::result::Result<String, String> {
    let trimmed = s.trim_end_matches('/');
    let parsed = url::Url::parse(trimmed).map_err(|e| format!("invalid PKI address URL: {e}"))?;
    match parsed.scheme() {
        "http" | "https" => {}
        other => {
            return Err(format!(
                "unsupported URL scheme: {other} (expected http or https)"
            ));
        }
    }
    if parsed.host_str().is_none() {
        return Err("PKI address URL must contain a host".to_string());
    }
    Ok(trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::Args;

    #[test]
    fn defaults_parse() {
        let args =
            Args::try_parse_from(["uptrakit-agent", "--url", "https://controller.local:8443"])
                .expect("should parse defaults");
        assert!(!args.tofu);
        assert!(args.ca_cert.is_none());
        assert!(args.friendly_name.is_none());
        assert!(args.enrollment_token.is_none());
        assert!(!args.force_enroll);
    }

    #[test]
    fn trust_first_use_and_ca_cert_conflict() {
        let result = Args::try_parse_from([
            "uptrakit-agent",
            "--url",
            "https://host:8443",
            "--tofu",
            "--ca-cert",
            "/some/path.pem",
        ]);
        assert!(result.is_err(), "--tofu and --ca-cert should conflict");
    }

    #[test]
    fn parsed_url_with_port() {
        let args =
            Args::try_parse_from(["uptrakit-agent", "--url", "https://myhost:9443"]).unwrap();
        let (host, port) = args.parsed_url().unwrap();
        assert_eq!(host, "myhost");
        assert_eq!(port, 9443);
    }

    #[test]
    fn parsed_url_default_port() {
        let args = Args::try_parse_from(["uptrakit-agent", "--url", "https://myhost"]).unwrap();
        let (host, port) = args.parsed_url().unwrap();
        assert_eq!(host, "myhost");
        assert_eq!(port, 443);
    }

    #[test]
    fn parsed_url_trailing_slash() {
        let args =
            Args::try_parse_from(["uptrakit-agent", "--url", "https://myhost:8443/"]).unwrap();
        let (host, port) = args.parsed_url().unwrap();
        assert_eq!(host, "myhost");
        assert_eq!(port, 8443);
    }

    #[test]
    fn parsed_url_rejects_http() {
        let args = Args::try_parse_from(["uptrakit-agent", "--url", "http://myhost:8443"]).unwrap();
        let err = args.parsed_url().unwrap_err();
        assert!(err.contains("https"), "should reject non-https: {err}");
    }

    #[test]
    fn pki_addr_accepts_http() {
        let args = Args::try_parse_from([
            "uptrakit-agent",
            "--url",
            "https://controller:8443",
            "--pki-addr",
            "http://controller:8080",
        ])
        .expect("should parse --pki-addr with http://");
        assert_eq!(args.pki_addr.as_deref(), Some("http://controller:8080"));
    }

    #[test]
    fn pki_addr_accepts_https() {
        let args = Args::try_parse_from([
            "uptrakit-agent",
            "--url",
            "https://controller:8443",
            "--pki-addr",
            "https://pki.example.com",
        ])
        .expect("should parse --pki-addr with https://");
        assert_eq!(args.pki_addr.as_deref(), Some("https://pki.example.com"));
    }

    #[test]
    fn pki_addr_strips_trailing_slash() {
        let args = Args::try_parse_from([
            "uptrakit-agent",
            "--url",
            "https://controller:8443",
            "--pki-addr",
            "http://controller:8080/",
        ])
        .expect("should strip trailing slash");
        assert_eq!(args.pki_addr.as_deref(), Some("http://controller:8080"));
    }

    #[test]
    fn pki_addr_rejects_ftp() {
        let result = Args::try_parse_from([
            "uptrakit-agent",
            "--url",
            "https://controller:8443",
            "--pki-addr",
            "ftp://controller:21",
        ]);
        assert!(result.is_err(), "should reject ftp:// scheme");
    }
}

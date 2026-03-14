use clap::Parser;
use uptrakit_service_sdk::cli::CommonServiceArgs;

#[derive(Parser, Debug)]
#[command(name = "uptrakit-agent")]
#[command(about = "Uptrakit agent that connects to the controller")]
#[command(disable_version_flag = true)]
pub(crate) struct Args {
    #[command(flatten)]
    pub common: CommonServiceArgs,
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
        assert!(!args.common.version);
        assert!(!args.common.tofu);
        assert!(args.common.ca_cert.is_none());
        assert!(args.common.config_dir.is_none());
        assert!(args.common.state_dir.is_none());
        assert!(args.common.friendly_name.is_none());
        assert!(args.common.enrollment_token.is_none());
        assert!(!args.common.force_enroll);
    }

    #[test]
    fn resolve_dirs_with_defaults() {
        let args =
            Args::try_parse_from(["uptrakit-agent", "--url", "https://controller.local:8443"])
                .expect("should parse defaults");
        let dirs = args
            .common
            .resolve_dirs("agent")
            .expect("should resolve dirs");
        assert!(!dirs.config_dir().as_os_str().is_empty());
        assert!(!dirs.state_dir().as_os_str().is_empty());
    }

    #[test]
    fn resolve_dirs_with_overrides() {
        let args = Args::try_parse_from([
            "uptrakit-agent",
            "--url",
            "https://controller.local:8443",
            "--config-dir",
            "/custom/config",
            "--state-dir",
            "/custom/state",
        ])
        .expect("should parse");
        let dirs = args
            .common
            .resolve_dirs("agent")
            .expect("should resolve dirs");
        assert_eq!(dirs.config_dir().to_str().unwrap(), "/custom/config");
        assert_eq!(dirs.state_dir().to_str().unwrap(), "/custom/state");
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
    fn tofu_and_pki_addr_conflict() {
        let result = Args::try_parse_from([
            "uptrakit-agent",
            "--url",
            "https://host:8443",
            "--tofu",
            "--pki-addr",
            "http://pki.local:8080",
        ]);
        assert!(result.is_err(), "--tofu and --pki-addr should conflict");
    }

    #[test]
    fn parsed_url_with_port() {
        let args =
            Args::try_parse_from(["uptrakit-agent", "--url", "https://myhost:9443"]).unwrap();
        let (host, port) = args.common.parsed_url().unwrap();
        assert_eq!(host, "myhost");
        assert_eq!(port, 9443);
    }

    #[test]
    fn parsed_url_default_port() {
        let args = Args::try_parse_from(["uptrakit-agent", "--url", "https://myhost"]).unwrap();
        let (host, port) = args.common.parsed_url().unwrap();
        assert_eq!(host, "myhost");
        assert_eq!(port, 443);
    }

    #[test]
    fn parsed_url_trailing_slash() {
        let args =
            Args::try_parse_from(["uptrakit-agent", "--url", "https://myhost:8443/"]).unwrap();
        let (host, port) = args.common.parsed_url().unwrap();
        assert_eq!(host, "myhost");
        assert_eq!(port, 8443);
    }

    #[test]
    fn parsed_url_rejects_http() {
        let args = Args::try_parse_from(["uptrakit-agent", "--url", "http://myhost:8443"]).unwrap();
        let err = args.common.parsed_url().unwrap_err();
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
        assert_eq!(
            args.common.pki_addr.as_deref(),
            Some("http://controller:8080")
        );
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
        assert_eq!(
            args.common.pki_addr.as_deref(),
            Some("https://pki.example.com")
        );
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
        assert_eq!(
            args.common.pki_addr.as_deref(),
            Some("http://controller:8080")
        );
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

    #[test]
    fn version_flag_parses_without_other_flags() {
        let args = Args::try_parse_from(["uptrakit-agent", "--version"]).expect("should parse");
        assert!(args.common.version);
    }
}

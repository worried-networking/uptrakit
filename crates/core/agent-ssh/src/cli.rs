use std::path::PathBuf;

use clap::Parser;
use uptrakit_service_sdk::cli::CommonServiceArgs;

#[derive(Parser, Debug)]
#[command(name = "uptrakit-agent-ssh")]
#[command(about = "Uptrakit SSH-backed agent that manages remote hosts over SSH")]
#[command(disable_version_flag = true)]
pub struct Args {
    #[command(flatten)]
    pub common: CommonServiceArgs,

    /// Path to a file containing the master encryption key (64-char hex string).
    /// The key is used for AES-256-GCM encryption of SSH private keys at rest.
    /// Alternative: set UPTRAKIT_MASTER_KEY environment variable.
    #[arg(long)]
    pub master_key_file: Option<PathBuf>,

    /// Allow the SSH agent to start without a master encryption key.
    /// Encryption at rest is disabled when no key is provided.
    /// This flag is for development only and logs a warning when used.
    #[arg(long)]
    pub allow_plaintext_secrets: bool,
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::Args;

    #[test]
    fn defaults_parse() {
        let args = Args::try_parse_from([
            "uptrakit-agent-ssh",
            "--url",
            "https://controller.local:8443",
        ])
        .expect("should parse defaults");
        assert!(!args.common.version);
        assert!(!args.common.tofu);
        assert!(args.common.ca_cert.is_none());
        assert!(args.common.config_dir.is_none());
        assert!(args.common.state_dir.is_none());
        assert!(args.common.friendly_name.is_none());
        assert!(args.common.enrollment_token.is_none());
        assert!(!args.common.force_enroll);
        assert!(args.master_key_file.is_none());
        assert!(!args.allow_plaintext_secrets);
    }

    #[test]
    fn resolve_dirs_with_defaults() {
        let args = Args::try_parse_from([
            "uptrakit-agent-ssh",
            "--url",
            "https://controller.local:8443",
        ])
        .expect("should parse defaults");
        let dirs = args
            .common
            .resolve_dirs("agent-ssh")
            .expect("should resolve dirs");
        assert!(!dirs.config_dir().as_os_str().is_empty());
        assert!(!dirs.state_dir().as_os_str().is_empty());
    }

    #[test]
    fn resolve_dirs_with_overrides() {
        let args = Args::try_parse_from([
            "uptrakit-agent-ssh",
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
            .resolve_dirs("agent-ssh")
            .expect("should resolve dirs");
        assert_eq!(dirs.config_dir().to_str().unwrap(), "/custom/config");
        assert_eq!(dirs.state_dir().to_str().unwrap(), "/custom/state");
    }

    #[test]
    fn trust_first_use_and_ca_cert_conflict() {
        let result = Args::try_parse_from([
            "uptrakit-agent-ssh",
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
            "uptrakit-agent-ssh",
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
            Args::try_parse_from(["uptrakit-agent-ssh", "--url", "https://myhost:9443"]).unwrap();
        let (host, port) = args.common.parsed_url().unwrap();
        assert_eq!(host, "myhost");
        assert_eq!(port, 9443);
    }

    #[test]
    fn parsed_url_default_port() {
        let args = Args::try_parse_from(["uptrakit-agent-ssh", "--url", "https://myhost"]).unwrap();
        let (host, port) = args.common.parsed_url().unwrap();
        assert_eq!(host, "myhost");
        assert_eq!(port, 443);
    }

    #[test]
    fn parsed_url_trailing_slash() {
        let args =
            Args::try_parse_from(["uptrakit-agent-ssh", "--url", "https://myhost:8443/"]).unwrap();
        let (host, port) = args.common.parsed_url().unwrap();
        assert_eq!(host, "myhost");
        assert_eq!(port, 8443);
    }

    #[test]
    fn parsed_url_rejects_http() {
        let args =
            Args::try_parse_from(["uptrakit-agent-ssh", "--url", "http://myhost:8443"]).unwrap();
        let err = args.common.parsed_url().unwrap_err();
        assert!(err.contains("https"), "should reject non-https: {err}");
    }

    #[test]
    fn master_key_file_parses() {
        let args = Args::try_parse_from([
            "uptrakit-agent-ssh",
            "--url",
            "https://controller:8443",
            "--master-key-file",
            "/etc/uptrakit/master.key",
        ])
        .expect("should parse --master-key-file");
        assert_eq!(
            args.master_key_file.as_ref().unwrap().to_str().unwrap(),
            "/etc/uptrakit/master.key"
        );
    }

    #[test]
    fn allow_plaintext_secrets_flag() {
        let args = Args::try_parse_from([
            "uptrakit-agent-ssh",
            "--url",
            "https://controller:8443",
            "--allow-plaintext-secrets",
        ])
        .expect("should parse --allow-plaintext-secrets");
        assert!(args.allow_plaintext_secrets);
    }

    #[test]
    fn version_flag_parses_without_other_flags() {
        let args = Args::try_parse_from(["uptrakit-agent-ssh", "--version"]).expect("should parse");
        assert!(args.common.version);
    }
}

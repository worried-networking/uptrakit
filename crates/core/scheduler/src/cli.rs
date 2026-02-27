use clap::Parser;
use uptrakit_service_sdk::cli::CommonServiceArgs;

#[derive(Parser, Debug)]
#[command(name = "uptrakit-scheduler")]
#[command(about = "Uptrakit external scheduler that connects to the controller")]
#[command(disable_version_flag = true)]
pub struct Args {
    #[command(flatten)]
    pub common: CommonServiceArgs,

    /// Override scheduler poll interval in seconds (default: 15).
    #[arg(long, default_value = "15")]
    pub poll_interval_secs: u64,
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::Args;

    #[test]
    fn defaults_parse() {
        let args = Args::try_parse_from([
            "uptrakit-scheduler",
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
        assert_eq!(args.poll_interval_secs, 15);
    }

    #[test]
    fn custom_poll_interval() {
        let args = Args::try_parse_from([
            "uptrakit-scheduler",
            "--url",
            "https://controller.local:8443",
            "--poll-interval-secs",
            "30",
        ])
        .expect("should parse");
        assert_eq!(args.poll_interval_secs, 30);
    }

    #[test]
    fn version_flag_parses_without_other_flags() {
        let args = Args::try_parse_from(["uptrakit-scheduler", "--version"]).expect("should parse");
        assert!(args.common.version);
    }

    #[test]
    fn resolve_dirs_with_defaults() {
        let args = Args::try_parse_from([
            "uptrakit-scheduler",
            "--url",
            "https://controller.local:8443",
        ])
        .expect("should parse defaults");
        let dirs = args
            .common
            .resolve_dirs("scheduler")
            .expect("should resolve dirs");
        assert!(!dirs.config_dir().as_os_str().is_empty());
        assert!(!dirs.state_dir().as_os_str().is_empty());
    }
}

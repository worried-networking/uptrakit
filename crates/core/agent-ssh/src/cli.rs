use std::path::PathBuf;

use clap::{Parser, Subcommand};
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

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Manage SSH host entries in the local database.
    Host {
        #[command(subcommand)]
        command: HostCommands,
    },
}

#[derive(Subcommand, Debug)]
pub enum HostCommands {
    /// Add a new SSH host.
    Add {
        /// Friendly name for this host (must be unique).
        #[arg(long)]
        name: String,

        /// SSH hostname or IP address.
        #[arg(long)]
        hostname: String,

        /// SSH port.
        #[arg(long, default_value = "22")]
        port: i32,

        /// SSH username.
        #[arg(long)]
        username: String,

        /// Path to the SSH private key file. Use `-` to read from stdin.
        #[arg(long)]
        private_key_file: PathBuf,

        /// Expected host key fingerprint (e.g., `SHA256:...`).
        #[arg(long)]
        host_key_fingerprint: Option<String>,
    },

    /// List all SSH hosts.
    List,

    /// Show details of an SSH host (by name or UUID).
    Show {
        /// Host name or UUID.
        name_or_id: String,
    },

    /// Update an SSH host (by name or UUID).
    Update {
        /// Host name or UUID to update.
        name_or_id: String,

        /// New friendly name.
        #[arg(long)]
        name: Option<String>,

        /// New SSH hostname or IP address.
        #[arg(long)]
        hostname: Option<String>,

        /// New SSH port.
        #[arg(long)]
        port: Option<i32>,

        /// New SSH username.
        #[arg(long)]
        username: Option<String>,

        /// Path to the new SSH private key file. Use `-` to read from stdin.
        #[arg(long)]
        private_key_file: Option<PathBuf>,

        /// New expected host key fingerprint.
        #[arg(long)]
        host_key_fingerprint: Option<String>,
    },

    /// Remove an SSH host (by name or UUID).
    Remove {
        /// Host name or UUID.
        name_or_id: String,
    },

    /// Bootstrap a remote host: create user, deploy SSH key, configure
    /// sudoers, verify connectivity, and save the host entry.
    Bootstrap {
        /// Friendly name for this host (must be unique).
        #[arg(long)]
        name: String,

        /// SSH hostname or IP address.
        #[arg(long)]
        hostname: String,

        /// Username for initial SSH authentication.
        #[arg(long)]
        auth_username: String,

        /// Password for initial SSH authentication.
        /// Use `--auth-password` (no value) to prompt securely at runtime.
        /// Use `--auth-password <VALUE>` to pass the password inline.
        /// Mutually exclusive with --auth-private-key-file.
        #[arg(long, num_args = 0..=1, default_missing_value = None, conflicts_with = "auth_private_key_file")]
        auth_password: Option<Option<String>>,

        /// Path to private key for initial authentication. Use `-` for stdin.
        /// Mutually exclusive with --auth-password.
        #[arg(long, conflicts_with = "auth_password")]
        auth_private_key_file: Option<PathBuf>,

        /// Username on the remote host for ongoing SSH access.
        /// Defaults to 'uptrakit' when --auth-username is 'root',
        /// otherwise defaults to --auth-username.
        #[arg(long)]
        target_username: Option<String>,

        /// Path to a private key for the target user. Use `-` for stdin.
        /// If omitted, a new Ed25519 keypair is generated.
        #[arg(long)]
        target_private_key_file: Option<PathBuf>,

        /// SSH port.
        #[arg(long, default_value = "22")]
        port: i32,

        /// Expected host key fingerprint (e.g., `SHA256:...`).
        /// If omitted, trust-on-first-use is applied.
        #[arg(long)]
        host_key_fingerprint: Option<String>,
    },
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

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
        assert!(args.command.is_none());
    }

    #[test]
    fn no_subcommand_is_daemon_mode() {
        let args = Args::try_parse_from(["uptrakit-agent-ssh", "--url", "https://controller:8443"])
            .expect("should parse");
        assert!(args.command.is_none());
    }

    #[test]
    fn host_add_parses() {
        let args = Args::try_parse_from([
            "uptrakit-agent-ssh",
            "--allow-plaintext-secrets",
            "host",
            "add",
            "--name",
            "myhost",
            "--hostname",
            "192.168.1.1",
            "--username",
            "root",
            "--private-key-file",
            "/path/to/key",
        ])
        .expect("should parse host add");

        match &args.command {
            Some(Commands::Host {
                command:
                    HostCommands::Add {
                        name,
                        hostname,
                        port,
                        username,
                        private_key_file,
                        host_key_fingerprint,
                    },
            }) => {
                assert_eq!(name, "myhost");
                assert_eq!(hostname, "192.168.1.1");
                assert_eq!(*port, 22);
                assert_eq!(username, "root");
                assert_eq!(private_key_file.to_str().expect("path"), "/path/to/key");
                assert!(host_key_fingerprint.is_none());
            }
            other => panic!("expected Host Add, got: {other:?}"),
        }
    }

    #[test]
    fn host_add_with_all_options() {
        let args = Args::try_parse_from([
            "uptrakit-agent-ssh",
            "--allow-plaintext-secrets",
            "host",
            "add",
            "--name",
            "myhost",
            "--hostname",
            "10.0.0.1",
            "--port",
            "2222",
            "--username",
            "deploy",
            "--private-key-file",
            "-",
            "--host-key-fingerprint",
            "SHA256:abc123",
        ])
        .expect("should parse");

        match &args.command {
            Some(Commands::Host {
                command:
                    HostCommands::Add {
                        port,
                        private_key_file,
                        host_key_fingerprint,
                        ..
                    },
            }) => {
                assert_eq!(*port, 2222);
                assert_eq!(private_key_file.to_str().expect("path"), "-");
                assert_eq!(host_key_fingerprint.as_deref(), Some("SHA256:abc123"));
            }
            other => panic!("expected Host Add, got: {other:?}"),
        }
    }

    #[test]
    fn host_list_parses() {
        let args = Args::try_parse_from([
            "uptrakit-agent-ssh",
            "--allow-plaintext-secrets",
            "host",
            "list",
        ])
        .expect("should parse host list");

        assert!(matches!(
            &args.command,
            Some(Commands::Host {
                command: HostCommands::List
            })
        ));
    }

    #[test]
    fn host_show_parses() {
        let args = Args::try_parse_from([
            "uptrakit-agent-ssh",
            "--allow-plaintext-secrets",
            "host",
            "show",
            "my-server",
        ])
        .expect("should parse host show");

        match &args.command {
            Some(Commands::Host {
                command: HostCommands::Show { name_or_id },
            }) => {
                assert_eq!(name_or_id, "my-server");
            }
            other => panic!("expected Host Show, got: {other:?}"),
        }
    }

    #[test]
    fn host_update_parses() {
        let args = Args::try_parse_from([
            "uptrakit-agent-ssh",
            "--allow-plaintext-secrets",
            "host",
            "update",
            "my-server",
            "--port",
            "2222",
            "--hostname",
            "new-host",
        ])
        .expect("should parse host update");

        match &args.command {
            Some(Commands::Host {
                command:
                    HostCommands::Update {
                        name_or_id,
                        port,
                        hostname,
                        name,
                        username,
                        private_key_file,
                        host_key_fingerprint,
                    },
            }) => {
                assert_eq!(name_or_id, "my-server");
                assert_eq!(*port, Some(2222));
                assert_eq!(hostname.as_deref(), Some("new-host"));
                assert!(name.is_none());
                assert!(username.is_none());
                assert!(private_key_file.is_none());
                assert!(host_key_fingerprint.is_none());
            }
            other => panic!("expected Host Update, got: {other:?}"),
        }
    }

    #[test]
    fn host_remove_parses() {
        let args = Args::try_parse_from([
            "uptrakit-agent-ssh",
            "--allow-plaintext-secrets",
            "host",
            "remove",
            "old-server",
        ])
        .expect("should parse host remove");

        match &args.command {
            Some(Commands::Host {
                command: HostCommands::Remove { name_or_id },
            }) => {
                assert_eq!(name_or_id, "old-server");
            }
            other => panic!("expected Host Remove, got: {other:?}"),
        }
    }

    #[test]
    fn host_list_without_url() {
        let args = Args::try_parse_from([
            "uptrakit-agent-ssh",
            "--allow-plaintext-secrets",
            "--state-dir",
            "/tmp/test",
            "host",
            "list",
        ])
        .expect("host list should parse without --url");

        assert!(args.common.url.is_none());
        assert!(matches!(
            &args.command,
            Some(Commands::Host {
                command: HostCommands::List
            })
        ));
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

    #[test]
    fn host_bootstrap_with_password_prompt() {
        let args = Args::try_parse_from([
            "uptrakit-agent-ssh",
            "--allow-plaintext-secrets",
            "host",
            "bootstrap",
            "--name",
            "new-host",
            "--hostname",
            "10.0.0.5",
            "--auth-username",
            "admin",
            "--auth-password",
        ])
        .expect("should parse bootstrap with password prompt");

        match &args.command {
            Some(Commands::Host {
                command:
                    HostCommands::Bootstrap {
                        name,
                        hostname,
                        auth_username,
                        auth_password,
                        auth_private_key_file,
                        target_username,
                        target_private_key_file,
                        port,
                        host_key_fingerprint,
                    },
            }) => {
                assert_eq!(name, "new-host");
                assert_eq!(hostname, "10.0.0.5");
                assert_eq!(auth_username, "admin");
                assert_eq!(
                    *auth_password,
                    Some(None),
                    "--auth-password with no value should be Some(None)"
                );
                assert!(auth_private_key_file.is_none());
                assert!(target_username.is_none());
                assert!(target_private_key_file.is_none());
                assert_eq!(*port, 22);
                assert!(host_key_fingerprint.is_none());
            }
            other => panic!("expected Host Bootstrap, got: {other:?}"),
        }
    }

    #[test]
    fn host_bootstrap_with_password_inline() {
        let args = Args::try_parse_from([
            "uptrakit-agent-ssh",
            "--allow-plaintext-secrets",
            "host",
            "bootstrap",
            "--name",
            "new-host",
            "--hostname",
            "10.0.0.5",
            "--auth-username",
            "admin",
            "--auth-password",
            "mypass123",
        ])
        .expect("should parse bootstrap with inline password");

        match &args.command {
            Some(Commands::Host {
                command: HostCommands::Bootstrap { auth_password, .. },
            }) => {
                assert_eq!(
                    *auth_password,
                    Some(Some("mypass123".to_string())),
                    "--auth-password mypass123 should be Some(Some(\"mypass123\"))"
                );
            }
            other => panic!("expected Host Bootstrap, got: {other:?}"),
        }
    }

    #[test]
    fn host_bootstrap_without_auth_flags() {
        let args = Args::try_parse_from([
            "uptrakit-agent-ssh",
            "--allow-plaintext-secrets",
            "host",
            "bootstrap",
            "--name",
            "new-host",
            "--hostname",
            "10.0.0.5",
            "--auth-username",
            "admin",
        ])
        .expect("should parse bootstrap without auth flags (agent fallback)");

        match &args.command {
            Some(Commands::Host {
                command:
                    HostCommands::Bootstrap {
                        auth_password,
                        auth_private_key_file,
                        ..
                    },
            }) => {
                assert!(
                    auth_password.is_none(),
                    "no --auth-password flag means None"
                );
                assert!(auth_private_key_file.is_none());
            }
            other => panic!("expected Host Bootstrap, got: {other:?}"),
        }
    }

    #[test]
    fn host_bootstrap_with_key_and_all_options() {
        let args = Args::try_parse_from([
            "uptrakit-agent-ssh",
            "--allow-plaintext-secrets",
            "host",
            "bootstrap",
            "--name",
            "prod-server",
            "--hostname",
            "192.168.1.50",
            "--auth-username",
            "root",
            "--auth-private-key-file",
            "/root/.ssh/id_ed25519",
            "--target-username",
            "uptrakit",
            "--target-private-key-file",
            "-",
            "--port",
            "2222",
            "--host-key-fingerprint",
            "SHA256:abc123",
        ])
        .expect("should parse bootstrap with all options");

        match &args.command {
            Some(Commands::Host {
                command:
                    HostCommands::Bootstrap {
                        name,
                        hostname,
                        auth_username,
                        auth_password,
                        auth_private_key_file,
                        target_username,
                        target_private_key_file,
                        port,
                        host_key_fingerprint,
                    },
            }) => {
                assert_eq!(name, "prod-server");
                assert_eq!(hostname, "192.168.1.50");
                assert_eq!(auth_username, "root");
                assert!(auth_password.is_none(), "key auth should have no password");
                assert_eq!(
                    auth_private_key_file
                        .as_ref()
                        .map(|p| p.to_str().expect("path")),
                    Some("/root/.ssh/id_ed25519")
                );
                assert_eq!(target_username.as_deref(), Some("uptrakit"));
                assert_eq!(
                    target_private_key_file
                        .as_ref()
                        .map(|p| p.to_str().expect("path")),
                    Some("-")
                );
                assert_eq!(*port, 2222);
                assert_eq!(host_key_fingerprint.as_deref(), Some("SHA256:abc123"));
            }
            other => panic!("expected Host Bootstrap, got: {other:?}"),
        }
    }

    #[test]
    fn host_bootstrap_root_auth_omits_target_username() {
        let args = Args::try_parse_from([
            "uptrakit-agent-ssh",
            "--allow-plaintext-secrets",
            "host",
            "bootstrap",
            "--name",
            "root-host",
            "--hostname",
            "10.0.0.1",
            "--auth-username",
            "root",
            "--auth-password",
            "pass",
        ])
        .expect("should parse bootstrap with root auth and no target-username");

        match &args.command {
            Some(Commands::Host {
                command: HostCommands::Bootstrap {
                    auth_username,
                    target_username,
                    ..
                },
            }) => {
                assert_eq!(auth_username, "root");
                assert!(
                    target_username.is_none(),
                    "target_username should be None at the CLI layer (defaulting happens in dispatch)"
                );
            }
            other => panic!("expected Host Bootstrap, got: {other:?}"),
        }
    }

    #[test]
    fn host_bootstrap_password_and_key_conflict() {
        let result = Args::try_parse_from([
            "uptrakit-agent-ssh",
            "--allow-plaintext-secrets",
            "host",
            "bootstrap",
            "--name",
            "test",
            "--hostname",
            "host",
            "--auth-username",
            "user",
            "--auth-password",
            "--auth-private-key-file",
            "/key",
        ]);
        assert!(
            result.is_err(),
            "--auth-password and --auth-private-key-file should conflict"
        );
    }
}

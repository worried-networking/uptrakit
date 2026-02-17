//! SSH configuration file resolution.
//!
//! Reads `~/.ssh/config` and resolves host-specific defaults for `User`,
//! `Port`, and `HostName` using the `ssh2-config` crate.

use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;

/// Defaults resolved from `~/.ssh/config` for a given host.
#[derive(Debug, Default)]
pub struct SshConfigDefaults {
    pub username: Option<String>,
    pub port: Option<u16>,
    pub hostname: Option<String>,
}

/// Resolve SSH config defaults for `host` from `~/.ssh/config`.
///
/// Returns empty defaults if the config file does not exist, cannot be read,
/// or fails to parse. This ensures the bootstrap command never fails solely
/// because of a broken SSH config.
pub fn resolve_defaults(host: &str) -> SshConfigDefaults {
    let Some(config_path) = ssh_config_path() else {
        tracing::debug!("could not determine home directory; skipping SSH config");
        return SshConfigDefaults::default();
    };

    resolve_defaults_from_path(&config_path, host)
}

/// Resolve SSH config defaults from a specific file path.
fn resolve_defaults_from_path(path: &PathBuf, host: &str) -> SshConfigDefaults {
    let file = match File::open(path) {
        Ok(f) => f,
        Err(e) => {
            if e.kind() != std::io::ErrorKind::NotFound {
                tracing::debug!("could not open SSH config at {}: {e}", path.display());
            }
            return SshConfigDefaults::default();
        }
    };

    let mut reader = BufReader::new(file);
    let config = match ssh2_config::SshConfig::default()
        .parse(&mut reader, ssh2_config::ParseRule::ALLOW_UNKNOWN_FIELDS)
    {
        Ok(cfg) => cfg,
        Err(e) => {
            tracing::debug!("failed to parse SSH config at {}: {e}", path.display());
            return SshConfigDefaults::default();
        }
    };

    let params = config.query(host);

    SshConfigDefaults {
        username: params.user,
        port: params.port,
        hostname: params.host_name,
    }
}

/// Return the platform path to `~/.ssh/config`.
fn ssh_config_path() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".ssh").join("config"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_config(content: &str) -> tempfile::NamedTempFile {
        let mut file = tempfile::NamedTempFile::new().expect("create temp file");
        file.write_all(content.as_bytes())
            .expect("write config content");
        file.flush().expect("flush");
        file
    }

    #[test]
    fn resolves_user_and_port() {
        let config =
            write_config("Host myserver\n  User deploy\n  Port 2222\n  HostName real.host.com\n");
        let defaults = resolve_defaults_from_path(&config.path().to_path_buf(), "myserver");
        assert_eq!(defaults.username.as_deref(), Some("deploy"));
        assert_eq!(defaults.port, Some(2222));
        assert_eq!(defaults.hostname.as_deref(), Some("real.host.com"));
    }

    #[test]
    fn wildcard_match() {
        let config = write_config("Host *\n  User defaultuser\n  Port 22\n");
        let defaults = resolve_defaults_from_path(&config.path().to_path_buf(), "anything");
        assert_eq!(defaults.username.as_deref(), Some("defaultuser"));
        assert_eq!(defaults.port, Some(22));
    }

    #[test]
    fn no_match_returns_empty() {
        let config = write_config("Host otherhost\n  User someone\n");
        let defaults = resolve_defaults_from_path(&config.path().to_path_buf(), "myserver");
        assert!(defaults.username.is_none());
        assert!(defaults.port.is_none());
        assert!(defaults.hostname.is_none());
    }

    #[test]
    fn missing_file_returns_empty() {
        let path = PathBuf::from("/nonexistent/path/to/ssh/config");
        let defaults = resolve_defaults_from_path(&path, "myserver");
        assert!(defaults.username.is_none());
        assert!(defaults.port.is_none());
        assert!(defaults.hostname.is_none());
    }

    #[test]
    fn malformed_config_returns_empty() {
        let config = write_config("this is not valid ssh config %%%\n");
        let defaults = resolve_defaults_from_path(&config.path().to_path_buf(), "myserver");
        // Should not panic; returns empty defaults.
        assert!(defaults.username.is_none());
    }

    #[test]
    fn partial_match() {
        let config = write_config("Host myserver\n  User admin\n");
        let defaults = resolve_defaults_from_path(&config.path().to_path_buf(), "myserver");
        assert_eq!(defaults.username.as_deref(), Some("admin"));
        assert!(defaults.port.is_none());
        assert!(defaults.hostname.is_none());
    }

    #[test]
    fn host_specific_overrides_wildcard() {
        let config = write_config(
            "Host myserver\n  User specific\n  Port 3333\n\nHost *\n  User default\n  Port 22\n",
        );
        let defaults = resolve_defaults_from_path(&config.path().to_path_buf(), "myserver");
        assert_eq!(defaults.username.as_deref(), Some("specific"));
        assert_eq!(defaults.port, Some(3333));
    }
}

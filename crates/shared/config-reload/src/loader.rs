use std::path::Path;

use rootcause::prelude::*;

use crate::config::RuntimeConfig;
use crate::error::ConfigReloadError;

/// The result of loading and parsing a TOML config file.
#[non_exhaustive]
pub struct LoadedConfig {
    /// The parsed and validated runtime configuration.
    pub config: RuntimeConfig,
    /// Warnings about unknown keys that were ignored during parse.
    pub warnings: Vec<String>,
}

/// Loads and validates a TOML config file from disk.
pub struct TomlConfigLoader;

impl TomlConfigLoader {
    /// Read, parse, validate, and return the config at `path`.
    ///
    /// Unknown keys are captured in each section's `extra` map and surfaced
    /// as [`LoadedConfig::warnings`] rather than causing a parse failure.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read, the TOML is malformed,
    /// or any section fails [`RuntimeConfig::validate`].
    pub fn load(path: impl AsRef<Path>) -> Result<LoadedConfig, Report> {
        let path = path.as_ref();
        let raw = std::fs::read_to_string(path).map_err(|e| {
            report!(ConfigReloadError::TomlIo {
                path: path.to_path_buf(),
                source_msg: e.to_string(),
            })
        })?;
        check_old_format_hint(&raw, path)?;
        let config: RuntimeConfig = toml::from_str(&raw).map_err(|e| {
            report!(ConfigReloadError::TomlParse {
                path: path.to_path_buf(),
                source_msg: e.to_string(),
            })
        })?;
        config.validate()?;
        check_config_permissions(path, &config)?;
        let warnings = config.warn_about_extras();
        Ok(LoadedConfig { config, warnings })
    }

    /// Read, parse, and validate the config at `path` without returning it.
    ///
    /// Suitable for pre-flight checks (e.g. `uptrakit-ctl config validate`).
    /// Any unknown-key warnings are printed to stderr.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read, the TOML is malformed,
    /// or validation fails.
    pub fn validate_only(path: impl AsRef<Path>) -> Result<(), Report> {
        let loaded = Self::load(path)?;
        for w in &loaded.warnings {
            eprintln!("warning: {w}");
        }
        Ok(())
    }
}

/// Emit a helpful migration error if the config still uses deprecated section names.
///
/// Detects `[master_key]`, `[network.https]`, and `[network.pki]` as standalone
/// section headers and returns a descriptive error before the TOML parser produces
/// a cryptic "unknown field" message.
fn check_old_format_hint(raw: &str, path: &Path) -> Result<(), Report> {
    let has_old_master_key = raw.lines().any(|l| l.trim() == "[master_key]");
    let has_old_network = raw
        .lines()
        .any(|l| l.trim() == "[network.https]" || l.trim() == "[network.pki]");

    if has_old_master_key {
        bail!(ConfigReloadError::Validate(format!(
            "{path:?}: config uses the old `[master_key]` section — \
             replace it with a top-level field: `master_key = \"file:<path>\"`"
        )));
    }
    if has_old_network {
        bail!(ConfigReloadError::Validate(format!(
            "{path:?}: config uses old `[network.https]` / `[network.pki]` sub-sections — \
             move all fields directly into `[network]` and rename `pki.addr` to `pki_addr`"
        )));
    }
    Ok(())
}

/// Check that the config file permissions are restrictive enough when the master key
/// is stored inline (i.e. not behind a `file:` or `env:` prefix).
///
/// On Unix: fails if group or other bits are set (`mode & 0o077 != 0`).
/// On non-Unix: emits a warning via `tracing::warn!` but does not fail.
fn check_config_permissions(path: &Path, config: &RuntimeConfig) -> Result<(), Report> {
    let key = config.master_key.expose_secret();
    if key.is_empty() || key.starts_with("file:") || key.starts_with("env:") {
        return Ok(());
    }
    check_config_permissions_inner(path)
}

#[cfg(unix)]
fn check_config_permissions_inner(path: &Path) -> Result<(), Report> {
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(path)
        .map_err(|e| {
            report!(ConfigReloadError::TomlIo {
                path: path.to_path_buf(),
                source_msg: e.to_string(),
            })
        })?
        .permissions()
        .mode();
    if mode & 0o077 != 0 {
        bail!(ConfigReloadError::Validate(format!(
            "config file {path:?} contains an inline master key and must not be readable \
             by group or other (current mode: {:04o}); run: chmod 0600 {path:?}",
            mode & 0o777,
        )));
    }
    Ok(())
}

#[cfg(not(unix))]
fn check_config_permissions_inner(path: &Path) -> Result<(), Report> {
    tracing::warn!(
        path = %path.display(),
        "inline master_key in config file — cannot verify file permissions on this platform"
    );
    Ok(())
}

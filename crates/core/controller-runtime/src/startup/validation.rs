//! Phase 8: Configuration validation.

use std::path::PathBuf;

use rootcause::prelude::*;

use super::{ReconciledSettings, ValidatedConfig};
use crate::AppError;

/// Validate TLS, CA, SAN, and PKI HTTP configuration from TOML.
///
/// `tls_cert_path` and `tls_key_path` come from `runtime.tls.cert_path` /
/// `runtime.tls.key_path`.  They are non-empty only when the operator
/// provides custom certificates; empty strings mean the internal CA is used.
///
/// `static_dir` is `runtime.plugins.static_dir` or `None` when absent.
/// `pki_http` is derived from the PKI addr scheme in the TOML config.
pub(crate) fn validate_configuration(
    runtime: &uptrakit_config_reload::RuntimeConfig,
    reconciled: &ReconciledSettings,
) -> crate::Result<ValidatedConfig> {
    let tls_cert = runtime.tls.cert_path.as_str();
    let tls_key = runtime.tls.key_path.as_str();
    let has_tls_cert = !tls_cert.is_empty();
    let has_tls_key = !tls_key.is_empty();

    // In debug builds (but not test builds), probe the filesystem first so
    // `frontend/build` changes are picked up without recompiling.
    // Falls back to embedded if the dir is absent.
    let static_dir_path: Option<PathBuf> = None;
    let static_dir =
        if !cfg!(feature = "embedded-frontend") || (cfg!(debug_assertions) && !cfg!(test)) {
            resolve_static_dir(static_dir_path)?
        } else {
            None
        };

    // Validate TLS paths: both or neither must be non-empty.
    if has_tls_cert != has_tls_key {
        bail!(AppError::Config(
            "both tls.cert_path and tls.key_path must be set together in the TOML config".into()
        ));
    }

    // SANs only make sense with managed (auto-generated) certificates.
    if !reconciled.sans.is_empty() && has_tls_cert {
        bail!(AppError::Config(
            "tls.sans cannot be used with tls.cert_path/tls.key_path; \
             SANs are only configurable for controller-managed certificates"
                .into()
        ));
    }

    // Determine PKI HTTP port from the PKI addr scheme.
    // An http:// pki addr without a dedicated HTTP listener just logs a warning;
    // there is no longer a separate --pki-http flag.
    let pki_http_port: Option<u16> = if let Some(ref url) = reconciled.pki_addr
        && url.starts_with("http://")
    {
        // Parse the port out of the PKI address URL for the built-in HTTP listener.
        match url.parse::<url::Url>() {
            Ok(parsed) => {
                if let Some(port) = parsed.port_or_known_default() {
                    Some(port)
                } else {
                    tracing::warn!(
                        "network.pki_addr uses http:// scheme but has no explicit port; \
                         the built-in PKI HTTP listener will not start"
                    );
                    None
                }
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "network.pki_addr URL could not be parsed; PKI HTTP listener disabled"
                );
                None
            }
        }
    } else {
        None
    };

    Ok(ValidatedConfig {
        static_dir,
        pki_http_port,
    })
}

/// Resolve the static directory for SPA serving.
///
/// Auto-detects by probing `frontend/build` and `frontend`
/// relative to the current working directory.
///
/// This function is always compiled so that static-dir auto-detection can
/// override the embedded frontend assets even when the `embedded-frontend`
/// feature is active.
fn resolve_static_dir(explicit: Option<PathBuf>) -> crate::Result<Option<PathBuf>> {
    if let Some(dir) = explicit {
        let index = dir.join("index.html");
        if !index.is_file() {
            bail!(AppError::Config(format!(
                "static_dir {}: missing index.html",
                dir.display()
            )));
        }
        tracing::info!("serving static files from {}", dir.display());
        return Ok(Some(dir));
    }

    for candidate in ["frontend/build", "frontend"] {
        let dir = PathBuf::from(candidate);
        if dir.join("index.html").is_file() {
            tracing::info!("auto-detected static files in {}", dir.display());
            return Ok(Some(dir));
        }
    }

    Ok(None)
}

//! Phase 8: Configuration validation.

use std::path::PathBuf;

use rootcause::prelude::*;

use super::{ReconciledSettings, ValidatedConfig};
use crate::AppError;

/// Validate TLS, CA, SAN, and PKI HTTP args.  Resolve the static directory.
pub(crate) fn validate_configuration(
    args: &crate::cli::Args,
    reconciled: &ReconciledSettings,
) -> crate::Result<ValidatedConfig> {
    // If --static-dir is given explicitly, always resolve and use it (overrides embedded assets).
    // Without an explicit path: auto-detect only when embed-frontend is not compiled in.
    let static_dir = if args.static_dir.is_some() || !cfg!(feature = "embed-frontend") {
        resolve_static_dir(args.static_dir.clone())?
    } else {
        None
    };

    // Validate TLS args
    if args.tls_cert.is_some() != args.tls_key.is_some() {
        bail!(AppError::Config(
            "both --tls-cert and --tls-key must be provided together".into()
        ));
    }

    // --san only makes sense with managed (auto-generated) certificates
    if !reconciled.sans.is_empty() && args.tls_cert.is_some() {
        bail!(AppError::Config(
            "--san cannot be used with --tls-cert/--tls-key; \
             SANs are only configurable for controller-managed certificates"
                .into()
        ));
    }

    // Validate CA args
    if args.ca_cert.is_some() != args.ca_key.is_some() {
        bail!(AppError::Config(
            "both --ca-cert and --ca-key must be provided together".into()
        ));
    }

    // Validate --pki-http
    let pki_http_port: Option<u16> = if let Some(mode) = args.pki_http {
        let pki_url = reconciled.pki_addr.as_deref().ok_or_else(|| {
            report!(AppError::Config(
                "--pki-http requires --pki-addr to be set".into()
            ))
        })?;
        match mode {
            crate::cli::PkiHttpMode::Listener => {
                let parsed: url::Url = pki_url.parse().map_err(|e| {
                    report!(AppError::Config(format!(
                        "--pki-addr URL is not valid: {e}"
                    )))
                })?;
                let port = parsed.port_or_known_default().ok_or_else(|| {
                    report!(AppError::Config(
                        "--pki-addr URL must have an explicit or default port".into()
                    ))
                })?;
                Some(port)
            }
            crate::cli::PkiHttpMode::External => None,
        }
    } else {
        if let Some(ref url) = reconciled.pki_addr
            && url.starts_with("http://")
        {
            tracing::warn!(
                "--pki-addr uses http:// scheme but --pki-http is not set; \
                 the controller is NOT serving PKI endpoints over plain HTTP. \
                 Add --pki-http listener to start the HTTP listener, or \
                 --pki-http external if PKI HTTP is handled by a reverse proxy."
            );
        }
        None
    };

    Ok(ValidatedConfig {
        static_dir,
        pki_http_port,
    })
}

/// Resolve the static directory for SPA serving.
///
/// If `--static-dir` is given, validates that it contains `index.html`.
/// Otherwise, auto-detects by probing `frontend/build` and `frontend`
/// relative to the current working directory.
///
/// This function is always compiled so that `--static-dir` can override the
/// embedded frontend assets even when the `embed-frontend` feature is active.
fn resolve_static_dir(explicit: Option<PathBuf>) -> crate::Result<Option<PathBuf>> {
    if let Some(dir) = explicit {
        let index = dir.join("index.html");
        if !index.is_file() {
            bail!(AppError::Config(format!(
                "--static-dir {}: missing index.html",
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

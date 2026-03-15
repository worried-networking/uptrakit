//! Phase 9: PKI + TLS initialization.

use std::sync::Arc;

use rootcause::prelude::*;

use super::{PkiRuntime, ReconciledSettings};
use crate::AppError;

/// Initialize the entire PKI subsystem: CA state, server certificate,
/// CRL manager, and TLS configuration.
pub(crate) async fn init_pki_runtime(
    args: &crate::cli::Args,
    db: &sea_orm::DatabaseConnection,
    config_dir: &std::path::Path,
    reconciled: &ReconciledSettings,
) -> crate::Result<PkiRuntime> {
    use crate::pki;

    let pki_path = pki::pki_dir(config_dir).context(AppError::Pki)?;

    // Load CA state
    let ca_state = if let (Some(ca_cert_path), Some(ca_key_path)) = (&args.ca_cert, &args.ca_key) {
        // External CA — not managed
        let ca = pki::load_external_ca(ca_cert_path, ca_key_path).context(AppError::Pki)?;
        let trusted = vec![
            pki::bundle_from_pem(ca.cert_pem.clone(), ca.key_pem.clone()).context(AppError::Pki)?,
        ];
        pki::CaState {
            active: ca,
            previous: None,
            trusted,
            managed: false,
        }
    } else {
        let mut state = pki::load_or_init_managed_ca(db, reconciled.pki_addr.as_deref())
            .await
            .context(AppError::Pki)?;

        if pki::should_rotate_ca(&state.active.cert_pem) {
            tracing::info!("CA certificate is within rotation window, rotating now");
            let active_fp = pki::ca_fingerprint(&state.active.cert_pem).context(AppError::Pki)?;
            let rotation = pki::rotate_managed_ca(db, reconciled.pki_addr.as_deref(), &active_fp)
                .await
                .context(AppError::Pki)?;
            state = rotation.state;
        }

        state
    };

    // Validate CA extensions match pki_addr (managed CAs only)
    if ca_state.managed {
        pki::validate_ca_pki_addr(&ca_state.active.cert_pem, reconciled.pki_addr.as_deref())
            .context(AppError::Pki)?;
    }

    let (ca_snapshot, ca_initial_key_store) = ca_state
        .to_snapshot(reconciled.pki_addr.clone())
        .context(AppError::Pki)?;
    let (ca_tx, ca_rx) = tokio::sync::watch::channel(ca_snapshot.clone());
    let ca_key_store: uptrakit_web_api::CaKeyStoreRef =
        Arc::new(tokio::sync::RwLock::new(ca_initial_key_store));

    // Resolve server certificate (using reconciled sans)
    let server_cert = if let (Some(cert_path), Some(key_path)) = (&args.tls_cert, &args.tls_key) {
        pki::load_external_cert(cert_path, key_path).context(AppError::Pki)?
    } else {
        let mut cert =
            pki::load_or_generate_server_cert(&pki_path, &ca_state.active, &reconciled.sans)
                .await
                .context(AppError::Pki)?;

        // Check if the existing cert needs SAN regeneration
        if pki::server_cert_needs_san_update(&cert.cert_pem, &reconciled.sans)
            .context(AppError::Pki)?
        {
            if pki::cert_signed_by_ca(&cert.cert_pem, &ca_state.active.cert_pem)
                .context(AppError::Pki)?
            {
                tracing::info!(
                    "server certificate SANs do not match configured values, regenerating"
                );
                cert = pki::renew_server_cert(&pki_path, &ca_state.active, &reconciled.sans)
                    .await
                    .context(AppError::Pki)?;
            } else {
                bail!(AppError::Config(
                    "The server certificate does not include the requested SANs and was signed by \
                     a different CA than the currently active one.\n\n\
                     To fix this:\n  \
                     1. Restart the controller without the --san flag(s) that are not yet in the certificate\n  \
                     2. Regenerate the server certificate via POST /api/v1/settings/renew-server-certificate or the UI\n  \
                     3. Restart the controller with the desired --san flag(s)"
                        .into()
                ));
            }
        }

        // Auto-renew if within renewal window
        if pki::should_renew_server_cert(&cert.cert_pem) {
            tracing::info!("server certificate is within renewal window, renewing now");
            cert = pki::renew_server_cert(&pki_path, &ca_state.active, &reconciled.sans)
                .await
                .context(AppError::Pki)?;
        }

        cert
    };

    // Install the default crypto provider for rustls
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    let revocation_notify = Arc::new(tokio::sync::Notify::const_new());
    let ca_rotation_trigger = Arc::new(tokio::sync::Notify::const_new());

    // Build initial CRLs — tries DB cache first, generates fresh if missing/stale.
    let crl_pem_cache = Arc::new(tokio::sync::RwLock::new(String::new()));
    let (initial_crls, initial_crl_pem, starting_crl_number) = {
        let ks = ca_key_store.read().await;
        crate::crl_manager::build_initial_crls(db, &ca_snapshot, &ks)
            .await
            .context(AppError::Pki)?
    };
    *crl_pem_cache.write().await = initial_crl_pem;

    // Build initial server config with CRLs
    let initial_server_config = pki::build_rustls_config_with_client_auth_and_crls(
        &server_cert.cert_pem,
        &server_cert.key_pem,
        &ca_snapshot.bundle_pem,
        initial_crls,
    )
    .context(AppError::Pki)?;

    let rustls_config =
        axum_server::tls_rustls::RustlsConfig::from_config(Arc::new(initial_server_config));

    // Create CRL manager
    let crl_manager = Arc::new({
        let ks = ca_key_store.read().await;
        crate::crl_manager::CrlManager::new(
            crate::crl_manager::CrlManagerConfig {
                server_cert_pem: server_cert.cert_pem.clone(),
                server_key_pem: server_cert.key_pem.clone(),
                db: db.clone(),
                rustls_config: rustls_config.clone(),
                revocation_notify: Arc::clone(&revocation_notify),
                crl_pem_cache: Arc::clone(&crl_pem_cache),
            },
            &ca_snapshot,
            &ks,
            starting_crl_number,
        )
        .context(AppError::Pki)?
    });

    let initial_ca_version = if ca_state.managed {
        pki::load_ca_version(db).await.context(AppError::Pki)?
    } else {
        0
    };

    Ok(PkiRuntime {
        ca_managed: ca_state.managed,
        pki_path,
        ca_tx,
        ca_rx,
        ca_key_store,
        rustls_config,
        revocation_notify,
        ca_rotation_trigger,
        crl_pem_cache,
        crl_manager,
        initial_ca_version,
    })
}

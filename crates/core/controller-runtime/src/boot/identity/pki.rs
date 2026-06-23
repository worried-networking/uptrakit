//! Phase 9: PKI + TLS initialization and cert-signer construction.
//!
//! Thin wrappers around [`crate::startup::init_pki_runtime`] and the
//! `RcgenAgentCertSigner` builder that previously lived inline in
//! `boot::run_server`.

use std::sync::Arc;

use uptrakit_web_api::cert_signer::AgentCertSigner;

use crate::startup::PkiRuntime;

/// Phase 9: initialize the entire PKI subsystem.
///
/// Delegates to [`crate::startup::init_pki_runtime`].
pub(super) async fn init(
    runtime: &uptrakit_config_reload::RuntimeConfig,
    db: &sea_orm::DatabaseConnection,
    config_dir: &std::path::Path,
    reconciled: &crate::startup::ReconciledSettings,
) -> crate::Result<PkiRuntime> {
    crate::startup::init_pki_runtime(runtime, db, config_dir, reconciled).await
}

/// Build a [`AgentCertSigner`] from an already-initialized [`PkiRuntime`].
///
/// Resolves the effective trust domain from `runtime.tls` and constructs an
/// [`crate::cert_signer::RcgenAgentCertSigner`], optionally wrapping it with a
/// SPIFFE trust domain when one is configured.
pub(super) fn build_cert_signer(
    pki: &PkiRuntime,
    runtime: &uptrakit_config_reload::RuntimeConfig,
) -> Arc<dyn AgentCertSigner> {
    // Two-step: clone as concrete type then coerce to Arc<dyn IssuerSource>.
    // Arc::clone resolves its argument type from the return annotation, so
    // we cannot pass &Arc<CrlManager> when the binding expects Arc<dyn Trait>.
    let issuer_source: Arc<dyn crate::cert_signer::IssuerSource> = {
        let concrete: Arc<crate::crl_manager::CrlManager> = Arc::clone(&pki.crl_manager);
        concrete
    };

    // Resolve the effective trust domain: explicit tls.trust_domain wins;
    // falls back to tls.sans[0] (legacy derivation); empty = SPIFFE disabled.
    let effective_trust_domain = runtime
        .tls
        .effective_trust_domain(&runtime.tls.sans)
        .to_owned();

    let signer = crate::cert_signer::RcgenAgentCertSigner::new(pki.ca_rx.clone(), issuer_source);

    if effective_trust_domain.is_empty() {
        Arc::new(signer) as Arc<dyn AgentCertSigner>
    } else {
        Arc::new(signer.with_trust_domain(effective_trust_domain)) as Arc<dyn AgentCertSigner>
    }
}

//! Certificate renewal helpers.
//!
//! Contains the `sign_renewal_csr` and `record_renewal_certificate` functions
//! extracted from the unified handler module.

#![expect(
    clippy::let_underscore_must_use,
    reason = "fire-and-forget sends in renewal handler intentionally drop results"
)]
#![expect(
    clippy::map_err_ignore,
    reason = "original renewal errors carry no useful context"
)]

use sea_orm::{ActiveModelTrait, Set};

use rootcause::prelude::*;
use thiserror::Error;
use uptrakit_shared_db::entity::{
    service, service_certificate, system_service, system_service_certificate,
};
use uptrakit_shared_macros::impl_report_conversion;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Error type for certificate renewal (sign without invalidating secret).
#[derive(Debug, Error)]
pub(super) enum RenewalError {
    #[error("database error: {0}")]
    Database(#[from] sea_orm::DbErr),
    #[error("certificate signing failed: {0}")]
    Signing(String),
    #[error("PEM parse error")]
    PemParse,
    #[error("invalid timestamp: {0}")]
    Timestamp(String),
}

impl_report_conversion!(sea_orm::DbErr => RenewalError::Database);

// ---------------------------------------------------------------------------
// sign_renewal_csr
// ---------------------------------------------------------------------------

/// Sign a CSR for certificate renewal without invalidating the enrollment
/// secret hash. Used by the `RenewCertificate` handler for already-
/// authenticated services.
pub(super) async fn sign_renewal_csr(
    cert_signer: &dyn crate::cert_signer::AgentCertSigner,
    settings: &crate::settings::Settings,
    db: &sea_orm::DatabaseConnection,
    svc: service::Model,
    csr_pem: &str,
) -> Result<crate::cert_signer::SignedCertBundle, Report<RenewalError>> {
    let effective_hours = svc
        .cert_lifetime_hours
        .map(|h| h as u32)
        .unwrap_or_else(|| settings.agent_cert_lifetime_hours());
    let validity = time::Duration::hours(i64::from(effective_hours));

    let ca_fp = cert_signer.active_ca_fingerprint();

    let bundle = cert_signer
        .sign_agent_csr(csr_pem, &svc.id, validity)
        .await
        .map_err(|e| report!(RenewalError::Signing(format!("failed to sign CSR: {e}"))))?;

    // Record the new certificate.
    record_renewal_certificate(db, svc.id, &bundle.cert_pem, &ca_fp).await?;

    // Update timestamps (no enrollment secret invalidation).
    let mut active: service::ActiveModel = svc.into();
    active.last_seen_at = Set(Some(time::OffsetDateTime::now_utc()));
    active.updated_at = Set(time::OffsetDateTime::now_utc());
    let _ = active.update(db).await;

    Ok(bundle)
}

// ---------------------------------------------------------------------------
// record_renewal_certificate
// ---------------------------------------------------------------------------

/// Record a certificate in the service_certificates table (renewal path).
async fn record_renewal_certificate(
    db: &sea_orm::DatabaseConnection,
    service_id: uuid::Uuid,
    cert_pem: &str,
    ca_fingerprint: &str,
) -> Result<(), Report<RenewalError>> {
    use der::DecodePem;
    use x509_cert::Certificate;
    let cert =
        Certificate::from_pem(cert_pem.as_bytes()).map_err(|_| report!(RenewalError::PemParse))?;
    let tbs = &cert.tbs_certificate;
    let serial = tbs.serial_number.to_string();
    let not_before = time::OffsetDateTime::from_unix_timestamp(
        tbs.validity.not_before.to_unix_duration().as_secs() as i64,
    )
    .map_err(|e| report!(RenewalError::Timestamp(format!("not_before: {e}"))))?;
    let not_after = time::OffsetDateTime::from_unix_timestamp(
        tbs.validity.not_after.to_unix_duration().as_secs() as i64,
    )
    .map_err(|e| report!(RenewalError::Timestamp(format!("not_after: {e}"))))?;

    let record = service_certificate::ActiveModel {
        ca_fingerprint: Set(ca_fingerprint.to_string()),
        serial_number: Set(serial),
        service_id: Set(service_id),
        not_before: Set(not_before),
        not_after: Set(not_after),
        revoked_at: Set(None),
        revocation_reason: Set(None),
        created_at: Set(time::OffsetDateTime::now_utc()),
        last_seen_at: Set(None),
    };

    record.insert(db).await.context_to::<RenewalError>()?;

    Ok(())
}

// ---------------------------------------------------------------------------
// sign_renewal_csr_system
// ---------------------------------------------------------------------------

/// Sign a CSR for system service certificate renewal.
///
/// Writes the new cert to `system_service_certificates`. Does NOT invalidate
/// the enrollment secret (renewal, not re-enrollment).
pub(super) async fn sign_renewal_csr_system(
    cert_signer: &dyn crate::cert_signer::AgentCertSigner,
    settings: &crate::settings::Settings,
    db: &sea_orm::DatabaseConnection,
    svc: system_service::Model,
    csr_pem: &str,
) -> Result<crate::cert_signer::SignedCertBundle, Report<RenewalError>> {
    let effective_hours = svc
        .cert_lifetime_hours
        .map(|h| h as u32)
        .unwrap_or_else(|| settings.agent_cert_lifetime_hours());
    let validity = time::Duration::hours(i64::from(effective_hours));
    let ca_fp = cert_signer.active_ca_fingerprint();

    let bundle = cert_signer
        .sign_agent_csr(csr_pem, &svc.id, validity)
        .await
        .map_err(|e| report!(RenewalError::Signing(format!("failed to sign CSR: {e}"))))?;

    // Parse cert metadata.
    use der::DecodePem;
    use x509_cert::Certificate as X509Certificate;
    let sys_cert = X509Certificate::from_pem(bundle.cert_pem.as_bytes())
        .map_err(|_| report!(RenewalError::PemParse))?;
    let sys_tbs = &sys_cert.tbs_certificate;
    let serial = sys_tbs.serial_number.to_string();
    let not_before = time::OffsetDateTime::from_unix_timestamp(
        sys_tbs.validity.not_before.to_unix_duration().as_secs() as i64,
    )
    .map_err(|e| report!(RenewalError::Timestamp(format!("not_before: {e}"))))?;
    let not_after = time::OffsetDateTime::from_unix_timestamp(
        sys_tbs.validity.not_after.to_unix_duration().as_secs() as i64,
    )
    .map_err(|e| report!(RenewalError::Timestamp(format!("not_after: {e}"))))?;

    let record = system_service_certificate::ActiveModel {
        ca_fingerprint: Set(ca_fp),
        serial_number: Set(serial),
        system_service_id: Set(svc.id),
        not_before: Set(not_before),
        not_after: Set(not_after),
        revoked_at: Set(None),
        revocation_reason: Set(None),
        created_at: Set(time::OffsetDateTime::now_utc()),
        last_seen_at: Set(None),
    };
    record.insert(db).await.context_to::<RenewalError>()?;

    // Update timestamps (no enrollment secret invalidation).
    let mut active: system_service::ActiveModel = svc.into();
    active.last_seen_at = Set(Some(time::OffsetDateTime::now_utc()));
    active.updated_at = Set(time::OffsetDateTime::now_utc());
    let _ = active.update(db).await;

    Ok(bundle)
}

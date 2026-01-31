use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use rcgen::{
    CertificateRevocationListParams, Issuer, KeyIdMethod, KeyPair, RevokedCertParams, SerialNumber,
};
use rustls::pki_types::CertificateRevocationListDer;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};
use time::OffsetDateTime;
use tokio::sync::{Notify, RwLock};
use uptrakit_shared_db::entity::{agent_certificate, prelude::*};

use crate::pki::{self, CaSnapshot};

/// Configuration for the CRL manager.
pub struct CrlManagerConfig {
    pub server_cert_pem: String,
    pub server_key_pem: String,
    pub db: DatabaseConnection,
    pub rustls_config: axum_server::tls_rustls::RustlsConfig,
    pub revocation_notify: Arc<Notify>,
}

/// Mutable CA material that can be updated at runtime when the CA rotates.
struct CaIssuers {
    active: Issuer<'static, KeyPair>,
    active_fingerprint: String,
    active_bundle_pem: String,
    prev: Option<(Issuer<'static, KeyPair>, String)>,
}

/// CRL lifecycle manager.
///
/// Builds CRLs from the database and hot-reloads the TLS configuration
/// so that `WebPkiClientVerifier` rejects revoked client certificates.
pub struct CrlManager {
    config: CrlManagerConfig,
    crl_number: AtomicU64,
    issuers: RwLock<CaIssuers>,
    server_cert: RwLock<(String, String)>,
}

/// Build DER-encoded CRLs from the database (standalone, for initial startup).
pub async fn build_initial_crls_der(
    db: &DatabaseConnection,
    snapshot: &CaSnapshot,
) -> pki::Result<Vec<CertificateRevocationListDer<'static>>> {
    let active_key =
        KeyPair::from_pem(&snapshot.active_key_pem).context_to::<pki::PkiError>()?;
    let active_issuer = Issuer::from_ca_cert_pem(&snapshot.active_cert_pem, active_key)
        .context_to::<pki::PkiError>()?;

    let active_revoked =
        query_revoked_certs_for_ca(db, &snapshot.active_fingerprint).await?;
    let active_crl = sign_crl(&active_issuer, active_revoked, 0)?;
    let mut crls = vec![active_crl];

    if let (Some(prev_cert_pem), Some(prev_key_pem), Some(prev_fp)) = (
        &snapshot.previous_cert_pem,
        &snapshot.previous_key_pem,
        &snapshot.previous_fingerprint,
    ) {
        let prev_key = KeyPair::from_pem(prev_key_pem).context_to::<pki::PkiError>()?;
        let prev_issuer =
            Issuer::from_ca_cert_pem(prev_cert_pem, prev_key).context_to::<pki::PkiError>()?;
        let prev_revoked = query_revoked_certs_for_ca(db, prev_fp).await?;
        let prev_crl = sign_crl(&prev_issuer, prev_revoked, 0)?;
        crls.push(prev_crl);
    }

    Ok(crls)
}

impl CrlManager {
    pub fn new(config: CrlManagerConfig, snapshot: &CaSnapshot) -> pki::Result<Self> {
        let active_key =
            KeyPair::from_pem(&snapshot.active_key_pem).context_to::<pki::PkiError>()?;
        let active_issuer = Issuer::from_ca_cert_pem(&snapshot.active_cert_pem, active_key)
            .context_to::<pki::PkiError>()?;

        let prev = if let (Some(prev_cert_pem), Some(prev_key_pem), Some(prev_fp)) = (
            &snapshot.previous_cert_pem,
            &snapshot.previous_key_pem,
            &snapshot.previous_fingerprint,
        ) {
            let prev_key = KeyPair::from_pem(prev_key_pem).context_to::<pki::PkiError>()?;
            let prev_issuer = Issuer::from_ca_cert_pem(prev_cert_pem, prev_key)
                .context_to::<pki::PkiError>()?;
            Some((prev_issuer, prev_fp.clone()))
        } else {
            None
        };

        let server_cert_pem = config.server_cert_pem.clone();
        let server_key_pem = config.server_key_pem.clone();

        Ok(Self {
            config,
            crl_number: AtomicU64::new(1),
            issuers: RwLock::new(CaIssuers {
                active: active_issuer,
                active_fingerprint: snapshot.active_fingerprint.clone(),
                active_bundle_pem: snapshot.bundle_pem.clone(),
                prev,
            }),
            server_cert: RwLock::new((server_cert_pem, server_key_pem)),
        })
    }

    /// Update CA issuers after a rotation event.
    pub async fn update_ca(&self, snapshot: &CaSnapshot) -> pki::Result<()> {
        let active_key =
            KeyPair::from_pem(&snapshot.active_key_pem).context_to::<pki::PkiError>()?;
        let active_issuer = Issuer::from_ca_cert_pem(&snapshot.active_cert_pem, active_key)
            .context_to::<pki::PkiError>()?;

        let prev = if let (Some(prev_cert_pem), Some(prev_key_pem), Some(prev_fp)) = (
            &snapshot.previous_cert_pem,
            &snapshot.previous_key_pem,
            &snapshot.previous_fingerprint,
        ) {
            let prev_key = KeyPair::from_pem(prev_key_pem).context_to::<pki::PkiError>()?;
            let prev_issuer = Issuer::from_ca_cert_pem(prev_cert_pem, prev_key)
                .context_to::<pki::PkiError>()?;
            Some((prev_issuer, prev_fp.clone()))
        } else {
            None
        };

        let mut issuers = self.issuers.write().await;
        issuers.active = active_issuer;
        issuers.active_fingerprint = snapshot.active_fingerprint.clone();
        issuers.active_bundle_pem = snapshot.bundle_pem.clone();
        issuers.prev = prev;

        Ok(())
    }

    /// Update server cert material (after renewal).
    pub async fn update_server_cert(&self, cert_pem: String, key_pem: String) {
        let mut cert = self.server_cert.write().await;
        *cert = (cert_pem, key_pem);
    }

    /// Build DER-encoded CRLs from revoked certificates in the database.
    async fn build_crls_der(
        &self,
    ) -> pki::Result<Vec<CertificateRevocationListDer<'static>>> {
        let issuers = self.issuers.read().await;
        let crl_number = self.crl_number.fetch_add(1, Ordering::Relaxed);

        let active_revoked =
            query_revoked_certs_for_ca(&self.config.db, &issuers.active_fingerprint).await?;
        let active_crl = sign_crl(&issuers.active, active_revoked, crl_number)?;
        let mut crls = vec![active_crl];

        if let Some((ref prev_issuer, ref prev_fp)) = issuers.prev {
            let prev_revoked =
                query_revoked_certs_for_ca(&self.config.db, prev_fp).await?;
            let prev_crl = sign_crl(prev_issuer, prev_revoked, crl_number)?;
            crls.push(prev_crl);
        }

        Ok(crls)
    }

    /// Rebuild the CRLs and hot-reload the TLS configuration.
    pub async fn reload_tls_config(&self) -> pki::Result<()> {
        let crls = self.build_crls_der().await?;
        let issuers = self.issuers.read().await;
        let server_cert = self.server_cert.read().await;

        let server_config = pki::build_rustls_config_with_client_auth_and_crls(
            &server_cert.0,
            &server_cert.1,
            &issuers.active_bundle_pem,
            crls,
        )?;

        self.config
            .rustls_config
            .reload_from_config(Arc::new(server_config));

        tracing::info!("TLS configuration reloaded with updated CRL");
        Ok(())
    }

    /// Background task: rebuilds CRL on revocation events or periodic timer.
    pub async fn run(self: Arc<Self>) {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(3600));
        // The first tick completes immediately — skip it since we already
        // built the initial CRL synchronously before starting.
        interval.tick().await;

        loop {
            tokio::select! {
                _ = interval.tick() => {
                    tracing::debug!("periodic CRL refresh");
                }
                _ = self.config.revocation_notify.notified() => {
                    tracing::debug!("CRL rebuild triggered by revocation event");
                }
            }

            if let Err(e) = self.reload_tls_config().await {
                tracing::error!(error = ?e, "failed to reload TLS config with updated CRL");
            }
        }
    }
}

/// Query revoked certificates for a specific CA (by fingerprint).
async fn query_revoked_certs_for_ca(
    db: &DatabaseConnection,
    ca_fingerprint: &str,
) -> pki::Result<Vec<RevokedCertParams>> {
    let now = OffsetDateTime::now_utc();
    let grace = time::Duration::hours(24);

    let revoked_certs = AgentCertificate::find()
        .filter(agent_certificate::Column::CaFingerprint.eq(ca_fingerprint))
        .filter(agent_certificate::Column::RevokedAt.is_not_null())
        .filter(agent_certificate::Column::NotAfter.gt(now - grace))
        .all(db)
        .await
        .map_err(|e| {
            rootcause::report!(pki::PkiError::Hostname(format!("DB query failed: {e}")))
        })?;

    let mut revoked = Vec::with_capacity(revoked_certs.len());
    for cert in &revoked_certs {
        let Some(serial_bytes) = parse_serial_string(&cert.serial_number) else {
            tracing::warn!(
                serial = %cert.serial_number,
                "skipping certificate with unparseable serial number"
            );
            continue;
        };

        let revocation_time = cert.revoked_at.unwrap_or(now);
        let reason_code = cert.revocation_reason.map(map_reason);

        revoked.push(RevokedCertParams {
            serial_number: SerialNumber::from_slice(&serial_bytes),
            revocation_time,
            reason_code,
            invalidity_date: None,
        });
    }

    Ok(revoked)
}

/// Sign a CRL with the given issuer and return the DER bytes.
fn sign_crl(
    ca_issuer: &Issuer<'_, KeyPair>,
    revoked_certs: Vec<RevokedCertParams>,
    crl_number: u64,
) -> pki::Result<CertificateRevocationListDer<'static>> {
    let now = OffsetDateTime::now_utc();
    let params = CertificateRevocationListParams {
        this_update: now,
        next_update: now + time::Duration::hours(24),
        crl_number: SerialNumber::from(crl_number),
        issuing_distribution_point: None,
        revoked_certs,
        key_identifier_method: KeyIdMethod::Sha256,
    };

    let crl = params.signed_by(ca_issuer).context_to::<pki::PkiError>()?;

    Ok(CertificateRevocationListDer::from(crl.der().to_vec()))
}

/// Parse a colon-hex serial string (e.g. `"00:ab:cd"`) into raw bytes.
fn parse_serial_string(s: &str) -> Option<Vec<u8>> {
    s.split(':')
        .map(|hex| u8::from_str_radix(hex, 16).ok())
        .collect()
}

/// Map application-level `RevocationReason` to rcgen's RFC 5280 reason.
fn map_reason(reason: RevocationReason) -> rcgen::RevocationReason {
    match reason {
        RevocationReason::CertificateRenewed => rcgen::RevocationReason::Superseded,
        RevocationReason::AgentDeactivated => rcgen::RevocationReason::CessationOfOperation,
        RevocationReason::AgentMerged => rcgen::RevocationReason::CessationOfOperation,
    }
}

use rootcause::prelude::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_serial_string_valid() {
        assert_eq!(
            parse_serial_string("00:ab:cd"),
            Some(vec![0x00, 0xab, 0xcd])
        );
    }

    #[test]
    fn parse_serial_string_single_byte() {
        assert_eq!(parse_serial_string("ff"), Some(vec![0xff]));
    }

    #[test]
    fn parse_serial_string_invalid() {
        assert_eq!(parse_serial_string("zz:00"), None);
    }

    #[test]
    fn map_reason_coverage() {
        assert_eq!(
            map_reason(RevocationReason::CertificateRenewed) as u8,
            rcgen::RevocationReason::Superseded as u8
        );
        assert_eq!(
            map_reason(RevocationReason::AgentDeactivated) as u8,
            rcgen::RevocationReason::CessationOfOperation as u8
        );
        assert_eq!(
            map_reason(RevocationReason::AgentMerged) as u8,
            rcgen::RevocationReason::CessationOfOperation as u8
        );
    }
}

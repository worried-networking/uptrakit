use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use rcgen::{
    CertificateRevocationListParams, Issuer, KeyIdMethod, KeyPair, RevokedCertParams, SerialNumber,
};
use rustls::pki_types::CertificateRevocationListDer;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};
use time::OffsetDateTime;
use tokio::sync::Notify;
use uptrakit_shared_db::entity::{agent_certificate, prelude::*};

use crate::pki;

/// Configuration for the CRL manager.
pub struct CrlManagerConfig {
    pub ca_cert_pem: String,
    pub ca_key_pem: String,
    pub server_cert_pem: String,
    pub server_key_pem: String,
    pub db: DatabaseConnection,
    pub rustls_config: axum_server::tls_rustls::RustlsConfig,
    pub revocation_notify: Arc<Notify>,
}

/// CRL lifecycle manager.
///
/// Builds CRLs from the database and hot-reloads the TLS configuration
/// so that `WebPkiClientVerifier` rejects revoked client certificates.
pub struct CrlManager {
    config: CrlManagerConfig,
    crl_number: AtomicU64,
    ca_issuer: Issuer<'static, KeyPair>,
}

/// Build a DER-encoded CRL from the database (standalone, for initial startup).
pub async fn build_initial_crl_der(
    db: &DatabaseConnection,
    ca_cert_pem: &str,
    ca_key_pem: &str,
) -> pki::Result<CertificateRevocationListDer<'static>> {
    let ca_key = KeyPair::from_pem(ca_key_pem).context_to::<pki::PkiError>()?;
    let ca_issuer = Issuer::from_ca_cert_pem(ca_cert_pem, ca_key).context_to::<pki::PkiError>()?;

    let revoked = query_revoked_certs(db).await?;
    sign_crl(&ca_issuer, revoked, 0)
}

impl CrlManager {
    pub fn new(config: CrlManagerConfig) -> pki::Result<Self> {
        let ca_key = KeyPair::from_pem(&config.ca_key_pem).context_to::<pki::PkiError>()?;
        let ca_issuer =
            Issuer::from_ca_cert_pem(&config.ca_cert_pem, ca_key).context_to::<pki::PkiError>()?;

        Ok(Self {
            config,
            crl_number: AtomicU64::new(1),
            ca_issuer,
        })
    }

    /// Build a DER-encoded CRL from revoked certificates in the database.
    async fn build_crl_der(&self) -> pki::Result<CertificateRevocationListDer<'static>> {
        let revoked = query_revoked_certs(&self.config.db).await?;
        let crl_number = self.crl_number.fetch_add(1, Ordering::Relaxed);
        sign_crl(&self.ca_issuer, revoked, crl_number)
    }

    /// Rebuild the CRL and hot-reload the TLS configuration.
    pub async fn reload_tls_config(&self) -> pki::Result<()> {
        let crl_der = self.build_crl_der().await?;

        let server_config = pki::build_rustls_config_with_client_auth_and_crl(
            &self.config.server_cert_pem,
            &self.config.server_key_pem,
            &self.config.ca_cert_pem,
            crl_der,
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

/// Query revoked certificates from the database.
async fn query_revoked_certs(db: &DatabaseConnection) -> pki::Result<Vec<RevokedCertParams>> {
    let now = OffsetDateTime::now_utc();
    let grace = time::Duration::hours(24);

    let revoked_certs = AgentCertificate::find()
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

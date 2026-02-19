use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};

use rcgen::{
    CertificateRevocationListParams, Issuer, KeyIdMethod, KeyPair, RevokedCertParams, SerialNumber,
};
use rustls::pki_types::CertificateRevocationListDer;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};
use time::OffsetDateTime;
use tokio::sync::{Notify, RwLock};
use uptrakit_shared_db::entity::{prelude::*, service_certificate};

use crate::pki::{self, CaSnapshot};

/// Configuration for the CRL manager.
pub struct CrlManagerConfig {
    pub server_cert_pem: String,
    pub server_key_pem: String,
    pub db: DatabaseConnection,
    pub rustls_config: axum_server::tls_rustls::RustlsConfig,
    pub revocation_notify: Arc<Notify>,
    pub crl_pem_cache: Arc<tokio::sync::RwLock<String>>,
    pub default_tenant_id: uuid::Uuid,
    pub initial_revocation_version: i64,
}

/// Mutable CA material that can be updated at runtime when the CA rotates.
struct TrustedIssuer {
    issuer: Issuer<'static, KeyPair>,
    fingerprint: String,
}

struct CaIssuers {
    trusted: Vec<TrustedIssuer>,
    bundle_pem: String,
}

/// CRL lifecycle manager.
///
/// Builds CRLs from the database and hot-reloads the TLS configuration
/// so that `WebPkiClientVerifier` rejects revoked client certificates.
pub struct CrlManager {
    config: CrlManagerConfig,
    crl_number: AtomicU64,
    cached_revocation_version: AtomicI64,
    issuers: RwLock<CaIssuers>,
    server_cert: RwLock<(String, String)>,
}

/// Build DER-encoded CRLs and combined PEM from the database (standalone, for initial startup).
pub async fn build_initial_crls(
    db: &DatabaseConnection,
    snapshot: &CaSnapshot,
    key_store: &pki::CaKeyStore,
) -> pki::Result<(Vec<CertificateRevocationListDer<'static>>, String)> {
    if snapshot.trusted_cas.is_empty() {
        bail!(pki::PkiError::CaValidation(
            "no trusted CA material available".into()
        ));
    }

    let mut crls = Vec::new();
    let mut combined_pem = String::new();

    for ca in &snapshot.trusted_cas {
        let ca_key = key_store
            .trusted_ca_keys
            .iter()
            .find(|k| k.fingerprint == ca.fingerprint)
            .ok_or_else(|| {
                report!(pki::PkiError::CaValidation(format!(
                    "missing key for CA fingerprint {}",
                    ca.fingerprint
                )))
            })?;
        let key = KeyPair::from_pem(&ca_key.key_pem).context_to::<pki::PkiError>()?;
        let issuer = Issuer::from_ca_cert_pem(&ca.cert_pem, key).context_to::<pki::PkiError>()?;
        let revoked = query_revoked_certs_for_ca(db, &ca.fingerprint).await?;
        let (crl, pem) = sign_crl(&issuer, revoked, 0)?;
        crls.push(crl);
        combined_pem.push_str(&pem);
    }

    Ok((crls, combined_pem))
}

impl CrlManager {
    pub fn new(
        config: CrlManagerConfig,
        snapshot: &CaSnapshot,
        key_store: &pki::CaKeyStore,
    ) -> pki::Result<Self> {
        if snapshot.trusted_cas.is_empty() {
            bail!(pki::PkiError::CaValidation(
                "no trusted CA material available".into()
            ));
        }

        let mut trusted = Vec::new();
        for ca in &snapshot.trusted_cas {
            let ca_key = key_store
                .trusted_ca_keys
                .iter()
                .find(|k| k.fingerprint == ca.fingerprint)
                .ok_or_else(|| {
                    report!(pki::PkiError::CaValidation(format!(
                        "missing key for CA fingerprint {}",
                        ca.fingerprint
                    )))
                })?;
            let key = KeyPair::from_pem(&ca_key.key_pem).context_to::<pki::PkiError>()?;
            let issuer =
                Issuer::from_ca_cert_pem(&ca.cert_pem, key).context_to::<pki::PkiError>()?;
            trusted.push(TrustedIssuer {
                issuer,
                fingerprint: ca.fingerprint.clone(),
            });
        }

        let server_cert_pem = config.server_cert_pem.clone();
        let server_key_pem = config.server_key_pem.clone();
        let initial_revocation_version = config.initial_revocation_version;

        Ok(Self {
            config,
            crl_number: AtomicU64::new(1),
            cached_revocation_version: AtomicI64::new(initial_revocation_version),
            issuers: RwLock::new(CaIssuers {
                trusted,
                bundle_pem: snapshot.bundle_pem.clone(),
            }),
            server_cert: RwLock::new((server_cert_pem, server_key_pem)),
        })
    }

    /// Update CA issuers after a rotation event.
    pub async fn update_ca(
        &self,
        snapshot: &CaSnapshot,
        key_store: &pki::CaKeyStore,
    ) -> pki::Result<()> {
        if snapshot.trusted_cas.is_empty() {
            bail!(pki::PkiError::CaValidation(
                "no trusted CA material available".into()
            ));
        }

        let mut trusted = Vec::new();
        for ca in &snapshot.trusted_cas {
            let ca_key = key_store
                .trusted_ca_keys
                .iter()
                .find(|k| k.fingerprint == ca.fingerprint)
                .ok_or_else(|| {
                    report!(pki::PkiError::CaValidation(format!(
                        "missing key for CA fingerprint {}",
                        ca.fingerprint
                    )))
                })?;
            let key = KeyPair::from_pem(&ca_key.key_pem).context_to::<pki::PkiError>()?;
            let issuer =
                Issuer::from_ca_cert_pem(&ca.cert_pem, key).context_to::<pki::PkiError>()?;
            trusted.push(TrustedIssuer {
                issuer,
                fingerprint: ca.fingerprint.clone(),
            });
        }

        let mut issuers = self.issuers.write().await;
        issuers.trusted = trusted;
        issuers.bundle_pem = snapshot.bundle_pem.clone();

        Ok(())
    }

    /// Update server cert material (after renewal).
    pub async fn update_server_cert(&self, cert_pem: String, key_pem: String) {
        let mut cert = self.server_cert.write().await;
        *cert = (cert_pem, key_pem);
    }

    /// Build DER-encoded CRLs and combined PEM from revoked certificates in the database.
    async fn build_crls(
        &self,
    ) -> pki::Result<(Vec<CertificateRevocationListDer<'static>>, String)> {
        let issuers = self.issuers.read().await;
        let crl_number = self.crl_number.fetch_add(1, Ordering::Relaxed);

        let mut crls = Vec::new();
        let mut combined_pem = String::new();
        for issuer in &issuers.trusted {
            let revoked = query_revoked_certs_for_ca(&self.config.db, &issuer.fingerprint).await?;
            let (crl, pem) = sign_crl(&issuer.issuer, revoked, crl_number)?;
            crls.push(crl);
            combined_pem.push_str(&pem);
        }

        Ok((crls, combined_pem))
    }

    /// Rebuild the CRLs and hot-reload the TLS configuration.
    pub async fn reload_tls_config(&self) -> pki::Result<()> {
        let (crls, crl_pem) = self.build_crls().await?;
        let issuers = self.issuers.read().await;
        let server_cert = self.server_cert.read().await;

        let server_config = pki::build_rustls_config_with_client_auth_and_crls(
            &server_cert.0,
            &server_cert.1,
            &issuers.bundle_pem,
            crls,
        )?;

        self.config
            .rustls_config
            .reload_from_config(Arc::new(server_config));

        // Update the CRL PEM cache for the HTTP endpoint
        *self.config.crl_pem_cache.write().await = crl_pem;

        tracing::info!("TLS configuration reloaded with updated CRL");
        Ok(())
    }

    /// Background task: rebuilds CRL on revocation events or version-gated periodic poll.
    ///
    /// Uses a 60-second poll to check the `revocation_version` counter in the database.
    /// If the version is unchanged, the CRL rebuild is skipped. This enables cross-instance
    /// revocation propagation in multi-instance deployments while keeping the local `Notify`
    /// for instant same-instance rebuilds.
    ///
    /// Accepts an optional `CancellationToken` for graceful shutdown. When the
    /// token is cancelled, the task exits cleanly.
    pub async fn run(self: Arc<Self>, shutdown_token: Option<tokio_util::sync::CancellationToken>) {
        let mut interval = tokio::time::interval(crate::durations::CRL_POLL_INTERVAL);
        // The first tick completes immediately — skip it since we already
        // built the initial CRL synchronously before starting.
        interval.tick().await;

        loop {
            tokio::select! {
                _ = interval.tick() => {
                    // Check DB version — only rebuild if changed
                    match uptrakit_web_api::settings_store::get_revocation_version(
                        &self.config.db, self.config.default_tenant_id,
                    ).await {
                        Ok(db_ver) => {
                            let cached = self.cached_revocation_version.load(Ordering::Relaxed);
                            if db_ver == cached {
                                tracing::debug!("revocation version unchanged, skipping CRL rebuild");
                                continue;
                            }
                            tracing::info!(cached, db_ver, "revocation version changed, rebuilding CRL");
                            self.cached_revocation_version.store(db_ver, Ordering::Release);
                        }
                        Err(e) => {
                            tracing::warn!(error = ?e, "failed to check revocation version, forcing CRL rebuild");
                        }
                    }
                }
                _ = self.config.revocation_notify.notified() => {
                    tracing::debug!("CRL rebuild triggered by local revocation event");
                    // Optimistic version bump to avoid redundant rebuild on next poll
                    self.cached_revocation_version.fetch_add(1, Ordering::Release);
                }
                _ = async {
                    if let Some(ref token) = shutdown_token {
                        token.cancelled().await
                    } else {
                        std::future::pending::<()>().await
                    }
                } => {
                    tracing::debug!("CRL manager shutting down");
                    return;
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

    let revoked_certs = ServiceCertificate::find()
        .filter(service_certificate::Column::CaFingerprint.eq(ca_fingerprint))
        .filter(service_certificate::Column::RevokedAt.is_not_null())
        .filter(service_certificate::Column::NotAfter.gt(now - grace))
        .all(db)
        .await
        .context_to::<pki::PkiError>()?;

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

/// Sign a CRL with the given issuer and return both DER bytes and PEM string.
fn sign_crl(
    ca_issuer: &Issuer<'_, KeyPair>,
    revoked_certs: Vec<RevokedCertParams>,
    crl_number: u64,
) -> pki::Result<(CertificateRevocationListDer<'static>, String)> {
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
    let pem = crl.pem().context_to::<pki::PkiError>()?;
    let der = CertificateRevocationListDer::from(crl.der().to_vec());

    Ok((der, pem))
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
        RevocationReason::ServiceDeactivated => rcgen::RevocationReason::CessationOfOperation,
        RevocationReason::ServiceMerged => rcgen::RevocationReason::CessationOfOperation,
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
            map_reason(RevocationReason::ServiceDeactivated) as u8,
            rcgen::RevocationReason::CessationOfOperation as u8
        );
        assert_eq!(
            map_reason(RevocationReason::ServiceMerged) as u8,
            rcgen::RevocationReason::CessationOfOperation as u8
        );
    }

    /// Generate a self-signed CA certificate and key pair for testing.
    fn generate_test_ca_issuer() -> (String, rcgen::KeyPair) {
        let key_pair =
            rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).expect("keygen");
        let mut params = rcgen::CertificateParams::default();
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, "Test CA");
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let cert = params.self_signed(&key_pair).expect("self-sign");
        (cert.pem(), key_pair)
    }

    #[test]
    fn sign_crl_empty_revoked_list() {
        let (cert_pem, key_pair) = generate_test_ca_issuer();
        let issuer =
            Issuer::from_ca_cert_pem(&cert_pem, key_pair).expect("creating test CA issuer");

        let (der, pem) =
            sign_crl(&issuer, vec![], 1).expect("signing CRL with empty revoked list");

        assert!(!der.is_empty(), "DER output should be non-empty");
        assert!(
            pem.starts_with("-----BEGIN X509 CRL-----"),
            "PEM should start with X509 CRL header, got: {}",
            &pem[..pem.len().min(40)]
        );
    }

    #[test]
    fn sign_crl_with_revoked_certs() {
        let (cert_pem, key_pair) = generate_test_ca_issuer();
        let issuer =
            Issuer::from_ca_cert_pem(&cert_pem, key_pair).expect("creating test CA issuer");

        let now = OffsetDateTime::now_utc();
        let revoked = vec![
            RevokedCertParams {
                serial_number: SerialNumber::from_slice(&[0x00, 0xAB, 0xCD]),
                revocation_time: now,
                reason_code: Some(rcgen::RevocationReason::KeyCompromise),
                invalidity_date: None,
            },
            RevokedCertParams {
                serial_number: SerialNumber::from_slice(&[0x01, 0x02, 0x03]),
                revocation_time: now,
                reason_code: Some(rcgen::RevocationReason::Superseded),
                invalidity_date: None,
            },
        ];

        let (der, pem) =
            sign_crl(&issuer, revoked, 42).expect("signing CRL with revoked certificates");

        assert!(!der.is_empty(), "DER output should be non-empty");
        assert!(
            pem.starts_with("-----BEGIN X509 CRL-----"),
            "PEM should start with X509 CRL header"
        );
    }

    #[test]
    fn sign_crl_increments_crl_number() {
        let (cert_pem, key_pair) = generate_test_ca_issuer();
        let issuer =
            Issuer::from_ca_cert_pem(&cert_pem, key_pair).expect("creating test CA issuer");

        let (der_1, pem_1) =
            sign_crl(&issuer, vec![], 1).expect("signing first CRL");
        let (der_2, pem_2) =
            sign_crl(&issuer, vec![], 2).expect("signing second CRL");

        assert!(!der_1.is_empty(), "first CRL DER should be non-empty");
        assert!(!der_2.is_empty(), "second CRL DER should be non-empty");
        assert!(
            pem_1.starts_with("-----BEGIN X509 CRL-----"),
            "first CRL PEM should have correct header"
        );
        assert!(
            pem_2.starts_with("-----BEGIN X509 CRL-----"),
            "second CRL PEM should have correct header"
        );
        // Different CRL numbers should produce different DER outputs.
        assert_ne!(
            der_1.as_ref(),
            der_2.as_ref(),
            "CRLs with different numbers should produce different DER"
        );
    }

    #[test]
    fn parse_serial_string_empty_string_returns_none() {
        // An empty string splits into a single empty segment, which cannot be
        // parsed as a hex byte, so the result should be `None`.
        assert_eq!(parse_serial_string(""), None);
    }
}

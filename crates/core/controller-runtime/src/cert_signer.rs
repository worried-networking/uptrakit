use std::sync::Arc;

use rcgen::{
    CertificateParams, CertificateSigningRequestParams, DnType, ExtendedKeyUsagePurpose, IsCa,
    Issuer, KeyPair,
};
use rootcause::prelude::*;
use time::{OffsetDateTime, UtcDateTime};
use tokio::sync::watch;
use uptrakit_web_api::cert_signer::{AgentCertSigner, CertSignerError, SignedCertBundle};
use uuid::Uuid;

use crate::pki::CaSnapshot;

/// Abstraction over the cached `Arc<Issuer>` store so that tests can inject a
/// simple `HashMap`-backed stub without pulling in the full `CrlManager`
/// (which requires a live `DatabaseConnection`).
#[async_trait::async_trait]
pub(crate) trait IssuerSource: Send + Sync {
    async fn issuer_for(&self, fingerprint: &str) -> Option<Arc<Issuer<'static, KeyPair>>>;
}

#[async_trait::async_trait]
impl IssuerSource for crate::crl_manager::CrlManager {
    async fn issuer_for(&self, fingerprint: &str) -> Option<Arc<Issuer<'static, KeyPair>>> {
        self.issuer_for(fingerprint).await
    }
}

pub(crate) struct RcgenAgentCertSigner {
    ca_rx: watch::Receiver<CaSnapshot>,
    issuer_source: Arc<dyn IssuerSource>,
    trust_domain: Option<String>,
}

/// Maximum certificate lifetime: 2 years (730 days).
const MAX_CERT_LIFETIME: time::Duration = time::Duration::days(730);

impl RcgenAgentCertSigner {
    pub(crate) fn new(
        ca_rx: watch::Receiver<CaSnapshot>,
        issuer_source: Arc<dyn IssuerSource>,
    ) -> Self {
        Self {
            ca_rx,
            issuer_source,
            trust_domain: None,
        }
    }

    pub(crate) fn with_trust_domain(mut self, domain: String) -> Self {
        self.trust_domain = if domain.is_empty() {
            None
        } else {
            Some(domain)
        };
        self
    }
}

#[async_trait::async_trait]
impl AgentCertSigner for RcgenAgentCertSigner {
    async fn sign_agent_csr(
        &self,
        csr_pem: &str,
        agent_id: &Uuid,
        lifetime: time::Duration,
    ) -> std::result::Result<SignedCertBundle, Report<CertSignerError>> {
        let snapshot = self.ca_rx.borrow().clone();
        let issuer = self
            .issuer_source
            .issuer_for(&snapshot.active_fingerprint)
            .await
            .ok_or_else(|| {
                report!(CertSignerError::CaKeyParse(format!(
                    "no cached issuer for fingerprint {}",
                    snapshot.active_fingerprint
                )))
            })?;
        sign_agent_csr(
            csr_pem,
            &issuer,
            agent_id,
            lifetime,
            snapshot.active_not_after,
            snapshot.pki_addr.as_deref(),
            self.trust_domain.as_deref(),
        )
    }

    fn active_ca_fingerprint(&self) -> String {
        self.ca_rx.borrow().active_fingerprint.clone()
    }
}

fn sign_agent_csr(
    csr_pem: &str,
    issuer: &Issuer<'_, KeyPair>,
    agent_id: &Uuid,
    lifetime: time::Duration,
    ca_not_after: OffsetDateTime,
    pki_addr: Option<&str>,
    trust_domain: Option<&str>,
) -> std::result::Result<SignedCertBundle, Report<CertSignerError>> {
    // Parse and validate CSR signature
    let csr_params = CertificateSigningRequestParams::from_pem(csr_pem)
        .map_err(|e| report!(CertSignerError::CsrValidation(format!("invalid CSR: {e}"))))?;

    // Extract CN from CSR and verify it matches agent_id
    let csr_cn = csr_params
        .params
        .distinguished_name
        .iter()
        .find_map(|(dn_type, value)| {
            if dn_type == &DnType::CommonName {
                match value {
                    rcgen::DnValue::Utf8String(s) => Some(s.clone()),
                    rcgen::DnValue::PrintableString(s) => Some(s.to_string()),
                    rcgen::DnValue::TeletexString(s) => Some(s.to_string()),
                    rcgen::DnValue::Ia5String(s) => Some(s.to_string()),
                    // UniversalString and BmpString are raw byte-based; not expected in agent CSRs
                    _ => None,
                }
            } else {
                None
            }
        })
        .ok_or_else(|| {
            report!(CertSignerError::CsrValidation(
                "CSR has no CommonName".to_string()
            ))
        })?;

    if csr_cn != agent_id.to_string() {
        bail!(CertSignerError::CsrValidation(format!(
            "CSR CN '{csr_cn}' does not match agent_id '{agent_id}'"
        )));
    }

    // If a trust domain is configured, validate the SPIFFE URI SAN (if present).
    if let Some(expected_domain) = trust_domain {
        // Find SPIFFE URI SAN in CSR params.
        let spiffe_uri = csr_params.params.subject_alt_names.iter().find_map(|san| {
            if let rcgen::SanType::URI(uri) = san
                && uri.as_str().starts_with("spiffe://")
            {
                return Some(uri.as_str().to_owned());
            }
            None
        });

        if let Some(uri) = spiffe_uri {
            let parsed = url::Url::parse(&uri)
                .map_err(|e| report!(CertSignerError::CsrSpiffeParse(e.to_string())))?;
            let actual_domain = parsed.host_str().ok_or_else(|| {
                report!(CertSignerError::CsrSpiffeParse(
                    "SPIFFE URI has no host".to_string()
                ))
            })?;
            if actual_domain != expected_domain {
                bail!(CertSignerError::CsrTrustDomainMismatch {
                    expected: expected_domain.to_owned(),
                    actual: actual_domain.to_owned(),
                });
            }
            let segments: Vec<&str> = parsed
                .path_segments()
                .map(Iterator::collect)
                .unwrap_or_default();
            let (kind, id_str) = match (segments.first(), segments.get(1)) {
                (Some(k), Some(i)) => (*k, *i),
                _ => bail!(CertSignerError::CsrSpiffePath(uri)),
            };
            if segments.len() != 2 || kind != "service" {
                bail!(CertSignerError::CsrSpiffePath(uri));
            }
            let csr_service_id: uuid::Uuid = id_str.parse().map_err(|e: uuid::Error| {
                report!(CertSignerError::CsrServiceIdParse(e.to_string()))
            })?;
            if csr_service_id != *agent_id {
                bail!(CertSignerError::CsrServiceIdMismatch {
                    expected: agent_id.to_string(),
                    actual: csr_service_id.to_string(),
                });
            }
        }
        // No SPIFFE SAN: legacy CSR, accepted during migration tail.
    }

    // Build new CertificateParams with controller-controlled values
    let capped = lifetime.min(MAX_CERT_LIFETIME);
    if lifetime > MAX_CERT_LIFETIME {
        tracing::warn!(
            agent_id = %agent_id,
            requested_hours = lifetime.whole_hours(),
            capped_hours = MAX_CERT_LIFETIME.whole_hours(),
            "Certificate lifetime capped to maximum allowed value"
        );
    }

    let now = OffsetDateTime::now_utc();
    let mut not_after = now + capped;
    if not_after > ca_not_after {
        not_after = ca_not_after;
    }

    if not_after <= now {
        bail!(CertSignerError::Signing(
            "CA certificate is expired or too close to expiry".to_string()
        ));
    }

    let mut params = CertificateParams::default();
    params
        .distinguished_name
        .push(DnType::CommonName, agent_id.to_string());
    params
        .distinguished_name
        .push(DnType::OrganizationName, "Uptrakit Agent");
    params.is_ca = IsCa::NoCa;
    params
        .extended_key_usages
        .push(ExtendedKeyUsagePurpose::ClientAuth);
    params.not_before = now;
    params.not_after = not_after;
    params.subject_alt_names = csr_params.params.subject_alt_names.clone();

    if let Some(url) = pki_addr {
        crate::pki::add_pki_extensions(&mut params, url).map_err(|e| {
            report!(CertSignerError::Signing(format!(
                "failed to add PKI extensions: {e}"
            )))
        })?;
    }

    // Sign using the public key from the CSR
    let cert = params
        .signed_by(&csr_params.public_key, issuer)
        .map_err(|e| report!(CertSignerError::Signing(e.to_string())))?;

    Ok(SignedCertBundle {
        cert_pem: cert.pem(),
        not_after: UtcDateTime::from(not_after),
    })
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::let_underscore_must_use,
        reason = "test code: discarding `install_default` result is idiomatic — it returns `Err` once the provider is already installed"
    )]

    use std::collections::HashMap;

    use super::*;
    use crate::pki;

    /// Minimal `IssuerSource` backed by an in-memory map — avoids the need for
    /// a live `DatabaseConnection` in unit tests.
    struct MapIssuerSource {
        map: HashMap<String, Arc<Issuer<'static, KeyPair>>>,
    }

    #[async_trait::async_trait]
    impl IssuerSource for MapIssuerSource {
        async fn issuer_for(&self, fingerprint: &str) -> Option<Arc<Issuer<'static, KeyPair>>> {
            self.map.get(fingerprint).cloned()
        }
    }

    fn make_test_signer() -> (RcgenAgentCertSigner, watch::Sender<CaSnapshot>) {
        let key_pair = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
        let mut params = CertificateParams::default();
        params
            .distinguished_name
            .push(DnType::CommonName, "Test CA");
        params
            .distinguished_name
            .push(DnType::OrganizationName, "Uptrakit");
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let cert = params.self_signed(&key_pair).unwrap();
        let cert_pem = cert.pem();
        let key_pem = key_pair.serialize_pem();

        let ca = pki::bundle_from_pem(cert_pem.clone(), key_pem.clone()).unwrap();
        let state = pki::CaState {
            active: pki::bundle_from_pem(cert_pem.clone(), key_pem.clone()).unwrap(),
            previous: None,
            trusted: vec![ca],
            managed: true,
        };
        let (snapshot, _key_store) = state.to_snapshot(None).unwrap();

        let fingerprint = snapshot.active_fingerprint.clone();
        let key_pair2 = KeyPair::from_pem(&key_pem).unwrap();
        let issuer = Arc::new(Issuer::from_ca_cert_pem(&cert_pem, key_pair2).unwrap());

        let mut map = HashMap::new();
        map.insert(fingerprint, issuer);
        let issuer_source: Arc<dyn IssuerSource> = Arc::new(MapIssuerSource { map });

        let (tx, rx) = watch::channel(snapshot);
        (RcgenAgentCertSigner::new(rx, issuer_source), tx)
    }

    /// Generate a test CSR with the given CN using rcgen.
    fn generate_test_csr(cn: &str) -> String {
        let key_pair = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
        let mut params = CertificateParams::default();
        params
            .distinguished_name
            .push(DnType::CommonName, cn.to_string());
        params
            .distinguished_name
            .push(DnType::OrganizationName, "Uptrakit Agent");
        let csr = params.serialize_request(&key_pair).unwrap();
        csr.pem().unwrap()
    }

    /// Generate a test CSR with a SPIFFE URI SAN.
    fn generate_test_csr_with_spiffe(service_id: uuid::Uuid, trust_domain: &str) -> String {
        let key_pair = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
        let mut params = CertificateParams::default();
        params
            .distinguished_name
            .push(DnType::CommonName, service_id.to_string());
        let spiffe_uri = format!("spiffe://{trust_domain}/service/{service_id}");
        let ia5: rcgen::SanType = rcgen::SanType::URI(spiffe_uri.as_str().try_into().unwrap());
        params.subject_alt_names.push(ia5);
        let csr = params.serialize_request(&key_pair).unwrap();
        csr.pem().unwrap()
    }

    /// Create a signer with trust domain validation enabled.
    fn make_test_signer_with_trust_domain(
        domain: &str,
    ) -> (RcgenAgentCertSigner, watch::Sender<CaSnapshot>) {
        let (signer, tx) = make_test_signer();
        (signer.with_trust_domain(domain.to_owned()), tx)
    }

    /// Extract the Common Name from a PEM-encoded certificate (test helper).
    fn extract_cn_from_cert_pem(cert_pem: &str) -> Option<String> {
        use const_oid::db::rfc4519::COMMON_NAME;
        use der::{
            DecodePem,
            asn1::{PrintableStringRef, Utf8StringRef},
        };
        use x509_cert::Certificate;
        let cert = Certificate::from_pem(cert_pem.as_bytes()).ok()?;
        cert.tbs_certificate
            .subject
            .0
            .iter()
            .flat_map(|rdn| rdn.0.iter())
            .filter(|atv| atv.oid == COMMON_NAME)
            .find_map(|atv| {
                atv.value
                    .decode_as::<Utf8StringRef<'_>>()
                    .map(|s| s.as_str().to_owned())
                    .ok()
                    .or_else(|| {
                        atv.value
                            .decode_as::<PrintableStringRef<'_>>()
                            .map(|s| s.as_str().to_owned())
                            .ok()
                    })
            })
    }

    /// Check if the certificate has the ClientAuth EKU (test helper).
    fn cert_has_client_auth_eku(cert_pem: &str) -> bool {
        use const_oid::db::rfc5280::{ID_CE_EXT_KEY_USAGE, ID_KP_CLIENT_AUTH};
        use der::{Decode, DecodePem};
        use x509_cert::Certificate;
        use x509_cert::ext::pkix::ExtendedKeyUsage;
        let Ok(cert) = Certificate::from_pem(cert_pem.as_bytes()) else {
            return false;
        };
        let Some(exts) = cert.tbs_certificate.extensions.as_deref() else {
            return false;
        };
        for ext in exts {
            if ext.extn_id == ID_CE_EXT_KEY_USAGE
                && let Ok(eku) = ExtendedKeyUsage::from_der(ext.extn_value.as_bytes())
            {
                return eku.0.contains(&ID_KP_CLIENT_AUTH);
            }
        }
        false
    }

    /// Check if the certificate has the CA basic constraint set (test helper).
    fn cert_is_ca(cert_pem: &str) -> bool {
        use const_oid::db::rfc5280::ID_CE_BASIC_CONSTRAINTS;
        use der::{Decode, DecodePem};
        use x509_cert::Certificate;
        use x509_cert::ext::pkix::constraints::BasicConstraints;
        let Ok(cert) = Certificate::from_pem(cert_pem.as_bytes()) else {
            return false;
        };
        let Some(exts) = cert.tbs_certificate.extensions.as_deref() else {
            return false;
        };
        for ext in exts {
            if ext.extn_id == ID_CE_BASIC_CONSTRAINTS
                && let Ok(bc) = BasicConstraints::from_der(ext.extn_value.as_bytes())
            {
                return bc.ca;
            }
        }
        false
    }

    #[tokio::test]
    async fn agent_csr_signed_by_ca() {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

        let (signer, _tx) = make_test_signer();

        let agent_id = Uuid::now_v7();
        let csr_pem = generate_test_csr(&agent_id.to_string());
        let bundle = signer
            .sign_agent_csr(&csr_pem, &agent_id, time::Duration::hours(168))
            .await
            .unwrap();

        assert!(bundle.cert_pem.contains("BEGIN CERTIFICATE"));
        // not_after should be ~168 hours (7 days) from now
        let now = UtcDateTime::now();
        assert!(bundle.not_after > now + time::Duration::hours(166));
        assert!(bundle.not_after < now + time::Duration::hours(170));

        // Verify CN contains the agent UUID
        let cn = extract_cn_from_cert_pem(&bundle.cert_pem).expect("parse CN");
        assert_eq!(cn, agent_id.to_string());

        // Verify EKU includes ClientAuth
        assert!(
            cert_has_client_auth_eku(&bundle.cert_pem),
            "EKU must include clientAuth"
        );

        // Verify it's not a CA
        assert!(!cert_is_ca(&bundle.cert_pem), "cert must not be a CA");
    }

    #[tokio::test]
    async fn agent_csr_lifetime_is_capped() {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

        let (signer, _tx) = make_test_signer();
        let agent_id = Uuid::now_v7();
        let csr_pem = generate_test_csr(&agent_id.to_string());

        let bundle = signer
            .sign_agent_csr(&csr_pem, &agent_id, time::Duration::hours(24_000))
            .await
            .unwrap();

        let now = UtcDateTime::now();
        assert!(bundle.not_after > now + time::Duration::days(728));
        assert!(bundle.not_after < now + time::Duration::days(732));
    }

    #[tokio::test]
    async fn cn_parses_as_uuid() {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

        let (signer, _tx) = make_test_signer();

        let agent_id = Uuid::now_v7();
        let csr_pem = generate_test_csr(&agent_id.to_string());
        let bundle = signer
            .sign_agent_csr(&csr_pem, &agent_id, time::Duration::hours(720))
            .await
            .unwrap();

        let cn = extract_cn_from_cert_pem(&bundle.cert_pem).expect("parse CN");
        let parsed = Uuid::parse_str(&cn).unwrap();
        assert_eq!(parsed, agent_id);
    }

    #[tokio::test]
    async fn csr_cn_mismatch_rejected() {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

        let (signer, _tx) = make_test_signer();

        let agent_id = Uuid::now_v7();
        let wrong_id = Uuid::now_v7();
        let csr_pem = generate_test_csr(&wrong_id.to_string());

        let result = signer
            .sign_agent_csr(&csr_pem, &agent_id, time::Duration::hours(168))
            .await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        let msg = err.current_context().to_string();
        assert!(
            msg.contains("does not match"),
            "expected CN mismatch error, got: {msg}"
        );
    }

    #[tokio::test]
    async fn invalid_csr_rejected() {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

        let (signer, _tx) = make_test_signer();

        let agent_id = Uuid::now_v7();
        let result = signer
            .sign_agent_csr("not-a-csr", &agent_id, time::Duration::hours(168))
            .await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        let msg = err.current_context().to_string();
        assert!(
            msg.contains("invalid CSR"),
            "expected invalid CSR error, got: {msg}"
        );
    }

    #[test]
    fn active_ca_fingerprint_returns_value() {
        let (signer, _tx) = make_test_signer();
        let fp = signer.active_ca_fingerprint();
        assert_eq!(fp.len(), 64);
    }

    #[tokio::test]
    async fn spiffe_san_matching_trust_domain_accepted() {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

        let (signer, _tx) = make_test_signer_with_trust_domain("example.org");
        let agent_id = Uuid::now_v7();
        let csr_pem = generate_test_csr_with_spiffe(agent_id, "example.org");

        let result = signer
            .sign_agent_csr(&csr_pem, &agent_id, time::Duration::hours(168))
            .await;
        assert!(result.is_ok(), "expected Ok, got: {result:?}");
    }

    #[tokio::test]
    async fn spiffe_san_wrong_trust_domain_rejected() {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

        let (signer, _tx) = make_test_signer_with_trust_domain("example.org");
        let agent_id = Uuid::now_v7();
        let csr_pem = generate_test_csr_with_spiffe(agent_id, "evil.com");

        let result = signer
            .sign_agent_csr(&csr_pem, &agent_id, time::Duration::hours(168))
            .await;
        assert!(result.is_err());
        let msg = result.unwrap_err().current_context().to_string();
        assert!(
            msg.contains("trust-domain mismatch"),
            "expected trust-domain mismatch error, got: {msg}"
        );
    }

    #[tokio::test]
    async fn spiffe_san_wrong_service_id_rejected() {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

        let (signer, _tx) = make_test_signer_with_trust_domain("example.org");
        let agent_id = Uuid::now_v7();
        let other_id = Uuid::now_v7();
        // CSR CN matches agent_id but SPIFFE SAN refers to other_id.
        let key_pair = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
        let mut params = CertificateParams::default();
        params
            .distinguished_name
            .push(DnType::CommonName, agent_id.to_string());
        let other_spiffe = format!("spiffe://example.org/service/{other_id}");
        params.subject_alt_names.push(rcgen::SanType::URI(
            other_spiffe.as_str().try_into().unwrap(),
        ));
        let csr_pem = params.serialize_request(&key_pair).unwrap().pem().unwrap();

        let result = signer
            .sign_agent_csr(&csr_pem, &agent_id, time::Duration::hours(168))
            .await;
        assert!(result.is_err());
        let msg = result.unwrap_err().current_context().to_string();
        assert!(
            msg.contains("service-id mismatch"),
            "expected service-id mismatch error, got: {msg}"
        );
    }

    #[tokio::test]
    async fn csr_without_spiffe_san_accepted_when_trust_domain_configured() {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

        let (signer, _tx) = make_test_signer_with_trust_domain("example.org");
        let agent_id = Uuid::now_v7();
        // Legacy CSR — no SPIFFE SAN at all.
        let csr_pem = generate_test_csr(&agent_id.to_string());

        let result = signer
            .sign_agent_csr(&csr_pem, &agent_id, time::Duration::hours(168))
            .await;
        assert!(
            result.is_ok(),
            "legacy CSR without SPIFFE SAN must be accepted, got: {result:?}"
        );
    }
}

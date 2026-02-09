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

pub struct RcgenAgentCertSigner {
    ca_rx: watch::Receiver<CaSnapshot>,
    ca_key_store: uptrakit_web_api::CaKeyStoreRef,
}

/// Maximum certificate lifetime: 2 years (730 days).
const MAX_CERT_LIFETIME: time::Duration = time::Duration::days(730);

impl RcgenAgentCertSigner {
    pub fn new(
        ca_rx: watch::Receiver<CaSnapshot>,
        ca_key_store: uptrakit_web_api::CaKeyStoreRef,
    ) -> Self {
        Self {
            ca_rx,
            ca_key_store,
        }
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
        let key_pem = {
            let key_store = self.ca_key_store.read().await;
            key_store.active_key_pem.clone()
        };
        let key_pair = KeyPair::from_pem(&key_pem)
            .map_err(|e| report!(CertSignerError::CaKeyParse(e.to_string())))?;
        let issuer = Issuer::from_ca_cert_pem(&snapshot.active_cert_pem, key_pair)
            .map_err(|e| report!(CertSignerError::CaIssuer(e.to_string())))?;
        sign_agent_csr(
            csr_pem,
            &issuer,
            agent_id,
            lifetime,
            snapshot.active_not_after,
            snapshot.pki_addr.as_deref(),
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
        return Err(report!(CertSignerError::CsrValidation(format!(
            "CSR CN '{csr_cn}' does not match agent_id '{agent_id}'"
        ))));
    }

    // Build new CertificateParams with controller-controlled values
    let capped = lifetime.min(MAX_CERT_LIFETIME);
    if lifetime > MAX_CERT_LIFETIME {
        tracing::warn!(
            agent_id = %agent_id,
            requested_days = lifetime.whole_days(),
            capped_days = MAX_CERT_LIFETIME.whole_days(),
            "Certificate lifetime capped to maximum allowed value"
        );
    }

    let now = OffsetDateTime::now_utc();
    let mut not_after = now + capped;
    if not_after > ca_not_after {
        not_after = ca_not_after;
    }

    if not_after <= now {
        return Err(report!(CertSignerError::Signing(
            "CA certificate is expired or too close to expiry".to_string()
        )));
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

    if let Some(url) = pki_addr {
        crate::pki::add_pki_extensions(&mut params, url);
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
    use super::*;
    use crate::pki;

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
        let (snapshot, key_store) = state.to_snapshot(None).unwrap();
        let ca_key_store = std::sync::Arc::new(tokio::sync::RwLock::new(key_store));
        let (tx, rx) = watch::channel(snapshot);
        (RcgenAgentCertSigner::new(rx, ca_key_store), tx)
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

    #[tokio::test]
    async fn agent_csr_signed_by_ca() {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

        let (signer, _tx) = make_test_signer();

        let agent_id = Uuid::now_v7();
        let csr_pem = generate_test_csr(&agent_id.to_string());
        let bundle = signer
            .sign_agent_csr(&csr_pem, &agent_id, time::Duration::days(7))
            .await
            .unwrap();

        assert!(bundle.cert_pem.contains("BEGIN CERTIFICATE"));
        // not_after should be ~7 days from now
        let now = UtcDateTime::now();
        assert!(bundle.not_after > now + time::Duration::days(6));
        assert!(bundle.not_after < now + time::Duration::days(8));

        // Verify CN contains the agent UUID
        let (_, pem_block) = x509_parser::pem::parse_x509_pem(bundle.cert_pem.as_bytes()).unwrap();
        let cert = pem_block.parse_x509().unwrap();
        let cn = cert
            .subject()
            .iter_common_name()
            .next()
            .unwrap()
            .as_str()
            .unwrap();
        assert_eq!(cn, agent_id.to_string());

        // Verify EKU includes ClientAuth
        let eku = cert
            .extended_key_usage()
            .expect("EKU extension present")
            .expect("EKU parsed");
        assert!(eku.value.client_auth);

        // Verify it's not a CA
        assert!(!cert.is_ca());
    }

    #[tokio::test]
    async fn agent_csr_lifetime_is_capped() {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

        let (signer, _tx) = make_test_signer();
        let agent_id = Uuid::now_v7();
        let csr_pem = generate_test_csr(&agent_id.to_string());

        let bundle = signer
            .sign_agent_csr(&csr_pem, &agent_id, time::Duration::days(1000))
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
            .sign_agent_csr(&csr_pem, &agent_id, time::Duration::days(30))
            .await
            .unwrap();

        let (_, pem_block) = x509_parser::pem::parse_x509_pem(bundle.cert_pem.as_bytes()).unwrap();
        let cert = pem_block.parse_x509().unwrap();
        let cn = cert
            .subject()
            .iter_common_name()
            .next()
            .unwrap()
            .as_str()
            .unwrap();
        let parsed = Uuid::parse_str(cn).unwrap();
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
            .sign_agent_csr(&csr_pem, &agent_id, time::Duration::days(7))
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
            .sign_agent_csr("not-a-csr", &agent_id, time::Duration::days(7))
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
}

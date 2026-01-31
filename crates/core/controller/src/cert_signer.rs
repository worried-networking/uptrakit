use rcgen::{CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair};
use time::{OffsetDateTime, UtcDateTime};
use tokio::sync::watch;
use uptrakit_web_api::cert_signer::{AgentCertBundle, AgentCertSigner};
use uuid::Uuid;

use crate::pki::CaSnapshot;

pub struct RcgenAgentCertSigner {
    ca_rx: watch::Receiver<CaSnapshot>,
}

impl RcgenAgentCertSigner {
    pub fn new(ca_rx: watch::Receiver<CaSnapshot>) -> Self {
        Self { ca_rx }
    }
}

impl AgentCertSigner for RcgenAgentCertSigner {
    fn sign_agent_cert(
        &self,
        agent_id: &Uuid,
        lifetime: time::Duration,
    ) -> Result<AgentCertBundle, String> {
        let snapshot = self.ca_rx.borrow();
        let key_pair = KeyPair::from_pem(&snapshot.active_key_pem)
            .map_err(|e| format!("CA key parse: {e}"))?;
        let issuer = Issuer::from_ca_cert_pem(&snapshot.active_cert_pem, key_pair)
            .map_err(|e| format!("CA issuer: {e}"))?;
        generate_agent_cert(&issuer, agent_id, lifetime)
    }

    fn active_ca_fingerprint(&self) -> String {
        self.ca_rx.borrow().active_fingerprint.clone()
    }
}

fn generate_agent_cert(
    issuer: &Issuer<'_, KeyPair>,
    agent_id: &Uuid,
    lifetime: time::Duration,
) -> Result<AgentCertBundle, String> {
    let key_pair = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)
        .map_err(|e| format!("key generation: {e}"))?;

    let not_after = OffsetDateTime::now_utc() + lifetime;

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
    params.not_before = OffsetDateTime::now_utc();
    params.not_after = not_after;

    let cert = params
        .signed_by(&key_pair, issuer)
        .map_err(|e| format!("cert signing: {e}"))?;

    Ok(AgentCertBundle {
        cert_pem: cert.pem(),
        key_pem: key_pair.serialize_pem(),
        not_after: UtcDateTime::from(not_after),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pki;

    fn make_test_signer() -> (RcgenAgentCertSigner, watch::Sender<CaSnapshot>) {
        let ca = pki::load_or_generate_ca(tempfile::tempdir().unwrap().path()).unwrap();
        let state = pki::CaState {
            active: ca,
            previous: None,
            managed: true,
        };
        let snapshot = state.to_snapshot().unwrap();
        let (tx, rx) = watch::channel(snapshot);
        (RcgenAgentCertSigner::new(rx), tx)
    }

    #[test]
    fn agent_cert_signed_by_ca() {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

        let (signer, _tx) = make_test_signer();

        let agent_id = Uuid::now_v7();
        let bundle = signer
            .sign_agent_cert(&agent_id, time::Duration::days(7))
            .unwrap();

        assert!(bundle.cert_pem.contains("BEGIN CERTIFICATE"));
        assert!(bundle.key_pem.contains("BEGIN"));
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

    #[test]
    fn cn_parses_as_uuid() {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

        let (signer, _tx) = make_test_signer();

        let agent_id = Uuid::now_v7();
        let bundle = signer
            .sign_agent_cert(&agent_id, time::Duration::days(30))
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

    #[test]
    fn active_ca_fingerprint_returns_value() {
        let (signer, _tx) = make_test_signer();
        let fp = signer.active_ca_fingerprint();
        assert_eq!(fp.len(), 64);
    }
}

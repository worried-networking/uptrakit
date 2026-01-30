use rcgen::{CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair};
use time::OffsetDateTime;
use uptrakit_web_api::cert_signer::{AgentCertBundle, AgentCertSigner};
use uuid::Uuid;

pub struct RcgenAgentCertSigner {
    ca_cert_pem: String,
    ca_key_pem: String,
}

impl RcgenAgentCertSigner {
    pub fn new(ca_cert_pem: String, ca_key_pem: String) -> Self {
        Self {
            ca_cert_pem,
            ca_key_pem,
        }
    }
}

impl AgentCertSigner for RcgenAgentCertSigner {
    fn sign_agent_cert(
        &self,
        agent_id: &Uuid,
        lifetime_days: u16,
    ) -> Result<AgentCertBundle, String> {
        let key_pair =
            KeyPair::from_pem(&self.ca_key_pem).map_err(|e| format!("CA key parse: {e}"))?;
        let issuer = Issuer::from_ca_cert_pem(&self.ca_cert_pem, key_pair)
            .map_err(|e| format!("CA issuer: {e}"))?;
        generate_agent_cert(&issuer, agent_id, lifetime_days)
    }
}

fn generate_agent_cert(
    issuer: &Issuer<'_, KeyPair>,
    agent_id: &Uuid,
    lifetime_days: u16,
) -> Result<AgentCertBundle, String> {
    let key_pair = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)
        .map_err(|e| format!("key generation: {e}"))?;

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
    params.not_after =
        OffsetDateTime::now_utc() + time::Duration::days(i64::from(lifetime_days));

    let cert = params
        .signed_by(&key_pair, issuer)
        .map_err(|e| format!("cert signing: {e}"))?;

    Ok(AgentCertBundle {
        cert_pem: cert.pem(),
        key_pem: key_pair.serialize_pem(),
        lifetime_days,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pki;

    #[test]
    fn agent_cert_signed_by_ca() {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

        let ca = pki::load_or_generate_ca(tempfile::tempdir().unwrap().path()).unwrap();
        let signer = RcgenAgentCertSigner::new(ca.cert_pem, ca.key_pem);

        let agent_id = Uuid::now_v7();
        let bundle = signer.sign_agent_cert(&agent_id, 7).unwrap();

        assert!(bundle.cert_pem.contains("BEGIN CERTIFICATE"));
        assert!(bundle.key_pem.contains("BEGIN"));
        assert_eq!(bundle.lifetime_days, 7);

        // Verify CN contains the agent UUID
        let (_, pem_block) =
            x509_parser::pem::parse_x509_pem(bundle.cert_pem.as_bytes()).unwrap();
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

        let ca = pki::load_or_generate_ca(tempfile::tempdir().unwrap().path()).unwrap();
        let signer = RcgenAgentCertSigner::new(ca.cert_pem, ca.key_pem);

        let agent_id = Uuid::now_v7();
        let bundle = signer.sign_agent_cert(&agent_id, 30).unwrap();

        let (_, pem_block) =
            x509_parser::pem::parse_x509_pem(bundle.cert_pem.as_bytes()).unwrap();
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
}

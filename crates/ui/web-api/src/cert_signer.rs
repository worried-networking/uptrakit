use rootcause::Report;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CertSignerError {
    #[error("CA key parse error: {0}")]
    CaKeyParse(String),

    #[error("CA issuer creation error: {0}")]
    CaIssuer(String),

    #[error("key generation error: {0}")]
    KeyGeneration(String),

    #[error("certificate signing error: {0}")]
    Signing(String),
}

pub struct AgentCertBundle {
    pub cert_pem: String,
    pub key_pem: String,
    /// Certificate "not valid after" timestamp.
    pub not_after: time::UtcDateTime,
}

pub trait AgentCertSigner: Send + Sync + 'static {
    fn sign_agent_cert(
        &self,
        agent_id: &uuid::Uuid,
        lifetime: time::Duration,
    ) -> std::result::Result<AgentCertBundle, Report<CertSignerError>>;

    /// Return the SHA-256 hex fingerprint of the active CA cert.
    fn active_ca_fingerprint(&self) -> String;
}

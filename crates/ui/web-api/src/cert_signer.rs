use rootcause::Report;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CertSignerError {
    #[error("CA key parse error: {0}")]
    CaKeyParse(String),

    #[error("CA issuer creation error: {0}")]
    CaIssuer(String),

    #[error("CSR validation error: {0}")]
    CsrValidation(String),

    #[error("certificate signing error: {0}")]
    Signing(String),
}

pub type Result<T> = std::result::Result<T, Report<CertSignerError>>;

#[derive(Debug)]
pub struct SignedCertBundle {
    pub cert_pem: String,
    /// Certificate "not valid after" timestamp.
    pub not_after: time::UtcDateTime,
}

pub trait AgentCertSigner: Send + Sync + 'static {
    fn sign_agent_csr(
        &self,
        csr_pem: &str,
        agent_id: &uuid::Uuid,
        lifetime: time::Duration,
    ) -> Result<SignedCertBundle>;

    /// Return the SHA-256 hex fingerprint of the active CA cert.
    fn active_ca_fingerprint(&self) -> String;
}

use async_trait::async_trait;
use rootcause::prelude::*;
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

    #[error("SPIFFE URI in CSR cannot be parsed: {0}")]
    CsrSpiffeParse(String),

    #[error("SPIFFE trust-domain mismatch: expected {expected:?}, got {actual:?}")]
    CsrTrustDomainMismatch { expected: String, actual: String },

    #[error("SPIFFE URI has unexpected path format: {0}")]
    CsrSpiffePath(String),

    #[error("SPIFFE service-id in CSR cannot be parsed as UUID: {0}")]
    CsrServiceIdParse(String),

    #[error("SPIFFE service-id mismatch: expected {expected}, got {actual}")]
    CsrServiceIdMismatch { expected: String, actual: String },
}

pub type Result<T> = std::result::Result<T, Report<CertSignerError>>;

#[derive(Debug)]
pub struct SignedCertBundle {
    pub cert_pem: String,
    /// Certificate "not valid after" timestamp.
    pub not_after: time::UtcDateTime,
}

#[async_trait]
pub trait AgentCertSigner: Send + Sync + 'static {
    async fn sign_agent_csr(
        &self,
        csr_pem: &str,
        agent_id: &uuid::Uuid,
        lifetime: time::Duration,
    ) -> Result<SignedCertBundle>;

    /// Return the SHA-256 hex fingerprint of the active CA cert.
    fn active_ca_fingerprint(&self) -> String;
}

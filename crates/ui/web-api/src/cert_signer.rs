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
    ) -> Result<AgentCertBundle, String>;
}

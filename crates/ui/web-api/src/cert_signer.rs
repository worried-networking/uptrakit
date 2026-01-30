pub struct AgentCertBundle {
    pub cert_pem: String,
    pub key_pem: String,
    pub lifetime_days: u16,
}

pub trait AgentCertSigner: Send + Sync + 'static {
    fn sign_agent_cert(
        &self,
        agent_id: &uuid::Uuid,
        lifetime_days: u16,
    ) -> Result<AgentCertBundle, String>;
}

use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, IsCa,
    Issuer, KeyPair, KeyUsagePurpose, SanType,
};
use std::net::Ipv4Addr;

/// A complete test PKI: CA, server certificate, and agent client certificate.
pub struct TestPki {
    /// PEM-encoded CA certificate.
    pub ca_cert_pem: String,

    /// PEM-encoded server certificate.
    pub server_cert_pem: String,
    /// PEM-encoded server private key.
    pub server_key_pem: String,

    /// PEM-encoded agent client certificate.
    pub agent_cert_pem: String,
    /// PEM-encoded agent private key.
    pub agent_key_pem: String,

    /// The agent UUID (CN of the agent cert).
    pub agent_id: uuid::Uuid,
}

impl TestPki {
    /// Generate a complete PKI for integration tests.
    ///
    /// - CA: CN=`Test CA`, ECDSA P-256, self-signed, 1-day validity
    /// - Server cert: signed by CA, SANs = `localhost`, `host.docker.internal`, `127.0.0.1`
    /// - Agent cert: signed by CA, CN = random UUID v7, EKU = ClientAuth
    pub fn generate() -> Self {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

        let ca_cn = "Test CA".to_string();

        // --- CA ---
        let ca_key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)
            .expect("CA key generation failed");

        let mut ca_params = CertificateParams::new(vec![]).expect("CA params");
        ca_params.distinguished_name = DistinguishedName::new();
        ca_params
            .distinguished_name
            .push(DnType::CommonName, &ca_cn);
        ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        ca_params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        ca_params.not_before = ::time::OffsetDateTime::now_utc() - ::time::Duration::hours(1);
        ca_params.not_after = ::time::OffsetDateTime::now_utc() + ::time::Duration::days(1);

        let ca_cert = ca_params.self_signed(&ca_key).expect("CA self-sign failed");

        let ca_cert_pem = ca_cert.pem();

        // Build an Issuer for signing child certificates.
        let ca_issuer =
            Issuer::from_ca_cert_pem(&ca_cert_pem, ca_key).expect("CA issuer creation failed");

        // --- Server cert ---
        let server_key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)
            .expect("server key generation failed");

        let mut server_params = CertificateParams::new(vec![
            "localhost".to_string(),
            "host.docker.internal".to_string(),
        ])
        .expect("server cert params");
        server_params
            .subject_alt_names
            .push(SanType::IpAddress(Ipv4Addr::LOCALHOST.into()));
        server_params.not_before = ::time::OffsetDateTime::now_utc() - ::time::Duration::hours(1);
        server_params.not_after = ::time::OffsetDateTime::now_utc() + ::time::Duration::days(1);

        let server_cert = server_params
            .signed_by(&server_key, &ca_issuer)
            .expect("server cert signing failed");

        let server_cert_pem = server_cert.pem();
        let server_key_pem = server_key.serialize_pem();

        // --- Agent cert ---
        let agent_id = uuid::Uuid::now_v7();
        let agent_key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)
            .expect("agent key generation failed");

        let mut agent_params = CertificateParams::new(vec![]).expect("agent cert params");
        agent_params.distinguished_name = DistinguishedName::new();
        agent_params
            .distinguished_name
            .push(DnType::CommonName, agent_id.to_string());
        agent_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
        agent_params.not_before = ::time::OffsetDateTime::now_utc() - ::time::Duration::hours(1);
        agent_params.not_after = ::time::OffsetDateTime::now_utc() + ::time::Duration::days(1);

        let agent_cert = agent_params
            .signed_by(&agent_key, &ca_issuer)
            .expect("agent cert signing failed");

        let agent_cert_pem = agent_cert.pem();
        let agent_key_pem = agent_key.serialize_pem();

        Self {
            ca_cert_pem,
            server_cert_pem,
            server_key_pem,
            agent_cert_pem,
            agent_key_pem,
            agent_id,
        }
    }
}

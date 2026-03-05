use rcgen::{
    BasicConstraints, CertificateParams, CertificateRevocationListParams, DistinguishedName,
    DnType, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyIdMethod, KeyPair, KeyUsagePurpose,
    RevokedCertParams, SanType, SerialNumber,
};
use std::net::Ipv4Addr;

/// A complete test PKI: CA, server certificate, and agent client certificate.
pub struct TestPki {
    /// PEM-encoded CA certificate.
    pub ca_cert_pem: String,
    /// PEM-encoded CA private key.
    pub ca_key_pem: String,

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
        let ca_key_pem_saved = ca_key.serialize_pem();

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
        server_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
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
            ca_key_pem: ca_key_pem_saved,
            server_cert_pem,
            server_key_pem,
            agent_cert_pem,
            agent_key_pem,
            agent_id,
        }
    }

    /// Generate a second agent certificate (for revocation testing).
    ///
    /// Returns `(cert_pem, key_pem, agent_id)`.
    pub fn generate_extra_agent_cert(&self) -> (String, String, uuid::Uuid) {
        let ca_issuer = Issuer::from_ca_cert_pem(&self.ca_cert_pem, self.ca_key_pair())
            .expect("CA issuer from PEM");

        let agent_id = uuid::Uuid::now_v7();
        let agent_key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)
            .expect("extra agent key generation failed");

        let mut agent_params = CertificateParams::new(vec![]).expect("extra agent cert params");
        agent_params.distinguished_name = DistinguishedName::new();
        agent_params
            .distinguished_name
            .push(DnType::CommonName, agent_id.to_string());
        agent_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
        agent_params.not_before = ::time::OffsetDateTime::now_utc() - ::time::Duration::hours(1);
        agent_params.not_after = ::time::OffsetDateTime::now_utc() + ::time::Duration::days(1);

        let cert = agent_params
            .signed_by(&agent_key, &ca_issuer)
            .expect("extra agent cert signing failed");

        (cert.pem(), agent_key.serialize_pem(), agent_id)
    }

    /// Generate a second agent certificate with an AIA extension embedding the given OCSP URL.
    ///
    /// Returns `(cert_pem, key_pem, agent_id)`.
    pub fn generate_extra_agent_cert_with_aia(
        &self,
        ocsp_url: &str,
    ) -> (String, String, uuid::Uuid) {
        let ca_issuer = Issuer::from_ca_cert_pem(&self.ca_cert_pem, self.ca_key_pair())
            .expect("CA issuer from PEM");

        let agent_id = uuid::Uuid::now_v7();
        let agent_key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)
            .expect("AIA agent key generation failed");

        let mut agent_params = CertificateParams::new(vec![]).expect("AIA agent cert params");
        agent_params.distinguished_name = DistinguishedName::new();
        agent_params
            .distinguished_name
            .push(DnType::CommonName, agent_id.to_string());
        agent_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
        agent_params.not_before = ::time::OffsetDateTime::now_utc() - ::time::Duration::hours(1);
        agent_params.not_after = ::time::OffsetDateTime::now_utc() + ::time::Duration::days(1);

        // Embed AIA extension with the OCSP responder URL
        let aia_der = build_aia_extension_der(ocsp_url);
        agent_params
            .custom_extensions
            .push(rcgen::CustomExtension::from_oid_content(
                &[1, 3, 6, 1, 5, 5, 7, 1, 1],
                aia_der,
            ));

        let cert = agent_params
            .signed_by(&agent_key, &ca_issuer)
            .expect("AIA agent cert signing failed");

        (cert.pem(), agent_key.serialize_pem(), agent_id)
    }

    /// Generate a PEM-encoded CRL containing the specified revoked certificate
    /// serial numbers. The serial numbers should be the hex-encoded serials
    /// from the X.509 certificates.
    pub fn generate_crl_pem(&self, revoked_serial_hex: &[&str]) -> String {
        let ca_key = self.ca_key_pair();
        let ca_issuer =
            Issuer::from_ca_cert_pem(&self.ca_cert_pem, ca_key).expect("CA issuer for CRL");

        let now = ::time::OffsetDateTime::now_utc();

        let revoked_certs: Vec<RevokedCertParams> = revoked_serial_hex
            .iter()
            .map(|hex_serial| {
                let bytes = hex_to_bytes(hex_serial);
                RevokedCertParams {
                    serial_number: SerialNumber::from(bytes),
                    revocation_time: now - ::time::Duration::minutes(5),
                    reason_code: Some(rcgen::RevocationReason::KeyCompromise),
                    invalidity_date: None,
                }
            })
            .collect();

        let params = CertificateRevocationListParams {
            this_update: now,
            next_update: now + ::time::Duration::days(1),
            crl_number: SerialNumber::from(1u64),
            issuing_distribution_point: None,
            revoked_certs,
            key_identifier_method: KeyIdMethod::Sha256,
        };

        let crl = params.signed_by(&ca_issuer).expect("CRL signing failed");
        crl.pem().expect("CRL PEM encoding failed")
    }

    fn ca_key_pair(&self) -> KeyPair {
        KeyPair::from_pem(&self.ca_key_pem).expect("CA key pair from PEM")
    }
}

/// Extract the hex-encoded serial number from a PEM certificate.
pub fn extract_serial_hex(cert_pem: &str) -> String {
    let (_, pem) = x509_parser::pem::parse_x509_pem(cert_pem.as_bytes())
        .expect("parse PEM for serial extraction");
    let (_, cert) = x509_parser::parse_x509_certificate(&pem.contents)
        .expect("parse X.509 for serial extraction");
    let serial_bytes = cert.serial.to_bytes_be();
    serial_bytes
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>()
}

fn hex_to_bytes(hex: &str) -> Vec<u8> {
    // Support both with-colon and without-colon hex formats
    let clean: String = hex.replace(':', "");
    (0..clean.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&clean[i..i + 2], 16).expect("valid hex"))
        .collect()
}

// --- DER encoding helpers for AIA extension (mirrors production crates/core/controller/src/pki.rs) ---

/// Build the DER-encoded value for an Authority Information Access (AIA) extension
/// containing only an OCSP responder URL.
fn build_aia_extension_der(ocsp_url: &str) -> Vec<u8> {
    // OCSP access description: SEQUENCE { OID(id-ad-ocsp), [6] URI }
    let access_descriptions = encode_access_description(
        &[0x06, 0x08, 0x2b, 0x06, 0x01, 0x05, 0x05, 0x07, 0x30, 0x01], // id-ad-ocsp OID
        ocsp_url,
    );

    // Wrap in SEQUENCE (AuthorityInfoAccessSyntax)
    encode_der_sequence(&access_descriptions)
}

/// Encode a single AccessDescription as a DER SEQUENCE.
fn encode_access_description(method_oid_der: &[u8], uri: &str) -> Vec<u8> {
    let uri_bytes = uri.as_bytes();
    // GeneralName uniformResourceIdentifier [6] IMPLICIT IA5String
    let mut general_name = vec![0x86]; // context tag 6, primitive
    general_name.extend_from_slice(&encode_der_length(uri_bytes.len()));
    general_name.extend_from_slice(uri_bytes);

    let mut content = Vec::new();
    content.extend_from_slice(method_oid_der);
    content.extend_from_slice(&general_name);

    encode_der_sequence(&content)
}

/// Encode a DER SEQUENCE tag + length + content.
fn encode_der_sequence(content: &[u8]) -> Vec<u8> {
    let mut result = vec![0x30]; // SEQUENCE tag
    result.extend_from_slice(&encode_der_length(content.len()));
    result.extend_from_slice(content);
    result
}

/// Encode a DER length in the minimum number of octets.
fn encode_der_length(len: usize) -> Vec<u8> {
    if len < 0x80 {
        vec![len as u8]
    } else if len < 0x100 {
        vec![0x81, len as u8]
    } else {
        vec![0x82, (len >> 8) as u8, len as u8]
    }
}

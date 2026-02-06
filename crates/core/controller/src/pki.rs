use std::fs;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, Issuer, KeyPair};
use rootcause::ReportConversion;
use rootcause::prelude::*;
use sha2::{Digest, Sha256};
use thiserror::Error;
use time::OffsetDateTime;
use uptrakit_web_api::pki_utils::{self, SanCollection};

/// Build the DER-encoded value for an Authority Information Access (AIA) extension.
///
/// The AIA extension (OID 1.3.6.1.5.5.7.1.1) contains:
/// - `id-ad-ocsp` access description pointing to the OCSP responder URL
/// - `id-ad-caIssuers` access description pointing to the CA certificate URL
fn build_aia_extension_der(ocsp_url: &str, ca_issuers_url: &str) -> Vec<u8> {
    let mut access_descriptions = Vec::new();

    // OCSP access description: SEQUENCE { OID(id-ad-ocsp), [6] URI }
    access_descriptions.extend_from_slice(&encode_access_description(
        &[0x06, 0x08, 0x2b, 0x06, 0x01, 0x05, 0x05, 0x07, 0x30, 0x01], // id-ad-ocsp OID
        ocsp_url,
    ));

    // CA Issuers access description: SEQUENCE { OID(id-ad-caIssuers), [6] URI }
    access_descriptions.extend_from_slice(&encode_access_description(
        &[0x06, 0x08, 0x2b, 0x06, 0x01, 0x05, 0x05, 0x07, 0x30, 0x02], // id-ad-caIssuers OID
        ca_issuers_url,
    ));

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

/// Add AIA and CDP extensions to certificate parameters when a backend URL is set.
pub fn add_pki_extensions(params: &mut CertificateParams, pki_addr: &str) {
    let ocsp_url = format!("{pki_addr}/api/v1/pki/ocsp");
    let ca_issuers_url = format!("{pki_addr}/api/v1/pki/ca.crt");
    let crl_url = format!("{pki_addr}/api/v1/pki/ca.crl");

    // AIA extension (OID 1.3.6.1.5.5.7.1.1)
    let aia_der = build_aia_extension_der(&ocsp_url, &ca_issuers_url);
    params
        .custom_extensions
        .push(rcgen::CustomExtension::from_oid_content(
            &[1, 3, 6, 1, 5, 5, 7, 1, 1],
            aia_der,
        ));

    // CDP extension (CRL Distribution Points)
    params.crl_distribution_points = vec![rcgen::CrlDistributionPoint {
        uris: vec![crl_url],
    }];
}

#[derive(Debug, Error)]
pub enum PkiError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("certificate generation error: {0}")]
    Rcgen(#[from] rcgen::Error),

    #[error("rustls error: {0}")]
    Rustls(#[from] rustls::Error),

    #[error("PEM parsing error")]
    PemParse,

    #[error("client verifier builder error: {0}")]
    VerifierBuilder(String),

    #[error("hostname resolution failed: {0}")]
    Hostname(String),

    #[error("timestamp error: {0}")]
    Timestamp(String),

    #[error("database error: {0}")]
    Database(String),

    #[error("CA validation failed: {0}")]
    CaValidation(String),
}

pub type Result<T> = std::result::Result<T, Report<PkiError>>;

impl<T> ReportConversion<std::io::Error, markers::Mutable, T> for PkiError
where
    PkiError: markers::ObjectMarkerFor<T>,
{
    fn convert_report(
        report: Report<std::io::Error, markers::Mutable, T>,
    ) -> Report<Self, markers::Mutable, T> {
        report.context_transform(PkiError::Io)
    }
}

impl<T> ReportConversion<rcgen::Error, markers::Mutable, T> for PkiError
where
    PkiError: markers::ObjectMarkerFor<T>,
{
    fn convert_report(
        report: Report<rcgen::Error, markers::Mutable, T>,
    ) -> Report<Self, markers::Mutable, T> {
        report.context_transform(PkiError::Rcgen)
    }
}

impl<T> ReportConversion<rustls::Error, markers::Mutable, T> for PkiError
where
    PkiError: markers::ObjectMarkerFor<T>,
{
    fn convert_report(
        report: Report<rustls::Error, markers::Mutable, T>,
    ) -> Report<Self, markers::Mutable, T> {
        report.context_transform(PkiError::Rustls)
    }
}

impl<T> ReportConversion<sea_orm::DbErr, markers::Mutable, T> for PkiError
where
    PkiError: markers::ObjectMarkerFor<T>,
{
    fn convert_report(
        report: Report<sea_orm::DbErr, markers::Mutable, T>,
    ) -> Report<Self, markers::Mutable, T> {
        report.context_transform(|e| PkiError::Database(e.to_string()))
    }
}

impl<T> ReportConversion<pki_utils::PkiUtilError, markers::Mutable, T> for PkiError
where
    PkiError: markers::ObjectMarkerFor<T>,
{
    fn convert_report(
        report: Report<pki_utils::PkiUtilError, markers::Mutable, T>,
    ) -> Report<Self, markers::Mutable, T> {
        report.context_transform(|e| match e {
            pki_utils::PkiUtilError::Hostname(s) => PkiError::Hostname(s),
            pki_utils::PkiUtilError::PemParse => PkiError::PemParse,
        })
    }
}

/// Loaded CA material.
pub struct CaBundle {
    pub cert_pem: String,
    pub key_pem: String,
    pub issuer: Issuer<'static, KeyPair>,
}

/// Loaded server certificate material.
pub struct ServerCertBundle {
    pub cert_pem: String,
    pub key_pem: String,
}

/// Active + optional previous CA state.
pub struct CaState {
    pub active: CaBundle,
    pub previous: Option<CaBundle>,
    pub managed: bool,
}

/// Type alias for the canonical CA snapshot type from the web-api crate.
pub type CaSnapshot = uptrakit_web_api::ca_snapshot::CaSnapshotData;

impl CaState {
    /// Build a PEM bundle of all trusted CA certs (active + optional previous).
    pub fn ca_bundle_pem(&self) -> String {
        let mut bundle = self.active.cert_pem.clone();
        if let Some(prev) = &self.previous {
            if !bundle.ends_with('\n') {
                bundle.push('\n');
            }
            bundle.push_str(&prev.cert_pem);
        }
        bundle
    }

    /// Build a shareable snapshot.
    pub fn to_snapshot(&self, pki_addr: Option<String>) -> Result<CaSnapshot> {
        let active_fingerprint = ca_fingerprint(&self.active.cert_pem)?;
        let previous_fingerprint = match &self.previous {
            Some(prev) => Some(ca_fingerprint(&prev.cert_pem)?),
            None => None,
        };
        let bundle_pem = self.ca_bundle_pem();
        let bundle_hash = sha256_hex(bundle_pem.as_bytes());
        let active_not_after = cert_not_after(&self.active.cert_pem)?;

        Ok(CaSnapshot {
            active_cert_pem: self.active.cert_pem.clone(),
            active_key_pem: self.active.key_pem.clone(),
            active_fingerprint,
            previous_cert_pem: self.previous.as_ref().map(|p| p.cert_pem.clone()),
            previous_key_pem: self.previous.as_ref().map(|p| p.key_pem.clone()),
            previous_fingerprint,
            bundle_pem,
            bundle_hash,
            managed: self.managed,
            active_not_after,
            pki_addr,
        })
    }
}

/// Ensure the PKI directory exists and return its path.
pub fn pki_dir(data_dir: &Path) -> Result<PathBuf> {
    let dir = data_dir.join("pki");
    fs::create_dir_all(&dir).context_to::<PkiError>()?;
    Ok(dir)
}

/// Load the full CA state from the PKI directory (active + optional previous).
pub fn load_ca_state(pki: &Path, pki_addr: Option<&str>) -> Result<CaState> {
    let active = load_or_generate_ca(pki, pki_addr)?;
    let previous = load_previous_ca(pki)?;
    let managed = is_ca_managed(pki);

    Ok(CaState {
        active,
        previous,
        managed,
    })
}

/// Load or generate the internal CA.
pub fn load_or_generate_ca(pki: &Path, pki_addr: Option<&str>) -> Result<CaBundle> {
    let cert_path = pki.join("ca.crt");
    let key_path = pki.join("ca.key");

    if cert_path.exists() && key_path.exists() {
        load_ca(&cert_path, &key_path)
    } else {
        let bundle = generate_ca(pki_addr)?;
        fs::write(&cert_path, &bundle.cert_pem).context_to::<PkiError>()?;
        fs::write(&key_path, &bundle.key_pem).context_to::<PkiError>()?;
        mark_ca_managed(pki)?;
        tracing::info!("generated new internal CA at {}", pki.display());
        Ok(bundle)
    }
}

/// Load the previous (rotated-out) CA if present.
fn load_previous_ca(pki: &Path) -> Result<Option<CaBundle>> {
    let cert_path = pki.join("ca-previous.crt");
    let key_path = pki.join("ca-previous.key");

    if cert_path.exists() && key_path.exists() {
        let bundle = load_ca(&cert_path, &key_path)?;
        tracing::info!("loaded previous CA from {}", cert_path.display());
        Ok(Some(bundle))
    } else {
        Ok(None)
    }
}

fn generate_ca(pki_addr: Option<&str>) -> Result<CaBundle> {
    let key_pair =
        KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).context_to::<PkiError>()?;

    let mut params = CertificateParams::default();
    params
        .distinguished_name
        .push(DnType::CommonName, "Uptrakit Internal CA");
    params
        .distinguished_name
        .push(DnType::OrganizationName, "Uptrakit");
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.not_before = OffsetDateTime::now_utc();
    params.not_after = OffsetDateTime::now_utc() + time::Duration::days(1825);

    if let Some(url) = pki_addr {
        add_pki_extensions(&mut params, url);
    }

    let cert = params.self_signed(&key_pair).context_to::<PkiError>()?;
    let cert_pem = cert.pem();
    let key_pem = key_pair.serialize_pem();
    let issuer = Issuer::new(params, key_pair);

    Ok(CaBundle {
        cert_pem,
        key_pem,
        issuer,
    })
}

fn load_ca(cert_path: &Path, key_path: &Path) -> Result<CaBundle> {
    let cert_pem = fs::read_to_string(cert_path).context_to::<PkiError>()?;
    let key_pem = fs::read_to_string(key_path).context_to::<PkiError>()?;

    let key_pair = KeyPair::from_pem(&key_pem).context_to::<PkiError>()?;
    let issuer = Issuer::from_ca_cert_pem(&cert_pem, key_pair).context_to::<PkiError>()?;

    tracing::info!("loaded existing CA from {}", cert_path.display());
    Ok(CaBundle {
        cert_pem,
        key_pem,
        issuer,
    })
}

/// Load a CA from user-provided (external) paths.
pub fn load_external_ca(cert_path: &Path, key_path: &Path) -> Result<CaBundle> {
    let bundle = load_ca(cert_path, key_path)?;
    tracing::info!("using external CA certificate from {}", cert_path.display());
    Ok(bundle)
}

/// Load a server certificate from user-provided paths.
pub fn load_external_cert(cert_path: &Path, key_path: &Path) -> Result<ServerCertBundle> {
    let cert_pem = fs::read_to_string(cert_path).context_to::<PkiError>()?;
    let key_pem = fs::read_to_string(key_path).context_to::<PkiError>()?;
    tracing::info!(
        "using external TLS certificate from {}",
        cert_path.display()
    );
    Ok(ServerCertBundle { cert_pem, key_pem })
}

/// Load or generate a server certificate signed by the internal CA.
pub fn load_or_generate_server_cert(
    pki: &Path,
    ca: &CaBundle,
    extra_sans: &[String],
) -> Result<ServerCertBundle> {
    let cert_path = pki.join("server.crt");
    let key_path = pki.join("server.key");

    if cert_path.exists() && key_path.exists() {
        let cert_pem = fs::read_to_string(&cert_path).context_to::<PkiError>()?;
        let key_pem = fs::read_to_string(&key_path).context_to::<PkiError>()?;

        if !is_cert_expired(&cert_pem) {
            tracing::info!(
                "loaded existing server certificate from {}",
                cert_path.display()
            );
            return Ok(ServerCertBundle { cert_pem, key_pem });
        }
        tracing::warn!("server certificate expired, regenerating");
    }

    let bundle = generate_server_cert(ca, extra_sans)?;
    fs::write(&cert_path, &bundle.cert_pem).context_to::<PkiError>()?;
    fs::write(&key_path, &bundle.key_pem).context_to::<PkiError>()?;
    tracing::info!("generated new server certificate at {}", pki.display());
    Ok(bundle)
}

fn generate_server_cert(ca: &CaBundle, extra_sans: &[String]) -> Result<ServerCertBundle> {
    let key_pair =
        KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).context_to::<PkiError>()?;

    let sans = pki_utils::collect_sans(extra_sans).context_to::<PkiError>()?;

    let mut params = CertificateParams::new(sans.dns_names.clone()).context_to::<PkiError>()?;
    for ip in &sans.ip_addrs {
        params
            .subject_alt_names
            .push(rcgen::SanType::IpAddress(*ip));
    }
    params
        .distinguished_name
        .push(DnType::CommonName, "Uptrakit Controller");
    params
        .distinguished_name
        .push(DnType::OrganizationName, "Uptrakit");
    params.not_before = OffsetDateTime::now_utc();
    params.not_after = OffsetDateTime::now_utc() + time::Duration::days(90);

    let cert = params
        .signed_by(&key_pair, &ca.issuer)
        .context_to::<PkiError>()?;

    Ok(ServerCertBundle {
        cert_pem: cert.pem(),
        key_pem: key_pair.serialize_pem(),
    })
}

// --- CA fingerprint ---

/// Compute SHA-256 hex fingerprint of a PEM-encoded certificate.
pub fn ca_fingerprint(cert_pem: &str) -> Result<String> {
    let (_, pem_block) = x509_parser::pem::parse_x509_pem(cert_pem.as_bytes())
        .map_err(|_| report!(PkiError::PemParse))?;
    Ok(sha256_hex(&pem_block.contents))
}

/// SHA-256 hex digest of arbitrary bytes.
pub fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

// --- Managed CA marker ---

/// Mark the CA as auto-generated (managed).
pub fn mark_ca_managed(pki: &Path) -> Result<()> {
    fs::write(pki.join("ca-managed"), "").context_to::<PkiError>()?;
    Ok(())
}

/// Check if the CA was auto-generated (managed).
pub fn is_ca_managed(pki: &Path) -> bool {
    pki.join("ca-managed").exists()
}

// --- Cert introspection ---

/// Check if a PEM-encoded certificate is expired.
/// Returns `true` if the certificate is expired or unparseable.
pub fn is_cert_expired(pem: &str) -> bool {
    let Ok((_, pem_block)) = x509_parser::pem::parse_x509_pem(pem.as_bytes()) else {
        return true;
    };
    let Ok(cert) = pem_block.parse_x509() else {
        return true;
    };
    !cert.validity().is_valid()
}

/// Extract the `not_after` timestamp from a PEM-encoded certificate.
pub fn cert_not_after(pem: &str) -> Result<OffsetDateTime> {
    let (_, pem_block) = x509_parser::pem::parse_x509_pem(pem.as_bytes())
        .map_err(|_| report!(PkiError::PemParse))?;
    let cert = pem_block
        .parse_x509()
        .map_err(|_| report!(PkiError::PemParse))?;
    OffsetDateTime::from_unix_timestamp(cert.validity().not_after.timestamp())
        .map_err(|e| report!(PkiError::Timestamp(e.to_string())))
}

// --- Rotation helpers ---

/// Returns `true` if the CA certificate expires within 183 days (6 months).
pub fn should_rotate_ca(cert_pem: &str) -> bool {
    let Ok(not_after) = cert_not_after(cert_pem) else {
        return true;
    };
    let threshold = OffsetDateTime::now_utc() + time::Duration::days(183);
    not_after <= threshold
}

/// Rotate the CA: move current → previous, generate new active CA.
pub fn rotate_ca(pki: &Path, pki_addr: Option<&str>) -> Result<CaState> {
    let active_cert = pki.join("ca.crt");
    let active_key = pki.join("ca.key");
    let prev_cert = pki.join("ca-previous.crt");
    let prev_key = pki.join("ca-previous.key");

    // Move current active → previous
    fs::copy(&active_cert, &prev_cert).context_to::<PkiError>()?;
    fs::copy(&active_key, &prev_key).context_to::<PkiError>()?;

    // Generate new CA
    let new_ca = generate_ca(pki_addr)?;
    fs::write(&active_cert, &new_ca.cert_pem).context_to::<PkiError>()?;
    fs::write(&active_key, &new_ca.key_pem).context_to::<PkiError>()?;

    // Load previous for issuer
    let previous = load_ca(&prev_cert, &prev_key)?;

    tracing::info!("CA rotated: new CA generated, previous CA preserved");

    Ok(CaState {
        active: new_ca,
        previous: Some(previous),
        managed: true,
    })
}

// --- Server cert renewal ---

/// Returns `true` if the server certificate expires within 30 days.
pub fn should_renew_server_cert(cert_pem: &str) -> bool {
    let Ok(not_after) = cert_not_after(cert_pem) else {
        return true;
    };
    let threshold = OffsetDateTime::now_utc() + time::Duration::days(30);
    not_after <= threshold
}

/// Generate a new server cert signed by the given CA and save to the PKI directory.
pub fn renew_server_cert(
    pki: &Path,
    ca: &CaBundle,
    extra_sans: &[String],
) -> Result<ServerCertBundle> {
    let bundle = generate_server_cert(ca, extra_sans)?;
    let cert_path = pki.join("server.crt");
    let key_path = pki.join("server.key");
    fs::write(&cert_path, &bundle.cert_pem).context_to::<PkiError>()?;
    fs::write(&key_path, &bundle.key_pem).context_to::<PkiError>()?;
    tracing::info!("server certificate renewed at {}", pki.display());
    Ok(bundle)
}

// --- SAN sanity checks ---

/// Extract Subject Alternative Names from a PEM-encoded certificate.
///
/// Returns a `SanCollection` with the DNS names and IP addresses found in
/// the certificate's SAN extension.
pub fn extract_sans_from_cert(cert_pem: &str) -> Result<SanCollection> {
    use x509_parser::extensions::GeneralName;

    let (_, pem_block) = x509_parser::pem::parse_x509_pem(cert_pem.as_bytes())
        .map_err(|_| report!(PkiError::PemParse))?;
    let cert = pem_block
        .parse_x509()
        .map_err(|_| report!(PkiError::PemParse))?;

    let mut dns_names = Vec::new();
    let mut ip_addrs = Vec::new();

    if let Ok(Some(san_ext)) = cert.tbs_certificate.subject_alternative_name() {
        for name in &san_ext.value.general_names {
            match name {
                GeneralName::DNSName(dns) => {
                    dns_names.push((*dns).to_string());
                }
                GeneralName::IPAddress(ip_bytes) => {
                    match ip_bytes.len() {
                        4 => {
                            let octets: [u8; 4] = (*ip_bytes).try_into().unwrap_or([0; 4]);
                            ip_addrs.push(IpAddr::V4(std::net::Ipv4Addr::from(octets)));
                        }
                        16 => {
                            let octets: [u8; 16] = (*ip_bytes).try_into().unwrap_or([0; 16]);
                            ip_addrs.push(IpAddr::V6(std::net::Ipv6Addr::from(octets)));
                        }
                        _ => {} // skip malformed IP entries
                    }
                }
                _ => {}
            }
        }
    }

    dns_names.sort();
    dns_names.dedup();
    ip_addrs.sort();
    ip_addrs.dedup();

    Ok(SanCollection {
        dns_names,
        ip_addrs,
    })
}

/// Check whether the server certificate needs to be regenerated because its
/// SANs do not include all the requested `extra_sans`.
///
/// Returns `false` if `extra_sans` is empty (no user-requested SANs to check).
/// Otherwise computes the expected SAN set via `collect_sans(extra_sans)` and
/// compares against the certificate's actual SANs.
pub fn server_cert_needs_san_update(cert_pem: &str, extra_sans: &[String]) -> Result<bool> {
    if extra_sans.is_empty() {
        return Ok(false);
    }

    let expected = pki_utils::collect_sans(extra_sans).context_to::<PkiError>()?;
    let actual = extract_sans_from_cert(cert_pem)?;

    let mut expected_dns = expected.dns_names;
    expected_dns.sort();
    let mut actual_dns = actual.dns_names;
    actual_dns.sort();

    let mut expected_ips = expected.ip_addrs;
    expected_ips.sort();
    let mut actual_ips = actual.ip_addrs;
    actual_ips.sort();

    // Check that every expected SAN is present in actual
    let dns_match = expected_dns.iter().all(|name| actual_dns.contains(name));
    let ip_match = expected_ips.iter().all(|ip| actual_ips.contains(ip));

    Ok(!dns_match || !ip_match)
}

/// Check if a certificate was signed by the given CA.
///
/// Thin wrapper around `pki_utils::cert_signed_by_ca` that converts errors
/// to `PkiError`.
pub fn cert_signed_by_ca(cert_pem: &str, ca_pem: &str) -> Result<bool> {
    pki_utils::cert_signed_by_ca(cert_pem, ca_pem).context_to::<PkiError>()
}

// --- PKI URL extraction and validation ---

/// URLs extracted from a certificate's AIA and CDP extensions.
#[derive(Debug, Default, PartialEq)]
pub struct CertPkiUrls {
    pub ocsp_url: Option<String>,
    pub ca_issuers_url: Option<String>,
    pub crl_url: Option<String>,
}

impl CertPkiUrls {
    /// Returns `true` if the certificate has any AIA or CDP extensions.
    pub fn has_extensions(&self) -> bool {
        self.ocsp_url.is_some() || self.ca_issuers_url.is_some() || self.crl_url.is_some()
    }
}

/// Extract AIA and CDP URLs from a PEM-encoded certificate.
pub fn extract_cert_pki_urls(cert_pem: &str) -> Result<CertPkiUrls> {
    use x509_parser::extensions::ParsedExtension;

    let (_, pem_block) = x509_parser::pem::parse_x509_pem(cert_pem.as_bytes())
        .map_err(|_| report!(PkiError::PemParse))?;
    let cert = pem_block
        .parse_x509()
        .map_err(|_| report!(PkiError::PemParse))?;

    let mut urls = CertPkiUrls::default();

    for ext in cert.extensions() {
        match ext.parsed_extension() {
            ParsedExtension::AuthorityInfoAccess(aia) => {
                for desc in &aia.accessdescs {
                    // id-ad-ocsp = 1.3.6.1.5.5.7.48.1
                    if desc.access_method.to_id_string() == "1.3.6.1.5.5.7.48.1"
                        && let x509_parser::extensions::GeneralName::URI(uri) = desc.access_location
                    {
                        urls.ocsp_url = Some(uri.to_string());
                    }
                    // id-ad-caIssuers = 1.3.6.1.5.5.7.48.2
                    if desc.access_method.to_id_string() == "1.3.6.1.5.5.7.48.2"
                        && let x509_parser::extensions::GeneralName::URI(uri) = desc.access_location
                    {
                        urls.ca_issuers_url = Some(uri.to_string());
                    }
                }
            }
            ParsedExtension::CRLDistributionPoints(cdp) => {
                for point in cdp.iter() {
                    if let Some(name) = &point.distribution_point
                        && let x509_parser::extensions::DistributionPointName::FullName(names) =
                            name
                    {
                        for general_name in names {
                            if let x509_parser::extensions::GeneralName::URI(uri) = general_name {
                                urls.crl_url = Some(uri.to_string());
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    Ok(urls)
}

/// Validate that the existing CA certificate's AIA/CDP extensions match the
/// reconciled `pki_addr`. Only applies to managed CAs.
///
/// Call this after loading CA state and before building the snapshot.
/// Mismatch causes a hard startup failure with descriptive error.
pub fn validate_ca_pki_addr(cert_pem: &str, pki_addr: Option<&str>, pki_path: &Path) -> Result<()> {
    let cert_urls = extract_cert_pki_urls(cert_pem)?;
    let has_extensions = cert_urls.has_extensions();

    match (pki_addr, has_extensions) {
        // pki_addr set, CA has extensions — check they match
        (Some(url), true) => {
            let expected_ocsp = format!("{url}/api/v1/pki/ocsp");
            let expected_ca_issuers = format!("{url}/api/v1/pki/ca.crt");
            let expected_crl = format!("{url}/api/v1/pki/ca.crl");

            let mismatch = cert_urls.ocsp_url.as_deref() != Some(&expected_ocsp)
                || cert_urls.ca_issuers_url.as_deref() != Some(&expected_ca_issuers)
                || cert_urls.crl_url.as_deref() != Some(&expected_crl);

            if mismatch {
                return Err(report!(PkiError::CaValidation(format!(
                    "The CA certificate's AIA/CDP URLs do not match --pki-addr ({url}).\n\
                     \n\
                     CA certificate contains:\n\
                     \x20 OCSP:       {}\n\
                     \x20 CA Issuers: {}\n\
                     \x20 CRL:        {}\n\
                     \n\
                     Expected (from --pki-addr):\n\
                     \x20 OCSP:       {expected_ocsp}\n\
                     \x20 CA Issuers: {expected_ca_issuers}\n\
                     \x20 CRL:        {expected_crl}\n\
                     \n\
                     To fix this, either:\n\
                     \x20 1. Update --pki-addr to match the CA certificate's URLs, or\n\
                     \x20 2. Delete the CA files in {} and restart to regenerate with the new URL",
                    cert_urls.ocsp_url.as_deref().unwrap_or("<none>"),
                    cert_urls.ca_issuers_url.as_deref().unwrap_or("<none>"),
                    cert_urls.crl_url.as_deref().unwrap_or("<none>"),
                    pki_path.display(),
                ))));
            }
            Ok(())
        }
        // pki_addr set, CA has no extensions — need to regenerate
        (Some(url), false) => Err(report!(PkiError::CaValidation(format!(
            "The CA certificate has no AIA/CDP extensions, but --pki-addr ({url}) is set.\n\
                 \n\
                 The CA needs to be regenerated with the backend URL to embed OCSP, CA Issuers,\n\
                 and CRL Distribution Point URLs in certificates.\n\
                 \n\
                 To fix this, delete the CA files in {} and restart the controller.\n\
                 A new CA will be generated with the correct extensions.",
            pki_path.display(),
        )))),
        // pki_addr not set, CA has extensions — unexpected
        (None, true) => Err(report!(PkiError::CaValidation(format!(
            "The CA certificate contains AIA/CDP extensions but no --pki-addr is configured.\n\
                 \n\
                 CA certificate contains:\n\
                 \x20 OCSP:       {}\n\
                 \x20 CA Issuers: {}\n\
                 \x20 CRL:        {}\n\
                 \n\
                 To fix this, either:\n\
                 \x20 1. Provide --pki-addr matching the URLs in the CA certificate, or\n\
                 \x20 2. Delete the CA files in {} and restart to regenerate without extensions",
            cert_urls.ocsp_url.as_deref().unwrap_or("<none>"),
            cert_urls.ca_issuers_url.as_deref().unwrap_or("<none>"),
            cert_urls.crl_url.as_deref().unwrap_or("<none>"),
            pki_path.display(),
        )))),
        // pki_addr not set, CA has no extensions — OK
        (None, false) => Ok(()),
    }
}

// --- TLS config builders ---

/// Build a `rustls::ServerConfig` from PEM-encoded cert and key (no client auth).
#[cfg(test)]
pub fn build_rustls_config(cert_pem: &str, key_pem: &str) -> Result<rustls::ServerConfig> {
    use rustls::pki_types::pem::PemObject;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer};

    let certs: Vec<_> = CertificateDer::pem_slice_iter(cert_pem.as_bytes())
        .collect::<std::result::Result<Vec<_>, _>>()
        .context_transform(|_| PkiError::PemParse)?;

    let key = PrivateKeyDer::from_pem_slice(key_pem.as_bytes())
        .context_transform(|_| PkiError::PemParse)?;

    let config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .context_to::<PkiError>()?;

    Ok(config)
}

/// Build a `rustls::ServerConfig` with mTLS client authentication and multiple CRLs.
///
/// Each CA in the bundle gets its own CRL. The verifier checks client certificates
/// against all supplied CRLs.
pub fn build_rustls_config_with_client_auth_and_crls(
    cert_pem: &str,
    key_pem: &str,
    ca_bundle_pem: &str,
    crls: Vec<rustls::pki_types::CertificateRevocationListDer<'static>>,
) -> Result<rustls::ServerConfig> {
    use rustls::RootCertStore;
    use rustls::pki_types::pem::PemObject;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer};
    use rustls::server::WebPkiClientVerifier;

    let certs: Vec<_> = CertificateDer::pem_slice_iter(cert_pem.as_bytes())
        .collect::<std::result::Result<Vec<_>, _>>()
        .context_transform(|_| PkiError::PemParse)?;

    let key = PrivateKeyDer::from_pem_slice(key_pem.as_bytes())
        .context_transform(|_| PkiError::PemParse)?;

    let ca_certs: Vec<_> = CertificateDer::pem_slice_iter(ca_bundle_pem.as_bytes())
        .collect::<std::result::Result<Vec<_>, _>>()
        .context_transform(|_| PkiError::PemParse)?;

    let mut root_store = RootCertStore::empty();
    for ca_cert in ca_certs {
        root_store.add(ca_cert).context_to::<PkiError>()?;
    }

    let verifier = WebPkiClientVerifier::builder(Arc::new(root_store))
        .with_crls(crls)
        .allow_unauthenticated()
        .only_check_end_entity_revocation()
        .build()
        .map_err(|e| report!(PkiError::VerifierBuilder(e.to_string())))?;

    let config = rustls::ServerConfig::builder()
        .with_client_cert_verifier(verifier)
        .with_single_cert(certs, key)
        .context_to::<PkiError>()?;

    Ok(config)
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    use super::*;

    #[test]
    fn ca_generation_produces_valid_material() {
        let ca = generate_ca(None).unwrap();
        assert!(ca.cert_pem.contains("BEGIN CERTIFICATE"));
        assert!(ca.key_pem.contains("BEGIN"));
    }

    #[test]
    fn ca_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let pki = dir.path();

        let ca1 = load_or_generate_ca(pki, None).unwrap();
        let ca2 = load_or_generate_ca(pki, None).unwrap();

        assert_eq!(ca1.cert_pem, ca2.cert_pem);
    }

    #[test]
    fn server_cert_signed_by_ca() {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

        let ca = generate_ca(None).unwrap();
        let bundle = generate_server_cert(&ca, &[]).unwrap();
        assert!(bundle.cert_pem.contains("BEGIN CERTIFICATE"));
        assert!(bundle.key_pem.contains("BEGIN"));

        // Should build a valid rustls config
        build_rustls_config(&bundle.cert_pem, &bundle.key_pem).unwrap();
    }

    #[test]
    fn server_cert_includes_localhost() {
        let sans = pki_utils::collect_sans(&[]).unwrap();
        assert!(sans.dns_names.contains(&"localhost".to_string()));
    }

    #[test]
    fn server_cert_includes_extra_sans() {
        let extras = vec![
            "myhost.example.com".to_string(),
            "192.168.1.1".to_string(),
            "::1".to_string(),
        ];
        let sans = pki_utils::collect_sans(&extras).unwrap();
        assert!(sans.dns_names.contains(&"myhost.example.com".to_string()));
        assert!(
            sans.ip_addrs
                .contains(&IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)))
        );
        assert!(sans.ip_addrs.contains(&IpAddr::V6(Ipv6Addr::LOCALHOST)));
    }

    #[test]
    fn hostname_deduplication() {
        let hostname = hostname::get().unwrap().to_string_lossy().to_string();
        let extras = vec![hostname.clone()];
        let sans = pki_utils::collect_sans(&extras).unwrap();
        let count = sans.dns_names.iter().filter(|n| **n == hostname).count();
        assert_eq!(count, 1);
    }

    #[test]
    fn expired_cert_detection() {
        let ca = generate_ca(None).unwrap();
        assert!(!is_cert_expired(&ca.cert_pem));
    }

    #[test]
    fn malformed_pem_is_expired() {
        assert!(is_cert_expired("not a cert"));
    }

    #[test]
    fn server_cert_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let pki = dir.path();

        let ca = generate_ca(None).unwrap();
        let b1 = load_or_generate_server_cert(pki, &ca, &[]).unwrap();
        let b2 = load_or_generate_server_cert(pki, &ca, &[]).unwrap();

        assert_eq!(b1.cert_pem, b2.cert_pem);
    }

    #[test]
    fn san_ipv6_address() {
        let extras = vec!["fd00::1".to_string()];
        let sans = pki_utils::collect_sans(&extras).unwrap();
        assert!(
            sans.ip_addrs
                .contains(&IpAddr::V6(Ipv6Addr::new(0xfd00, 0, 0, 0, 0, 0, 0, 1)))
        );
    }

    #[test]
    fn ca_fingerprint_deterministic() {
        let ca = generate_ca(None).unwrap();
        let fp1 = ca_fingerprint(&ca.cert_pem).unwrap();
        let fp2 = ca_fingerprint(&ca.cert_pem).unwrap();
        assert_eq!(fp1, fp2);
        assert_eq!(fp1.len(), 64); // SHA-256 hex is 64 chars
    }

    #[test]
    fn ca_fingerprint_differs_between_cas() {
        let ca1 = generate_ca(None).unwrap();
        let ca2 = generate_ca(None).unwrap();
        let fp1 = ca_fingerprint(&ca1.cert_pem).unwrap();
        let fp2 = ca_fingerprint(&ca2.cert_pem).unwrap();
        assert_ne!(fp1, fp2);
    }

    #[test]
    fn should_rotate_ca_not_yet() {
        let ca = generate_ca(None).unwrap();
        // Fresh CA with 5 year validity should not need rotation
        assert!(!should_rotate_ca(&ca.cert_pem));
    }

    #[test]
    fn should_renew_server_cert_not_yet() {
        let ca = generate_ca(None).unwrap();
        let server = generate_server_cert(&ca, &[]).unwrap();
        // Fresh server cert with 90 day validity should not need renewal
        assert!(!should_renew_server_cert(&server.cert_pem));
    }

    #[test]
    fn ca_state_bundle_pem() {
        let ca1 = generate_ca(None).unwrap();
        let ca2 = generate_ca(None).unwrap();
        let state = CaState {
            active: ca1,
            previous: Some(ca2),
            managed: true,
        };
        let bundle = state.ca_bundle_pem();
        // Bundle should contain two certificates
        assert_eq!(bundle.matches("BEGIN CERTIFICATE").count(), 2);
    }

    #[test]
    fn ca_snapshot_roundtrip() {
        let ca = generate_ca(None).unwrap();
        let state = CaState {
            active: ca,
            previous: None,
            managed: true,
        };
        let snapshot = state.to_snapshot(None).unwrap();
        assert!(!snapshot.active_fingerprint.is_empty());
        assert!(snapshot.previous_fingerprint.is_none());
        assert!(!snapshot.bundle_hash.is_empty());
    }

    #[test]
    fn managed_marker_file() {
        let dir = tempfile::tempdir().unwrap();
        let pki = dir.path();
        assert!(!is_ca_managed(pki));
        mark_ca_managed(pki).unwrap();
        assert!(is_ca_managed(pki));
    }

    #[test]
    fn ca_rotation() {
        let dir = tempfile::tempdir().unwrap();
        let pki = dir.path();

        // Generate initial CA
        let initial = load_or_generate_ca(pki, None).unwrap();
        let initial_fp = ca_fingerprint(&initial.cert_pem).unwrap();

        // Rotate
        let state = rotate_ca(pki, None).unwrap();
        let new_fp = ca_fingerprint(&state.active.cert_pem).unwrap();
        assert_ne!(initial_fp, new_fp);

        let prev = state.previous.as_ref().unwrap();
        let prev_fp = ca_fingerprint(&prev.cert_pem).unwrap();
        assert_eq!(initial_fp, prev_fp);
    }

    #[test]
    fn extract_sans_dns_only() {
        let ca = generate_ca(None).unwrap();
        let server = generate_server_cert(&ca, &[]).unwrap();
        let sans = extract_sans_from_cert(&server.cert_pem).unwrap();
        assert!(sans.dns_names.contains(&"localhost".to_string()));
        assert!(sans.ip_addrs.is_empty());
    }

    #[test]
    fn extract_sans_dns_and_ip() {
        let ca = generate_ca(None).unwrap();
        let extras = vec!["192.168.1.1".to_string(), "myhost.example.com".to_string()];
        let server = generate_server_cert(&ca, &extras).unwrap();
        let sans = extract_sans_from_cert(&server.cert_pem).unwrap();
        assert!(sans.dns_names.contains(&"localhost".to_string()));
        assert!(sans.dns_names.contains(&"myhost.example.com".to_string()));
        assert!(
            sans.ip_addrs
                .contains(&IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)))
        );
    }

    #[test]
    fn server_cert_needs_san_update_empty_extra() {
        let ca = generate_ca(None).unwrap();
        let server = generate_server_cert(&ca, &[]).unwrap();
        // Empty extra_sans always returns false
        assert!(!server_cert_needs_san_update(&server.cert_pem, &[]).unwrap());
    }

    #[test]
    fn server_cert_needs_san_update_matching() {
        let ca = generate_ca(None).unwrap();
        let extras = vec!["myhost.example.com".to_string()];
        let server = generate_server_cert(&ca, &extras).unwrap();
        // Cert already includes the requested SAN
        assert!(!server_cert_needs_san_update(&server.cert_pem, &extras).unwrap());
    }

    #[test]
    fn server_cert_needs_san_update_mismatched() {
        let ca = generate_ca(None).unwrap();
        let server = generate_server_cert(&ca, &[]).unwrap();
        let extras = vec!["new-host.example.com".to_string()];
        // Cert does not include the requested SAN
        assert!(server_cert_needs_san_update(&server.cert_pem, &extras).unwrap());
    }

    #[test]
    fn server_cert_needs_san_update_ip_missing() {
        let ca = generate_ca(None).unwrap();
        let server = generate_server_cert(&ca, &[]).unwrap();
        let extras = vec!["10.0.0.1".to_string()];
        // Cert does not include the requested IP SAN
        assert!(server_cert_needs_san_update(&server.cert_pem, &extras).unwrap());
    }

    #[test]
    fn cert_signed_by_ca_same() {
        let ca = generate_ca(None).unwrap();
        let server = generate_server_cert(&ca, &[]).unwrap();
        assert!(cert_signed_by_ca(&server.cert_pem, &ca.cert_pem).unwrap());
    }

    #[test]
    fn cert_signed_by_ca_different() {
        // Use CAs with different DNs so issuer check can distinguish them
        let key1 = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
        let mut params1 = CertificateParams::default();
        params1
            .distinguished_name
            .push(DnType::CommonName, "Test CA 1");
        params1.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        let _ca1_cert = params1.self_signed(&key1).unwrap();
        let issuer1 = Issuer::new(params1, key1);

        let server_key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
        let server_params = CertificateParams::new(vec!["localhost".into()]).unwrap();
        let server_cert = server_params.signed_by(&server_key, &issuer1).unwrap();
        let server_pem = server_cert.pem();

        let key2 = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
        let mut params2 = CertificateParams::default();
        params2
            .distinguished_name
            .push(DnType::CommonName, "Test CA 2");
        params2.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        let ca2_cert = params2.self_signed(&key2).unwrap();
        let ca2_pem = ca2_cert.pem();

        assert!(!cert_signed_by_ca(&server_pem, &ca2_pem).unwrap());
    }

    // --- AIA/CDP extension tests ---

    #[test]
    fn ca_with_pki_addr_has_aia_cdp() {
        let ca = generate_ca(Some("https://controller.example.com")).unwrap();
        let urls = extract_cert_pki_urls(&ca.cert_pem).unwrap();
        assert!(urls.has_extensions());
        assert_eq!(
            urls.ocsp_url.as_deref(),
            Some("https://controller.example.com/api/v1/pki/ocsp")
        );
        assert_eq!(
            urls.ca_issuers_url.as_deref(),
            Some("https://controller.example.com/api/v1/pki/ca.crt")
        );
        assert_eq!(
            urls.crl_url.as_deref(),
            Some("https://controller.example.com/api/v1/pki/ca.crl")
        );
    }

    #[test]
    fn ca_without_pki_addr_has_no_aia_cdp() {
        let ca = generate_ca(None).unwrap();
        let urls = extract_cert_pki_urls(&ca.cert_pem).unwrap();
        assert!(!urls.has_extensions());
        assert!(urls.ocsp_url.is_none());
        assert!(urls.ca_issuers_url.is_none());
        assert!(urls.crl_url.is_none());
    }

    #[test]
    fn extract_pki_urls_roundtrip() {
        let pki_addr = "https://my-controller:8443";
        let ca = generate_ca(Some(pki_addr)).unwrap();
        let urls = extract_cert_pki_urls(&ca.cert_pem).unwrap();
        assert_eq!(
            urls,
            CertPkiUrls {
                ocsp_url: Some(format!("{pki_addr}/api/v1/pki/ocsp")),
                ca_issuers_url: Some(format!("{pki_addr}/api/v1/pki/ca.crt")),
                crl_url: Some(format!("{pki_addr}/api/v1/pki/ca.crl")),
            }
        );
    }

    #[test]
    fn validate_ca_pki_addr_matching() {
        let url = "https://controller.example.com";
        let ca = generate_ca(Some(url)).unwrap();
        let dir = tempfile::tempdir().unwrap();
        assert!(validate_ca_pki_addr(&ca.cert_pem, Some(url), dir.path()).is_ok());
    }

    #[test]
    fn validate_ca_pki_addr_mismatched() {
        let ca = generate_ca(Some("https://old-url.example.com")).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let result = validate_ca_pki_addr(
            &ca.cert_pem,
            Some("https://new-url.example.com"),
            dir.path(),
        );
        assert!(result.is_err());
        let err = format!("{:?}", result.unwrap_err());
        assert!(err.contains("do not match"));
    }

    #[test]
    fn validate_ca_pki_addr_set_but_no_extensions() {
        let ca = generate_ca(None).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let result = validate_ca_pki_addr(&ca.cert_pem, Some("https://example.com"), dir.path());
        assert!(result.is_err());
        let err = format!("{:?}", result.unwrap_err());
        assert!(err.contains("no AIA/CDP extensions"));
    }

    #[test]
    fn validate_ca_pki_addr_not_set_but_has_extensions() {
        let ca = generate_ca(Some("https://example.com")).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let result = validate_ca_pki_addr(&ca.cert_pem, None, dir.path());
        assert!(result.is_err());
        let err = format!("{:?}", result.unwrap_err());
        assert!(err.contains("no --pki-addr is configured"));
    }

    #[test]
    fn validate_ca_pki_addr_neither_set() {
        let ca = generate_ca(None).unwrap();
        let dir = tempfile::tempdir().unwrap();
        assert!(validate_ca_pki_addr(&ca.cert_pem, None, dir.path()).is_ok());
    }

    #[test]
    fn rotate_ca_with_pki_addr_preserves_extensions() {
        let dir = tempfile::tempdir().unwrap();
        let pki = dir.path();
        let url = "https://controller.internal:8443";

        // Generate initial CA with pki_addr
        let _initial = load_or_generate_ca(pki, Some(url)).unwrap();

        // Rotate with same URL
        let state = rotate_ca(pki, Some(url)).unwrap();
        let urls = extract_cert_pki_urls(&state.active.cert_pem).unwrap();
        assert!(urls.has_extensions());
        assert_eq!(
            urls.ocsp_url.as_deref(),
            Some(&format!("{url}/api/v1/pki/ocsp") as &str)
        );
    }

    #[test]
    fn build_aia_der_produces_valid_extension() {
        // Generate a CA with extensions and verify parsing
        let ca = generate_ca(Some("https://test.example.com")).unwrap();
        let urls = extract_cert_pki_urls(&ca.cert_pem).unwrap();

        // Verify all three URLs are present and correctly formatted
        assert_eq!(
            urls.ocsp_url.unwrap(),
            "https://test.example.com/api/v1/pki/ocsp"
        );
        assert_eq!(
            urls.ca_issuers_url.unwrap(),
            "https://test.example.com/api/v1/pki/ca.crt"
        );
        assert_eq!(
            urls.crl_url.unwrap(),
            "https://test.example.com/api/v1/pki/ca.crl"
        );
    }
}

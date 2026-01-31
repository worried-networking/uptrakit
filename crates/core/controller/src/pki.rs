use std::fs;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, Issuer, KeyPair};
use rootcause::{Report, ReportConversion, markers, prelude::*};
use sha2::{Digest, Sha256};
use thiserror::Error;
use time::OffsetDateTime;

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
    pub fn to_snapshot(&self) -> Result<CaSnapshot> {
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
pub fn load_ca_state(pki: &Path) -> Result<CaState> {
    let active = load_or_generate_ca(pki)?;
    let previous = load_previous_ca(pki)?;
    let managed = is_ca_managed(pki);

    Ok(CaState {
        active,
        previous,
        managed,
    })
}

/// Load or generate the internal CA.
pub fn load_or_generate_ca(pki: &Path) -> Result<CaBundle> {
    let cert_path = pki.join("ca.crt");
    let key_path = pki.join("ca.key");

    if cert_path.exists() && key_path.exists() {
        load_ca(&cert_path, &key_path)
    } else {
        let bundle = generate_ca()?;
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

fn generate_ca() -> Result<CaBundle> {
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

    let sans = collect_sans(extra_sans)?;

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

struct SanCollection {
    dns_names: Vec<String>,
    ip_addrs: Vec<IpAddr>,
}

fn collect_sans(extra: &[String]) -> Result<SanCollection> {
    let mut dns_names = Vec::new();
    let mut ip_addrs = Vec::new();

    // Add system hostname
    let hostname = hostname::get()
        .context_transform(|e| PkiError::Hostname(e.to_string()))?
        .to_string_lossy()
        .to_string();

    if !hostname.is_empty() {
        dns_names.push(hostname.clone());
    }

    // Try to get FQDN — on many systems the hostname already is the FQDN.
    // We add both the short name and FQDN if they differ.
    if let Some(dot_pos) = hostname.find('.') {
        let short = &hostname[..dot_pos];
        if !short.is_empty() && short != hostname {
            // hostname is FQDN, also add short name
            dns_names.push(short.to_string());
        }
    }

    // Always include localhost
    if !dns_names.iter().any(|n| n == "localhost") {
        dns_names.push("localhost".to_string());
    }

    // Add extra SANs from CLI
    for san in extra {
        if let Ok(ip) = san.parse::<IpAddr>() {
            if !ip_addrs.contains(&ip) {
                ip_addrs.push(ip);
            }
        } else if !dns_names.iter().any(|n| n == san) {
            dns_names.push(san.clone());
        }
    }

    // Deduplicate dns_names
    dns_names.sort();
    dns_names.dedup();

    Ok(SanCollection {
        dns_names,
        ip_addrs,
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
pub fn rotate_ca(pki: &Path) -> Result<CaState> {
    let active_cert = pki.join("ca.crt");
    let active_key = pki.join("ca.key");
    let prev_cert = pki.join("ca-previous.crt");
    let prev_key = pki.join("ca-previous.key");

    // Move current active → previous
    fs::copy(&active_cert, &prev_cert).context_to::<PkiError>()?;
    fs::copy(&active_key, &prev_key).context_to::<PkiError>()?;

    // Generate new CA
    let new_ca = generate_ca()?;
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
        let ca = generate_ca().unwrap();
        assert!(ca.cert_pem.contains("BEGIN CERTIFICATE"));
        assert!(ca.key_pem.contains("BEGIN"));
    }

    #[test]
    fn ca_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let pki = dir.path();

        let ca1 = load_or_generate_ca(pki).unwrap();
        let ca2 = load_or_generate_ca(pki).unwrap();

        assert_eq!(ca1.cert_pem, ca2.cert_pem);
    }

    #[test]
    fn server_cert_signed_by_ca() {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

        let ca = generate_ca().unwrap();
        let bundle = generate_server_cert(&ca, &[]).unwrap();
        assert!(bundle.cert_pem.contains("BEGIN CERTIFICATE"));
        assert!(bundle.key_pem.contains("BEGIN"));

        // Should build a valid rustls config
        build_rustls_config(&bundle.cert_pem, &bundle.key_pem).unwrap();
    }

    #[test]
    fn server_cert_includes_localhost() {
        let sans = collect_sans(&[]).unwrap();
        assert!(sans.dns_names.contains(&"localhost".to_string()));
    }

    #[test]
    fn server_cert_includes_extra_sans() {
        let extras = vec![
            "myhost.example.com".to_string(),
            "192.168.1.1".to_string(),
            "::1".to_string(),
        ];
        let sans = collect_sans(&extras).unwrap();
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
        let sans = collect_sans(&extras).unwrap();
        let count = sans.dns_names.iter().filter(|n| **n == hostname).count();
        assert_eq!(count, 1);
    }

    #[test]
    fn expired_cert_detection() {
        let ca = generate_ca().unwrap();
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

        let ca = generate_ca().unwrap();
        let b1 = load_or_generate_server_cert(pki, &ca, &[]).unwrap();
        let b2 = load_or_generate_server_cert(pki, &ca, &[]).unwrap();

        assert_eq!(b1.cert_pem, b2.cert_pem);
    }

    #[test]
    fn san_ipv6_address() {
        let extras = vec!["fd00::1".to_string()];
        let sans = collect_sans(&extras).unwrap();
        assert!(
            sans.ip_addrs
                .contains(&IpAddr::V6(Ipv6Addr::new(0xfd00, 0, 0, 0, 0, 0, 0, 1)))
        );
    }

    #[test]
    fn ca_fingerprint_deterministic() {
        let ca = generate_ca().unwrap();
        let fp1 = ca_fingerprint(&ca.cert_pem).unwrap();
        let fp2 = ca_fingerprint(&ca.cert_pem).unwrap();
        assert_eq!(fp1, fp2);
        assert_eq!(fp1.len(), 64); // SHA-256 hex is 64 chars
    }

    #[test]
    fn ca_fingerprint_differs_between_cas() {
        let ca1 = generate_ca().unwrap();
        let ca2 = generate_ca().unwrap();
        let fp1 = ca_fingerprint(&ca1.cert_pem).unwrap();
        let fp2 = ca_fingerprint(&ca2.cert_pem).unwrap();
        assert_ne!(fp1, fp2);
    }

    #[test]
    fn should_rotate_ca_not_yet() {
        let ca = generate_ca().unwrap();
        // Fresh CA with 5 year validity should not need rotation
        assert!(!should_rotate_ca(&ca.cert_pem));
    }

    #[test]
    fn should_renew_server_cert_not_yet() {
        let ca = generate_ca().unwrap();
        let server = generate_server_cert(&ca, &[]).unwrap();
        // Fresh server cert with 90 day validity should not need renewal
        assert!(!should_renew_server_cert(&server.cert_pem));
    }

    #[test]
    fn ca_state_bundle_pem() {
        let ca1 = generate_ca().unwrap();
        let ca2 = generate_ca().unwrap();
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
        let ca = generate_ca().unwrap();
        let state = CaState {
            active: ca,
            previous: None,
            managed: true,
        };
        let snapshot = state.to_snapshot().unwrap();
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
        let initial = load_or_generate_ca(pki).unwrap();
        let initial_fp = ca_fingerprint(&initial.cert_pem).unwrap();

        // Rotate
        let state = rotate_ca(pki).unwrap();
        let new_fp = ca_fingerprint(&state.active.cert_pem).unwrap();
        assert_ne!(initial_fp, new_fp);

        let prev = state.previous.as_ref().unwrap();
        let prev_fp = ca_fingerprint(&prev.cert_pem).unwrap();
        assert_eq!(initial_fp, prev_fp);
    }
}

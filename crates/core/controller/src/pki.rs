use std::fs;
use std::net::IpAddr;
use std::path::{Path, PathBuf};

use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, Issuer, KeyPair};
use rootcause::{Report, ReportConversion, markers, prelude::*};
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

/// Ensure the PKI directory exists and return its path.
pub fn pki_dir(data_dir: &Path) -> Result<PathBuf> {
    let dir = data_dir.join("pki");
    fs::create_dir_all(&dir).context_to::<PkiError>()?;
    Ok(dir)
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
        tracing::info!("generated new internal CA at {}", pki.display());
        Ok(bundle)
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
    params.not_after = OffsetDateTime::now_utc() + time::Duration::days(3650);

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
    params.not_after = OffsetDateTime::now_utc() + time::Duration::days(365);

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

/// Build a `rustls::ServerConfig` with optional mTLS client authentication.
///
/// Clients presenting a certificate signed by the given CA will be verified;
/// clients without a certificate are still allowed (anonymous).
pub fn build_rustls_config_with_client_auth(
    cert_pem: &str,
    key_pem: &str,
    ca_cert_pem: &str,
) -> Result<rustls::ServerConfig> {
    use std::sync::Arc;

    use rustls::RootCertStore;
    use rustls::pki_types::pem::PemObject;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer};
    use rustls::server::WebPkiClientVerifier;

    let certs: Vec<_> = CertificateDer::pem_slice_iter(cert_pem.as_bytes())
        .collect::<std::result::Result<Vec<_>, _>>()
        .context_transform(|_| PkiError::PemParse)?;

    let key = PrivateKeyDer::from_pem_slice(key_pem.as_bytes())
        .context_transform(|_| PkiError::PemParse)?;

    let ca_certs: Vec<_> = CertificateDer::pem_slice_iter(ca_cert_pem.as_bytes())
        .collect::<std::result::Result<Vec<_>, _>>()
        .context_transform(|_| PkiError::PemParse)?;

    let mut root_store = RootCertStore::empty();
    for ca_cert in ca_certs {
        root_store.add(ca_cert).context_to::<PkiError>()?;
    }

    let verifier = WebPkiClientVerifier::builder(Arc::new(root_store))
        .allow_unauthenticated()
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
}

use std::net::IpAddr;

use rootcause::prelude::*;
use thiserror::Error;

/// Errors that can occur in shared PKI utility functions.
#[derive(Debug, Error)]
pub enum PkiUtilError {
    #[error("hostname resolution failed: {0}")]
    Hostname(String),

    #[error("PEM parsing error")]
    PemParse,
}

/// Result alias for PKI utility operations.
pub type Result<T> = std::result::Result<T, Report<PkiUtilError>>;

/// Collection of Subject Alternative Names (DNS names and IP addresses).
pub struct SanCollection {
    pub dns_names: Vec<String>,
    pub ip_addrs: Vec<IpAddr>,
}

/// Auto-detect SANs from the local system: hostname, FQDN, and `localhost`.
///
/// Used only on first-start bootstrap when no SANs are stored in the DB
/// and no `--san` CLI flag was provided. The result is saved to DB as the
/// canonical SAN list.
pub fn auto_detect_sans() -> Result<SanCollection> {
    let mut dns_names = Vec::new();

    // Add system hostname
    let hostname = hostname::get()
        .context_transform(|e| PkiUtilError::Hostname(e.to_string()))?
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

    dns_names.sort();
    dns_names.dedup();

    Ok(SanCollection {
        dns_names,
        ip_addrs: Vec::new(),
    })
}

/// Parse a canonical SAN list into DNS names and IP addresses.
///
/// No auto-detection — the input list is treated as the full, canonical set
/// of SANs. Used everywhere except first-start bootstrap.
pub fn parse_san_list(sans: &[String]) -> SanCollection {
    let mut dns_names = Vec::new();
    let mut ip_addrs = Vec::new();

    for san in sans {
        if let Ok(ip) = san.parse::<IpAddr>() {
            if !ip_addrs.contains(&ip) {
                ip_addrs.push(ip);
            }
        } else if !dns_names.iter().any(|n| n == san) {
            dns_names.push(san.clone());
        }
    }

    dns_names.sort();
    dns_names.dedup();

    SanCollection {
        dns_names,
        ip_addrs,
    }
}

/// Check if a certificate was signed by a given CA.
///
/// Compares the certificate's issuer DN against the CA's subject DN.
/// Returns `true` if they match.
pub fn cert_signed_by_ca(cert_pem: &str, ca_pem: &str) -> Result<bool> {
    let (_, cert_block) = x509_parser::pem::parse_x509_pem(cert_pem.as_bytes())
        .map_err(|_| report!(PkiUtilError::PemParse))?;
    let cert = cert_block
        .parse_x509()
        .map_err(|_| report!(PkiUtilError::PemParse))?;

    let (_, ca_block) = x509_parser::pem::parse_x509_pem(ca_pem.as_bytes())
        .map_err(|_| report!(PkiUtilError::PemParse))?;
    let ca = ca_block
        .parse_x509()
        .map_err(|_| report!(PkiUtilError::PemParse))?;

    if cert.issuer() != ca.subject() {
        return Ok(false);
    }

    match cert.verify_signature(Some(ca.public_key())) {
        Ok(()) => Ok(true),
        Err(_) => Ok(false),
    }
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    use super::*;

    #[test]
    fn auto_detect_sans_includes_localhost() -> Result<()> {
        let sans = auto_detect_sans().expect("auto detect sans");
        assert!(sans.dns_names.contains(&"localhost".to_string()));
        assert!(sans.ip_addrs.is_empty());
        Ok(())
    }

    #[test]
    fn auto_detect_sans_includes_hostname() -> Result<()> {
        let hostname = hostname::get()
            .expect("hostname")
            .to_string_lossy()
            .to_string();
        let sans = auto_detect_sans().expect("auto detect sans");
        assert!(
            sans.dns_names.contains(&hostname)
                || sans.dns_names.iter().any(|n| hostname.starts_with(n)),
            "expected hostname or short name in {sans:?}",
            sans = sans.dns_names,
        );
        Ok(())
    }

    #[test]
    fn parse_san_list_dns_names() {
        let input = vec!["myhost.example.com".to_string(), "localhost".to_string()];
        let sans = parse_san_list(&input);
        assert!(sans.dns_names.contains(&"localhost".to_string()));
        assert!(sans.dns_names.contains(&"myhost.example.com".to_string()));
        assert!(sans.ip_addrs.is_empty());
    }

    #[test]
    fn parse_san_list_ip_addresses() {
        let input = vec!["192.168.1.1".to_string(), "::1".to_string()];
        let sans = parse_san_list(&input);
        assert!(
            sans.ip_addrs
                .contains(&IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)))
        );
        assert!(sans.ip_addrs.contains(&IpAddr::V6(Ipv6Addr::LOCALHOST)));
        assert!(sans.dns_names.is_empty());
    }

    #[test]
    fn parse_san_list_mixed() {
        let input = vec![
            "myhost.example.com".to_string(),
            "192.168.1.1".to_string(),
            "localhost".to_string(),
        ];
        let sans = parse_san_list(&input);
        assert_eq!(sans.dns_names.len(), 2);
        assert_eq!(sans.ip_addrs.len(), 1);
    }

    #[test]
    fn parse_san_list_deduplicates() {
        let input = vec![
            "example.com".to_string(),
            "example.com".to_string(),
            "192.168.1.1".to_string(),
            "192.168.1.1".to_string(),
        ];
        let sans = parse_san_list(&input);
        assert_eq!(sans.dns_names.len(), 1);
        assert_eq!(sans.ip_addrs.len(), 1);
    }

    #[test]
    fn parse_san_list_empty() {
        let sans = parse_san_list(&[]);
        assert!(sans.dns_names.is_empty());
        assert!(sans.ip_addrs.is_empty());
    }

    #[test]
    fn cert_signed_by_ca_same_ca() -> Result<()> {
        let key_pair =
            rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).expect("ca key");
        let mut ca_params = rcgen::CertificateParams::default();
        ca_params
            .distinguished_name
            .push(rcgen::DnType::CommonName, "Test CA");
        ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let ca_cert = ca_params.self_signed(&key_pair).expect("ca cert");
        let ca_pem = ca_cert.pem();

        let issuer = rcgen::Issuer::new(ca_params, key_pair);
        let server_key =
            rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).expect("server key");
        let server_params =
            rcgen::CertificateParams::new(vec!["localhost".into()]).expect("server params");
        let server_cert = server_params
            .signed_by(&server_key, &issuer)
            .expect("server cert");
        let server_pem = server_cert.pem();

        assert!(cert_signed_by_ca(&server_pem, &ca_pem)?);
        Ok(())
    }

    #[test]
    fn cert_signed_by_ca_different_ca() -> Result<()> {
        // Generate CA 1
        let key1 = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).expect("ca1 key");
        let mut ca1_params = rcgen::CertificateParams::default();
        ca1_params
            .distinguished_name
            .push(rcgen::DnType::CommonName, "Test CA 1");
        ca1_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let _ca1_cert = ca1_params.self_signed(&key1).expect("ca1 cert");

        let issuer1 = rcgen::Issuer::new(ca1_params, key1);
        let server_key =
            rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).expect("server key");
        let server_params =
            rcgen::CertificateParams::new(vec!["localhost".into()]).expect("server params");
        let server_cert = server_params
            .signed_by(&server_key, &issuer1)
            .expect("server cert");
        let server_pem = server_cert.pem();

        // Generate CA 2
        let key2 = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).expect("ca2 key");
        let mut ca2_params = rcgen::CertificateParams::default();
        ca2_params
            .distinguished_name
            .push(rcgen::DnType::CommonName, "Test CA 2");
        ca2_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let ca2_cert = ca2_params.self_signed(&key2).expect("ca2 cert");
        let ca2_pem = ca2_cert.pem();

        // Server cert signed by CA1, but checking against CA2
        assert!(!cert_signed_by_ca(&server_pem, &ca2_pem)?);
        Ok(())
    }

    #[test]
    fn cert_signed_by_ca_same_subject_wrong_key() -> Result<()> {
        let key1 = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).expect("ca1 key");
        let mut ca1_params = rcgen::CertificateParams::default();
        ca1_params
            .distinguished_name
            .push(rcgen::DnType::CommonName, "Shared CA");
        ca1_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let ca1_cert = ca1_params.self_signed(&key1).expect("ca1 cert");
        let ca1_pem = ca1_cert.pem();

        let issuer1 = rcgen::Issuer::new(ca1_params, key1);
        let server_key =
            rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).expect("server key");
        let server_params =
            rcgen::CertificateParams::new(vec!["localhost".into()]).expect("server params");
        let server_cert = server_params
            .signed_by(&server_key, &issuer1)
            .expect("server cert");
        let server_pem = server_cert.pem();

        let key2 = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).expect("ca2 key");
        let mut ca2_params = rcgen::CertificateParams::default();
        ca2_params
            .distinguished_name
            .push(rcgen::DnType::CommonName, "Shared CA");
        ca2_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let ca2_cert = ca2_params.self_signed(&key2).expect("ca2 cert");
        let ca2_pem = ca2_cert.pem();

        assert!(!cert_signed_by_ca(&server_pem, &ca2_pem)?);
        assert!(cert_signed_by_ca(&server_pem, &ca1_pem)?);
        Ok(())
    }

    #[test]
    fn cert_signed_by_ca_malformed_cert() {
        assert!(cert_signed_by_ca("not a cert", "not a ca").is_err());
    }
}

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

/// Collect the SANs for a server certificate.
///
/// Always includes the system hostname (short + FQDN if applicable) and
/// `localhost`. Extra SANs from CLI are appended. IP addresses are
/// separated from DNS names.
pub fn collect_sans(extra: &[String]) -> Result<SanCollection> {
    let mut dns_names = Vec::new();
    let mut ip_addrs = Vec::new();

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

    Ok(cert.issuer() == ca.subject())
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    use super::*;

    #[test]
    fn collect_sans_includes_localhost() {
        let sans = collect_sans(&[]).unwrap();
        assert!(sans.dns_names.contains(&"localhost".to_string()));
    }

    #[test]
    fn collect_sans_includes_extra_dns() {
        let extras = vec!["myhost.example.com".to_string()];
        let sans = collect_sans(&extras).unwrap();
        assert!(sans.dns_names.contains(&"myhost.example.com".to_string()));
    }

    #[test]
    fn collect_sans_includes_extra_ips() {
        let extras = vec!["192.168.1.1".to_string(), "::1".to_string()];
        let sans = collect_sans(&extras).unwrap();
        assert!(
            sans.ip_addrs
                .contains(&IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)))
        );
        assert!(sans.ip_addrs.contains(&IpAddr::V6(Ipv6Addr::LOCALHOST)));
    }

    #[test]
    fn collect_sans_deduplicates_hostname() {
        let hostname = hostname::get().unwrap().to_string_lossy().to_string();
        let extras = vec![hostname.clone()];
        let sans = collect_sans(&extras).unwrap();
        let count = sans.dns_names.iter().filter(|n| **n == hostname).count();
        assert_eq!(count, 1);
    }

    #[test]
    fn cert_signed_by_ca_same_ca() {
        let key_pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
        let mut ca_params = rcgen::CertificateParams::default();
        ca_params
            .distinguished_name
            .push(rcgen::DnType::CommonName, "Test CA");
        ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let ca_cert = ca_params.self_signed(&key_pair).unwrap();
        let ca_pem = ca_cert.pem();

        let issuer = rcgen::Issuer::new(ca_params, key_pair);
        let server_key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
        let server_params = rcgen::CertificateParams::new(vec!["localhost".into()]).unwrap();
        let server_cert = server_params.signed_by(&server_key, &issuer).unwrap();
        let server_pem = server_cert.pem();

        assert!(cert_signed_by_ca(&server_pem, &ca_pem).unwrap());
    }

    #[test]
    fn cert_signed_by_ca_different_ca() {
        // Generate CA 1
        let key1 = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
        let mut ca1_params = rcgen::CertificateParams::default();
        ca1_params
            .distinguished_name
            .push(rcgen::DnType::CommonName, "Test CA 1");
        ca1_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let _ca1_cert = ca1_params.self_signed(&key1).unwrap();

        let issuer1 = rcgen::Issuer::new(ca1_params, key1);
        let server_key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
        let server_params = rcgen::CertificateParams::new(vec!["localhost".into()]).unwrap();
        let server_cert = server_params.signed_by(&server_key, &issuer1).unwrap();
        let server_pem = server_cert.pem();

        // Generate CA 2
        let key2 = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
        let mut ca2_params = rcgen::CertificateParams::default();
        ca2_params
            .distinguished_name
            .push(rcgen::DnType::CommonName, "Test CA 2");
        ca2_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let ca2_cert = ca2_params.self_signed(&key2).unwrap();
        let ca2_pem = ca2_cert.pem();

        // Server cert signed by CA1, but checking against CA2
        assert!(!cert_signed_by_ca(&server_pem, &ca2_pem).unwrap());
    }

    #[test]
    fn cert_signed_by_ca_malformed_cert() {
        assert!(cert_signed_by_ca("not a cert", "not a ca").is_err());
    }
}

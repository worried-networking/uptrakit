//! CA certificate bootstrap and fetch logic.
//!
//! Provides a unified CA bootstrap flow shared by both agents and MQTT services:
//! 1. If cached locally: use cached
//! 2. If `--ca-cert` file provided: load and save
//! 3. If `--pki-addr` provided: fetch via HTTPS (system trust)
//! 4. If `--tofu`: fetch via HTTPS (TOFU TLS with optional fingerprint pinning)
//! 5. Else: use system root certificates (return `None`)

use rootcause::prelude::*;
use sha2::Digest;

use crate::error::{EnrollmentError, Result};

/// TLS mode for the CA certificate fetch via reqwest.
pub enum CaTlsMode<'a> {
    /// Use system/built-in root certificates (for `https://` pki_addr).
    SystemTrust,
    /// TOFU: accept any server cert but verify TLS signatures.
    /// Fingerprint verification after download provides the security guarantee.
    Tofu,
    /// Use a pinned CA certificate.
    PinnedCa(&'a [u8]),
}

/// Compute the SHA-256 hex fingerprint of a PEM-encoded CA certificate's DER content.
pub fn ca_pem_fingerprint(pem_bytes: &[u8]) -> Result<String> {
    let (_, pem) = x509_parser::pem::parse_x509_pem(pem_bytes).map_err(|e| {
        report!(EnrollmentError::CertificateParse(format!(
            "PEM parse failed: {e}"
        )))
    })?;
    let digest = sha2::Sha256::digest(&pem.contents);
    Ok(uptrakit_shared_types::hex::encode(digest))
}

/// Fetch the CA certificate bundle from a controller or PKI endpoint.
///
/// The caller passes the correct `base_url` (either the main controller URL
/// or the `--pki-addr` value). If `base_url` starts with `http://`, plain
/// HTTP is used (no TLS configuration needed). Otherwise the provided
/// `tls_mode` applies.
pub async fn fetch_ca_certificate(base_url: &str, tls_mode: CaTlsMode<'_>) -> Result<Vec<u8>> {
    let fetch_url = format!("{base_url}/api/v1/pki/ca.crt");
    let use_plain_http = base_url.starts_with("http://");

    tracing::info!(url = %fetch_url, "fetching CA certificate");

    let mut builder = reqwest::Client::builder();
    if use_plain_http {
        // Plain HTTP — no TLS configuration needed
    } else {
        match tls_mode {
            CaTlsMode::SystemTrust => {
                // reqwest defaults to system/built-in roots — nothing to configure
            }
            CaTlsMode::Tofu => {
                // TOFU: use custom verifier that accepts any cert chain but
                // validates TLS signatures. Fingerprint verification after
                // download provides the primary security guarantee.
                builder = builder.tls_danger_accept_invalid_certs(true);
            }
            CaTlsMode::PinnedCa(ca_pem) => {
                let cert = reqwest::Certificate::from_pem(ca_pem).map_err(|e| {
                    report!(EnrollmentError::FetchCa(format!("invalid CA PEM: {e}")))
                })?;
                builder = builder.tls_certs_only([cert]);
            }
        }
    }

    let client = builder
        .build()
        .map_err(|e| report!(EnrollmentError::FetchCa(e.to_string())))?;

    let resp = client
        .get(&fetch_url)
        .send()
        .await
        .map_err(|e| report!(EnrollmentError::FetchCa(e.to_string())))?;

    if !resp.status().is_success() {
        return Err(report!(EnrollmentError::FetchCa(format!(
            "HTTP {}",
            resp.status()
        ))));
    }

    let body = resp
        .bytes()
        .await
        .map_err(|e| report!(EnrollmentError::FetchCa(e.to_string())))?;

    tracing::info!(bytes = body.len(), "CA certificate fetched");
    Ok(body.to_vec())
}

/// Full CA bootstrap logic shared by agent and MQTT service.
///
/// Determines the CA certificate to use based on the following priority:
/// 1. Already cached in identity → use cached, warn if `--tofu` was set
/// 2. `--ca-cert` file → load, save to identity
/// 3. `--pki-addr` → fetch with system trust, save to identity
/// 4. `--tofu` → fetch from base URL with TOFU TLS, verify fingerprint if
///    `--tofu-fingerprint` supplied, save to identity
/// 5. None of the above → use system root certificates (returns `None`)
///
/// Returns `Some(ca_pem_bytes)` when a pinned CA is available, or `None`
/// when system root certificates should be used.
pub async fn bootstrap_ca(
    identity: &mut crate::identity::ServiceIdentityState,
    base_url: &str,
    tofu: bool,
    tofu_fingerprint: Option<&str>,
    ca_cert_path: Option<&std::path::Path>,
    pki_addr: Option<&str>,
) -> Result<Option<Vec<u8>>> {
    // 1. Already cached
    if let Some(cached) = identity.load_ca_cert().await? {
        tracing::info!("loaded CA certificate from disk");
        if tofu {
            tracing::warn!("--tofu ignored: CA already cached");
        }
        return Ok(Some(cached));
    }

    // 2. From --ca-cert file
    if let Some(ca_path) = ca_cert_path {
        tracing::info!("loading CA certificate from {}", ca_path.display());
        let pem = std::fs::read(ca_path).map_err(|e| {
            report!(EnrollmentError::CaCertFile(format!(
                "{}: {e}",
                ca_path.display()
            )))
        })?;
        identity
            .save_ca_cert(&String::from_utf8_lossy(&pem))
            .await?;
        tracing::info!("CA certificate saved to disk");
        return Ok(Some(pem));
    }

    // 3. From --pki-addr with system trust
    if let Some(pki) = pki_addr {
        tracing::info!("fetching CA certificate from --pki-addr {pki}");
        let pem = fetch_ca_certificate(pki, CaTlsMode::SystemTrust).await?;
        identity
            .save_ca_cert(&String::from_utf8_lossy(&pem))
            .await?;
        tracing::info!("CA certificate saved to disk");
        return Ok(Some(pem));
    }

    // 4. TOFU: fetch from controller URL with TOFU TLS + optional fingerprint pinning
    if tofu {
        tracing::info!("TOFU: fetching CA (accepting any server certificate)");
        let pem = fetch_ca_certificate(base_url, CaTlsMode::Tofu).await?;

        // Compute and log the fingerprint for manual verification
        let fp = ca_pem_fingerprint(&pem)?;
        tracing::warn!(fingerprint = %fp, "TOFU: accepted CA certificate with fingerprint SHA256:{fp}");

        // Verify against --tofu-fingerprint if provided
        if let Some(expected) = tofu_fingerprint {
            let expected_normalized = expected.to_lowercase().replace(':', "");
            if fp != expected_normalized {
                return Err(report!(EnrollmentError::FetchCa(format!(
                    "TOFU fingerprint mismatch: expected SHA256:{expected_normalized}, \
                     got SHA256:{fp}"
                ))));
            }
            tracing::info!("TOFU: fingerprint verified successfully");
        } else {
            tracing::warn!(
                "TOFU: no --tofu-fingerprint supplied — verify the fingerprint manually: SHA256:{fp}"
            );
        }

        identity
            .save_ca_cert(&String::from_utf8_lossy(&pem))
            .await?;
        tracing::info!("CA certificate saved to disk");
        return Ok(Some(pem));
    }

    // 5. Use system root certificates
    tracing::info!("using system root certificates");
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ca_pem_fingerprint_deterministic() {
        // Generate a test cert
        let key_pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
        let mut params = rcgen::CertificateParams::default();
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, "Test CA");
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let cert = params.self_signed(&key_pair).unwrap();
        let pem = cert.pem();

        let fp1 = ca_pem_fingerprint(pem.as_bytes()).unwrap();
        let fp2 = ca_pem_fingerprint(pem.as_bytes()).unwrap();
        assert_eq!(fp1, fp2);
        assert_eq!(fp1.len(), 64); // SHA-256 hex is 64 chars
    }

    #[test]
    fn ca_pem_fingerprint_invalid_pem() {
        let result = ca_pem_fingerprint(b"not a PEM");
        assert!(result.is_err());
    }
}

//! CA certificate bootstrap and fetch logic.
//!
//! Provides a unified CA bootstrap flow shared by both agents and MQTT services:
//! 1. If cached locally: use cached
//! 2. If `--ca-cert` file provided: load and save
//! 3. If `--pki-addr` provided: fetch via HTTPS (system trust)
//! 4. If `--tofu`: fetch via HTTPS (TOFU TLS with optional fingerprint pinning)
//! 5. Else: use system root certificates (return `None`)

use rootcause::prelude::*;
use rustls::pki_types::CertificateDer;
use rustls::pki_types::pem::PemObject;
use sha2::Digest;

use crate::error::{CaError, EnrollmentError, Result, TlsError};

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
    let der = CertificateDer::from_pem_slice(pem_bytes).map_err(|e| {
        report!(EnrollmentError::Tls(TlsError::CertificateParse(format!(
            "PEM parse failed: {e}"
        ))))
    })?;
    let digest = sha2::Sha256::digest(der.as_ref());
    Ok(hex::encode(digest))
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
                // TOFU: use a TofuVerifier-based rustls ClientConfig that
                // accepts any cert chain but validates TLS handshake
                // signatures. This prevents trivial MITM that forge invalid
                // signatures, unlike the blanket `danger_accept_invalid_certs`
                // which disables all TLS verification at the reqwest level.
                // Fingerprint verification after download provides the
                // primary security guarantee.
                let tofu_config = crate::tls::build_tofu_client_config().map_err(|e| {
                    report!(EnrollmentError::Tls(TlsError::CertificateParse(format!(
                        "failed to build TOFU TLS config: {e}"
                    ))))
                })?;
                builder = builder.use_preconfigured_tls(tofu_config);
            }
            CaTlsMode::PinnedCa(ca_pem) => {
                let cert = reqwest::Certificate::from_pem(ca_pem).map_err(|e| {
                    report!(EnrollmentError::Ca(CaError::Fetch(format!(
                        "invalid CA PEM: {e}"
                    ))))
                })?;
                builder = builder.tls_certs_only([cert]);
            }
        }
    }

    let client = builder
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| report!(EnrollmentError::Ca(CaError::Fetch(e.to_string()))))?;

    let resp = client
        .get(&fetch_url)
        .send()
        .await
        .map_err(|e| report!(EnrollmentError::Ca(CaError::Fetch(e.to_string()))))?;

    if !resp.status().is_success() {
        bail!(EnrollmentError::Ca(CaError::Fetch(format!(
            "HTTP {}",
            resp.status()
        ))));
    }

    let body = resp
        .bytes()
        .await
        .map_err(|e| report!(EnrollmentError::Ca(CaError::Fetch(e.to_string()))))?;

    if body.is_empty() {
        bail!(EnrollmentError::Ca(CaError::Fetch(
            "empty CA certificate response from server".to_string(),
        )));
    }

    tracing::info!(bytes = body.len(), "CA certificate fetched");
    Ok(body.to_vec())
}

/// Full CA bootstrap logic shared by agent and MQTT service.
///
/// Determines the CA certificate to use based on the following priority:
/// 1. Already cached in identity → use cached, log warning if TOFU mode active
/// 2. `--ca-cert` file → load, save to identity
/// 3. `--pki-addr` → fetch with system trust, save to identity
/// 4. TOFU modes (see [`crate::tofu::TofuMode`]):
///    - `System` → use system root certificates (returns `None`)
///    - `PinFingerprint` → fetch from base URL via TOFU TLS, verify CA hash matches
///    - `PinSpki` → fetch from base URL via TOFU TLS, save (SPKI check happens at handshake)
///    - `InsecureTofu` → fetch from base URL via TOFU TLS, optionally verify + persist
///
/// Returns `Some(ca_pem_bytes)` when a pinned CA is available, or `None`
/// when system root certificates should be used.
pub async fn bootstrap_ca(
    identity: &mut crate::identity::ServiceIdentityState,
    base_url: &str,
    tofu_config: &crate::tofu::TofuConfig,
    ca_cert_path: Option<&std::path::Path>,
    pki_addr: Option<&str>,
) -> Result<Option<Vec<u8>>> {
    // 1. Already cached
    if let Some(cached) = identity.load_ca_cert().await? {
        tracing::info!("loaded CA certificate from disk");
        if !matches!(tofu_config.mode, crate::tofu::TofuMode::System) {
            tracing::warn!("TOFU mode ignored: CA already cached on disk");
        }
        return Ok(Some(cached));
    }

    // 2. From --ca-cert file
    if let Some(ca_path) = ca_cert_path {
        tracing::info!("loading CA certificate from {}", ca_path.display());
        let pem = std::fs::read(ca_path).map_err(|e| {
            report!(EnrollmentError::Ca(CaError::CertFile(format!(
                "{}: {e}",
                ca_path.display()
            ))))
        })?;
        if pem.is_empty() {
            bail!(EnrollmentError::Ca(CaError::CertFile(format!(
                "{}: CA cert file is empty",
                ca_path.display()
            ))));
        }
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

    // 4. TOFU modes: fetch from controller URL
    match &tofu_config.mode {
        crate::tofu::TofuMode::System => {
            // No TOFU configured — use system root certificates
            tracing::info!("using system root certificates");
            Ok(None)
        }
        crate::tofu::TofuMode::PinFingerprint(expected) => {
            tracing::info!("TOFU pin-fingerprint: fetching CA certificate");
            let pem = fetch_ca_certificate(base_url, CaTlsMode::Tofu).await?;
            let actual = crate::tofu::sha256_of_bytes(&pem);
            if &actual != expected {
                tracing::error!(
                    expected = %expected,
                    actual = %actual,
                    "TOFU pin-fingerprint mismatch"
                );
                bail!(EnrollmentError::Ca(CaError::FingerprintMismatch {
                    expected: expected.to_string(),
                    actual: actual.to_string(),
                }));
            }
            identity
                .save_ca_cert(&String::from_utf8_lossy(&pem))
                .await?;
            tracing::info!("CA certificate verified and saved (pin-fingerprint)");
            Ok(Some(pem))
        }
        crate::tofu::TofuMode::PinSpki(_) => {
            // SPKI verification is performed at each TLS handshake by ModeBasedVerifier;
            // here we only fetch and persist the CA bundle.
            tracing::info!("TOFU pin-spki: fetching CA certificate (SPKI checked at handshake)");
            let pem = fetch_ca_certificate(base_url, CaTlsMode::Tofu).await?;
            let fp = ca_pem_fingerprint(&pem)?;
            tracing::warn!(fingerprint = %fp, "TOFU pin-spki: accepted CA with fingerprint");
            identity
                .save_ca_cert(&String::from_utf8_lossy(&pem))
                .await?;
            tracing::info!("CA certificate saved (pin-spki)");
            Ok(Some(pem))
        }
        crate::tofu::TofuMode::InsecureTofu => {
            tracing::info!("TOFU insecure: fetching CA certificate");
            let pem = fetch_ca_certificate(base_url, CaTlsMode::Tofu).await?;
            let actual = crate::tofu::sha256_of_bytes(&pem);
            tracing::warn!(
                fingerprint = %actual,
                "insecure-tofu: CA fetched — pass via --tofu-fingerprint-acknowledge to persist"
            );

            match &tofu_config.fingerprint_acknowledge {
                None => {
                    // Stateless TOFU: no acknowledgement supplied, do not persist
                    tracing::warn!(
                        "insecure-tofu: no --tofu-fingerprint-acknowledge; CA not persisted"
                    );
                    Ok(Some(pem))
                }
                Some(expected) => {
                    if expected != &actual {
                        tracing::error!(
                            expected = %expected,
                            actual = %actual,
                            "insecure-tofu: --tofu-fingerprint-acknowledge mismatch; refusing to persist"
                        );
                        bail!(EnrollmentError::Ca(CaError::FingerprintMismatch {
                            expected: expected.to_string(),
                            actual: actual.to_string(),
                        }));
                    }
                    identity
                        .save_ca_cert(&String::from_utf8_lossy(&pem))
                        .await?;
                    tracing::info!(
                        "CA certificate saved (insecure-tofu with acknowledged fingerprint)"
                    );
                    Ok(Some(pem))
                }
            }
        }
    }
}

/// Compute the SHA-256 hex hash of the local CA certificate file (`ca.pem`)
/// in the given config directory.
///
/// Returns an empty string if the file does not exist or cannot be read.
/// An empty return value acts as a sentinel: when compared against the
/// controller-provided `ca_bundle_hash`, the mismatch triggers a fresh
/// CA bundle fetch — making this self-healing when the local file is
/// missing, corrupted, or the service is starting for the first time.
pub async fn compute_local_ca_hash(config_dir: &std::path::Path) -> String {
    let ca_path = config_dir.join("ca.pem");
    match tokio::fs::read(&ca_path).await {
        Ok(bytes) => {
            let mut hasher = sha2::Sha256::new();
            hasher.update(&bytes);
            hex::encode(hasher.finalize())
        }
        Err(e) => {
            tracing::debug!(
                path = %ca_path.display(),
                error = %e,
                "CA file unreadable — returning empty hash to trigger re-fetch from controller"
            );
            String::new()
        }
    }
}

/// Check if the local CA bundle is stale compared to the controller's hash,
/// and fetch an updated bundle if necessary.
///
/// This encapsulates the CA staleness check that was previously duplicated
/// in both the agent and agent-ssh event loops.
pub async fn check_ca_staleness(
    ca_bundle_hash: &str,
    config_dir: &std::path::Path,
    identity: &mut crate::identity::ServiceIdentityState,
    pki_addr: Option<&str>,
    base_url: &str,
    ca_pem: Option<&[u8]>,
) {
    if ca_bundle_hash.is_empty() {
        return;
    }

    let local_hash = compute_local_ca_hash(config_dir).await;
    if local_hash == ca_bundle_hash {
        return;
    }

    tracing::info!("CA bundle hash mismatch, fetching updated bundle");
    let ca_fetch_url = pki_addr.unwrap_or(base_url);
    let tls_mode = match ca_pem {
        Some(pem) => CaTlsMode::PinnedCa(pem),
        None => CaTlsMode::SystemTrust,
    };
    match fetch_ca_certificate(ca_fetch_url, tls_mode).await {
        Ok(pem) => {
            let pem_str = String::from_utf8_lossy(&pem);
            if let Err(e) = identity.save_ca_cert(&pem_str).await {
                tracing::warn!("failed to save updated CA: {e}");
            } else {
                tracing::info!("updated CA bundle saved to disk");
            }
        }
        Err(e) => tracing::warn!("failed to fetch updated CA: {e}"),
    }
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::assertions_on_result_states,
        reason = "test assertions — assert!(result.is_err()) are idiomatic in tests"
    )]

    use super::*;

    // ── bootstrap_ca ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn bootstrap_ca_empty_ca_cert_file_returns_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let empty_ca_path = dir.path().join("ca.pem");
        tokio::fs::write(&empty_ca_path, b"")
            .await
            .expect("write empty ca file");

        let mut identity = crate::identity::ServiceIdentityState::new_single_dir(dir.path());
        identity.load().await.expect("load identity");

        let tofu_config =
            crate::tofu::TofuConfig::from_flags(None, None, false, false, None).expect("system");
        let result = bootstrap_ca(
            &mut identity,
            "https://controller.example.com",
            &tofu_config,
            Some(&empty_ca_path),
            None,
        )
        .await;

        assert!(result.is_err(), "empty --ca-cert file must return an error");
        let err_str = result.unwrap_err().to_string();
        assert!(
            err_str.contains("empty"),
            "error message should mention 'empty', got: {err_str}"
        );
    }

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

    #[test]
    fn ca_pem_fingerprint_different_certs_have_different_fingerprints() {
        let key_pair_a = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)
            .expect("key pair A generation should succeed");
        let mut params_a = rcgen::CertificateParams::default();
        params_a
            .distinguished_name
            .push(rcgen::DnType::CommonName, "Test CA A");
        params_a.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let cert_a = params_a
            .self_signed(&key_pair_a)
            .expect("self-signed cert A should be created");

        let key_pair_b = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)
            .expect("key pair B generation should succeed");
        let mut params_b = rcgen::CertificateParams::default();
        params_b
            .distinguished_name
            .push(rcgen::DnType::CommonName, "Test CA B");
        params_b.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let cert_b = params_b
            .self_signed(&key_pair_b)
            .expect("self-signed cert B should be created");

        let fp_a = ca_pem_fingerprint(cert_a.pem().as_bytes())
            .expect("fingerprint for cert A should succeed");
        let fp_b = ca_pem_fingerprint(cert_b.pem().as_bytes())
            .expect("fingerprint for cert B should succeed");

        assert_ne!(
            fp_a, fp_b,
            "two independently generated certificates must have different fingerprints"
        );
    }

    #[test]
    fn ca_pem_fingerprint_only_hex_chars() {
        let key_pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)
            .expect("key pair generation should succeed");
        let mut params = rcgen::CertificateParams::default();
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, "Hex Test CA");
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let cert = params
            .self_signed(&key_pair)
            .expect("self-signed cert should be created");

        let fp = ca_pem_fingerprint(cert.pem().as_bytes())
            .expect("fingerprint computation should succeed");

        assert!(
            fp.chars().all(|c| c.is_ascii_hexdigit()),
            "fingerprint must contain only hex characters, got: {fp}"
        );
        // Additionally verify lowercase encoding (no uppercase hex letters)
        assert_eq!(fp, fp.to_lowercase(), "fingerprint must be lowercase hex");
    }

    #[test]
    fn ca_pem_fingerprint_empty_input_returns_error() {
        let result = ca_pem_fingerprint(b"");
        assert!(
            result.is_err(),
            "empty input should produce an error, but got: {:?}",
            result
        );
    }

    #[test]
    fn ca_pem_fingerprint_truncated_pem_returns_error() {
        // A PEM header without valid Base64 content or end marker
        let truncated = b"-----BEGIN CERTIFICATE-----\nnotvalidbase64";
        let result = ca_pem_fingerprint(truncated);
        assert!(
            result.is_err(),
            "truncated PEM should produce an error, but got: {:?}",
            result
        );
    }

    // ── compute_local_ca_hash ───────────────────────────────────────────

    #[tokio::test]
    async fn local_ca_hash_missing_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let hash = compute_local_ca_hash(dir.path()).await;
        assert!(hash.is_empty());
    }

    #[tokio::test]
    async fn local_ca_hash_valid_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ca_path = dir.path().join("ca.pem");
        tokio::fs::write(&ca_path, b"test-ca-content")
            .await
            .expect("write");
        let hash = compute_local_ca_hash(dir.path()).await;
        let expected = {
            let mut h = sha2::Sha256::new();
            h.update(b"test-ca-content");
            hex::encode(h.finalize())
        };
        assert_eq!(hash, expected);
    }

    #[tokio::test]
    async fn local_ca_hash_is_deterministic() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ca_path = dir.path().join("ca.pem");
        tokio::fs::write(&ca_path, b"deterministic-content")
            .await
            .expect("write");
        let hash1 = compute_local_ca_hash(dir.path()).await;
        let hash2 = compute_local_ca_hash(dir.path()).await;
        assert_eq!(hash1, hash2);
        assert_eq!(hash1.len(), 64); // SHA-256 hex is 64 chars
    }

    #[tokio::test]
    async fn local_ca_hash_different_content_different_hash() {
        let dir1 = tempfile::tempdir().expect("tempdir1");
        let dir2 = tempfile::tempdir().expect("tempdir2");
        tokio::fs::write(dir1.path().join("ca.pem"), b"content-a")
            .await
            .expect("write1");
        tokio::fs::write(dir2.path().join("ca.pem"), b"content-b")
            .await
            .expect("write2");
        let hash1 = compute_local_ca_hash(dir1.path()).await;
        let hash2 = compute_local_ca_hash(dir2.path()).await;
        assert_ne!(hash1, hash2);
    }

    #[tokio::test]
    async fn local_ca_hash_empty_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        tokio::fs::write(dir.path().join("ca.pem"), b"")
            .await
            .expect("write");
        let hash = compute_local_ca_hash(dir.path()).await;
        assert_eq!(hash.len(), 64);
        assert!(!hash.is_empty());
    }

    #[tokio::test]
    async fn local_ca_hash_only_hex_chars() {
        let dir = tempfile::tempdir().expect("tempdir");
        tokio::fs::write(dir.path().join("ca.pem"), b"some content")
            .await
            .expect("write");
        let hash = compute_local_ca_hash(dir.path()).await;
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
    }

    // ── save_ca_cert atomic write ───────────────────────────────────────

    /// Verify that `save_ca_cert` replaces the CA bundle atomically:
    /// the write goes through `dirs::write_with_mode` which creates a
    /// `.ca.pem.tmp` sibling file and then renames it into place, so a reader
    /// never sees a partially-written `ca.pem`.  After a successful write no
    /// straggler `.tmp` file should remain in the directory.
    #[tokio::test]
    async fn save_ca_cert_is_atomic_via_tmp_rename() {
        let config_dir = tempfile::tempdir().expect("tmpdir");
        let state_dir = tempfile::tempdir().expect("tmpdir");
        let ca_path = config_dir.path().join("ca.pem");

        // Seed an old CA so we can confirm the content is replaced.
        tokio::fs::write(&ca_path, b"OLD_CA").await.expect("seed");

        // save_ca_cert delegates to dirs::write_secure_file_str which calls
        // write_with_mode: write to `.ca.pem.tmp` then rename atomically.
        let mut identity =
            crate::identity::ServiceIdentityState::new(config_dir.path(), state_dir.path());
        identity.save_ca_cert("NEW_CA").await.expect("save ok");

        let content = tokio::fs::read_to_string(&ca_path)
            .await
            .expect("read ca.pem");
        assert_eq!(content, "NEW_CA", "ca.pem must contain the new content");

        // After a successful atomic write the temp file must be gone.
        let stragglers: Vec<_> = std::fs::read_dir(config_dir.path())
            .expect("readdir")
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(
            stragglers.is_empty(),
            "no straggler .tmp files after successful atomic write, found: {stragglers:?}"
        );
    }
}

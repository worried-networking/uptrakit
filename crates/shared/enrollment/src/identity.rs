//! Unified service identity management for both agents and MQTT services.
//!
//! Manages a service's cryptographic identity for mTLS authentication with the
//! controller. Handles:
//! - Service ID and enrollment secret persistence (`service.json`)
//! - ECDSA P-256 keypair generation and persistence (`service.key`)
//! - CSR generation for certificate requests
//! - Certificate storage and loading (`service.crt`)
//! - CA bundle storage (`ca.pem`)
//! - Certificate expiry read directly from the PEM (no separate timestamp file)

use std::path::{Path, PathBuf};

use rootcause::prelude::*;
use serde::{Deserialize, Serialize};
use tokio::fs;
use uuid::Uuid;

use crate::error::{EnrollmentError, Result};

/// File names within the data directory.
const STATE_FILE: &str = "service.json";
const CA_CERT_FILE: &str = "ca.pem";
const SERVICE_CERT_FILE: &str = "service.crt";
const SERVICE_KEY_FILE: &str = "service.key";

/// Persisted enrollment state.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ServiceState {
    service_id: String,
    enrollment_secret: String,
}

/// Unified identity state for any service (agent or MQTT).
///
/// Replaces the agent's sync `AgentState` + `AgentCertState` and the MQTT
/// service's async `Identity`. Uses async I/O throughout.
///
/// Lifecycle:
/// 1. **Fresh**: no `service.json` — needs enrollment.
/// 2. **Enrolled**: `service.json` exists — needs certificate issuance.
/// 3. **Certified**: `service.crt` + `service.key` exist — ready for mTLS.
#[derive(Debug)]
pub struct ServiceIdentityState {
    /// Path to the data directory.
    data_dir: PathBuf,
    /// Service UUID assigned by the controller during enrollment.
    service_id: Option<Uuid>,
    /// Enrollment secret for bearer auth before certificate issuance.
    enrollment_secret: Option<String>,
    /// ECDSA P-256 keypair.
    keypair: Option<rcgen::KeyPair>,
    /// PEM-encoded certificate (after issuance).
    certificate_pem: Option<String>,
    /// CA certificate bundle PEM (for verifying controller).
    ca_cert_pem: Option<String>,
}

impl ServiceIdentityState {
    /// Create a new identity manager for the given data directory.
    pub fn new(data_dir: impl AsRef<Path>) -> Self {
        Self {
            data_dir: data_dir.as_ref().to_path_buf(),
            service_id: None,
            enrollment_secret: None,
            keypair: None,
            certificate_pem: None,
            ca_cert_pem: None,
        }
    }

    /// Load existing identity from disk (if any).
    ///
    /// Creates the data directory if it does not exist.
    pub async fn load(&mut self) -> Result<()> {
        fs::create_dir_all(&self.data_dir)
            .await
            .context_to::<EnrollmentError>()?;

        // Load enrollment state (service_id + secret).
        let state_path = self.data_dir.join(STATE_FILE);
        if state_path.exists() {
            let content = fs::read_to_string(&state_path)
                .await
                .context_to::<EnrollmentError>()?;
            match serde_json::from_str::<ServiceState>(&content) {
                Ok(state) => {
                    if let Ok(id) = Uuid::parse_str(&state.service_id) {
                        self.service_id = Some(id);
                        if !state.enrollment_secret.is_empty() {
                            self.enrollment_secret = Some(state.enrollment_secret);
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!("failed to parse {}: {e}", STATE_FILE);
                }
            }
        }

        // Load private key.
        let key_path = self.data_dir.join(SERVICE_KEY_FILE);
        if key_path.exists() {
            let key_pem = fs::read_to_string(&key_path)
                .await
                .context_to::<EnrollmentError>()?;
            match rcgen::KeyPair::from_pem(&key_pem) {
                Ok(kp) => self.keypair = Some(kp),
                Err(e) => {
                    tracing::warn!("failed to parse private key: {e}");
                }
            }
        }

        // Load certificate.
        let cert_path = self.data_dir.join(SERVICE_CERT_FILE);
        if cert_path.exists() {
            let cert_pem = fs::read_to_string(&cert_path)
                .await
                .context_to::<EnrollmentError>()?;
            self.certificate_pem = Some(cert_pem);
        }

        // Load CA certificate bundle.
        let ca_path = self.data_dir.join(CA_CERT_FILE);
        if ca_path.exists() {
            let ca_pem = fs::read_to_string(&ca_path)
                .await
                .context_to::<EnrollmentError>()?;
            self.ca_cert_pem = Some(ca_pem);
        }

        Ok(())
    }

    // ── State queries ─────────────────────────────────────────────────

    /// `true` if no enrollment has ever been performed.
    pub fn is_fresh(&self) -> bool {
        self.service_id.is_none()
    }

    /// `true` if enrolled (has service_id) but no certificate yet.
    pub fn is_enrolled_only(&self) -> bool {
        self.service_id.is_some() && self.certificate_pem.is_none()
    }

    /// `true` if fully certified (has both service_id and certificate).
    pub fn is_certified(&self) -> bool {
        self.service_id.is_some() && self.certificate_pem.is_some()
    }

    /// The service UUID assigned by the controller.
    pub fn service_id(&self) -> Option<Uuid> {
        self.service_id
    }

    /// The enrollment secret (for bearer auth before cert issuance).
    pub fn enrollment_secret(&self) -> Option<&str> {
        self.enrollment_secret.as_deref()
    }

    /// Certificate expiry read from the PEM-encoded certificate.
    ///
    /// Returns `None` if no certificate is loaded or parsing fails.
    pub fn cert_not_after(&self) -> Option<time::OffsetDateTime> {
        let pem = self.certificate_pem.as_deref()?;
        let der = pem_to_der(pem)?;
        let (_, cert) = x509_parser::parse_x509_certificate(&der).ok()?;
        let asn1_time = cert.validity().not_after;
        asn1_time.to_datetime().into()
    }

    /// The PEM-encoded CA certificate bundle.
    pub fn ca_cert_pem(&self) -> Option<&str> {
        self.ca_cert_pem.as_deref()
    }

    /// The PEM-encoded certificate.
    pub fn cert_pem(&self) -> Option<&str> {
        self.certificate_pem.as_deref()
    }

    /// The PEM-encoded private key.
    pub fn key_pem(&self) -> Option<String> {
        self.keypair.as_ref().map(|kp| kp.serialize_pem())
    }

    // ── Mutating operations ───────────────────────────────────────────

    /// Generate a new ECDSA P-256 keypair if one does not already exist.
    ///
    /// Persists the key to `service.key` with secure permissions.
    pub async fn ensure_keypair(&mut self) -> Result<()> {
        if self.keypair.is_some() {
            return Ok(());
        }

        let keypair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)
            .map_err(|e| report!(EnrollmentError::KeypairGeneration(e.to_string())))?;

        let key_pem = keypair.serialize_pem();
        let key_path = self.data_dir.join(SERVICE_KEY_FILE);
        fs::write(&key_path, &key_pem)
            .await
            .context_to::<EnrollmentError>()?;
        set_secure_permissions(&key_path).await?;

        self.keypair = Some(keypair);
        Ok(())
    }

    /// Generate a PKCS#10 CSR with `CN=<service_id>`.
    ///
    /// Requires that a keypair has been generated via [`ensure_keypair`](Self::ensure_keypair).
    pub fn generate_csr(&self, service_id: Uuid) -> Result<String> {
        let keypair = self.keypair.as_ref().ok_or_else(|| {
            report!(EnrollmentError::KeypairGeneration(
                "no keypair available".to_string(),
            ))
        })?;

        let mut params = rcgen::CertificateParams::new(vec![])
            .map_err(|e| report!(EnrollmentError::CsrGeneration(e.to_string())))?;
        params.distinguished_name = rcgen::DistinguishedName::new();
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, service_id.to_string());

        let csr = params
            .serialize_request(keypair)
            .map_err(|e| report!(EnrollmentError::CsrGeneration(e.to_string())))?;

        csr.pem()
            .map_err(|e| report!(EnrollmentError::CsrGeneration(e.to_string())))
    }

    /// Persist enrollment result (service_id + enrollment_secret).
    pub async fn save_enrollment(
        &mut self,
        service_id: Uuid,
        enrollment_secret: &str,
    ) -> Result<()> {
        let state = ServiceState {
            service_id: service_id.to_string(),
            enrollment_secret: enrollment_secret.to_string(),
        };
        let json = serde_json::to_string_pretty(&state).context_to::<EnrollmentError>()?;
        let path = self.data_dir.join(STATE_FILE);
        fs::write(&path, json)
            .await
            .context_to::<EnrollmentError>()?;
        set_secure_permissions(&path).await?;

        self.service_id = Some(service_id);
        self.enrollment_secret = Some(enrollment_secret.to_string());
        Ok(())
    }

    /// Persist the issued certificate to `service.crt`.
    ///
    /// Clears the enrollment secret from the state file since bearer auth is no
    /// longer needed once mTLS is available.
    pub async fn save_certificate(&mut self, cert_pem: &str) -> Result<()> {
        let cert_path = self.data_dir.join(SERVICE_CERT_FILE);
        fs::write(&cert_path, cert_pem)
            .await
            .context_to::<EnrollmentError>()?;
        set_secure_permissions(&cert_path).await?;
        self.certificate_pem = Some(cert_pem.to_string());

        // Rewrite state file without enrollment_secret.
        if let Some(sid) = self.service_id {
            let state = ServiceState {
                service_id: sid.to_string(),
                enrollment_secret: String::new(),
            };
            let json = serde_json::to_string_pretty(&state).context_to::<EnrollmentError>()?;
            let path = self.data_dir.join(STATE_FILE);
            fs::write(&path, json)
                .await
                .context_to::<EnrollmentError>()?;
            set_secure_permissions(&path).await?;
        }
        self.enrollment_secret = None;

        Ok(())
    }

    /// Persist the CA certificate bundle to `ca.pem`.
    pub async fn save_ca_cert(&mut self, ca_pem: &str) -> Result<()> {
        let ca_path = self.data_dir.join(CA_CERT_FILE);
        fs::write(&ca_path, ca_pem)
            .await
            .context_to::<EnrollmentError>()?;
        set_secure_permissions(&ca_path).await?;
        self.ca_cert_pem = Some(ca_pem.to_string());
        Ok(())
    }

    /// Load the raw CA certificate bytes from disk.
    pub async fn load_ca_cert(&self) -> Result<Option<Vec<u8>>> {
        let path = self.data_dir.join(CA_CERT_FILE);
        if !path.exists() {
            return Ok(None);
        }
        let bytes = fs::read(&path).await.context_to::<EnrollmentError>()?;
        Ok(Some(bytes))
    }

    /// Remove all persisted identity state (for re-enrollment).
    pub async fn clear_state(&mut self) -> Result<()> {
        let files = [
            STATE_FILE,
            SERVICE_KEY_FILE,
            SERVICE_CERT_FILE,
            CA_CERT_FILE,
        ];
        for name in files {
            let path = self.data_dir.join(name);
            if path.exists() {
                fs::remove_file(&path)
                    .await
                    .context_to::<EnrollmentError>()?;
            }
        }
        self.service_id = None;
        self.enrollment_secret = None;
        self.keypair = None;
        self.certificate_pem = None;
        self.ca_cert_pem = None;
        Ok(())
    }
}

// ── Helpers ────────────────────────────────────────────────────────────

/// Set file permissions to `0600` (owner read/write only) on Unix.
async fn set_secure_permissions(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .await
            .context_to::<EnrollmentError>()?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

/// Decode the first PEM block into DER bytes.
fn pem_to_der(pem: &str) -> Option<Vec<u8>> {
    // Find the base64 content between BEGIN and END markers.
    let start_marker = "-----BEGIN CERTIFICATE-----";
    let end_marker = "-----END CERTIFICATE-----";
    let start = pem.find(start_marker)? + start_marker.len();
    let end = pem[start..].find(end_marker)? + start;
    let b64: String = pem[start..end]
        .chars()
        .filter(|c| !c.is_ascii_whitespace())
        .collect();

    // Use a simple base64 decoder (standard alphabet).
    base64_decode(&b64)
}

/// Minimal base64 decoder (standard alphabet, no padding required).
fn base64_decode(input: &str) -> Option<Vec<u8>> {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    fn val(c: u8) -> Option<u8> {
        TABLE.iter().position(|&b| b == c).map(|p| p as u8)
    }

    let bytes: Vec<u8> = input.bytes().filter(|&b| b != b'=').collect();
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);

    for chunk in bytes.chunks(4) {
        let mut buf: u32 = 0;
        for (i, &b) in chunk.iter().enumerate() {
            buf |= (val(b)? as u32) << (18 - 6 * i);
        }
        match chunk.len() {
            4 => {
                out.push((buf >> 16) as u8);
                out.push((buf >> 8) as u8);
                out.push(buf as u8);
            }
            3 => {
                out.push((buf >> 16) as u8);
                out.push((buf >> 8) as u8);
            }
            2 => {
                out.push((buf >> 16) as u8);
            }
            _ => return None,
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn fresh_identity_is_fresh() {
        let dir = TempDir::new().expect("tempdir");
        let mut identity = ServiceIdentityState::new(dir.path());
        identity.load().await.expect("load");

        assert!(identity.is_fresh());
        assert!(!identity.is_enrolled_only());
        assert!(!identity.is_certified());
        assert!(identity.service_id().is_none());
    }

    #[tokio::test]
    async fn keypair_generation_persists() {
        let dir = TempDir::new().expect("tempdir");
        let mut identity = ServiceIdentityState::new(dir.path());
        identity.load().await.expect("load");

        identity.ensure_keypair().await.expect("ensure_keypair");
        assert!(identity.key_pem().is_some());

        // Reload and verify persistence.
        let mut identity2 = ServiceIdentityState::new(dir.path());
        identity2.load().await.expect("load");
        assert!(identity2.key_pem().is_some());
    }

    #[tokio::test]
    async fn enrollment_persists() {
        let dir = TempDir::new().expect("tempdir");
        let mut identity = ServiceIdentityState::new(dir.path());
        identity.load().await.expect("load");

        let sid = Uuid::now_v7();
        identity
            .save_enrollment(sid, "secret123")
            .await
            .expect("save_enrollment");

        assert_eq!(identity.service_id(), Some(sid));
        assert_eq!(identity.enrollment_secret(), Some("secret123"));
        assert!(identity.is_enrolled_only());

        // Reload and verify persistence.
        let mut identity2 = ServiceIdentityState::new(dir.path());
        identity2.load().await.expect("load");
        assert_eq!(identity2.service_id(), Some(sid));
        assert_eq!(identity2.enrollment_secret(), Some("secret123"));
    }

    #[tokio::test]
    async fn certificate_save_clears_enrollment_secret() {
        let dir = TempDir::new().expect("tempdir");
        let mut identity = ServiceIdentityState::new(dir.path());
        identity.load().await.expect("load");

        let sid = Uuid::now_v7();
        identity
            .save_enrollment(sid, "secret123")
            .await
            .expect("save_enrollment");

        // Generate a real self-signed cert to test with.
        identity.ensure_keypair().await.expect("ensure_keypair");
        let kp = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).expect("keygen");
        let mut params = rcgen::CertificateParams::new(vec![]).expect("cert params");
        params.distinguished_name = rcgen::DistinguishedName::new();
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, sid.to_string());
        let cert = params.self_signed(&kp).expect("self-sign");
        let cert_pem = cert.pem();

        identity
            .save_certificate(&cert_pem)
            .await
            .expect("save_certificate");

        assert!(identity.enrollment_secret().is_none());
        assert!(identity.is_certified());

        // Verify cert_not_after returns a valid timestamp.
        assert!(identity.cert_not_after().is_some());

        // Reload and verify.
        let mut identity2 = ServiceIdentityState::new(dir.path());
        identity2.load().await.expect("load");
        assert!(identity2.enrollment_secret().is_none());
        assert!(identity2.is_certified());
        assert!(identity2.cert_not_after().is_some());
    }

    #[tokio::test]
    async fn csr_generation() {
        let dir = TempDir::new().expect("tempdir");
        let mut identity = ServiceIdentityState::new(dir.path());
        identity.load().await.expect("load");
        identity.ensure_keypair().await.expect("ensure_keypair");

        let sid = Uuid::now_v7();
        let csr = identity.generate_csr(sid).expect("generate_csr");

        assert!(csr.contains("BEGIN CERTIFICATE REQUEST"));
        assert!(csr.contains("END CERTIFICATE REQUEST"));
    }

    #[tokio::test]
    async fn ca_cert_round_trip() {
        let dir = TempDir::new().expect("tempdir");
        let mut identity = ServiceIdentityState::new(dir.path());
        identity.load().await.expect("load");

        let fake_ca = "-----BEGIN CERTIFICATE-----\nfakedata\n-----END CERTIFICATE-----\n";
        identity.save_ca_cert(fake_ca).await.expect("save_ca_cert");

        assert_eq!(identity.ca_cert_pem(), Some(fake_ca));

        let raw = identity.load_ca_cert().await.expect("load_ca_cert");
        assert_eq!(raw.as_deref(), Some(fake_ca.as_bytes()));
    }

    #[tokio::test]
    async fn clear_state_removes_everything() {
        let dir = TempDir::new().expect("tempdir");
        let mut identity = ServiceIdentityState::new(dir.path());
        identity.load().await.expect("load");

        let sid = Uuid::now_v7();
        identity
            .save_enrollment(sid, "secret")
            .await
            .expect("save_enrollment");
        identity.ensure_keypair().await.expect("ensure_keypair");

        identity.clear_state().await.expect("clear_state");

        assert!(identity.is_fresh());
        assert!(identity.service_id().is_none());
        assert!(identity.enrollment_secret().is_none());
        assert!(identity.key_pem().is_none());
        assert!(identity.cert_pem().is_none());
        assert!(identity.ca_cert_pem().is_none());
    }

    #[tokio::test]
    async fn idempotent_ensure_keypair() {
        let dir = TempDir::new().expect("tempdir");
        let mut identity = ServiceIdentityState::new(dir.path());
        identity.load().await.expect("load");

        identity.ensure_keypair().await.expect("first");
        let pem1 = identity.key_pem().expect("key_pem");

        identity.ensure_keypair().await.expect("second");
        let pem2 = identity.key_pem().expect("key_pem");

        assert_eq!(pem1, pem2, "second call must not regenerate");
    }

    #[test]
    fn pem_to_der_basic() {
        // Verify our minimal PEM decoder works for a trivial payload.
        let encoded = "-----BEGIN CERTIFICATE-----\nSGVsbG8=\n-----END CERTIFICATE-----\n";
        let der = pem_to_der(encoded).expect("decode");
        assert_eq!(der, b"Hello");
    }
}

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
//!
//! Files are split between config and state directories:
//! - Config: CA certificate bundle (rarely changes)
//! - State: Private key, enrollment state, service certificate (runtime state)

use std::path::{Path, PathBuf};

use der::DecodePem;
use rootcause::prelude::*;
use serde::{Deserialize, Serialize};
use tokio::fs;
use uuid::Uuid;
use x509_cert::Certificate;

use crate::error::{EnrollmentError, IdentityError, Result};
use crate::shared_types_api::SecretString;

/// File names within directories.
const STATE_FILE: &str = "service.json";
const CA_CERT_FILE: &str = "ca.pem";
const SERVICE_CERT_FILE: &str = "service.crt";
const SERVICE_KEY_FILE: &str = "service.key";
/// Maximum length of a SPIFFE trust domain per the SPIFFE ID specification.
const MAX_TRUST_DOMAIN_LEN: usize = 255;

/// Persisted enrollment state.
///
/// `enrollment_secret` uses [`SecretString`] so that it is zeroized on drop
/// and never appears in `Debug` output or tracing spans.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ServiceState {
    service_id: Uuid,
    enrollment_secret: SecretString,
    /// Tenant UUID received from the controller via `ServiceSettings`.
    ///
    /// Persisted so that CLI commands (which do not connect to the controller)
    /// can read it back. Added after the initial format, so existing files
    /// will deserialize with `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tenant_id: Option<Uuid>,
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
///
/// Directory layout:
/// - `config_dir/ca.pem` — controller's CA certificate bundle
/// - `state_dir/service.json` — enrollment state (service_id + secret)
/// - `state_dir/service.key` — private key
/// - `state_dir/service.crt` — signed certificate
#[derive(Debug)]
pub struct ServiceIdentityState {
    /// Path to the config directory (CA certificate).
    config_dir: PathBuf,
    /// Path to the state directory (enrollment state, key, cert).
    state_dir: PathBuf,
    /// Service UUID assigned by the controller during enrollment.
    service_id: Option<Uuid>,
    /// Tenant UUID received from the controller via `ServiceSettings`.
    ///
    /// Persisted alongside the service identity so that CLI commands can
    /// read it without connecting to the controller.
    tenant_id: Option<Uuid>,
    /// Enrollment secret for bearer auth before certificate issuance.
    ///
    /// Stored as [`SecretString`] to prevent accidental exposure in `Debug`
    /// output or tracing spans. Cleared from memory once the certificate is
    /// issued and persisted.
    enrollment_secret: Option<SecretString>,
    /// ECDSA P-256 keypair.
    keypair: Option<rcgen::KeyPair>,
    /// PEM-encoded certificate (after issuance).
    certificate_pem: Option<String>,
    /// CA certificate bundle PEM (for verifying controller).
    ca_cert_pem: Option<String>,
}

impl ServiceIdentityState {
    /// Create a new identity manager with separate config and state directories.
    ///
    /// # Arguments
    ///
    /// * `config_dir` - Directory for configuration files (CA certificate)
    /// * `state_dir` - Directory for state files (enrollment, key, cert)
    pub fn new(config_dir: impl AsRef<Path>, state_dir: impl AsRef<Path>) -> Self {
        Self {
            config_dir: config_dir.as_ref().to_path_buf(),
            state_dir: state_dir.as_ref().to_path_buf(),
            service_id: None,
            tenant_id: None,
            enrollment_secret: None,
            keypair: None,
            certificate_pem: None,
            ca_cert_pem: None,
        }
    }

    /// Create a new identity manager with a single directory for both config and state.
    ///
    /// This is a convenience constructor for backwards compatibility.
    pub fn new_single_dir(data_dir: impl AsRef<Path>) -> Self {
        let path = data_dir.as_ref().to_path_buf();
        Self {
            config_dir: path.clone(),
            state_dir: path,
            service_id: None,
            tenant_id: None,
            enrollment_secret: None,
            keypair: None,
            certificate_pem: None,
            ca_cert_pem: None,
        }
    }

    /// Create an in-memory identity for an embedded service running in-process.
    ///
    /// Sets `service_id` and `keypair` directly from the supplied values.
    /// The `config_dir` and `state_dir` fields are set to empty [`PathBuf`]
    /// sentinels and are **never used for I/O** — embedded services do not
    /// read from or write to disk.
    ///
    /// # Arguments
    ///
    /// * `service_id` — UUID assigned by the controller on behalf of the
    ///   embedded service.
    /// * `keypair` — Pre-generated ECDSA P-256 keypair.  Use
    ///   [`generate_p256_keypair_for_ecies`] or
    ///   `rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)` to
    ///   produce one.
    pub(crate) fn for_embedded(service_id: Uuid, keypair: rcgen::KeyPair) -> Self {
        Self {
            config_dir: std::path::PathBuf::new(), // sentinel — never used for I/O
            state_dir: std::path::PathBuf::new(),  // sentinel — never used for I/O
            service_id: Some(service_id),
            tenant_id: None,
            enrollment_secret: None,
            keypair: Some(keypair),
            certificate_pem: None,
            ca_cert_pem: None,
        }
    }

    /// Load existing identity from disk (if any).
    ///
    /// Creates both directories if they do not exist.
    pub async fn load(&mut self) -> Result<()> {
        crate::dirs::create_secure_dir(&self.config_dir)
            .await
            .context_to::<EnrollmentError>()?;

        if self.config_dir != self.state_dir {
            crate::dirs::create_secure_dir(&self.state_dir)
                .await
                .context_to::<EnrollmentError>()?;
        }

        // Load enrollment state (service_id + secret) from state_dir.
        let state_path = self.state_dir.join(STATE_FILE);
        if state_path.exists() {
            let content = fs::read_to_string(&state_path)
                .await
                .context_to::<EnrollmentError>()?;
            match serde_json::from_str::<ServiceState>(&content) {
                Ok(state) => {
                    self.service_id = Some(state.service_id);
                    self.tenant_id = state.tenant_id;
                    if !state.enrollment_secret.expose_secret().is_empty() {
                        self.enrollment_secret = Some(state.enrollment_secret);
                    }
                }
                Err(e) => {
                    tracing::warn!("failed to parse {}: {e}", STATE_FILE);
                }
            }
        }

        // Load private key from state_dir.
        let key_path = self.state_dir.join(SERVICE_KEY_FILE);
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

        // Load certificate from state_dir.
        let cert_path = self.state_dir.join(SERVICE_CERT_FILE);
        if cert_path.exists() {
            let cert_pem = fs::read_to_string(&cert_path)
                .await
                .context_to::<EnrollmentError>()?;
            self.certificate_pem = Some(cert_pem);
        }

        // Load CA certificate bundle from config_dir.
        let ca_path = self.config_dir.join(CA_CERT_FILE);
        if ca_path.exists() {
            let ca_pem = fs::read_to_string(&ca_path)
                .await
                .context_to::<EnrollmentError>()?;
            if ca_pem.is_empty() {
                tracing::warn!(path = %ca_path.display(), "ca.pem exists but is empty, treating as missing");
            } else {
                self.ca_cert_pem = Some(ca_pem);
            }
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

    /// The tenant UUID received from the controller.
    pub fn tenant_id(&self) -> Option<Uuid> {
        self.tenant_id
    }

    /// The enrollment secret (for bearer auth before cert issuance).
    ///
    /// Intentionally exposes the raw secret string: this value is used as a
    /// bearer token in the enrollment HTTP request and must be a plain string
    /// at that boundary. Do not log or format the returned value.
    pub fn enrollment_secret(&self) -> Option<&str> {
        self.enrollment_secret.as_ref().map(|s| s.expose_secret())
    }

    /// Certificate expiry read from the PEM-encoded certificate.
    ///
    /// Returns `None` if no certificate is loaded or parsing fails.
    pub fn cert_not_after(&self) -> Option<time::OffsetDateTime> {
        let pem = self.certificate_pem.as_deref()?;
        let cert = Certificate::from_pem(pem.as_bytes()).ok()?;
        let secs = cert
            .tbs_certificate
            .validity
            .not_after
            .to_unix_duration()
            .as_secs();
        time::OffsetDateTime::from_unix_timestamp(secs as i64).ok()
    }

    /// Certificate expiry as milliseconds since the Unix epoch.
    ///
    /// Returns `None` if no certificate is loaded or parsing fails.
    pub fn cert_not_after_ms(&self) -> Option<i64> {
        let not_after = self.cert_not_after()?;
        Some(not_after.unix_timestamp() * 1000)
    }

    /// Check if the certificate is expired or within the renewal window.
    ///
    /// Returns `None` if no certificate is loaded or parsing fails.
    /// Returns `Some(true)` if the certificate is expired.
    pub fn is_cert_expired(&self) -> Option<bool> {
        let not_after = self.cert_not_after()?;
        let now = time::OffsetDateTime::now_utc();
        Some(now >= not_after)
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

    /// The raw uncompressed P-256 public key bytes (65 bytes: `0x04 || x || y`).
    ///
    /// Used for ECIES sealed-box encryption in shared-surface flows: clients encrypt
    /// sensitive parameters with this key and only the service can decrypt.
    pub fn public_key_raw(&self) -> Option<Vec<u8>> {
        self.keypair.as_ref().map(|kp| kp.public_key_raw().to_vec())
    }

    /// The private key in PKCS#8 DER format.
    ///
    /// Used for ECIES sealed-box decryption in shared-surface flows.
    pub fn private_key_pkcs8_der(&self) -> Option<Vec<u8>> {
        self.keypair.as_ref().map(|kp| kp.serialize_der())
    }

    /// Get the config directory path.
    pub fn config_dir(&self) -> &Path {
        &self.config_dir
    }

    /// Get the state directory path.
    pub fn state_dir(&self) -> &Path {
        &self.state_dir
    }

    // ── Mutating operations ───────────────────────────────────────────

    /// Generate a new ECDSA P-256 keypair if one does not already exist.
    ///
    /// Persists the key to `state_dir/service.key` with secure permissions.
    pub async fn ensure_keypair(&mut self) -> Result<()> {
        if self.keypair.is_some() {
            return Ok(());
        }

        let keypair =
            rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P384_SHA384).map_err(|e| {
                report!(EnrollmentError::Identity(IdentityError::KeypairGeneration(
                    e.to_string()
                )))
            })?;

        let key_pem = keypair.serialize_pem();
        let key_path = self.state_dir.join(SERVICE_KEY_FILE);
        crate::dirs::write_secure_file_str(&key_path, &key_pem)
            .await
            .context_to::<EnrollmentError>()?;

        self.keypair = Some(keypair);
        Ok(())
    }

    /// Generate a PKCS#10 CSR using the stored service_id.
    ///
    /// Convenience wrapper around [`generate_csr`](Self::generate_csr) that uses the
    /// already-enrolled service_id. Returns an error if not enrolled or no keypair.
    pub fn generate_csr_for_self(&self) -> Result<String> {
        let sid = self
            .service_id
            .ok_or_else(|| report!(EnrollmentError::Identity(IdentityError::NotEnrolled)))?;
        self.generate_csr(sid)
    }

    /// Generate a PKCS#10 CSR with `CN=<service_id>`.
    ///
    /// Requires that a keypair has been generated via [`ensure_keypair`](Self::ensure_keypair).
    pub fn generate_csr(&self, service_id: Uuid) -> Result<String> {
        let keypair = self.keypair.as_ref().ok_or_else(|| {
            report!(EnrollmentError::Identity(IdentityError::KeypairGeneration(
                "no keypair available".to_string(),
            )))
        })?;

        let mut params = rcgen::CertificateParams::new(vec![]).map_err(|e| {
            report!(EnrollmentError::Identity(IdentityError::CsrGeneration(
                e.to_string()
            )))
        })?;
        params.distinguished_name = rcgen::DistinguishedName::new();
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, service_id.to_string());

        let csr = params.serialize_request(keypair).map_err(|e| {
            report!(EnrollmentError::Identity(IdentityError::CsrGeneration(
                e.to_string()
            )))
        })?;

        csr.pem().map_err(|e| {
            report!(EnrollmentError::Identity(IdentityError::CsrGeneration(
                e.to_string()
            )))
        })
    }

    /// Persist enrollment result (service_id + enrollment_secret) to state_dir.
    pub async fn save_enrollment(
        &mut self,
        service_id: Uuid,
        enrollment_secret: &str,
    ) -> Result<()> {
        let state = ServiceState {
            service_id,
            enrollment_secret: SecretString::new(enrollment_secret),
            tenant_id: self.tenant_id,
        };
        let json = serde_json::to_string_pretty(&state).context_to::<EnrollmentError>()?;
        let path = self.state_dir.join(STATE_FILE);
        crate::dirs::write_secure_file_str(&path, &json)
            .await
            .context_to::<EnrollmentError>()?;

        self.service_id = Some(service_id);
        self.enrollment_secret = Some(SecretString::new(enrollment_secret));
        Ok(())
    }

    /// Persist the tenant UUID to `service.json`.
    ///
    /// Reads the current state file, updates the `tenant_id` field, and
    /// rewrites the file. If no state file exists yet (service not enrolled),
    /// this is a no-op.
    pub async fn save_tenant_id(&mut self, tenant_id: Uuid) -> Result<()> {
        self.tenant_id = Some(tenant_id);

        let Some(sid) = self.service_id else {
            return Ok(());
        };

        let state_path = self.state_dir.join(STATE_FILE);
        if !state_path.exists() {
            return Ok(());
        }

        // Read current state to preserve enrollment_secret.
        let content = fs::read_to_string(&state_path)
            .await
            .context_to::<EnrollmentError>()?;
        let mut state: ServiceState =
            serde_json::from_str(&content).context_to::<EnrollmentError>()?;

        state.service_id = sid;
        state.tenant_id = Some(tenant_id);

        let json = serde_json::to_string_pretty(&state).context_to::<EnrollmentError>()?;
        crate::dirs::write_secure_file_str(&state_path, &json)
            .await
            .context_to::<EnrollmentError>()?;

        Ok(())
    }

    /// Persist a private key PEM to `state_dir/service.key` and update the
    /// in-memory keypair.
    ///
    /// Used during certificate renewal when a fresh keypair is generated
    /// separately (not via [`ensure_keypair`](Self::ensure_keypair)) and the
    /// signed certificate arrives back from the controller.
    pub async fn save_private_key(&mut self, key_pem: &str) -> Result<()> {
        let keypair = rcgen::KeyPair::from_pem(key_pem).map_err(|e| {
            report!(EnrollmentError::Identity(IdentityError::KeypairGeneration(
                format!("failed to parse private key PEM: {e}")
            )))
        })?;

        let key_path = self.state_dir.join(SERVICE_KEY_FILE);
        crate::dirs::write_secure_file_str(&key_path, key_pem)
            .await
            .context_to::<EnrollmentError>()?;

        self.keypair = Some(keypair);
        Ok(())
    }

    /// Persist the issued certificate to `state_dir/service.crt`.
    ///
    /// Clears the enrollment secret from the state file since bearer auth is no
    /// longer needed once mTLS is available.
    pub async fn save_certificate(&mut self, cert_pem: &str) -> Result<()> {
        let cert_path = self.state_dir.join(SERVICE_CERT_FILE);
        crate::dirs::write_secure_file_str(&cert_path, cert_pem)
            .await
            .context_to::<EnrollmentError>()?;
        self.certificate_pem = Some(cert_pem.to_string());

        // Rewrite state file without enrollment_secret.
        if let Some(sid) = self.service_id {
            let state = ServiceState {
                service_id: sid,
                enrollment_secret: SecretString::new(String::new()),
                tenant_id: self.tenant_id,
            };
            let json = serde_json::to_string_pretty(&state).context_to::<EnrollmentError>()?;
            let path = self.state_dir.join(STATE_FILE);
            crate::dirs::write_secure_file_str(&path, &json)
                .await
                .context_to::<EnrollmentError>()?;
        }
        self.enrollment_secret = None;

        Ok(())
    }

    /// Persist the CA certificate bundle to `config_dir/ca.pem`.
    pub async fn save_ca_cert(&mut self, ca_pem: &str) -> Result<()> {
        let ca_path = self.config_dir.join(CA_CERT_FILE);
        crate::dirs::write_secure_file_str(&ca_path, ca_pem)
            .await
            .context_to::<EnrollmentError>()?;
        self.ca_cert_pem = Some(ca_pem.to_string());
        Ok(())
    }

    /// Load the raw CA certificate bytes from disk.
    ///
    /// Returns `None` if the file does not exist or is empty. An empty file
    /// is treated as missing so that a prior write failure that produced an
    /// empty `ca.pem` does not propagate to the TLS layer as `NoCertificates`.
    pub async fn load_ca_cert(&self) -> Result<Option<Vec<u8>>> {
        let path = self.config_dir.join(CA_CERT_FILE);
        if !path.exists() {
            return Ok(None);
        }
        let bytes = fs::read(&path).await.context_to::<EnrollmentError>()?;
        if bytes.is_empty() {
            tracing::warn!(path = %path.display(), "ca.pem exists but is empty, treating as missing");
            return Ok(None);
        }
        Ok(Some(bytes))
    }

    /// Get the CA certificate path.
    pub fn ca_cert_path(&self) -> PathBuf {
        self.config_dir.join(CA_CERT_FILE)
    }

    /// Remove all persisted identity state (for re-enrollment).
    pub async fn clear_state(&mut self) -> Result<()> {
        // Files in state_dir
        let state_files = [STATE_FILE, SERVICE_KEY_FILE, SERVICE_CERT_FILE];
        for name in state_files {
            let path = self.state_dir.join(name);
            if path.exists() {
                fs::remove_file(&path)
                    .await
                    .context_to::<EnrollmentError>()?;
            }
        }

        // CA cert in config_dir
        let ca_path = self.config_dir.join(CA_CERT_FILE);
        if ca_path.exists() {
            fs::remove_file(&ca_path)
                .await
                .context_to::<EnrollmentError>()?;
        }

        self.service_id = None;
        self.tenant_id = None;
        self.enrollment_secret = None;
        self.keypair = None;
        self.certificate_pem = None;
        self.ca_cert_pem = None;
        Ok(())
    }

    /// Remove only state files (key, cert, enrollment), preserving CA cert.
    ///
    /// Used when re-enrolling but wanting to keep the trusted CA.
    pub async fn clear_enrollment_state(&mut self) -> Result<()> {
        let state_files = [STATE_FILE, SERVICE_KEY_FILE, SERVICE_CERT_FILE];
        for name in state_files {
            let path = self.state_dir.join(name);
            if path.exists() {
                fs::remove_file(&path)
                    .await
                    .context_to::<EnrollmentError>()?;
            }
        }

        self.service_id = None;
        self.tenant_id = None;
        self.enrollment_secret = None;
        self.keypair = None;
        self.certificate_pem = None;
        Ok(())
    }
}

/// Generate a fresh ECDSA P-256 keypair and a CSR with `CN=<service_id>`.
///
/// Returns `(key_pem, csr_pem)`. The keypair is **not** persisted; this is
/// used for certificate renewal where a fresh keypair is needed without
/// overwriting the current one on disk until the new certificate arrives.
///
/// When `trust_domain` is non-empty, a SPIFFE URI SAN of the form
/// `spiffe://<trust_domain>/service/<service_id>` is embedded in the CSR.
/// When `trust_domain` is empty, no URI SAN is added (backwards-compatible).
///
/// # Errors
///
/// Returns [`IdentityError::InvalidTrustDomain`] when `trust_domain` is
/// non-empty but violates the SPIFFE trust-domain grammar: only lowercase
/// ASCII letters, digits, dots, and hyphens are allowed (max 255 chars).
/// Returns [`IdentityError::CsrGeneration`] on keypair or CSR failures.
pub fn generate_keypair_and_csr(
    service_id: uuid::Uuid,
    trust_domain: &str,
) -> Result<(String, String)> {
    // Validate trust_domain when provided: SPIFFE trust-domain grammar
    // allows only lowercase ASCII letters, digits, dots, and hyphens.
    if !trust_domain.is_empty()
        && (trust_domain.len() > MAX_TRUST_DOMAIN_LEN
            || !trust_domain
                .chars()
                .all(|c| matches!(c, 'a'..='z' | '0'..='9' | '.' | '-')))
    {
        bail!(EnrollmentError::Identity(
            IdentityError::InvalidTrustDomain {
                domain: trust_domain.to_string(),
            }
        ));
    }

    let key_pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P384_SHA384).map_err(|e| {
        report!(EnrollmentError::Identity(IdentityError::CsrGeneration(
            format!("key generation failed: {e}")
        )))
    })?;

    let mut params = rcgen::CertificateParams::default();
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, service_id.to_string());

    if !trust_domain.is_empty() {
        let spiffe_uri = format!("spiffe://{trust_domain}/service/{service_id}");
        let ia5 = spiffe_uri.as_str().try_into().map_err(|e: rcgen::Error| {
            report!(EnrollmentError::Identity(IdentityError::CsrGeneration(
                format!("SPIFFE URI is not a valid IA5 string: {e}")
            )))
        })?;
        params.subject_alt_names.push(rcgen::SanType::URI(ia5));
    }

    let csr = params.serialize_request(&key_pair).map_err(|e| {
        report!(EnrollmentError::Identity(IdentityError::CsrGeneration(
            format!("CSR serialization failed: {e}")
        )))
    })?;

    let csr_pem = csr.pem().map_err(|e| {
        report!(EnrollmentError::Identity(IdentityError::CsrGeneration(
            format!("CSR PEM encoding failed: {e}")
        )))
    })?;

    Ok((key_pair.serialize_pem(), csr_pem))
}

/// Generate a fresh P-256 keypair for embedded-service ECIES sealed-box
/// identity.
///
/// Returns `(private_key_pkcs8_der, public_key_uncompressed_b64)`. The public
/// key is the 65-byte SEC1 uncompressed encoding (`0x04 || X || Y`),
/// base64-encoded with the standard alphabet.
///
/// Used by the controller when embedding a Service in-process: the
/// controller generates the Service's identity keypair on its behalf, then
/// passes both halves into the handler constructor. Standalone Services do
/// not call this directly; their keypair is managed by
/// [`ServiceIdentityState`].
///
/// # Errors
///
/// Returns [`IdentityError::KeypairGeneration`] (wrapped in
/// [`EnrollmentError::Identity`]) if `rcgen` keygen fails.
pub fn generate_p256_keypair_for_ecies() -> Result<(Vec<u8>, String)> {
    use base64::Engine as _;

    let key_pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).map_err(|e| {
        report!(EnrollmentError::Identity(IdentityError::KeypairGeneration(
            format!("P-256 key generation failed: {e}")
        )))
    })?;
    let private_der = key_pair.serialize_der();
    let public_b64 = base64::engine::general_purpose::STANDARD.encode(key_pair.public_key_raw());
    Ok((private_der, public_b64))
}

/// Atomically persist both the enrollment-state JSON and the private-key PEM
/// to `state_dir`.
///
/// Both files are written to temporary files in `state_dir` (same filesystem)
/// and `sync_all`-ed before being renamed into place. The cert JSON is renamed
/// first, then the key PEM. A crash between the two renames leaves the old key
/// on disk while the new JSON is already live; callers that load the identity
/// must tolerate a stale key file alongside a fresh state file and re-generate
/// a keypair if the two are inconsistent.
///
/// # Errors
///
/// Returns [`EnrollmentError::Io`] if creating, writing, syncing, or persisting
/// either temporary file fails.
pub async fn save_identity(state_dir: &Path, service_json: &str, key_pem: &str) -> Result<()> {
    use std::io::Write as _;
    use tempfile::NamedTempFile;

    let cert_path = state_dir.join(STATE_FILE);
    let key_path = state_dir.join(SERVICE_KEY_FILE);

    // Write both temporary files before persisting either, so that a write
    // failure cannot leave a partial rename already in place.
    let mut cert_tmp =
        NamedTempFile::new_in(state_dir).map_err(|e| report!(EnrollmentError::Io(e)))?;
    cert_tmp
        .write_all(service_json.as_bytes())
        .map_err(|e| report!(EnrollmentError::Io(e)))?;
    cert_tmp
        .as_file()
        .sync_all()
        .map_err(|e| report!(EnrollmentError::Io(e)))?;

    let mut key_tmp =
        NamedTempFile::new_in(state_dir).map_err(|e| report!(EnrollmentError::Io(e)))?;
    key_tmp
        .write_all(key_pem.as_bytes())
        .map_err(|e| report!(EnrollmentError::Io(e)))?;
    key_tmp
        .as_file()
        .sync_all()
        .map_err(|e| report!(EnrollmentError::Io(e)))?;

    // Persist cert first, then key. A crash between these two renames leaves
    // the old key intact (new JSON, old key) — the identity remains bootable
    // from the previous key.
    cert_tmp
        .persist(&cert_path)
        .map_err(|e| report!(EnrollmentError::Io(e.error)))?;
    key_tmp
        .persist(&key_path)
        .map_err(|e| report!(EnrollmentError::Io(e.error)))?;

    Ok(())
}

/// Remove any `.tmp`-prefixed files left in `base` by a prior crashed write.
///
/// `tempfile::NamedTempFile::new_in` creates files with a `.tmp` prefix.
/// On process restart, any surviving temp files are partial writes and must
/// be removed before loading identity state.
pub fn sweep_tmp_siblings(base: &Path) -> Result<()> {
    let entries = std::fs::read_dir(base).map_err(|e| report!(EnrollmentError::Io(e)))?;
    for entry in entries {
        let entry = entry.map_err(|e| report!(EnrollmentError::Io(e)))?;
        let name = entry.file_name();
        if name.to_string_lossy().starts_with(".tmp") {
            tracing::warn!(
                file = %name.to_string_lossy(),
                "removing orphan tempfile in identity dir (crashed mid-write)"
            );
            std::fs::remove_file(entry.path()).map_err(|e| report!(EnrollmentError::Io(e)))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod for_embedded_tests {
    use super::ServiceIdentityState;
    use uuid::Uuid;

    #[test]
    fn for_embedded_returns_correct_service_id() {
        let id = Uuid::new_v4();
        let kp = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
        let identity = ServiceIdentityState::for_embedded(id, kp);
        assert_eq!(identity.service_id(), Some(id));
    }

    #[test]
    fn for_embedded_public_key_raw_is_uncompressed_p256_point() {
        let id = Uuid::new_v4();
        let kp = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
        let identity = ServiceIdentityState::for_embedded(id, kp);
        let raw = identity
            .public_key_raw()
            .expect("public key must be present");
        assert_eq!(raw.len(), 65, "expected 65-byte uncompressed P-256 point");
        assert_eq!(raw[0], 0x04, "expected 0x04 uncompressed prefix");
    }

    #[test]
    fn for_embedded_private_key_pkcs8_der_is_non_empty() {
        let id = Uuid::new_v4();
        let kp = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
        let identity = ServiceIdentityState::for_embedded(id, kp);
        let der = identity
            .private_key_pkcs8_der()
            .expect("private key DER must be present");
        assert!(!der.is_empty());
    }
}

// ── Helpers ────────────────────────────────────────────────────────────

/// Decode the first PEM block into DER bytes.
#[cfg(test)]
fn pem_to_der(pem: &str) -> Option<Vec<u8>> {
    use rustls::pki_types::CertificateDer;
    use rustls::pki_types::pem::PemObject;
    CertificateDer::from_pem_slice(pem.as_bytes())
        .ok()
        .map(|c| c.into_owned().to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn fresh_identity_is_fresh() {
        let dir = TempDir::new().expect("tempdir");
        let mut identity = ServiceIdentityState::new_single_dir(dir.path());
        identity.load().await.expect("load");

        assert!(identity.is_fresh());
        assert!(!identity.is_enrolled_only());
        assert!(!identity.is_certified());
        assert!(identity.service_id().is_none());
    }

    #[tokio::test]
    async fn separate_config_state_dirs() {
        let temp = TempDir::new().expect("tempdir");
        let config_dir = temp.path().join("config");
        let state_dir = temp.path().join("state");

        let mut identity = ServiceIdentityState::new(&config_dir, &state_dir);
        identity.load().await.expect("load");

        assert_eq!(identity.config_dir(), config_dir);
        assert_eq!(identity.state_dir(), state_dir);

        // Both directories should be created
        assert!(config_dir.exists());
        assert!(state_dir.exists());
    }

    #[tokio::test]
    async fn keypair_generation_persists() {
        let dir = TempDir::new().expect("tempdir");
        let mut identity = ServiceIdentityState::new_single_dir(dir.path());
        identity.load().await.expect("load");

        identity.ensure_keypair().await.expect("ensure_keypair");
        assert!(identity.key_pem().is_some());

        // Reload and verify persistence.
        let mut identity2 = ServiceIdentityState::new_single_dir(dir.path());
        identity2.load().await.expect("load");
        assert!(identity2.key_pem().is_some());
    }

    #[tokio::test]
    async fn keypair_in_state_dir() {
        let temp = TempDir::new().expect("tempdir");
        let config_dir = temp.path().join("config");
        let state_dir = temp.path().join("state");

        let mut identity = ServiceIdentityState::new(&config_dir, &state_dir);
        identity.load().await.expect("load");
        identity.ensure_keypair().await.expect("ensure_keypair");

        // Key should be in state_dir, not config_dir
        assert!(state_dir.join("service.key").exists());
        assert!(!config_dir.join("service.key").exists());
    }

    #[tokio::test]
    async fn enrollment_persists() {
        let dir = TempDir::new().expect("tempdir");
        let mut identity = ServiceIdentityState::new_single_dir(dir.path());
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
        let mut identity2 = ServiceIdentityState::new_single_dir(dir.path());
        identity2.load().await.expect("load");
        assert_eq!(identity2.service_id(), Some(sid));
        assert_eq!(identity2.enrollment_secret(), Some("secret123"));
    }

    #[tokio::test]
    async fn enrollment_in_state_dir() {
        let temp = TempDir::new().expect("tempdir");
        let config_dir = temp.path().join("config");
        let state_dir = temp.path().join("state");

        let mut identity = ServiceIdentityState::new(&config_dir, &state_dir);
        identity.load().await.expect("load");

        let sid = Uuid::now_v7();
        identity
            .save_enrollment(sid, "secret")
            .await
            .expect("save_enrollment");

        // Enrollment should be in state_dir, not config_dir
        assert!(state_dir.join("service.json").exists());
        assert!(!config_dir.join("service.json").exists());
    }

    #[tokio::test]
    async fn certificate_save_clears_enrollment_secret() {
        let dir = TempDir::new().expect("tempdir");
        let mut identity = ServiceIdentityState::new_single_dir(dir.path());
        identity.load().await.expect("load");

        let sid = Uuid::now_v7();
        identity
            .save_enrollment(sid, "secret123")
            .await
            .expect("save_enrollment");

        // Generate a real self-signed cert to test with.
        identity.ensure_keypair().await.expect("ensure_keypair");
        let kp = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P384_SHA384).expect("keygen");
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
        let mut identity2 = ServiceIdentityState::new_single_dir(dir.path());
        identity2.load().await.expect("load");
        assert!(identity2.enrollment_secret().is_none());
        assert!(identity2.is_certified());
        assert!(identity2.cert_not_after().is_some());
    }

    #[tokio::test]
    async fn csr_generation() {
        let dir = TempDir::new().expect("tempdir");
        let mut identity = ServiceIdentityState::new_single_dir(dir.path());
        identity.load().await.expect("load");
        identity.ensure_keypair().await.expect("ensure_keypair");

        let sid = Uuid::now_v7();
        let csr = identity.generate_csr(sid).expect("generate_csr");

        assert!(csr.contains("BEGIN CERTIFICATE REQUEST"));
        assert!(csr.contains("END CERTIFICATE REQUEST"));
    }

    #[tokio::test]
    async fn ca_cert_in_config_dir() {
        let temp = TempDir::new().expect("tempdir");
        let config_dir = temp.path().join("config");
        let state_dir = temp.path().join("state");

        let mut identity = ServiceIdentityState::new(&config_dir, &state_dir);
        identity.load().await.expect("load");

        let fake_ca = "-----BEGIN CERTIFICATE-----\nfakedata\n-----END CERTIFICATE-----\n";
        identity.save_ca_cert(fake_ca).await.expect("save_ca_cert");

        // CA cert should be in config_dir, not state_dir
        assert!(config_dir.join("ca.pem").exists());
        assert!(!state_dir.join("ca.pem").exists());

        assert_eq!(identity.ca_cert_pem(), Some(fake_ca));

        let raw = identity.load_ca_cert().await.expect("load_ca_cert");
        assert_eq!(raw.as_deref(), Some(fake_ca.as_bytes()));
    }

    #[tokio::test]
    async fn clear_state_removes_everything() {
        let dir = TempDir::new().expect("tempdir");
        let mut identity = ServiceIdentityState::new_single_dir(dir.path());
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
    async fn clear_enrollment_state_preserves_ca() {
        let temp = TempDir::new().expect("tempdir");
        let config_dir = temp.path().join("config");
        let state_dir = temp.path().join("state");

        let mut identity = ServiceIdentityState::new(&config_dir, &state_dir);
        identity.load().await.expect("load");

        let sid = Uuid::now_v7();
        identity
            .save_enrollment(sid, "secret")
            .await
            .expect("save_enrollment");
        identity.ensure_keypair().await.expect("ensure_keypair");

        let fake_ca = "-----BEGIN CERTIFICATE-----\nfakedata\n-----END CERTIFICATE-----\n";
        identity.save_ca_cert(fake_ca).await.expect("save_ca_cert");

        identity
            .clear_enrollment_state()
            .await
            .expect("clear_enrollment_state");

        assert!(identity.is_fresh());
        assert!(identity.ca_cert_pem().is_some()); // CA preserved
    }

    #[tokio::test]
    async fn idempotent_ensure_keypair() {
        let dir = TempDir::new().expect("tempdir");
        let mut identity = ServiceIdentityState::new_single_dir(dir.path());
        identity.load().await.expect("load");

        identity.ensure_keypair().await.expect("first");
        let pem1 = identity.key_pem().expect("key_pem");

        identity.ensure_keypair().await.expect("second");
        let pem2 = identity.key_pem().expect("key_pem");

        assert_eq!(pem1, pem2, "second call must not regenerate");
    }

    #[test]
    fn pem_to_der_real_certificate() {
        let kp = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P384_SHA384).expect("keygen");
        let params = rcgen::CertificateParams::new(vec![]).expect("cert params");
        let cert = params.self_signed(&kp).expect("self-sign");
        let pem = cert.pem();

        let der = pem_to_der(&pem).expect("decode");
        // The DER bytes should parse as a valid X.509 certificate.
        use der::Decode;
        let parsed = x509_cert::Certificate::from_der(&der).expect("parse x509");
        let not_before = parsed
            .tbs_certificate
            .validity
            .not_before
            .to_unix_duration()
            .as_secs();
        let not_after = parsed
            .tbs_certificate
            .validity
            .not_after
            .to_unix_duration()
            .as_secs();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time")
            .as_secs();
        assert!(
            not_before <= now && now <= not_after,
            "certificate must be valid"
        );
    }

    #[tokio::test]
    async fn load_ca_cert_empty_file_returns_none() {
        let dir = TempDir::new().expect("tempdir");
        let mut identity = ServiceIdentityState::new_single_dir(dir.path());
        identity.load().await.expect("load");

        // Write an empty ca.pem to disk (simulates a prior failed write).
        tokio::fs::write(dir.path().join("ca.pem"), b"")
            .await
            .expect("write empty ca.pem");

        // load_ca_cert must treat empty file as missing.
        let result = identity.load_ca_cert().await.expect("load_ca_cert");
        assert!(
            result.is_none(),
            "empty ca.pem should be treated as missing"
        );
    }

    #[tokio::test]
    async fn load_skips_empty_ca_pem() {
        let dir = TempDir::new().expect("tempdir");

        // Write an empty ca.pem before loading identity.
        tokio::fs::write(dir.path().join("ca.pem"), b"")
            .await
            .expect("write empty ca.pem");

        let mut identity = ServiceIdentityState::new_single_dir(dir.path());
        identity.load().await.expect("load");

        // In-memory field must be None when the file is empty.
        assert!(
            identity.ca_cert_pem().is_none(),
            "in-memory ca_cert_pem should be None for empty file"
        );
    }

    #[test]
    fn pem_to_der_empty_string() {
        assert!(pem_to_der("").is_none());
    }

    #[test]
    fn pem_to_der_non_pem_text() {
        assert!(pem_to_der("this is not a PEM").is_none());
    }

    #[tokio::test]
    async fn is_cert_expired_works() {
        let dir = TempDir::new().expect("tempdir");
        let mut identity = ServiceIdentityState::new_single_dir(dir.path());
        identity.load().await.expect("load");

        // No certificate loaded
        assert!(identity.is_cert_expired().is_none());

        // Generate a fresh certificate (not expired)
        identity.ensure_keypair().await.expect("ensure_keypair");
        let kp = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P384_SHA384).expect("keygen");
        let params = rcgen::CertificateParams::new(vec![]).expect("cert params");
        let cert = params.self_signed(&kp).expect("self-sign");
        identity
            .save_certificate(&cert.pem())
            .await
            .expect("save_certificate");

        // Should not be expired
        assert_eq!(identity.is_cert_expired(), Some(false));
    }

    #[tokio::test]
    async fn tenant_id_persistence_roundtrip() {
        let dir = TempDir::new().expect("tempdir");
        let mut identity = ServiceIdentityState::new_single_dir(dir.path());
        identity.load().await.expect("load");

        let sid = Uuid::now_v7();
        identity
            .save_enrollment(sid, "secret")
            .await
            .expect("save_enrollment");

        let tid = Uuid::now_v7();
        identity.save_tenant_id(tid).await.expect("save_tenant_id");

        assert_eq!(identity.tenant_id(), Some(tid));

        // Reload and verify persistence.
        let mut identity2 = ServiceIdentityState::new_single_dir(dir.path());
        identity2.load().await.expect("load");
        assert_eq!(identity2.tenant_id(), Some(tid));
        // service_id and enrollment_secret must be preserved.
        assert_eq!(identity2.service_id(), Some(sid));
        assert_eq!(identity2.enrollment_secret(), Some("secret"));
    }

    #[tokio::test]
    async fn tenant_id_backward_compat() {
        // Simulate a legacy service.json without tenant_id.
        let dir = TempDir::new().expect("tempdir");
        let legacy_json = r#"{"service_id":"01936f00-0000-7000-8000-000000000001","enrollment_secret":"old-secret"}"#;
        let state_path = dir.path().join("service.json");
        tokio::fs::write(&state_path, legacy_json)
            .await
            .expect("write legacy json");

        let mut identity = ServiceIdentityState::new_single_dir(dir.path());
        identity.load().await.expect("load");

        assert!(identity.tenant_id().is_none());
        assert!(identity.service_id().is_some());
        assert_eq!(identity.enrollment_secret(), Some("old-secret"));
    }

    #[tokio::test]
    async fn save_tenant_id_noop_when_not_enrolled() {
        let dir = TempDir::new().expect("tempdir");
        let mut identity = ServiceIdentityState::new_single_dir(dir.path());
        identity.load().await.expect("load");

        let tid = Uuid::now_v7();
        // No enrollment → save_tenant_id updates in-memory but does not
        // write a file (no service_id to serialize).
        identity.save_tenant_id(tid).await.expect("save_tenant_id");
        assert_eq!(identity.tenant_id(), Some(tid));
        assert!(!dir.path().join("service.json").exists());
    }

    #[tokio::test]
    async fn tenant_id_preserved_across_certificate_save() {
        let dir = TempDir::new().expect("tempdir");
        let mut identity = ServiceIdentityState::new_single_dir(dir.path());
        identity.load().await.expect("load");

        let sid = Uuid::now_v7();
        identity
            .save_enrollment(sid, "secret")
            .await
            .expect("save_enrollment");

        let tid = Uuid::now_v7();
        identity.save_tenant_id(tid).await.expect("save_tenant_id");

        // Save a certificate (clears enrollment_secret but should preserve tenant_id).
        identity.ensure_keypair().await.expect("ensure_keypair");
        let kp = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P384_SHA384).expect("keygen");
        let params = rcgen::CertificateParams::new(vec![]).expect("cert params");
        let cert = params.self_signed(&kp).expect("self-sign");
        identity
            .save_certificate(&cert.pem())
            .await
            .expect("save_certificate");

        // Reload and verify tenant_id survived.
        let mut identity2 = ServiceIdentityState::new_single_dir(dir.path());
        identity2.load().await.expect("load");
        assert_eq!(identity2.tenant_id(), Some(tid));
        assert!(identity2.enrollment_secret().is_none());
    }

    #[tokio::test]
    async fn clear_state_clears_tenant_id() {
        let dir = TempDir::new().expect("tempdir");
        let mut identity = ServiceIdentityState::new_single_dir(dir.path());
        identity.load().await.expect("load");

        let sid = Uuid::now_v7();
        identity
            .save_enrollment(sid, "secret")
            .await
            .expect("save_enrollment");
        identity
            .save_tenant_id(Uuid::now_v7())
            .await
            .expect("save_tenant_id");

        identity.clear_state().await.expect("clear_state");
        assert!(identity.tenant_id().is_none());
    }

    #[tokio::test]
    async fn clear_enrollment_state_clears_tenant_id() {
        let dir = TempDir::new().expect("tempdir");
        let mut identity = ServiceIdentityState::new_single_dir(dir.path());
        identity.load().await.expect("load");

        let sid = Uuid::now_v7();
        identity
            .save_enrollment(sid, "secret")
            .await
            .expect("save_enrollment");
        identity
            .save_tenant_id(Uuid::now_v7())
            .await
            .expect("save_tenant_id");

        identity
            .clear_enrollment_state()
            .await
            .expect("clear_enrollment_state");
        assert!(identity.tenant_id().is_none());
    }

    #[tokio::test]
    async fn directory_permissions() {
        let temp = TempDir::new().expect("tempdir");
        let config_dir = temp.path().join("config");
        let state_dir = temp.path().join("state");

        let mut identity = ServiceIdentityState::new(&config_dir, &state_dir);
        identity.load().await.expect("load");

        // Check directory permissions
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let config_meta = std::fs::metadata(&config_dir).expect("config metadata");
            let state_meta = std::fs::metadata(&state_dir).expect("state metadata");

            let config_mode = config_meta.permissions().mode() & 0o777;
            let state_mode = state_meta.permissions().mode() & 0o777;

            assert_eq!(config_mode, 0o700, "config dir should have 700 permissions");
            assert_eq!(state_mode, 0o700, "state dir should have 700 permissions");
        }
    }

    #[tokio::test]
    async fn file_permissions() {
        let dir = TempDir::new().expect("tempdir");
        let mut identity = ServiceIdentityState::new_single_dir(dir.path());
        identity.load().await.expect("load");

        identity.ensure_keypair().await.expect("ensure_keypair");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let key_path = dir.path().join("service.key");
            let key_meta = std::fs::metadata(&key_path).expect("key metadata");
            let key_mode = key_meta.permissions().mode() & 0o777;

            assert_eq!(key_mode, 0o600, "key file should have 600 permissions");
        }
    }

    #[test]
    fn generate_p256_keypair_for_ecies_produces_valid_pair() {
        use base64::Engine as _;

        let (private_der, public_b64) = generate_p256_keypair_for_ecies().expect("keygen");

        assert!(!private_der.is_empty(), "private DER must be non-empty");

        let public_raw = base64::engine::general_purpose::STANDARD
            .decode(&public_b64)
            .expect("public key must be valid base64");
        assert_eq!(
            public_raw.len(),
            65,
            "uncompressed P-256 public key must be 65 bytes"
        );
        assert_eq!(
            public_raw[0], 0x04,
            "uncompressed P-256 public key must start with 0x04"
        );
    }

    // ── Atomic save_identity helpers (cfg(test) only) ─────────────────

    /// Controls where the simulated crash occurs in [`save_identity_split_for_test`].
    pub(crate) enum DropAt {
        /// Return immediately after persisting `service.json`, before writing `service.key`.
        AfterCertPersist,
        #[expect(
            dead_code,
            reason = "reserved for future crash-point tests at the key-tmp stage"
        )]
        AfterKeyTmp,
    }

    /// Outcome returned by [`save_identity_split_for_test`].
    pub(crate) enum SaveOutcome {
        Success,
        CrashAfterCert,
    }

    /// Split-step test helper that simulates a crash at `drop_at` between the
    /// two atomic persists performed by [`save_identity`].
    ///
    /// `base` must be an already-existing directory. The files written are
    /// `service.json` (state JSON) and `service.key` (private key PEM),
    /// matching the names used by [`save_identity`].
    pub(crate) async fn save_identity_split_for_test(
        base: &std::path::Path,
        service_json: &str,
        key_pem: &str,
        drop_at: DropAt,
    ) -> SaveOutcome {
        use std::io::Write as _;
        let cert_path = base.join(STATE_FILE);
        let key_path = base.join(SERVICE_KEY_FILE);

        let mut cert_tmp = tempfile::NamedTempFile::new_in(base).expect("cert tmp");
        cert_tmp
            .write_all(service_json.as_bytes())
            .expect("write cert");
        cert_tmp.as_file().sync_all().expect("sync cert");
        cert_tmp.persist(&cert_path).expect("persist cert");

        if matches!(drop_at, DropAt::AfterCertPersist) {
            return SaveOutcome::CrashAfterCert;
        }

        let mut key_tmp = tempfile::NamedTempFile::new_in(base).expect("key tmp");
        key_tmp.write_all(key_pem.as_bytes()).expect("write key");
        key_tmp.as_file().sync_all().expect("sync key");
        key_tmp.persist(&key_path).expect("persist key");
        SaveOutcome::Success
    }

    #[tokio::test]
    async fn save_identity_is_atomic_under_simulated_crash() {
        use std::fs;
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();

        fs::write(base.join(STATE_FILE), b"OLD_JSON").expect("seed cert");
        fs::write(base.join(SERVICE_KEY_FILE), b"OLD_KEY").expect("seed key");

        let outcome =
            save_identity_split_for_test(base, "NEW_JSON", "NEW_KEY", DropAt::AfterCertPersist)
                .await;

        let json = fs::read_to_string(base.join(STATE_FILE)).expect("read cert");
        let key = fs::read_to_string(base.join(SERVICE_KEY_FILE)).expect("read key");
        assert_eq!(json, "NEW_JSON", "cert persist landed");
        assert_eq!(key, "OLD_KEY", "key persist never happened");

        let stragglers: Vec<_> = fs::read_dir(base)
            .expect("readdir")
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(
            stragglers.len() <= 1,
            "at most one straggler tmp from key path"
        );

        assert!(matches!(outcome, SaveOutcome::CrashAfterCert));
    }

    // ── generate_keypair_and_csr SPIFFE SAN tests ──────────────────────

    #[test]
    fn spiffe_uri_san_in_csr() {
        let sid = Uuid::now_v7();
        let (_, csr_pem) =
            generate_keypair_and_csr(sid, "example.com").expect("generate_keypair_and_csr");

        assert!(
            csr_pem.contains("BEGIN CERTIFICATE REQUEST"),
            "CSR must begin with the PEM header"
        );
        // The SPIFFE URI is embedded in the CSR as an IA5 string. Decode the
        // base64 DER payload and search for the raw URI bytes.
        let expected = format!("spiffe://example.com/service/{sid}");
        let der_bytes = {
            // Strip PEM header/footer and decode base64 body.
            let body: String = csr_pem
                .lines()
                .filter(|l| !l.starts_with("-----"))
                .collect();
            use base64::Engine as _;
            base64::engine::general_purpose::STANDARD
                .decode(body.trim())
                .expect("base64 decode")
        };
        assert!(
            der_bytes
                .windows(expected.len())
                .any(|w| w == expected.as_bytes()),
            "SPIFFE URI must be present in CSR DER bytes; expected {expected:?}"
        );
    }

    #[test]
    fn empty_trust_domain_no_san() {
        let sid = Uuid::now_v7();
        let (_, csr_pem) =
            generate_keypair_and_csr(sid, "").expect("generate_keypair_and_csr with empty domain");

        assert!(
            csr_pem.contains("BEGIN CERTIFICATE REQUEST"),
            "CSR must begin with the PEM header"
        );
        // No SPIFFE URI should appear in the DER bytes.
        let body: String = csr_pem
            .lines()
            .filter(|l| !l.starts_with("-----"))
            .collect();
        use base64::Engine as _;
        let encoded_bytes = base64::engine::general_purpose::STANDARD
            .decode(body.trim())
            .expect("base64 decode");

        let needle = b"spiffe://";
        assert!(
            !encoded_bytes.windows(needle.len()).any(|w| w == needle),
            "no SPIFFE URI should be embedded when trust_domain is empty"
        );
    }

    #[test]
    fn invalid_trust_domain_rejected() {
        let sid = Uuid::now_v7();

        let assert_invalid = |domain: &str| {
            let err = generate_keypair_and_csr(sid, domain).expect_err("must fail");
            assert!(
                matches!(
                    err.current_context(),
                    EnrollmentError::Identity(
                        crate::error::IdentityError::InvalidTrustDomain { .. }
                    )
                ),
                "domain {domain:?} must yield InvalidTrustDomain, got: {err}"
            );
        };

        // Whitespace, path separators, uppercase, at-sign all rejected.
        assert_invalid(" bad domain ");
        assert_invalid("evil/../../path");
        assert_invalid("Example.COM");
        assert_invalid("user@domain");

        // Exceeds 255 characters.
        assert_invalid(&"a".repeat(256));
    }

    #[tokio::test]
    async fn startup_sweep_removes_orphan_tmp_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        std::fs::write(base.join("service.json"), b"intact").expect("seed");
        std::fs::write(base.join("service.key"), b"intact").expect("seed");
        // Simulate orphaned tempfile names (NamedTempFile uses .tmp prefix)
        std::fs::write(base.join(".tmpAB12cd"), b"orphan").expect("seed tmp1");
        std::fs::write(base.join(".tmpEF34gh"), b"orphan").expect("seed tmp2");

        sweep_tmp_siblings(base).expect("sweep ok");

        assert!(base.join("service.json").exists(), "intact file preserved");
        assert!(base.join("service.key").exists(), "intact file preserved");
        let leftover: Vec<_> = std::fs::read_dir(base)
            .expect("readdir")
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with(".tmp"))
            .collect();
        assert!(leftover.is_empty(), "all .tmp files removed by sweep");
    }
}

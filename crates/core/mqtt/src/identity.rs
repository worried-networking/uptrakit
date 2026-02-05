//! Service identity management (keypair, certificate, service_id).
//!
//! Manages the MQTT service's cryptographic identity for mTLS authentication
//! with the controller. Handles:
//! - ECDSA P-256 keypair generation and persistence
//! - CSR generation for certificate requests
//! - Certificate storage and loading
//! - Service ID persistence

use std::path::{Path, PathBuf};

use thiserror::Error;
use tokio::fs;
use uuid::Uuid;

/// Errors that can occur during identity operations.
#[derive(Debug, Error)]
pub enum IdentityError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("keypair generation failed: {0}")]
    KeypairGeneration(String),

    #[error("CSR generation failed: {0}")]
    CsrGeneration(String),

    #[error("identity not enrolled")]
    NotEnrolled,

    #[error("identity not certified (no certificate)")]
    NotCertified,
}

/// File names for state directory (runtime state).
const SERVICE_ID_FILE: &str = "service_id";
const ENROLLMENT_SECRET_FILE: &str = "enrollment_secret";
const PRIVATE_KEY_FILE: &str = "private_key.pem";
const CERTIFICATE_FILE: &str = "certificate.pem";

/// File names for config directory (persistent configuration).
const CA_CERT_FILE: &str = "ca.crt";

/// The MQTT service's identity state.
#[derive(Debug)]
pub struct Identity {
    /// Path to the config directory (for CA cert).
    config_dir: PathBuf,
    /// Path to the state directory (for service identity, keys, cert).
    state_dir: PathBuf,
    /// The service's UUID (assigned by controller during enrollment).
    pub service_id: Option<Uuid>,
    /// The enrollment secret (for bearer auth before certificate issuance).
    enrollment_secret: Option<String>,
    /// The ECDSA P-256 keypair.
    keypair: Option<rcgen::KeyPair>,
    /// The PEM-encoded certificate (after issuance).
    certificate_pem: Option<String>,
    /// The CA certificate bundle (for verifying controller).
    pub ca_cert_pem: Option<String>,
}

impl Identity {
    /// Create a new identity manager with separate config and state directories.
    ///
    /// - config_dir: For persistent configuration (CA certificate)
    /// - state_dir: For runtime state (service_id, keys, certificate)
    pub fn new(config_dir: impl AsRef<Path>, state_dir: impl AsRef<Path>) -> Self {
        Self {
            config_dir: config_dir.as_ref().to_path_buf(),
            state_dir: state_dir.as_ref().to_path_buf(),
            service_id: None,
            enrollment_secret: None,
            keypair: None,
            certificate_pem: None,
            ca_cert_pem: None,
        }
    }

    /// Load existing identity from disk (if any).
    pub async fn load(&mut self) -> Result<(), IdentityError> {
        // Create directories if they don't exist
        fs::create_dir_all(&self.config_dir).await?;
        fs::create_dir_all(&self.state_dir).await?;

        // Try to load service_id (from state directory)
        let service_id_path = self.state_dir.join(SERVICE_ID_FILE);
        if service_id_path.exists() {
            let content = fs::read_to_string(&service_id_path).await?;
            if let Ok(id) = Uuid::parse_str(content.trim()) {
                self.service_id = Some(id);
            }
        }

        // Try to load enrollment secret (from state directory)
        let secret_path = self.state_dir.join(ENROLLMENT_SECRET_FILE);
        if secret_path.exists() {
            let content = fs::read_to_string(&secret_path).await?;
            self.enrollment_secret = Some(content.trim().to_string());
        }

        // Try to load private key (from state directory)
        let key_path = self.state_dir.join(PRIVATE_KEY_FILE);
        if key_path.exists() {
            let key_pem = fs::read_to_string(&key_path).await?;
            self.keypair = Some(
                rcgen::KeyPair::from_pem(&key_pem)
                    .map_err(|e| IdentityError::KeypairGeneration(e.to_string()))?,
            );
        }

        // Try to load certificate (from state directory)
        let cert_path = self.state_dir.join(CERTIFICATE_FILE);
        if cert_path.exists() {
            let cert_pem = fs::read_to_string(&cert_path).await?;
            self.certificate_pem = Some(cert_pem);
        }

        // Try to load CA certificate (from config directory)
        let ca_path = self.config_dir.join(CA_CERT_FILE);
        if ca_path.exists() {
            let ca_pem = fs::read_to_string(&ca_path).await?;
            self.ca_cert_pem = Some(ca_pem);
        }

        Ok(())
    }

    /// Check if this identity has never been enrolled.
    pub fn is_fresh(&self) -> bool {
        self.service_id.is_none()
    }

    /// Check if enrolled but not yet certified.
    pub fn is_enrolled_only(&self) -> bool {
        self.service_id.is_some() && self.certificate_pem.is_none()
    }

    /// Check if fully certified (has certificate).
    pub fn is_certified(&self) -> bool {
        self.service_id.is_some() && self.certificate_pem.is_some()
    }

    /// Get the enrollment secret (for bearer auth).
    pub fn enrollment_secret(&self) -> Option<&str> {
        self.enrollment_secret.as_deref()
    }

    /// Generate a new keypair if not already present.
    pub async fn ensure_keypair(&mut self) -> Result<(), IdentityError> {
        if self.keypair.is_some() {
            return Ok(());
        }

        // Generate new ECDSA P-256 keypair
        let keypair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)
            .map_err(|e| IdentityError::KeypairGeneration(e.to_string()))?;

        // Save to disk (state directory)
        let key_pem = keypair.serialize_pem();
        let key_path = self.state_dir.join(PRIVATE_KEY_FILE);
        fs::write(&key_path, &key_pem).await?;

        self.keypair = Some(keypair);
        Ok(())
    }

    /// Generate a CSR for the given service_id (CN).
    pub fn generate_csr(&self, service_id: Uuid) -> Result<String, IdentityError> {
        let keypair = self
            .keypair
            .as_ref()
            .ok_or(IdentityError::KeypairGeneration(
                "no keypair available".to_string(),
            ))?;

        let mut params = rcgen::CertificateParams::new(vec![])
            .map_err(|e| IdentityError::CsrGeneration(e.to_string()))?;
        params.distinguished_name = rcgen::DistinguishedName::new();
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, service_id.to_string());

        let csr = params
            .serialize_request(keypair)
            .map_err(|e| IdentityError::CsrGeneration(e.to_string()))?;

        csr.pem()
            .map_err(|e| IdentityError::CsrGeneration(e.to_string()))
    }

    /// Save enrollment result (service_id and enrollment_secret).
    pub async fn save_enrollment(
        &mut self,
        service_id: Uuid,
        enrollment_secret: &str,
    ) -> Result<(), IdentityError> {
        // Save service_id (state directory)
        let id_path = self.state_dir.join(SERVICE_ID_FILE);
        fs::write(&id_path, service_id.to_string()).await?;

        // Save enrollment secret (state directory)
        let secret_path = self.state_dir.join(ENROLLMENT_SECRET_FILE);
        fs::write(&secret_path, enrollment_secret).await?;

        self.service_id = Some(service_id);
        self.enrollment_secret = Some(enrollment_secret.to_string());
        Ok(())
    }

    /// Save the issued certificate.
    pub async fn save_certificate(&mut self, cert_pem: &str) -> Result<(), IdentityError> {
        // Save to state directory
        let cert_path = self.state_dir.join(CERTIFICATE_FILE);
        fs::write(&cert_path, cert_pem).await?;
        self.certificate_pem = Some(cert_pem.to_string());

        // Clear enrollment secret (no longer needed once we have a cert)
        let secret_path = self.state_dir.join(ENROLLMENT_SECRET_FILE);
        if secret_path.exists() {
            let _ = fs::remove_file(&secret_path).await;
        }
        self.enrollment_secret = None;

        Ok(())
    }

    /// Save the CA certificate bundle.
    pub async fn save_ca_cert(&mut self, ca_pem: &str) -> Result<(), IdentityError> {
        // Save to config directory
        let ca_path = self.config_dir.join(CA_CERT_FILE);
        fs::write(&ca_path, ca_pem).await?;
        self.ca_cert_pem = Some(ca_pem.to_string());
        Ok(())
    }

    /// Get the private key PEM for building TLS config.
    pub fn private_key_pem(&self) -> Option<String> {
        self.keypair.as_ref().map(|kp| kp.serialize_pem())
    }

    /// Get the certificate PEM.
    pub fn certificate_pem(&self) -> Option<&str> {
        self.certificate_pem.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn fresh_identity_is_fresh() {
        let config_dir = TempDir::new().unwrap();
        let state_dir = TempDir::new().unwrap();
        let mut identity = Identity::new(config_dir.path(), state_dir.path());
        identity.load().await.unwrap();

        assert!(identity.is_fresh());
        assert!(!identity.is_enrolled_only());
        assert!(!identity.is_certified());
    }

    #[tokio::test]
    async fn keypair_generation_persists() {
        let config_dir = TempDir::new().unwrap();
        let state_dir = TempDir::new().unwrap();
        let mut identity = Identity::new(config_dir.path(), state_dir.path());
        identity.load().await.unwrap();

        identity.ensure_keypair().await.unwrap();
        assert!(identity.keypair.is_some());

        // Reload and check persistence
        let mut identity2 = Identity::new(config_dir.path(), state_dir.path());
        identity2.load().await.unwrap();
        assert!(identity2.keypair.is_some());
    }

    #[tokio::test]
    async fn enrollment_persists() {
        let config_dir = TempDir::new().unwrap();
        let state_dir = TempDir::new().unwrap();
        let mut identity = Identity::new(config_dir.path(), state_dir.path());
        identity.load().await.unwrap();

        let service_id = Uuid::now_v7();
        identity
            .save_enrollment(service_id, "secret123")
            .await
            .unwrap();

        assert_eq!(identity.service_id, Some(service_id));
        assert_eq!(identity.enrollment_secret(), Some("secret123"));
        assert!(identity.is_enrolled_only());

        // Reload and check persistence
        let mut identity2 = Identity::new(config_dir.path(), state_dir.path());
        identity2.load().await.unwrap();
        assert_eq!(identity2.service_id, Some(service_id));
        assert_eq!(identity2.enrollment_secret(), Some("secret123"));
    }

    #[tokio::test]
    async fn certificate_save_clears_enrollment_secret() {
        let config_dir = TempDir::new().unwrap();
        let state_dir = TempDir::new().unwrap();
        let mut identity = Identity::new(config_dir.path(), state_dir.path());
        identity.load().await.unwrap();

        let service_id = Uuid::now_v7();
        identity
            .save_enrollment(service_id, "secret123")
            .await
            .unwrap();

        identity
            .save_certificate("-----BEGIN CERTIFICATE-----\ntest\n-----END CERTIFICATE-----\n")
            .await
            .unwrap();

        assert!(identity.enrollment_secret().is_none());
        assert!(identity.is_certified());

        // Reload and check
        let mut identity2 = Identity::new(config_dir.path(), state_dir.path());
        identity2.load().await.unwrap();
        assert!(identity2.enrollment_secret().is_none());
        assert!(identity2.is_certified());
    }

    #[tokio::test]
    async fn csr_generation() {
        let config_dir = TempDir::new().unwrap();
        let state_dir = TempDir::new().unwrap();
        let mut identity = Identity::new(config_dir.path(), state_dir.path());
        identity.load().await.unwrap();
        identity.ensure_keypair().await.unwrap();

        let service_id = Uuid::now_v7();
        let csr = identity.generate_csr(service_id).unwrap();

        assert!(csr.contains("BEGIN CERTIFICATE REQUEST"));
        assert!(csr.contains("END CERTIFICATE REQUEST"));
    }

    #[tokio::test]
    async fn ca_cert_saved_to_config_dir() {
        let config_dir = TempDir::new().unwrap();
        let state_dir = TempDir::new().unwrap();
        let mut identity = Identity::new(config_dir.path(), state_dir.path());
        identity.load().await.unwrap();

        identity.save_ca_cert("test-ca-cert").await.unwrap();

        // Verify it's in config_dir
        assert!(config_dir.path().join("ca.crt").exists());
        // Verify it's NOT in state_dir
        assert!(!state_dir.path().join("ca.crt").exists());
    }
}

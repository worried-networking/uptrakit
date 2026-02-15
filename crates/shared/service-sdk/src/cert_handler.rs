//! Shared handlers for certificate lifecycle controller messages.
//!
//! [`CertificateRenewalHandler`] encapsulates the state and logic for handling
//! `CaBundleUpdated`, `RequestCertRenewal`, and `Certificate` controller
//! messages. Both the agent and MQTT service delegate to this handler instead
//! of duplicating the same logic.

use rootcause::prelude::*;
use uptrakit_internal_wire::{
    CaBundleUpdatedPayload, CertificatePayload, RenewCertificatePayload,
    RequestCertRenewalPayload, ServiceMessage,
};

use crate::connection::ControllerConnection;
use crate::error::{EnrollmentError, IdentityError, Result};
use crate::identity::{ServiceIdentityState, generate_keypair_and_csr};
use crate::lifecycle::LoopOutcome;

/// Handles certificate lifecycle controller messages shared across all service
/// types.
///
/// Manages the private key for in-flight certificate renewals and provides
/// handlers for:
///
/// - [`handle_ca_bundle_updated`](Self::handle_ca_bundle_updated) — persist an
///   updated CA bundle pushed by the controller.
/// - [`handle_request_cert_renewal`](Self::handle_request_cert_renewal) —
///   generate a CSR and send a `RenewCertificate` message.
/// - [`handle_certificate`](Self::handle_certificate) — persist the renewed
///   certificate and private key, trigger reconnect.
///
/// Create one instance per connection and pass it across message-loop
/// iterations. The handler tracks the pending renewal key internally between
/// the `RequestCertRenewal` → `Certificate` pair (or between a timer-based
/// [`initiate_renewal`](Self::initiate_renewal) call and the subsequent
/// `Certificate` response).
pub struct CertificateRenewalHandler {
    /// Private key for a pending renewal CSR, held in memory until the signed
    /// certificate arrives from the controller.
    pending_renewal_key: Option<String>,
}

impl CertificateRenewalHandler {
    /// Create a new handler with no pending renewal.
    pub fn new() -> Self {
        Self {
            pending_renewal_key: None,
        }
    }

    /// Handle a `CaBundleUpdated` message by persisting the new CA bundle.
    ///
    /// Failures are logged as warnings but are not fatal — the service loop
    /// continues regardless.
    pub async fn handle_ca_bundle_updated(
        &self,
        identity: &mut ServiceIdentityState,
        payload: &CaBundleUpdatedPayload,
    ) {
        tracing::info!("received CA bundle update from controller");
        if let Err(e) = identity.save_ca_cert(&payload.ca_bundle_pem).await {
            tracing::warn!(error = %e, "failed to save updated CA bundle");
        } else {
            tracing::info!("updated CA bundle saved to disk");
        }
    }

    /// Handle a `RequestCertRenewal` message by generating a CSR and sending
    /// it to the controller.
    ///
    /// Returns `Some(LoopOutcome::Disconnected)` if the renewal cannot be
    /// initiated (no service ID, keypair generation failure, or send failure).
    /// Returns `None` on success — the loop continues, awaiting the
    /// `Certificate` response.
    pub async fn handle_request_cert_renewal(
        &mut self,
        identity: &ServiceIdentityState,
        conn: &mut ControllerConnection,
        payload: &RequestCertRenewalPayload,
    ) -> Option<LoopOutcome> {
        tracing::info!(reason = %payload.reason, "controller requested certificate renewal");
        let csr_pem = match self.initiate_renewal(identity) {
            Ok(csr) => csr,
            Err(e) => {
                tracing::error!(error = %e, "failed to initiate certificate renewal");
                return Some(LoopOutcome::Disconnected);
            }
        };
        if let Err(e) = conn
            .send(ServiceMessage::RenewCertificate(RenewCertificatePayload {
                csr_pem,
            }))
            .await
        {
            tracing::error!(error = %e, "failed to send renewal request");
            return Some(LoopOutcome::Disconnected);
        }
        tracing::debug!("sent RenewCertificate in response to RequestCertRenewal");
        None
    }

    /// Handle a `Certificate` message by persisting the renewed certificate
    /// and private key.
    ///
    /// Returns `LoopOutcome::Reconnect` on success (the service should
    /// reconnect with the new credentials). Returns
    /// `LoopOutcome::Disconnected` if no pending renewal key exists.
    pub async fn handle_certificate(
        &mut self,
        identity: &mut ServiceIdentityState,
        payload: &CertificatePayload,
    ) -> Result<LoopOutcome> {
        let key_pem = match self.pending_renewal_key.take() {
            Some(k) => k,
            None => {
                tracing::error!("received certificate but no pending renewal key");
                return Ok(LoopOutcome::Disconnected);
            }
        };
        identity.save_certificate(&payload.cert_pem).await?;
        identity.save_private_key(&key_pem).await?;
        tracing::info!("renewed certificate saved, reconnecting");
        Ok(LoopOutcome::Reconnect)
    }

    /// Generate a keypair and CSR for certificate renewal, storing the private
    /// key internally until the signed certificate arrives via
    /// [`handle_certificate`](Self::handle_certificate).
    ///
    /// Returns the CSR PEM string to be sent to the controller in a
    /// `RenewCertificate` message.
    ///
    /// Called internally by
    /// [`handle_request_cert_renewal`](Self::handle_request_cert_renewal)
    /// and can also be called directly for timer-based renewal (e.g. in the
    /// agent's renewal window logic).
    pub fn initiate_renewal(
        &mut self,
        identity: &ServiceIdentityState,
    ) -> Result<String> {
        let service_id = identity.service_id().ok_or_else(|| {
            report!(EnrollmentError::Identity(IdentityError::NotEnrolled))
        })?;
        let (key_pem, csr_pem) = generate_keypair_and_csr(&service_id.to_string())?;
        self.pending_renewal_key = Some(key_pem);
        Ok(csr_pem)
    }
}

impl Default for CertificateRenewalHandler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn initiate_renewal_returns_csr() {
        let dir = TempDir::new().expect("tempdir");
        let mut identity = ServiceIdentityState::new_single_dir(dir.path());
        identity.load().await.expect("load");

        let sid = uuid::Uuid::now_v7();
        identity
            .save_enrollment(sid, "secret")
            .await
            .expect("save_enrollment");

        let mut handler = CertificateRenewalHandler::new();
        let csr_pem = handler.initiate_renewal(&identity).expect("initiate");

        assert!(csr_pem.contains("BEGIN CERTIFICATE REQUEST"));
        assert!(handler.pending_renewal_key.is_some());
    }

    #[tokio::test]
    async fn initiate_renewal_not_enrolled() {
        let dir = TempDir::new().expect("tempdir");
        let mut identity = ServiceIdentityState::new_single_dir(dir.path());
        identity.load().await.expect("load");

        let mut handler = CertificateRenewalHandler::new();
        let result = handler.initiate_renewal(&identity);

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn handle_certificate_no_pending_key() {
        let dir = TempDir::new().expect("tempdir");
        let mut identity = ServiceIdentityState::new_single_dir(dir.path());
        identity.load().await.expect("load");

        let mut handler = CertificateRenewalHandler::new();
        // No prior initiate_renewal — no pending key.
        let payload: CertificatePayload = serde_json::from_value(serde_json::json!({
            "cert_pem": "-----BEGIN CERTIFICATE-----\ntest\n-----END CERTIFICATE-----",
            "not_after": 0
        }))
        .expect("deserialize");

        let outcome = handler
            .handle_certificate(&mut identity, &payload)
            .await
            .expect("should not error");
        assert_eq!(outcome, LoopOutcome::Disconnected);
    }

    #[tokio::test]
    async fn handle_certificate_saves_cert_and_key() {
        let dir = TempDir::new().expect("tempdir");
        let mut identity = ServiceIdentityState::new_single_dir(dir.path());
        identity.load().await.expect("load");

        let sid = uuid::Uuid::now_v7();
        identity
            .save_enrollment(sid, "secret")
            .await
            .expect("save_enrollment");

        let mut handler = CertificateRenewalHandler::new();
        let _csr = handler.initiate_renewal(&identity).expect("initiate");

        // Create a real self-signed cert for the payload.
        let kp =
            rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).expect("keygen");
        let params = rcgen::CertificateParams::new(vec![]).expect("cert params");
        let cert = params.self_signed(&kp).expect("self-sign");

        let payload: CertificatePayload = serde_json::from_value(serde_json::json!({
            "cert_pem": cert.pem(),
            "not_after": 9999999999000_i64
        }))
        .expect("deserialize");

        let outcome = handler
            .handle_certificate(&mut identity, &payload)
            .await
            .expect("handle_certificate");
        assert_eq!(outcome, LoopOutcome::Reconnect);

        // Verify files were written.
        assert!(identity.cert_pem().is_some());
        assert!(identity.key_pem().is_some());
        assert!(handler.pending_renewal_key.is_none());
    }

    #[tokio::test]
    async fn handle_ca_bundle_updated_saves() {
        let dir = TempDir::new().expect("tempdir");
        let mut identity = ServiceIdentityState::new_single_dir(dir.path());
        identity.load().await.expect("load");

        let handler = CertificateRenewalHandler::new();
        let payload = CaBundleUpdatedPayload {
            ca_bundle_pem: "-----BEGIN CERTIFICATE-----\nfake\n-----END CERTIFICATE-----"
                .to_string(),
        };

        handler
            .handle_ca_bundle_updated(&mut identity, &payload)
            .await;

        assert_eq!(identity.ca_cert_pem(), Some(payload.ca_bundle_pem.as_str()));
    }
}

//! Shared handlers for certificate lifecycle controller messages.
//!
//! [`CertificateRenewalHandler`] encapsulates the state and logic for handling
//! `CaBundleUpdated`, `RequestCertRenewal`, and `Certificate` controller
//! messages. Both the agent and MQTT service delegate to this handler instead
//! of duplicating the same logic.
//!
//! This module also provides shared helpers for proactive certificate renewal
//! timers:
//!
//! - [`FAR_FUTURE`] — default delay when no renewal is scheduled.
//! - [`compute_renewal_delay`] — calculates how long until the renewal window.
//! - [`create_renewal_sleep`] — creates a pinned `Sleep` initialized to
//!   [`FAR_FUTURE`].
//! - [`update_renewal_schedule`] — resets a pinned `Sleep` based on
//!   certificate expiry and renewal window.

use std::pin::Pin;
use std::time::Duration;

use crate::generated::wire::{
    CaBundleUpdatedPayload, CertificatePayload, RenewCertificatePayload, RequestCertRenewalPayload,
    ServiceMessage, now_millis,
};
use rootcause::prelude::*;

use crate::connection::ControllerConnection;
use crate::error::{EnrollmentError, IdentityError, Result};
use crate::identity::{ServiceIdentityState, generate_keypair_and_csr};
use crate::shared_types::LoopOutcome;

/// Far-future delay used when no renewal is scheduled (30 days).
pub const FAR_FUTURE: Duration = Duration::from_secs(30 * 24 * 3600);

/// Compute how long until the renewal window opens.
///
/// Returns [`FAR_FUTURE`] when `cert_not_after_ts` is `None` (no certificate
/// known). Returns `Duration::ZERO` when the renewal window is already open
/// or the certificate has expired.
pub fn compute_renewal_delay(cert_not_after_ts: Option<i64>, window_hours: u16) -> Duration {
    match cert_not_after_ts {
        Some(not_after) => {
            let renew_at = not_after - i64::from(window_hours) * 3600 * 1000;
            let delay_ms = (renew_at - now_millis()).max(0) as u64;
            Duration::from_millis(delay_ms)
        }
        None => FAR_FUTURE,
    }
}

/// Create a pinned `Sleep` future initialized to [`FAR_FUTURE`].
///
/// Use [`update_renewal_schedule`] to reset it when `ServiceSettings`
/// arrives.
pub fn create_renewal_sleep() -> Pin<Box<tokio::time::Sleep>> {
    Box::pin(tokio::time::sleep(FAR_FUTURE))
}

/// Reset the renewal timer based on the certificate's `not_after` timestamp
/// and the renewal window in hours.
pub fn update_renewal_schedule(
    sleep: &mut Pin<Box<tokio::time::Sleep>>,
    cert_not_after_ts: Option<i64>,
    window_hours: u16,
) {
    sleep.as_mut().reset(
        tokio::time::Instant::now() + compute_renewal_delay(cert_not_after_ts, window_hours),
    );
}

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
    /// continues regardless. An empty `ca_bundle_pem` is silently ignored to
    /// prevent overwriting a valid local CA with an empty file.
    pub async fn handle_ca_bundle_updated(
        &self,
        identity: &mut ServiceIdentityState,
        payload: &CaBundleUpdatedPayload,
    ) {
        if payload.ca_bundle_pem.is_empty() {
            tracing::warn!("received CA bundle update with empty PEM, ignoring");
            return;
        }
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
    pub fn initiate_renewal(&mut self, identity: &ServiceIdentityState) -> Result<String> {
        let service_id = identity
            .service_id()
            .ok_or_else(|| report!(EnrollmentError::Identity(IdentityError::NotEnrolled)))?;
        let (key_pem, csr_pem) = generate_keypair_and_csr(&service_id.to_string())?;
        self.pending_renewal_key = Some(key_pem);
        Ok(csr_pem)
    }

    /// Handle the proactive renewal timer firing: initiate renewal, send CSR,
    /// and reset the timer to [`FAR_FUTURE`].
    ///
    /// Returns `Some(LoopOutcome)` if the renewal cannot be initiated or sent
    /// (the caller should `break` the loop). Returns `None` on success.
    ///
    /// This is the recommended way to handle the `_ = &mut renewal_sleep`
    /// branch in `tokio::select!`. The timer itself is passed in because
    /// `tokio::select!` borrows `self` mutably at the same time.
    pub async fn handle_renewal_timer(
        &mut self,
        identity: &ServiceIdentityState,
        conn: &mut ControllerConnection,
        renewal_sleep: &mut Pin<Box<tokio::time::Sleep>>,
    ) -> Option<LoopOutcome> {
        tracing::info!("renewal window reached, requesting certificate renewal");
        let csr_pem = match self.initiate_renewal(identity) {
            Ok(csr) => csr,
            Err(e) => {
                tracing::error!(error = %e, "cannot renew certificate");
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
        // Reset to far-future so it doesn't fire again.
        renewal_sleep
            .as_mut()
            .reset(tokio::time::Instant::now() + FAR_FUTURE);
        None
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

    // ── compute_renewal_delay ───────────────────────────────────────────

    #[test]
    fn renewal_delay_no_cert() {
        let delay = compute_renewal_delay(None, 168);
        assert_eq!(delay, FAR_FUTURE);
    }

    #[test]
    fn renewal_delay_future_cert() {
        let thirty_days_ms = 30 * 24 * 3600 * 1000_i64;
        let not_after = now_millis() + thirty_days_ms;
        let delay = compute_renewal_delay(Some(not_after), 168);
        let twenty_three_days = Duration::from_millis(23 * 24 * 3600 * 1000);
        assert!(delay >= twenty_three_days - Duration::from_secs(1));
        assert!(delay <= twenty_three_days + Duration::from_secs(1));
    }

    #[test]
    fn renewal_delay_already_in_window() {
        let three_days_ms = 3 * 24 * 3600 * 1000_i64;
        let not_after = now_millis() + three_days_ms;
        let delay = compute_renewal_delay(Some(not_after), 168);
        assert_eq!(delay, Duration::ZERO);
    }

    #[test]
    fn renewal_delay_expired_cert() {
        let not_after = now_millis() - 1000;
        let delay = compute_renewal_delay(Some(not_after), 168);
        assert_eq!(delay, Duration::ZERO);
    }

    #[test]
    fn renewal_delay_zero_window() {
        let one_hour_ms = 3600 * 1000_i64;
        let not_after = now_millis() + one_hour_ms;
        let delay = compute_renewal_delay(Some(not_after), 0);
        assert!(delay >= Duration::from_secs(3599));
        assert!(delay <= Duration::from_secs(3601));
    }

    // ── CertificateRenewalHandler ──────────────────────────────────────

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
        let kp = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).expect("keygen");
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

    // ── create_renewal_sleep / update_renewal_schedule ──────────────────

    #[tokio::test(start_paused = true)]
    async fn create_renewal_sleep_initializes_to_far_future() {
        let mut sleep = create_renewal_sleep();
        // Advance time by 1 hour — should still be pending (FAR_FUTURE is 30 days).
        tokio::time::advance(Duration::from_secs(3600)).await;
        let result = tokio::time::timeout(Duration::ZERO, &mut sleep).await;
        assert!(
            result.is_err(),
            "sleep should not resolve after only 1 hour"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn update_renewal_schedule_resets_to_zero_when_expired() {
        let mut sleep = create_renewal_sleep();
        // Certificate already expired: delay should be zero, so sleep resolves immediately.
        let not_after = now_millis() - 1000;
        update_renewal_schedule(&mut sleep, Some(not_after), 168);

        let result = tokio::time::timeout(Duration::from_millis(100), &mut sleep).await;
        assert!(
            result.is_ok(),
            "sleep should resolve immediately for expired cert"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn update_renewal_schedule_far_future_when_no_cert() {
        let mut sleep = create_renewal_sleep();
        update_renewal_schedule(&mut sleep, None, 168);

        // Advance time by 1 hour — should still be pending (FAR_FUTURE = 30 days).
        tokio::time::advance(Duration::from_secs(3600)).await;
        let result = tokio::time::timeout(Duration::ZERO, &mut sleep).await;
        assert!(result.is_err(), "sleep should not resolve when no cert");
    }

    // ── CertificateRenewalHandler ────────────────────────────────────────

    #[test]
    fn handler_new_has_no_pending_key() {
        let handler = CertificateRenewalHandler::new();
        assert!(handler.pending_renewal_key.is_none());
    }

    #[test]
    fn handler_default_same_as_new() {
        let handler = CertificateRenewalHandler::default();
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

    #[tokio::test]
    async fn handle_ca_bundle_updated_empty_pem_does_not_save() {
        let dir = TempDir::new().expect("tempdir");
        let mut identity = ServiceIdentityState::new_single_dir(dir.path());
        identity.load().await.expect("load");

        let handler = CertificateRenewalHandler::new();
        let payload = CaBundleUpdatedPayload {
            ca_bundle_pem: String::new(),
        };

        handler
            .handle_ca_bundle_updated(&mut identity, &payload)
            .await;

        // No CA should have been written when the payload is empty.
        assert!(
            identity.ca_cert_pem().is_none(),
            "ca_cert_pem should remain None after empty CaBundleUpdated"
        );
        // The file must not exist on disk either.
        assert!(
            !dir.path().join("ca.pem").exists(),
            "ca.pem must not be created from empty CaBundleUpdated"
        );
    }
}

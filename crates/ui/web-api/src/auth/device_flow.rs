use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use rand::Rng;
use rootcause::{Report, prelude::*};
use thiserror::Error;
use time::OffsetDateTime;
use uuid::Uuid;

use super::token::generate_secure_token;

/// TTL for device flow sessions (10 minutes).
const DEVICE_CODE_TTL_SECONDS: i64 = 600;

/// Minimum interval between poll requests (5 seconds).
pub const MIN_POLL_INTERVAL_SECONDS: i64 = 5;

/// Consonant alphabet for user codes (avoids vowels to prevent offensive words).
const USER_CODE_ALPHABET: &[u8] = b"BCDFGHJKLMNPQRSTVWXZ";

#[derive(Debug, Error)]
pub enum DeviceFlowError {
    #[error("device flow not found or expired")]
    NotFound,

    #[error("device flow already authorized")]
    AlreadyAuthorized,

    #[error("device flow polling too fast")]
    RateLimited,

    #[error("token generation failed: {0}")]
    TokenGeneration(String),
}

pub type Result<T> = std::result::Result<T, Report<DeviceFlowError>>;

#[derive(Debug, Clone, PartialEq)]
pub enum DeviceFlowStatus {
    Pending,
    Authorized { user_id: Uuid },
    Expired,
}

struct PendingDeviceFlow {
    user_code: String,
    status: DeviceFlowStatus,
    created_at: OffsetDateTime,
    last_polled_at: Option<OffsetDateTime>,
    client_name: Option<String>,
}

/// In-memory store for pending device authorization flows.
#[derive(Clone)]
pub struct DeviceFlowStore {
    by_device_code: Arc<Mutex<HashMap<String, PendingDeviceFlow>>>,
    by_user_code: Arc<Mutex<HashMap<String, String>>>,
}

impl Default for DeviceFlowStore {
    fn default() -> Self {
        Self::new()
    }
}

impl DeviceFlowStore {
    pub fn new() -> Self {
        Self {
            by_device_code: Arc::new(Mutex::new(HashMap::new())),
            by_user_code: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Create a new device flow session. Returns `(device_code, user_code)`.
    pub fn create(&self, client_name: Option<String>) -> Result<(String, String)> {
        let device_code = generate_secure_token()
            .map_err(|e| report!(DeviceFlowError::TokenGeneration(e.to_string())))?;
        let user_code = generate_user_code();
        let raw_user_code = user_code.replace('-', "");

        let flow = PendingDeviceFlow {
            user_code: raw_user_code.clone(),
            status: DeviceFlowStatus::Pending,
            created_at: OffsetDateTime::now_utc(),
            last_polled_at: None,
            client_name,
        };

        {
            let mut by_dc = self.by_device_code.lock().unwrap();
            by_dc.insert(device_code.clone(), flow);
        }

        {
            let mut by_uc = self.by_user_code.lock().unwrap();
            by_uc.insert(raw_user_code, device_code.clone());
        }

        Ok((device_code, user_code))
    }

    /// Get the current status of a device flow by device code.
    pub fn get_status(&self, device_code: &str) -> Result<DeviceFlowStatus> {
        let by_dc = self.by_device_code.lock().unwrap();

        let flow = by_dc
            .get(device_code)
            .ok_or_else(|| report!(DeviceFlowError::NotFound))?;

        let cutoff = OffsetDateTime::now_utc() - time::Duration::seconds(DEVICE_CODE_TTL_SECONDS);
        if flow.created_at < cutoff {
            return Ok(DeviceFlowStatus::Expired);
        }

        Ok(flow.status.clone())
    }

    /// Check if polling is too fast (rate limiting).
    pub fn is_rate_limited(&self, device_code: &str) -> Result<bool> {
        let by_dc = self.by_device_code.lock().unwrap();

        let flow = by_dc
            .get(device_code)
            .ok_or_else(|| report!(DeviceFlowError::NotFound))?;

        if let Some(last_polled) = flow.last_polled_at {
            let min_interval = time::Duration::seconds(MIN_POLL_INTERVAL_SECONDS);
            if OffsetDateTime::now_utc() - last_polled < min_interval {
                return Ok(true);
            }
        }

        Ok(false)
    }

    /// Record a poll timestamp for rate limiting.
    pub fn record_poll(&self, device_code: &str) -> Result<()> {
        let mut by_dc = self.by_device_code.lock().unwrap();

        let flow = by_dc
            .get_mut(device_code)
            .ok_or_else(|| report!(DeviceFlowError::NotFound))?;
        flow.last_polled_at = Some(OffsetDateTime::now_utc());

        Ok(())
    }

    /// Look up the client name for a device code.
    pub fn get_client_name(&self, device_code: &str) -> Result<Option<String>> {
        let by_dc = self.by_device_code.lock().unwrap();

        let flow = by_dc
            .get(device_code)
            .ok_or_else(|| report!(DeviceFlowError::NotFound))?;
        Ok(flow.client_name.clone())
    }

    /// Approve a device flow by user code, setting the authorized user.
    pub fn approve(&self, user_code: &str, user_id: Uuid) -> Result<()> {
        let normalized = user_code.replace('-', "").to_uppercase();

        let device_code = {
            let by_uc = self.by_user_code.lock().unwrap();
            by_uc
                .get(&normalized)
                .cloned()
                .ok_or_else(|| report!(DeviceFlowError::NotFound))?
        };

        let mut by_dc = self.by_device_code.lock().unwrap();

        let flow = by_dc
            .get_mut(&device_code)
            .ok_or_else(|| report!(DeviceFlowError::NotFound))?;

        let cutoff = OffsetDateTime::now_utc() - time::Duration::seconds(DEVICE_CODE_TTL_SECONDS);
        if flow.created_at < cutoff {
            return Err(report!(DeviceFlowError::NotFound));
        }

        if matches!(flow.status, DeviceFlowStatus::Authorized { .. }) {
            return Err(report!(DeviceFlowError::AlreadyAuthorized));
        }

        flow.status = DeviceFlowStatus::Authorized { user_id };

        Ok(())
    }

    /// Consume a device flow (one-time use). Removes the flow from both maps.
    pub fn consume(&self, device_code: &str) -> Result<(Uuid, Option<String>)> {
        let mut by_dc = self.by_device_code.lock().unwrap();

        let flow = by_dc
            .remove(device_code)
            .ok_or_else(|| report!(DeviceFlowError::NotFound))?;

        // Also remove from user_code map
        let mut by_uc = self.by_user_code.lock().unwrap();
        by_uc.remove(&flow.user_code);

        match flow.status {
            DeviceFlowStatus::Authorized { user_id } => Ok((user_id, flow.client_name)),
            _ => Err(report!(DeviceFlowError::NotFound)),
        }
    }

    /// Remove expired device flows.
    pub fn cleanup_expired(&self) {
        let cutoff = OffsetDateTime::now_utc() - time::Duration::seconds(DEVICE_CODE_TTL_SECONDS);

        let mut by_dc = self.by_device_code.lock().unwrap();
        let expired_user_codes: Vec<String> = by_dc
            .iter()
            .filter(|(_, flow)| flow.created_at < cutoff)
            .map(|(_, flow)| flow.user_code.clone())
            .collect();

        by_dc.retain(|_, flow| flow.created_at >= cutoff);
        drop(by_dc);

        let mut by_uc = self.by_user_code.lock().unwrap();
        for uc in &expired_user_codes {
            by_uc.remove(uc);
        }
    }
}

/// Generate a user-friendly code: 8 uppercase consonants, formatted as XXXX-XXXX.
fn generate_user_code() -> String {
    let mut rng = rand::rng();
    let chars: Vec<u8> = (0..8)
        .map(|_| {
            let idx = rng.random_range(0..USER_CODE_ALPHABET.len());
            USER_CODE_ALPHABET[idx]
        })
        .collect();

    let first: String = chars[..4].iter().map(|&b| b as char).collect();
    let second: String = chars[4..].iter().map(|&b| b as char).collect();

    format!("{first}-{second}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_flow() {
        let store = DeviceFlowStore::new();
        let (device_code, user_code) = store.create(Some("test-client".into())).unwrap();

        assert!(!device_code.is_empty());
        assert_eq!(user_code.len(), 9); // XXXX-XXXX
        assert_eq!(&user_code[4..5], "-");

        // All chars should be consonants
        for ch in user_code.replace('-', "").chars() {
            assert!(
                USER_CODE_ALPHABET.contains(&(ch as u8)),
                "unexpected char: {ch}"
            );
        }
    }

    #[test]
    fn test_status_pending() {
        let store = DeviceFlowStore::new();
        let (device_code, _) = store.create(None).unwrap();

        let status = store.get_status(&device_code).unwrap();
        assert_eq!(status, DeviceFlowStatus::Pending);
    }

    #[test]
    fn test_approve_and_status() {
        let store = DeviceFlowStore::new();
        let (device_code, user_code) = store.create(None).unwrap();
        let user_id = Uuid::now_v7();

        store.approve(&user_code, user_id).unwrap();

        let status = store.get_status(&device_code).unwrap();
        assert_eq!(status, DeviceFlowStatus::Authorized { user_id });
    }

    #[test]
    fn test_approve_normalizes_code() {
        let store = DeviceFlowStore::new();
        let (_device_code, user_code) = store.create(None).unwrap();
        let user_id = Uuid::now_v7();

        // Approve with lowercase and hyphen
        let lower = user_code.to_lowercase();
        store.approve(&lower, user_id).unwrap();
    }

    #[test]
    fn test_approve_already_authorized() {
        let store = DeviceFlowStore::new();
        let (_device_code, user_code) = store.create(None).unwrap();
        let user_id = Uuid::now_v7();

        store.approve(&user_code, user_id).unwrap();

        let err = store.approve(&user_code, user_id).unwrap_err();
        assert!(matches!(
            err.current_context(),
            DeviceFlowError::AlreadyAuthorized
        ));
    }

    #[test]
    fn test_consume_one_time_use() {
        let store = DeviceFlowStore::new();
        let (device_code, user_code) = store.create(Some("cli-host-2026".into())).unwrap();
        let user_id = Uuid::now_v7();

        store.approve(&user_code, user_id).unwrap();

        let (uid, client_name) = store.consume(&device_code).unwrap();
        assert_eq!(uid, user_id);
        assert_eq!(client_name.as_deref(), Some("cli-host-2026"));

        // Second consume should fail
        let err = store.consume(&device_code).unwrap_err();
        assert!(matches!(err.current_context(), DeviceFlowError::NotFound));
    }

    #[test]
    fn test_consume_pending_fails() {
        let store = DeviceFlowStore::new();
        let (device_code, _) = store.create(None).unwrap();

        let err = store.consume(&device_code).unwrap_err();
        assert!(matches!(err.current_context(), DeviceFlowError::NotFound));
    }

    #[test]
    fn test_not_found() {
        let store = DeviceFlowStore::new();

        let err = store.get_status("nonexistent").unwrap_err();
        assert!(matches!(err.current_context(), DeviceFlowError::NotFound));

        let err = store.approve("NOPE-CODE", Uuid::now_v7()).unwrap_err();
        assert!(matches!(err.current_context(), DeviceFlowError::NotFound));
    }

    #[test]
    fn test_rate_limiting() {
        let store = DeviceFlowStore::new();
        let (device_code, _) = store.create(None).unwrap();

        // First poll: not rate limited
        assert!(!store.is_rate_limited(&device_code).unwrap());

        // Record poll
        store.record_poll(&device_code).unwrap();

        // Immediately poll again: should be rate limited
        assert!(store.is_rate_limited(&device_code).unwrap());
    }

    #[test]
    fn test_cleanup_expired() {
        let store = DeviceFlowStore::new();

        // Create a flow and manually backdate it
        let (device_code, user_code) = store.create(None).unwrap();
        let raw_user_code = user_code.replace('-', "");

        {
            let mut by_dc = store.by_device_code.lock().unwrap();
            if let Some(flow) = by_dc.get_mut(&device_code) {
                flow.created_at = OffsetDateTime::now_utc()
                    - time::Duration::seconds(DEVICE_CODE_TTL_SECONDS + 1);
            }
        }

        store.cleanup_expired();

        // Flow should be gone
        let err = store.get_status(&device_code).unwrap_err();
        assert!(matches!(err, DeviceFlowError::NotFound));

        // User code should also be gone
        let by_uc = store.by_user_code.lock().unwrap();
        assert!(!by_uc.contains_key(&raw_user_code));
    }

    #[test]
    fn test_expired_flow_returns_expired_status() {
        let store = DeviceFlowStore::new();
        let (device_code, _) = store.create(None).unwrap();

        // Backdate the flow
        {
            let mut by_dc = store.by_device_code.lock().unwrap();
            if let Some(flow) = by_dc.get_mut(&device_code) {
                flow.created_at = OffsetDateTime::now_utc()
                    - time::Duration::seconds(DEVICE_CODE_TTL_SECONDS + 1);
            }
        }

        let status = store.get_status(&device_code).unwrap();
        assert_eq!(status, DeviceFlowStatus::Expired);
    }

    #[test]
    fn test_user_code_format() {
        // Generate many codes and verify format
        for _ in 0..100 {
            let code = generate_user_code();
            assert_eq!(code.len(), 9);
            assert_eq!(code.as_bytes()[4], b'-');
            for ch in code.replace('-', "").chars() {
                assert!(USER_CODE_ALPHABET.contains(&(ch as u8)));
            }
        }
    }
}

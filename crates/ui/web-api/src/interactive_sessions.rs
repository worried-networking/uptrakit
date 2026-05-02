//! Registry for active interactive update sessions.
//!
//! Enforces single-writer semantics: only one user can send stdin to a given
//! update at a time. Multiple users can still observe the output via SSE.
//!
//! This entire module is gated on the `interactive` feature.

use std::collections::HashMap;

use time::OffsetDateTime;
use uuid::Uuid;

/// Tracks active interactive sessions and enforces single-writer access.
#[derive(Clone)]
pub struct InteractiveSessionRegistry {
    sessions: std::sync::Arc<parking_lot::Mutex<HashMap<Uuid, InteractiveSession>>>,
}

/// Metadata for an active interactive session.
struct InteractiveSession {
    user_id: Uuid,
    connected_at: OffsetDateTime,
}

impl Default for InteractiveSessionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl InteractiveSessionRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self {
            sessions: std::sync::Arc::new(parking_lot::Mutex::new(HashMap::new())),
        }
    }

    /// Try to claim an interactive session for the given update.
    ///
    /// Returns `Ok(())` if the session was claimed, or `Err(existing_user_id)`
    /// if another user already holds the session.
    pub fn try_claim(&self, update_history_id: Uuid, user_id: Uuid) -> Result<(), Uuid> {
        let mut sessions = self.sessions.lock();
        if let Some(existing) = sessions.get(&update_history_id)
            && existing.user_id != user_id
        {
            return Err(existing.user_id);
        }
        sessions.insert(
            update_history_id,
            InteractiveSession {
                user_id,
                connected_at: OffsetDateTime::now_utc(),
            },
        );
        Ok(())
    }

    /// Release the interactive session for the given update.
    ///
    /// Only releases if the session is held by the given user.
    pub fn release(&self, update_history_id: Uuid, user_id: Uuid) {
        let mut sessions = self.sessions.lock();
        if let Some(session) = sessions.get(&update_history_id)
            && session.user_id == user_id
        {
            sessions.remove(&update_history_id);
        }
    }

    /// Check if an interactive session is active for the given update.
    pub fn is_active(&self, update_history_id: &Uuid) -> bool {
        self.sessions.lock().contains_key(update_history_id)
    }

    /// Return the owner and connection time for an active session.
    pub fn session_info(&self, update_history_id: &Uuid) -> Option<(Uuid, OffsetDateTime)> {
        self.sessions
            .lock()
            .get(update_history_id)
            .map(|s| (s.user_id, s.connected_at))
    }
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::assertions_on_result_states,
        reason = "test assertions use assert!(result.is_ok()) pattern"
    )]

    use super::*;

    #[test]
    fn claim_and_release() {
        let registry = InteractiveSessionRegistry::new();
        let update_id = Uuid::now_v7();
        let user_id = Uuid::now_v7();

        assert!(registry.try_claim(update_id, user_id).is_ok());
        assert!(registry.is_active(&update_id));
        registry.release(update_id, user_id);
        assert!(!registry.is_active(&update_id));
    }

    #[test]
    fn single_writer_enforcement() {
        let registry = InteractiveSessionRegistry::new();
        let update_id = Uuid::now_v7();
        let user1 = Uuid::now_v7();
        let user2 = Uuid::now_v7();

        assert!(registry.try_claim(update_id, user1).is_ok());
        assert_eq!(registry.try_claim(update_id, user2), Err(user1));
    }

    #[test]
    fn same_user_can_reclaim() {
        let registry = InteractiveSessionRegistry::new();
        let update_id = Uuid::now_v7();
        let user_id = Uuid::now_v7();

        assert!(registry.try_claim(update_id, user_id).is_ok());
        assert!(registry.try_claim(update_id, user_id).is_ok());
    }

    #[test]
    fn release_by_wrong_user_is_noop() {
        let registry = InteractiveSessionRegistry::new();
        let update_id = Uuid::now_v7();
        let user1 = Uuid::now_v7();
        let user2 = Uuid::now_v7();

        assert!(registry.try_claim(update_id, user1).is_ok());
        registry.release(update_id, user2); // Should not release
        assert!(registry.is_active(&update_id));
    }
}

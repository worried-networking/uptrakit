use std::collections::{HashMap, VecDeque};
use std::hash::Hash;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use uuid::Uuid;

use uptrakit_wire::surfaces;

use super::SurfaceProxyError;

const MAX_IN_FLIGHT_PER_PROVIDER: usize = 32;
const MAX_IN_FLIGHT_PER_TENANT: usize = 128;
const IDEMPOTENCY_RETENTION: Duration = Duration::from_secs(20 * 60);
const FAILURE_WINDOW: Duration = Duration::from_secs(60);
const FAILURE_LIMIT: usize = 5;
const FAILURE_COOLDOWN: Duration = Duration::from_secs(60);
/// Extra grace beyond a request's own deadline before the backstop sweep reaps it.
/// Generous on purpose: the sweep is a last-resort GC for genuinely-orphaned entries
/// a live timeout path would already have removed — it must never contend the normal
/// per-request timeout (up to `MAX_TIMEOUT_SECONDS` in `proxy/validation.rs`).
pub(super) const IN_FLIGHT_SWEEP_MARGIN: Duration = Duration::from_secs(60);

// Fields are pub(super) as an artifact of the mechanical decomposition — mutate pending state
// through PendingState's methods, not the fields.
#[derive(Default)]
pub(super) struct PendingState {
    pub(super) pending: HashMap<Uuid, PendingRequest>,
    pub(super) in_flight_per_provider: HashMap<String, usize>,
    pub(super) in_flight_per_tenant: HashMap<Uuid, usize>,
    pub(super) in_flight_idempotency: HashMap<IdempotencyKey, IdempotencyInFlight>,
    pub(super) idempotency_cache: HashMap<IdempotencyKey, CachedIdempotent>,
    pub(super) provider_failures: HashMap<String, ProviderFailureState>,
}

#[derive(Debug)]
pub(super) struct PendingRequest {
    pub(super) provider_id: String,
    pub(super) tenant_id: Uuid,
    pub(super) idempotency_key: IdempotencyKey,
    pub(super) deadline: std::time::Instant,
    pub(super) sender: tokio::sync::oneshot::Sender<surfaces::SurfaceActionResponse>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct IdempotencyKey {
    pub(super) tenant_id: Uuid,
    pub(super) surface_id: String,
    pub(super) interaction_id: String,
    pub(super) caller_key: String,
    pub(super) idempotency_key: String,
}

#[derive(Debug, Clone)]
pub(super) struct IdempotencyInFlight {
    request_fingerprint: u64,
    owner: Uuid,
    deadline: std::time::Instant,
}

#[derive(Debug, Clone)]
pub(super) struct CachedIdempotent {
    pub(super) request_fingerprint: u64,
    pub(super) response: surfaces::SurfaceActionResponse,
    pub(super) stored_at: std::time::Instant,
}

#[derive(Debug, Default)]
pub(super) struct ProviderFailureState {
    failures: VecDeque<std::time::Instant>,
    blocked_until: Option<std::time::Instant>,
}

pub(super) struct PendingRegistration<'a> {
    pub(super) request_id: Uuid,
    pub(super) provider_id: &'a str,
    pub(super) tenant_id: Uuid,
    pub(super) idempotency_key: IdempotencyKey,
    pub(super) request_fingerprint: u64,
    pub(super) deadline: std::time::Instant,
    pub(super) sender: tokio::sync::oneshot::Sender<surfaces::SurfaceActionResponse>,
}

impl PendingState {
    pub(super) fn register_pending(&mut self, reg: PendingRegistration<'_>) {
        let PendingRegistration {
            request_id,
            provider_id,
            tenant_id,
            idempotency_key,
            request_fingerprint,
            deadline,
            sender,
        } = reg;
        self.pending.insert(
            request_id,
            PendingRequest {
                provider_id: provider_id.to_string(),
                tenant_id,
                idempotency_key: idempotency_key.clone(),
                deadline,
                sender,
            },
        );
        *self
            .in_flight_per_provider
            .entry(provider_id.to_string())
            .or_default() += 1;
        *self.in_flight_per_tenant.entry(tenant_id).or_default() += 1;
        self.reserve_idempotency(idempotency_key, request_fingerprint, request_id, deadline);
    }

    pub(super) fn take_pending(
        &mut self,
        request_id: &Uuid,
    ) -> Option<tokio::sync::oneshot::Sender<surfaces::SurfaceActionResponse>> {
        let pending = self.pending.remove(request_id)?;
        decrement_counter(&mut self.in_flight_per_provider, &pending.provider_id);
        decrement_counter(&mut self.in_flight_per_tenant, &pending.tenant_id);
        if self
            .in_flight_idempotency
            .get(&pending.idempotency_key)
            .is_some_and(|in_flight| in_flight.owner == *request_id)
        {
            self.in_flight_idempotency.remove(&pending.idempotency_key);
        }
        Some(pending.sender)
    }

    pub(super) fn remove_pending(&mut self, request_id: &Uuid) -> bool {
        self.take_pending(request_id).is_some()
    }

    pub(super) fn reserve_idempotency(
        &mut self,
        key: IdempotencyKey,
        request_fingerprint: u64,
        owner: Uuid,
        deadline: std::time::Instant,
    ) {
        self.in_flight_idempotency.insert(
            key,
            IdempotencyInFlight {
                request_fingerprint,
                owner,
                deadline,
            },
        );
    }

    pub(super) fn release_idempotency(&mut self, key: &IdempotencyKey, owner: Uuid) {
        if self
            .in_flight_idempotency
            .get(key)
            .is_some_and(|in_flight| in_flight.owner == owner)
        {
            self.in_flight_idempotency.remove(key);
        }
    }

    pub(super) fn ensure_budget(
        &self,
        provider_id: &str,
        tenant_id: Uuid,
    ) -> Result<(), SurfaceProxyError> {
        if self
            .in_flight_per_provider
            .get(provider_id)
            .copied()
            .unwrap_or(0)
            >= MAX_IN_FLIGHT_PER_PROVIDER
        {
            return Err(SurfaceProxyError::RateLimited);
        }
        if self
            .in_flight_per_tenant
            .get(&tenant_id)
            .copied()
            .unwrap_or(0)
            >= MAX_IN_FLIGHT_PER_TENANT
        {
            return Err(SurfaceProxyError::RateLimited);
        }
        Ok(())
    }

    pub(super) fn ensure_idempotency_available(
        &self,
        key: &IdempotencyKey,
        request_fingerprint: u64,
    ) -> Result<(), SurfaceProxyError> {
        if let Some(in_flight) = self.in_flight_idempotency.get(key) {
            if in_flight.request_fingerprint == request_fingerprint {
                return Err(SurfaceProxyError::DuplicateRequest);
            }
            return Err(SurfaceProxyError::DuplicateRequest);
        }
        if let Some(cached) = self.idempotency_cache.get(key)
            && cached.request_fingerprint != request_fingerprint
        {
            return Err(SurfaceProxyError::DuplicateRequest);
        }
        Ok(())
    }

    pub(super) fn ensure_provider_not_rate_limited(
        &self,
        provider_id: &str,
    ) -> Result<(), SurfaceProxyError> {
        let now = std::time::Instant::now();
        if let Some(state) = self.provider_failures.get(provider_id)
            && let Some(blocked_until) = state.blocked_until
            && blocked_until > now
        {
            return Err(SurfaceProxyError::RateLimited);
        }
        Ok(())
    }

    pub(super) fn record_provider_failure(&mut self, provider_id: &str) {
        let now = std::time::Instant::now();
        let tracker = self
            .provider_failures
            .entry(provider_id.to_string())
            .or_default();
        tracker.failures.push_back(now);
        while let Some(oldest) = tracker.failures.front().copied() {
            if now.duration_since(oldest) <= FAILURE_WINDOW {
                break;
            }
            tracker.failures.pop_front();
        }
        if tracker.failures.len() >= FAILURE_LIMIT {
            tracker.blocked_until = Some(now + FAILURE_COOLDOWN);
            tracker.failures.clear();
        }
    }

    pub(super) fn cleanup_expired(&mut self) {
        let now = std::time::Instant::now();
        self.idempotency_cache
            .retain(|_, cached| now.duration_since(cached.stored_at) <= IDEMPOTENCY_RETENTION);
        for tracker in self.provider_failures.values_mut() {
            while let Some(oldest) = tracker.failures.front().copied() {
                if now.duration_since(oldest) <= FAILURE_WINDOW {
                    break;
                }
                tracker.failures.pop_front();
            }
            if tracker
                .blocked_until
                .is_some_and(|blocked_until| blocked_until <= now)
            {
                tracker.blocked_until = None;
            }
        }

        // Backstop: reap in-flight reservations whose own deadline (plus a generous
        // margin) has passed — genuinely orphaned entries a live timeout path would
        // already have removed. Never a global threshold (would reap slow-but-alive
        // long-timeout requests and record a spurious provider failure).
        let reap_ids: Vec<Uuid> = self
            .pending
            .iter()
            .filter(|(_, pending)| pending.deadline + IN_FLIGHT_SWEEP_MARGIN < now)
            .map(|(request_id, _)| *request_id)
            .collect();
        for request_id in &reap_ids {
            let _ = self.take_pending(request_id);
        }
        self.in_flight_idempotency
            .retain(|_, in_flight| in_flight.deadline + IN_FLIGHT_SWEEP_MARGIN >= now);
    }
}

fn decrement_counter<K: Eq + Hash + Clone>(map: &mut HashMap<K, usize>, key: &K) {
    if let Some(counter) = map.get_mut(key) {
        *counter = counter.saturating_sub(1);
        if *counter == 0 {
            map.remove(key);
        }
    }
}

/// RAII cleanup guard for an in-flight ProviderProxied request.
///
/// Held by `invoke_inner` for the lifetime of a proxied request. If the future
/// is dropped at an `.await` (e.g. the HTTP client disconnects), `Drop` runs the
/// shared `take_pending` cleanup — removing the pending entry, decrementing both
/// budget counters, and releasing the owner-tagged idempotency reservation.
///
/// It is a pure backstop: every normal `invoke_inner` return path already removes
/// the entry via another actor (`complete`, `timeout_pending_request`,
/// `fail_pending_request`), so `Drop` is a presence-checked no-op on those paths.
///
/// Drop-safety: locks the `parking_lot::Mutex` for a synchronous `take_pending`
/// only — no `.await`, no nested lock, no `unwrap`; a missing entry is a no-op, so
/// cleanup is idempotent by construction.
pub(super) struct PendingGuard {
    pending: Arc<Mutex<PendingState>>,
    request_id: Uuid,
}

impl PendingGuard {
    pub(super) fn new(pending: Arc<Mutex<PendingState>>, request_id: Uuid) -> Self {
        Self {
            pending,
            request_id,
        }
    }
}

impl Drop for PendingGuard {
    fn drop(&mut self) {
        let _ = self.pending.lock().take_pending(&self.request_id);
    }
}

/// RAII cleanup guard for an in-flight ControllerLocal idempotency reservation.
///
/// The ControllerLocal transport reserves only an idempotency entry (no pending
/// map entry, no budget counters) before awaiting the plugin executor. If the
/// future is dropped there, `Drop` releases the owner-tagged reservation so the
/// key is not stuck at `DuplicateRequest`.
///
/// Sole release mechanism for the ControllerLocal reservation: the arm no
/// longer releases explicitly, so `Drop` runs on every exit (success, executor
/// error, validation error, future-drop). Drop-safety is the same as
/// `PendingGuard` — synchronous lock, no await, idempotent (owner-checked).
pub(super) struct IdempotencyGuard {
    pending: Arc<Mutex<PendingState>>,
    idem_key: IdempotencyKey,
    owner: Uuid,
}

impl IdempotencyGuard {
    pub(super) fn new(
        pending: Arc<Mutex<PendingState>>,
        idem_key: IdempotencyKey,
        owner: Uuid,
    ) -> Self {
        Self {
            pending,
            idem_key,
            owner,
        }
    }
}

impl Drop for IdempotencyGuard {
    fn drop(&mut self) {
        self.pending
            .lock()
            .release_idempotency(&self.idem_key, self.owner);
    }
}

use std::collections::{HashMap, VecDeque};
use std::hash::Hash;
use std::time::Duration;

use uuid::Uuid;

use uptrakit_internal_wire::surfaces;

use super::SurfaceProxyError;

const MAX_IN_FLIGHT_PER_PROVIDER: usize = 32;
const MAX_IN_FLIGHT_PER_TENANT: usize = 128;
const IDEMPOTENCY_RETENTION: Duration = Duration::from_secs(20 * 60);
const FAILURE_WINDOW: Duration = Duration::from_secs(60);
const FAILURE_LIMIT: usize = 5;
const FAILURE_COOLDOWN: Duration = Duration::from_secs(60);

#[derive(Default)]
pub(super) struct PendingState {
    pending: HashMap<Uuid, PendingRequest>,
    in_flight_per_provider: HashMap<String, usize>,
    in_flight_per_tenant: HashMap<Uuid, usize>,
    in_flight_idempotency: HashMap<IdempotencyKey, IdempotencyInFlight>,
    idempotency_cache: HashMap<IdempotencyKey, CachedIdempotent>,
    provider_failures: HashMap<String, ProviderFailureState>,
}

#[derive(Debug)]
pub(super) struct PendingRequest {
    provider_id: String,
    tenant_id: Uuid,
    idempotency_key: IdempotencyKey,
    sender: tokio::sync::oneshot::Sender<surfaces::SurfaceActionResponse>,
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
}

#[derive(Debug, Clone)]
pub(super) struct CachedIdempotent {
    request_fingerprint: u64,
    response: surfaces::SurfaceActionResponse,
    stored_at: std::time::Instant,
}

#[derive(Debug, Default)]
pub(super) struct ProviderFailureState {
    failures: VecDeque<std::time::Instant>,
    blocked_until: Option<std::time::Instant>,
}

impl PendingState {
    pub(super) fn register_pending(
        &mut self,
        request_id: Uuid,
        provider_id: &str,
        tenant_id: Uuid,
        idempotency_key: IdempotencyKey,
        request_fingerprint: u64,
        sender: tokio::sync::oneshot::Sender<surfaces::SurfaceActionResponse>,
    ) {
        self.pending.insert(
            request_id,
            PendingRequest {
                provider_id: provider_id.to_string(),
                tenant_id,
                idempotency_key: idempotency_key.clone(),
                sender,
            },
        );
        *self
            .in_flight_per_provider
            .entry(provider_id.to_string())
            .or_default() += 1;
        *self.in_flight_per_tenant.entry(tenant_id).or_default() += 1;
        self.reserve_idempotency(idempotency_key, request_fingerprint);
    }

    pub(super) fn take_pending(
        &mut self,
        request_id: &Uuid,
    ) -> Option<tokio::sync::oneshot::Sender<surfaces::SurfaceActionResponse>> {
        let pending = self.pending.remove(request_id)?;
        decrement_counter(&mut self.in_flight_per_provider, &pending.provider_id);
        decrement_counter(&mut self.in_flight_per_tenant, &pending.tenant_id);
        self.in_flight_idempotency.remove(&pending.idempotency_key);
        Some(pending.sender)
    }

    pub(super) fn remove_pending(&mut self, request_id: &Uuid) -> bool {
        self.take_pending(request_id).is_some()
    }

    pub(super) fn reserve_idempotency(&mut self, key: IdempotencyKey, request_fingerprint: u64) {
        self.in_flight_idempotency.insert(
            key,
            IdempotencyInFlight {
                request_fingerprint,
            },
        );
    }

    pub(super) fn release_idempotency(&mut self, key: &IdempotencyKey) {
        self.in_flight_idempotency.remove(key);
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

    pub(super) fn cached_response(
        &self,
        key: &IdempotencyKey,
        request_fingerprint: u64,
    ) -> Option<surfaces::SurfaceActionResponse> {
        let cached = self.idempotency_cache.get(key)?;
        if cached.request_fingerprint == request_fingerprint {
            return Some(cached.response.clone());
        }
        None
    }

    pub(super) fn store_cached_response(
        &mut self,
        key: IdempotencyKey,
        request_fingerprint: u64,
        response: surfaces::SurfaceActionResponse,
    ) {
        self.idempotency_cache.insert(
            key,
            CachedIdempotent {
                request_fingerprint,
                response,
                stored_at: std::time::Instant::now(),
            },
        );
    }

    pub(super) fn fail_in_flight_for_provider(&mut self, provider_id: &str) -> bool {
        let request_ids: Vec<Uuid> = self
            .pending
            .iter()
            .filter_map(|(request_id, pending)| {
                if pending.provider_id == provider_id {
                    Some(*request_id)
                } else {
                    None
                }
            })
            .collect();

        for request_id in &request_ids {
            let _ = self.remove_pending(request_id);
        }

        if !request_ids.is_empty() {
            self.record_provider_failure(provider_id);
            return true;
        }
        false
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

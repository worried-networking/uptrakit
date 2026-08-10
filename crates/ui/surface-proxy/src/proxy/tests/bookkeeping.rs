use std::time::{Duration, Instant};

use uuid::Uuid;

use super::super::bookkeeping::{
    IdempotencyKey, PendingRegistration, PendingRequest, PendingState,
};

fn idem_key(key: &str) -> IdempotencyKey {
    IdempotencyKey {
        tenant_id: Uuid::nil(),
        surface_id: "surface".to_string(),
        interaction_id: "interaction".to_string(),
        caller_key: "caller".to_string(),
        idempotency_key: key.to_string(),
    }
}

#[test]
fn release_idempotency_ignores_non_owner() {
    let mut state = PendingState::default();
    let owner_a = Uuid::now_v7();
    let owner_b = Uuid::now_v7();
    let key = idem_key("k");

    state.reserve_idempotency(
        key.clone(),
        111,
        owner_a,
        Instant::now() + Duration::from_secs(30),
    );

    // A non-owner must not remove A's reservation.
    state.release_idempotency(&key, owner_b);
    assert!(
        state.in_flight_idempotency.contains_key(&key),
        "reservation must survive a non-owner release"
    );

    // The real owner removes it.
    state.release_idempotency(&key, owner_a);
    assert!(
        !state.in_flight_idempotency.contains_key(&key),
        "owner release must remove the reservation"
    );
}

#[test]
fn take_pending_does_not_evict_a_reused_idempotency_reservation() {
    let mut state = PendingState::default();
    let owner_a = Uuid::now_v7();
    let owner_b = Uuid::now_v7();
    let key = idem_key("shared");

    // A's pending entry references `key`, but `key` is currently reserved by B
    // (A released, B re-reserved the same key with its own owner id).
    let (tx, _rx) = tokio::sync::oneshot::channel();
    state.pending.insert(
        owner_a,
        PendingRequest {
            provider_id: "service.provider-a".to_string(),
            tenant_id: Uuid::nil(),
            idempotency_key: key.clone(),
            deadline: Instant::now() + Duration::from_secs(30),
            sender: tx,
        },
    );
    *state
        .in_flight_per_provider
        .entry("service.provider-a".to_string())
        .or_default() += 1;
    *state.in_flight_per_tenant.entry(Uuid::nil()).or_default() += 1;
    state.reserve_idempotency(
        key.clone(),
        222,
        owner_b,
        Instant::now() + Duration::from_secs(30),
    );

    // A's cleanup removes A's pending entry + counters, but must NOT evict B's live reservation.
    let sender = state.take_pending(&owner_a);
    assert!(sender.is_some(), "A's pending entry should be removed");
    assert!(
        state.in_flight_idempotency.contains_key(&key),
        "B's live reservation must survive A's identity-blind-free cleanup"
    );
    assert!(
        state.in_flight_per_provider.is_empty(),
        "provider counter back to zero"
    );
    assert!(
        state.in_flight_per_tenant.is_empty(),
        "tenant counter back to zero"
    );
}

#[test]
fn cleanup_expired_reaps_orphaned_in_flight_past_deadline_plus_margin() {
    let mut state = PendingState::default();
    let request_id = Uuid::now_v7();
    let (tx, _rx) = tokio::sync::oneshot::channel();
    // Deadline already elapsed by more than the sweep margin ⇒ genuinely orphaned.
    let elapsed_deadline = Instant::now()
        .checked_sub(super::super::bookkeeping::IN_FLIGHT_SWEEP_MARGIN + Duration::from_secs(1))
        .expect("test clock is far enough past boot to subtract the sweep margin");
    state.register_pending(PendingRegistration {
        request_id,
        provider_id: "service.provider-a",
        tenant_id: Uuid::nil(),
        idempotency_key: idem_key("orphan"),
        request_fingerprint: 1,
        deadline: elapsed_deadline,
        sender: tx,
    });
    assert_eq!(
        state
            .in_flight_per_provider
            .get("service.provider-a")
            .copied(),
        Some(1)
    );

    state.cleanup_expired();

    assert!(
        state.pending.is_empty(),
        "orphaned pending entry must be reaped"
    );
    assert!(
        state.in_flight_per_provider.is_empty(),
        "provider counter decremented on reap"
    );
    assert!(
        state.in_flight_per_tenant.is_empty(),
        "tenant counter decremented on reap"
    );
    assert!(
        state.in_flight_idempotency.is_empty(),
        "idempotency reservation removed on reap"
    );
}

#[test]
fn cleanup_expired_spares_slow_but_alive_request_and_records_no_failure() {
    let mut state = PendingState::default();
    let request_id = Uuid::now_v7();
    let (tx, _rx) = tokio::sync::oneshot::channel();
    // Deadline still 300s in the future ⇒ slow-but-alive, must not be reaped.
    state.register_pending(PendingRegistration {
        request_id,
        provider_id: "service.provider-a",
        tenant_id: Uuid::nil(),
        idempotency_key: idem_key("slow"),
        request_fingerprint: 1,
        deadline: Instant::now() + Duration::from_secs(300),
        sender: tx,
    });

    state.cleanup_expired();

    assert!(
        state.pending.contains_key(&request_id),
        "slow-but-alive request must survive the sweep"
    );
    assert_eq!(
        state
            .in_flight_per_provider
            .get("service.provider-a")
            .copied(),
        Some(1),
        "counter untouched"
    );
    assert!(
        state.provider_failures.is_empty(),
        "the sweep must not record a provider failure for a live request"
    );
}

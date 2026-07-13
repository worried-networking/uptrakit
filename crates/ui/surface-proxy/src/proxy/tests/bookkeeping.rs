use uuid::Uuid;

use super::super::{IdempotencyKey, PendingRequest, PendingState};

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

    state.reserve_idempotency(key.clone(), 111, owner_a);

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
            provider_id: "provider-a".to_string(),
            tenant_id: Uuid::nil(),
            idempotency_key: key.clone(),
            sender: tx,
        },
    );
    *state
        .in_flight_per_provider
        .entry("provider-a".to_string())
        .or_default() += 1;
    *state.in_flight_per_tenant.entry(Uuid::nil()).or_default() += 1;
    state.reserve_idempotency(key.clone(), 222, owner_b);

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

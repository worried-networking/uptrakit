# Surfaces Dynamic Update Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement
> this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Emit `AdminEvent::SurfacesChanged` from all four service lifecycle sites and wire a debounced `loadSurfaceRegistry()` call on the frontend
so the surface list stays live without page refresh.

**Architecture:** New unit variant `AdminEvent::SurfacesChanged` (no payload) is broadcast over the per-tenant SSE channel at four emission sites in
the WS handler. Frontend adds `'surfaces_changed'` to `AdminEventType`, subscribes in the root layout, and debounces repeated events into a single
`loadSurfaceRegistry()` re-fetch. No changes to `SurfaceRegistry` itself.

**Tech Stack:** Rust (Axum, SeaORM, tokio broadcast), Svelte 5, Vitest

---

## File Map

| File                                                     | Action | What changes                                                                                      |
| -------------------------------------------------------- | ------ | ------------------------------------------------------------------------------------------------- |
| `crates/shared/web-api-types/src/events.rs`              | Modify | Add `SurfacesChanged` variant, `event_name` arm, update test array + count                        |
| `crates/ui/web-api/src/routes/service_ws/handler/mod.rs` | Modify | 4 emission sites, signature change on `cleanup_embedded_service_session`, update 2 existing tests |
| `frontend/src/lib/sse.ts`                                | Modify | Add `'surfaces_changed'` to `AdminEventType` union                                                |
| `frontend/src/routes/+layout.svelte`                     | Modify | Subscribe to `'surfaces_changed'`, call `loadSurfaceRegistry()`                                   |
| `frontend/src/lib/stores/events.svelte.test.ts`          | Create | Debounce + `loadSurfaceRegistry` call assertions                                                  |

---

## Task 1: Add `AdminEvent::SurfacesChanged` variant

**Files:**

- Modify: `crates/shared/web-api-types/src/events.rs`

- [ ] **Step 1: Add the variant**

In `crates/shared/web-api-types/src/events.rs`, add after `DataReset,`:

```rust
    /// The surface provider registry changed (provider joined or left).
    ///
    /// Carries no payload — coarse invalidation signal. The frontend re-fetches
    /// `GET /api/v1/surfaces` and provider availability on receipt.
    SurfacesChanged,
```

- [ ] **Step 2: Add `event_name()` arm**

In the `event_name()` match block, add after `Self::DataReset => "data_reset",`:

```rust
            Self::SurfacesChanged => "surfaces_changed",
```

- [ ] **Step 3: Add to `all_variants()` test array and update count assertion**

In `all_variants()` (inside `#[cfg(test)] mod tests`), add after `AdminEvent::DataReset,`:

```rust
            AdminEvent::SurfacesChanged,
```

Change the count assertion:

```rust
        assert_eq!(all_variants().len(), 21);
```

- [ ] **Step 4: Add wire format test**

Add after the existing `sse_data_unit_variant_emits_empty_object` test in `events.rs`, in `crates/ui/web-api/src/routes/events.rs`:

```rust
    #[test]
    fn sse_data_surfaces_changed_emits_empty_object() {
        let event = AdminEvent::SurfacesChanged;
        let data = extract_sse_data(&event);
        assert!(data.is_object(), "expected object, got: {data}");
        assert_eq!(
            data.as_object().map(|m| m.len()).unwrap_or(1),
            0,
            "expected empty object: {data}"
        );
    }
```

- [ ] **Step 5: Verify compilation and tests pass**

```bash
cargo check --no-default-features --features db-sqlite -p uptrakit-web-api-types
cargo test -p uptrakit-web-api-types
cargo test -p uptrakit-web-api --no-default-features --features db-sqlite -- routes::events
```

Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add crates/shared/web-api-types/src/events.rs crates/ui/web-api/src/routes/events.rs
git commit --only crates/shared/web-api-types/src/events.rs crates/ui/web-api/src/routes/events.rs -m "feat(events): add AdminEvent::SurfacesChanged unit variant"
```

---

## Task 2: Emit from `cleanup_embedded_service_session`

**Files:**

- Modify: `crates/ui/web-api/src/routes/service_ws/handler/mod.rs`

The function at (grep: `async fn cleanup_embedded_service_session`) currently lacks a `tenant_id` parameter. The call site passes
`session.service_tenant_id`.

- [ ] **Step 1: Add `AdminEvent` import**

Add to the import block at the top of `handler/mod.rs`:

```rust
use uptrakit_web_api_types::events::AdminEvent;
```

- [ ] **Step 2: Add `tenant_id` parameter to function signature**

Change:

```rust
async fn cleanup_embedded_service_session(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    _service_app_name: &str,
    has_workload_claims: bool,
) {
```

To:

```rust
async fn cleanup_embedded_service_session(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    _service_app_name: &str,
    has_workload_claims: bool,
    tenant_id: Option<uuid::Uuid>,
) {
```

- [ ] **Step 3: Add broadcast inside the existing `provider_id_for_service` guard**

The function already has `if let Some(provider_id) = state.surface_proxy_deps.registry.provider_id_for_service(&service_id)` before
`fail_in_flight_for_provider`. Add the broadcast inside that block — only emit when a provider was actually registered, not on every disconnect. The
full block becomes:

```rust
    if let Some(provider_id) = state
        .surface_proxy_deps
        .registry
        .provider_id_for_service(&service_id)
    {
        state
            .surface_proxy_deps
            .proxy
            .fail_in_flight_for_provider(&provider_id);
        if let Some(tid) = tenant_id {
            state
                .notification
                .event_broadcaster
                .send(tid, AdminEvent::SurfacesChanged)
                .await;
        }
    }
    state
        .surface_proxy_deps
        .registry
        .unregister_service(&service_id);
```

`unregister_service` stays unconditional (idempotent no-op when nothing is registered).

- [ ] **Step 4: Update the call site**

Find the `cleanup_embedded_service_session(` call site (grep: `cleanup_embedded_service_session(`). Change it from:

```rust
    cleanup_embedded_service_session(
        &state,
        session.service_id,
        session.app_name,
        has_workload_claims,
    )
    .await;
```

To:

```rust
    cleanup_embedded_service_session(
        &state,
        session.service_id,
        session.app_name,
        has_workload_claims,
        session.service_tenant_id,
    )
    .await;
```

- [ ] **Step 5: Write failing tests**

Add inside `#[cfg(test)] mod tests` in `handler/mod.rs`:

```rust
    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn cleanup_embedded_session_broadcasts_surfaces_changed_when_tenant_present() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db, tenant_id).await;
        let service_id = uuid::Uuid::now_v7();
        register_test_runtime_state(&state, service_id, tenant_id);

        let mut rx = state.notification.event_broadcaster.subscribe(tenant_id).await;

        cleanup_embedded_service_session(
            &state,
            service_id,
            "uptrakit-agent-ssh",
            false,
            Some(tenant_id),
        )
        .await;

        match rx.try_recv() {
            Ok(AdminEvent::SurfacesChanged) => {}
            other => panic!("expected SurfacesChanged, got {other:?}"),
        }
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn cleanup_embedded_session_skips_broadcast_when_no_tenant_id() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db, tenant_id).await;
        let service_id = uuid::Uuid::now_v7();
        register_test_runtime_state(&state, service_id, tenant_id);

        let mut rx = state.notification.event_broadcaster.subscribe(tenant_id).await;

        cleanup_embedded_service_session(
            &state,
            service_id,
            "uptrakit-agent-ssh",
            false,
            None, // system service — no tenant
        )
        .await;

        assert!(
            rx.try_recv().is_err(),
            "no broadcast expected for system service"
        );
    }
```

- [ ] **Step 6: Run tests (expect pass — broadcast is now wired)**

```bash
cargo test -p uptrakit-web-api --no-default-features --features db-sqlite -- cleanup_embedded_session
```

Expected: both tests pass.

- [ ] **Step 7: Commit**

```bash
git commit --only crates/ui/web-api/src/routes/service_ws/handler/mod.rs -m "feat(service-ws): emit SurfacesChanged from cleanup_embedded_service_session"
```

---

## Task 3: Emit from `cleanup_authenticated_session`

**Files:**

- Modify: `crates/ui/web-api/src/routes/service_ws/handler/mod.rs`

`cleanup_authenticated_session` already destructures `service_tenant_id` from `AuthenticatedSessionState`. Add broadcast after the
`unregister_service` call inside that function.

- [ ] **Step 1: Write failing tests first**

Add inside `#[cfg(test)] mod tests`:

```rust
    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn cleanup_authenticated_session_broadcasts_surfaces_changed_when_tenant_present() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db, tenant_id).await;
        let service_id = uuid::Uuid::now_v7();
        register_test_runtime_state(&state, service_id, tenant_id);
        let connected_at = register_test_connection(&state, service_id).await;

        let mut rx = state.notification.event_broadcaster.subscribe(tenant_id).await;

        let (_push_tx, push_rx) = tokio::sync::mpsc::channel(1);
        let (msg_tx, _msg_rx) = tokio::sync::mpsc::channel(1);
        let (_resp_tx, resp_rx) = tokio::sync::mpsc::channel(1);
        cleanup_authenticated_session(
            &state,
            AuthenticatedSessionState {
                service_id,
                connected_at,
                is_system: false,
                has_update_tracking: false,
                has_software_discovery: false,
                has_workload_claims: false,
                service_tenant_id: Some(tenant_id),
                linked_host_ids: Arc::new(parking_lot::Mutex::new(HashSet::new())),
                push_rx,
                cancel_token: tokio_util::sync::CancellationToken::new(),
                msg_tx,
                resp_rx,
                processor_cancel: tokio_util::sync::CancellationToken::new(),
                processor_handle: tokio::spawn(async {}),
                rate_limiter: MessageRateLimiter::new(WS_MESSAGE_RATE_WINDOW, WS_MESSAGE_RATE_LIMIT),
            },
        )
        .await;

        match rx.try_recv() {
            Ok(AdminEvent::SurfacesChanged) => {}
            other => panic!("expected SurfacesChanged, got {other:?}"),
        }
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn cleanup_authenticated_session_skips_broadcast_when_no_tenant_id() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db, tenant_id).await;
        let service_id = uuid::Uuid::now_v7();
        register_test_runtime_state(&state, service_id, tenant_id);
        let connected_at = register_test_connection(&state, service_id).await;

        let mut rx = state.notification.event_broadcaster.subscribe(tenant_id).await;

        let (_push_tx, push_rx) = tokio::sync::mpsc::channel(1);
        let (msg_tx, _msg_rx) = tokio::sync::mpsc::channel(1);
        let (_resp_tx, resp_rx) = tokio::sync::mpsc::channel(1);
        cleanup_authenticated_session(
            &state,
            AuthenticatedSessionState {
                service_id,
                connected_at,
                is_system: true, // system service
                has_update_tracking: false,
                has_software_discovery: false,
                has_workload_claims: false,
                service_tenant_id: None, // no tenant
                linked_host_ids: Arc::new(parking_lot::Mutex::new(HashSet::new())),
                push_rx,
                cancel_token: tokio_util::sync::CancellationToken::new(),
                msg_tx,
                resp_rx,
                processor_cancel: tokio_util::sync::CancellationToken::new(),
                processor_handle: tokio::spawn(async {}),
                rate_limiter: MessageRateLimiter::new(WS_MESSAGE_RATE_WINDOW, WS_MESSAGE_RATE_LIMIT),
            },
        )
        .await;

        assert!(
            rx.try_recv().is_err(),
            "no broadcast expected for system service"
        );
    }
```

- [ ] **Step 2: Run to confirm tests fail**

```bash
cargo test -p uptrakit-web-api --no-default-features --features db-sqlite -- cleanup_authenticated_session_broadcasts_surfaces_changed
```

Expected: FAIL (no broadcast yet).

- [ ] **Step 3: Add broadcast inside the existing `provider_id_for_service` guard in `cleanup_authenticated_session`**

The function already has `if let Some(provider_id) = state.surface_proxy_deps.registry.provider_id_for_service(&service_id)` before
`fail_in_flight_for_provider`. Add the broadcast inside that block. The full block becomes:

```rust
    if let Some(provider_id) = state
        .surface_proxy_deps
        .registry
        .provider_id_for_service(&service_id)
    {
        state
            .surface_proxy_deps
            .proxy
            .fail_in_flight_for_provider(&provider_id);
        if let Some(tenant_id) = service_tenant_id {
            state
                .notification
                .event_broadcaster
                .send(tenant_id, AdminEvent::SurfacesChanged)
                .await;
        }
    }
    state
        .surface_proxy_deps
        .registry
        .unregister_service(&service_id);
```

- [ ] **Step 4: Run tests (expect pass)**

```bash
cargo test -p uptrakit-web-api --no-default-features --features db-sqlite -- cleanup_authenticated_session
```

Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git commit --only crates/ui/web-api/src/routes/service_ws/handler/mod.rs -m "feat(service-ws): emit SurfacesChanged from cleanup_authenticated_session"
```

---

## Task 4: Emit from `handle_surface_registration` success path

**Files:**

- Modify: `crates/ui/web-api/src/routes/service_ws/handler/mod.rs`

The success path is after `register_surface_provider` returns `Ok` — specifically after
`emit_surface_registration_audit_event(...AuditOutcome::Success...)` and before `ProcessorResponse::cont()`.

- [ ] **Step 1: Write failing tests first**

In `#[cfg(test)] mod tests`, add:

```rust
    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn surface_registration_success_broadcasts_surfaces_changed() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let state = build_db_audited_state(db.clone(), tenant_id).await;
        let service_id = uuid::Uuid::now_v7();
        insert_test_service_row(&db, tenant_id, service_id, "uptrakit-agent-ssh").await;

        let mut rx = state.notification.event_broadcaster.subscribe(tenant_id).await;

        let processor = MessageProcessor {
            state: Arc::clone(&state),
            service_id,
            cert: None,
            is_system: false,
            has_update_tracking: false,
            has_software_discovery: false,
            has_update_hooks: false,
            has_ui_surfaces: true,
            has_workload_claims: false,
            runtime_instance_id: None,
            service_app_name: Some("uptrakit-agent-ssh".to_string()),
            service_tenant_id: Some(tenant_id),
            linked_host_ids: Arc::new(parking_lot::Mutex::new(HashSet::new())),
            report_tracker: ReportTracker::new(),
        };
        let response = processor
            .handle_surface_registration(test_surface_registration("provider-a", tenant_id))
            .await;

        assert!(response.replies.is_empty(), "success path returns cont()");

        match rx.try_recv() {
            Ok(AdminEvent::SurfacesChanged) => {}
            other => panic!("expected SurfacesChanged on success, got {other:?}"),
        }
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn surface_registration_rejection_does_not_broadcast() {
        // Register provider-a first, then try to register conflicting provider-b.
        // The second registration will be rejected (conflict), so no broadcast.
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let state = build_db_audited_state(db.clone(), tenant_id).await;
        let service_id = uuid::Uuid::now_v7();
        let service_id_b = uuid::Uuid::now_v7();
        insert_test_service_row(&db, tenant_id, service_id, "uptrakit-agent-ssh").await;
        insert_test_service_row(&db, tenant_id, service_id_b, "uptrakit-agent-ssh-2").await;

        // Register provider-a from service_id (succeeds).
        state
            .surface_proxy_deps
            .registry
            .register_service(
                service_id,
                "uptrakit-agent-ssh",
                Some(tenant_id),
                test_surface_registration("provider-a", tenant_id),
            )
            .expect("first registration should succeed");

        let mut rx = state.notification.event_broadcaster.subscribe(tenant_id).await;

        // Try to register the SAME surface from service_id_b with a different provider
        // (provider-b). This will be rejected because provider-a already owns the surface.
        let processor = MessageProcessor {
            state: Arc::clone(&state),
            service_id: service_id_b,
            cert: None,
            is_system: false,
            has_update_tracking: false,
            has_software_discovery: false,
            has_update_hooks: false,
            has_ui_surfaces: true,
            has_workload_claims: false,
            runtime_instance_id: None,
            service_app_name: Some("uptrakit-agent-ssh-2".to_string()),
            service_tenant_id: Some(tenant_id),
            linked_host_ids: Arc::new(parking_lot::Mutex::new(HashSet::new())),
            report_tracker: ReportTracker::new(),
        };
        let response = processor
            .handle_surface_registration(test_surface_registration("provider-b", tenant_id))
            .await;

        assert!(
            !response.replies.is_empty(),
            "rejection path returns an error reply"
        );
        assert!(
            rx.try_recv().is_err(),
            "no broadcast expected on rejected registration"
        );
    }
```

- [ ] **Step 2: Run to confirm first test fails, second passes**

```bash
cargo test -p uptrakit-web-api --no-default-features --features db-sqlite -- surface_registration
```

Expected: `surface_registration_success_broadcasts_surfaces_changed` FAILS, rejection test passes.

- [ ] **Step 3: Add broadcast on success path**

In `handle_surface_registration`, before `ProcessorResponse::cont()` (after the audit event emission on the success path), add:

```rust
        if let Some(tenant_id) = self.service_tenant_id {
            self.state
                .notification
                .event_broadcaster
                .send(tenant_id, AdminEvent::SurfacesChanged)
                .await;
        }
        ProcessorResponse::cont()
```

- [ ] **Step 4: Run tests (expect all pass)**

```bash
cargo test -p uptrakit-web-api --no-default-features --features db-sqlite -- surface_registration
```

Expected: both pass.

- [ ] **Step 5: Commit**

```bash
git commit --only crates/ui/web-api/src/routes/service_ws/handler/mod.rs -m "feat(service-ws): emit SurfacesChanged on successful SurfaceRegistration"
```

---

## Task 5: Handle `Replaced` branch in `finalize_authenticated_session`

**Files:**

- Modify: `crates/ui/web-api/src/routes/service_ws/handler/mod.rs`

The `Replaced` branch (grep: `AuthenticatedSessionOwnership::Replaced`) currently cancels the processor but skips surface cleanup. After this task, it
also unregisters the surface provider and emits `SurfacesChanged`.

Two existing tests assert the provider is PRESERVED on supersession. After this task they must assert it is REMOVED (the replacement session will
re-register it via `SurfaceRegistration`).

- [ ] **Step 1: Update the two existing supersession tests**

Find and update `finalized_authenticated_session_skips_runtime_cleanup_when_session_is_replaced`:

Change:

```rust
        assert_eq!(
            state
                .surface_proxy_deps
                .registry
                .provider_id_for_service(&service_id),
            Some("provider-a".to_string())
        );
```

To:

```rust
        assert!(
            state
                .surface_proxy_deps
                .registry
                .provider_id_for_service(&service_id)
                .is_none(),
            "Replaced branch now unregisters the old provider so the replacement session re-registers"
        );
```

Find and update `cancelled_authenticated_session_skips_runtime_cleanup_for_genuine_supersession`:

Make the same provider assertion change (`Some("provider-a")` → `is_none()`).

- [ ] **Step 2: Verify these tests now fail (implementation not changed yet)**

```bash
cargo test -p uptrakit-web-api --no-default-features --features db-sqlite -- "finalized_authenticated_session_skips_runtime_cleanup\|cancelled_authenticated_session_skips_runtime_cleanup_for_genuine"
```

Expected: both FAIL.

- [ ] **Step 3: Write new broadcast tests**

Add inside `#[cfg(test)] mod tests`:

```rust
    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn finalize_replaced_session_broadcasts_surfaces_changed_when_provider_registered() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db, tenant_id).await;

        let service_id = uuid::Uuid::now_v7();
        register_test_runtime_state(&state, service_id, tenant_id);
        let superseded_at = register_test_connection(&state, service_id).await;
        let _replacement_at = register_test_connection(&state, service_id).await;

        let mut rx = state.notification.event_broadcaster.subscribe(tenant_id).await;

        let (_push_tx, push_rx) = tokio::sync::mpsc::channel(1);
        let (msg_tx, _msg_rx) = tokio::sync::mpsc::channel(1);
        let (_resp_tx, resp_rx) = tokio::sync::mpsc::channel(1);
        finalize_authenticated_session(
            &state,
            AuthenticatedSessionState {
                service_id,
                connected_at: superseded_at,
                is_system: false,
                has_update_tracking: false,
                has_software_discovery: false,
                has_workload_claims: false,
                service_tenant_id: Some(tenant_id),
                linked_host_ids: Arc::new(parking_lot::Mutex::new(HashSet::new())),
                push_rx,
                cancel_token: tokio_util::sync::CancellationToken::new(),
                msg_tx,
                resp_rx,
                processor_cancel: tokio_util::sync::CancellationToken::new(),
                processor_handle: tokio::spawn(async {}),
                rate_limiter: MessageRateLimiter::new(WS_MESSAGE_RATE_WINDOW, WS_MESSAGE_RATE_LIMIT),
            },
        )
        .await;

        match rx.try_recv() {
            Ok(AdminEvent::SurfacesChanged) => {}
            other => panic!("expected SurfacesChanged from Replaced branch, got {other:?}"),
        }
        assert!(
            state
                .surface_proxy_deps
                .registry
                .provider_id_for_service(&service_id)
                .is_none(),
            "provider should be removed by Replaced branch"
        );
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn finalize_replaced_session_skips_broadcast_when_no_provider() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db, tenant_id).await;

        let service_id = uuid::Uuid::now_v7();
        // Do NOT register a provider — this service never had UiSurfaces.
        let superseded_at = register_test_connection(&state, service_id).await;
        let _replacement_at = register_test_connection(&state, service_id).await;

        let mut rx = state.notification.event_broadcaster.subscribe(tenant_id).await;

        let (_push_tx, push_rx) = tokio::sync::mpsc::channel(1);
        let (msg_tx, _msg_rx) = tokio::sync::mpsc::channel(1);
        let (_resp_tx, resp_rx) = tokio::sync::mpsc::channel(1);
        finalize_authenticated_session(
            &state,
            AuthenticatedSessionState {
                service_id,
                connected_at: superseded_at,
                is_system: false,
                has_update_tracking: false,
                has_software_discovery: false,
                has_workload_claims: false,
                service_tenant_id: Some(tenant_id),
                linked_host_ids: Arc::new(parking_lot::Mutex::new(HashSet::new())),
                push_rx,
                cancel_token: tokio_util::sync::CancellationToken::new(),
                msg_tx,
                resp_rx,
                processor_cancel: tokio_util::sync::CancellationToken::new(),
                processor_handle: tokio::spawn(async {}),
                rate_limiter: MessageRateLimiter::new(WS_MESSAGE_RATE_WINDOW, WS_MESSAGE_RATE_LIMIT),
            },
        )
        .await;

        assert!(
            rx.try_recv().is_err(),
            "no broadcast when service had no surface provider"
        );
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn finalize_replaced_session_skips_broadcast_when_no_tenant_id() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db, tenant_id).await;

        let service_id = uuid::Uuid::now_v7();
        register_test_runtime_state(&state, service_id, tenant_id);
        let superseded_at = register_test_connection(&state, service_id).await;
        let _replacement_at = register_test_connection(&state, service_id).await;

        let mut rx = state.notification.event_broadcaster.subscribe(tenant_id).await;

        let (_push_tx, push_rx) = tokio::sync::mpsc::channel(1);
        let (msg_tx, _msg_rx) = tokio::sync::mpsc::channel(1);
        let (_resp_tx, resp_rx) = tokio::sync::mpsc::channel(1);
        finalize_authenticated_session(
            &state,
            AuthenticatedSessionState {
                service_id,
                connected_at: superseded_at,
                is_system: true,
                has_update_tracking: false,
                has_software_discovery: false,
                has_workload_claims: false,
                service_tenant_id: None, // system service — no tenant channel
                linked_host_ids: Arc::new(parking_lot::Mutex::new(HashSet::new())),
                push_rx,
                cancel_token: tokio_util::sync::CancellationToken::new(),
                msg_tx,
                resp_rx,
                processor_cancel: tokio_util::sync::CancellationToken::new(),
                processor_handle: tokio::spawn(async {}),
                rate_limiter: MessageRateLimiter::new(WS_MESSAGE_RATE_WINDOW, WS_MESSAGE_RATE_LIMIT),
            },
        )
        .await;

        assert!(
            rx.try_recv().is_err(),
            "no broadcast for system service (no tenant_id)"
        );
    }
```

- [ ] **Step 4: Update `finalize_authenticated_session` Replaced branch**

Find `AuthenticatedSessionOwnership::Replaced =>` (grep in the file). Change from:

```rust
        AuthenticatedSessionOwnership::Replaced => {
            let AuthenticatedSessionState {
                processor_cancel,
                processor_handle,
                ..
            } = session;
            processor_cancel.cancel();
            let _ = processor_handle.await;
        }
```

To:

```rust
        AuthenticatedSessionOwnership::Replaced => {
            let AuthenticatedSessionState {
                service_id,
                service_tenant_id,
                processor_cancel,
                processor_handle,
                ..
            } = session;
            processor_cancel.cancel();
            let _ = processor_handle.await;

            if let Some(provider_id) = state
                .surface_proxy_deps
                .registry
                .provider_id_for_service(&service_id)
            {
                state
                    .surface_proxy_deps
                    .proxy
                    .fail_in_flight_for_provider(&provider_id);
                if let Some(tenant_id) = service_tenant_id {
                    state
                        .notification
                        .event_broadcaster
                        .send(tenant_id, AdminEvent::SurfacesChanged)
                        .await;
                }
            }
            // Always call unregister_service — idempotent no-op when nothing is registered.
            // Matches the unconditional placement in cleanup_embedded_service_session and
            // cleanup_authenticated_session.
            state
                .surface_proxy_deps
                .registry
                .unregister_service(&service_id);
        }
```

- [ ] **Step 5: Run all handler tests**

```bash
cargo test -p uptrakit-web-api --no-default-features --features db-sqlite -- finalize_replaced finalized_authenticated_session cancelled_authenticated_session_skips_runtime_cleanup_for_genuine
```

Expected: all pass.

- [ ] **Step 6: Run full backend test suite**

```bash
cargo test --no-default-features --features db-sqlite
```

Expected: all pass.

- [ ] **Step 7: Commit**

```bash
git commit --only crates/ui/web-api/src/routes/service_ws/handler/mod.rs -m "feat(service-ws): unregister provider and emit SurfacesChanged in Replaced session branch"
```

---

## Task 6: Frontend — add `'surfaces_changed'` to `AdminEventType` and wire layout subscription

**Files:**

- Modify: `frontend/src/lib/sse.ts`
- Modify: `frontend/src/routes/+layout.svelte`

- [ ] **Step 1: Add `SurfacesChanged` to `AdminEventType` enum in `sse.ts`**

`AdminEventType` is an `export enum` (not a union type) at line 184 of `frontend/src/lib/sse.ts`. Add a new member after `HostTagsChanged`:

```typescript
export enum AdminEventType {
  // ... existing members ...
  HostTagsChanged = "host_tags_changed",
  SurfacesChanged = "surfaces_changed",
}
```

Only add the one line `SurfacesChanged = 'surfaces_changed',` — do not rewrite the rest of the enum. `BatchUpdateCompleted = 'batch_update_completed'`
is a pre-existing member with no backend counterpart; leave it as-is.

- [ ] **Step 2: Add `subscribeToEvent` subscription in `+layout.svelte`**

In `frontend/src/routes/+layout.svelte`, add `subscribeToEvent` and `AdminEventType` to imports:

```typescript
import { subscribeToEvent } from "$lib/stores/events.svelte";
import { AdminEventType } from "$lib/sse";
```

Then find the `onMount(() => {` block and add a subscription using `AdminEventType.SurfacesChanged` (not the raw string — TypeScript will reject
`'surfaces_changed'` where `AdminEventType` is expected). Wire it inside the same `onMount` return cleanup as the resize listener:

```typescript
onMount(() => {
  const syncViewport = () => {
    viewportWidth = window.innerWidth;
  };

  syncViewport();
  window.addEventListener("resize", syncViewport);

  const unsubscribeSurfaces = subscribeToEvent(AdminEventType.SurfacesChanged, () => {
    void loadSurfaceRegistry();
  });

  return () => {
    window.removeEventListener("resize", syncViewport);
    unsubscribeSurfaces();
  };
});
```

- [ ] **Step 3: Type-check**

```bash
cd frontend && npm run check
```

Expected: no errors.

- [ ] **Step 4: Commit**

```bash
cd frontend && git commit --only src/lib/sse.ts src/routes/+layout.svelte -m "feat(frontend): add surfaces_changed to AdminEventType and subscribe in layout"
```

---

## Task 7: Frontend unit tests

**Files:**

- Create: `frontend/src/lib/stores/events.svelte.test.ts`

These tests verify that:

1. Receiving `surfaces_changed` triggers exactly one `loadSurfaceRegistry()` call.
2. A burst of three rapid events debounces to one call.
3. The raw SSE wire frame (`event: surfaces_changed\ndata: {}\n\n`) is parsed and dispatched (not dropped by `parseSseEvent`).

- [ ] **Step 1: Create the test file**

Create `frontend/src/lib/stores/events.svelte.test.ts`.

Use `vi.doMock` inside `beforeEach` (after `vi.resetModules()`) — NOT top-level `vi.mock`. Top-level `vi.mock` is hoisted once; after
`vi.resetModules()` clears the module cache, fresh `import(...)` calls in subsequent tests would re-execute module-level code with a stale mock
factory. `vi.doMock` re-registers the mock factory each time, matching the pattern used in `software-updates.test.ts`.

```typescript
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
// Static import — gets the real enum values regardless of vi.doMock later.
// AdminEventType is a TypeScript string enum so values are plain strings at runtime.
import { AdminEventType } from "$lib/sse";

type OnEventFn = (eventType: AdminEventType, data: Record<string, unknown>) => void;
let capturedOnEvent: OnEventFn | undefined;
const mockLoadSurfaceRegistry = vi.fn().mockResolvedValue(undefined);

describe("events.svelte — surfaces_changed handling", () => {
  beforeEach(() => {
    vi.resetModules();
    capturedOnEvent = undefined;
    mockLoadSurfaceRegistry.mockClear();

    vi.doMock("$lib/sse", () => ({
      // Include AdminEventType in the mock so the mocked module exports it.
      AdminEventType,
      connectEventStream: vi.fn((callbacks: { onEvent?: OnEventFn }) => {
        capturedOnEvent = callbacks.onEvent;
        return () => {
          capturedOnEvent = undefined;
        };
      }),
    }));

    vi.doMock("$lib/surfaces/registry.svelte", () => ({
      loadSurfaceRegistry: mockLoadSurfaceRegistry,
    }));

    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("single surfaces_changed event calls loadSurfaceRegistry once after debounce", async () => {
    const { subscribeToEvent } = await import("$lib/stores/events.svelte");

    let called = false;
    const unsub = subscribeToEvent(AdminEventType.SurfacesChanged, () => {
      void mockLoadSurfaceRegistry();
      called = true;
    });

    capturedOnEvent?.(AdminEventType.SurfacesChanged, {});
    expect(called).toBe(false); // not yet — debounce pending

    await vi.advanceTimersByTimeAsync(200);
    expect(called).toBe(true);
    expect(mockLoadSurfaceRegistry).toHaveBeenCalledTimes(1);

    unsub();
  });

  it("burst of three surfaces_changed events debounces to one loadSurfaceRegistry call", async () => {
    const { subscribeToEvent } = await import("$lib/stores/events.svelte");

    let callCount = 0;
    const unsub = subscribeToEvent(AdminEventType.SurfacesChanged, () => {
      void mockLoadSurfaceRegistry();
      callCount++;
    });

    capturedOnEvent?.(AdminEventType.SurfacesChanged, {});
    capturedOnEvent?.(AdminEventType.SurfacesChanged, {});
    capturedOnEvent?.(AdminEventType.SurfacesChanged, {});

    await vi.advanceTimersByTimeAsync(200);
    expect(callCount).toBe(1);
    expect(mockLoadSurfaceRegistry).toHaveBeenCalledTimes(1);

    unsub();
  });

  it("surfaces_changed event with empty data object is not dropped (parseSseEvent passes {})", async () => {
    const { subscribeToEvent } = await import("$lib/stores/events.svelte");

    let received = false;
    const unsub = subscribeToEvent(AdminEventType.SurfacesChanged, () => {
      received = true;
    });

    // Simulate what readAdminEventStream does: JSON.parse('{}') → {}
    capturedOnEvent?.(AdminEventType.SurfacesChanged, JSON.parse("{}") as Record<string, unknown>);
    await vi.advanceTimersByTimeAsync(200);

    expect(received).toBe(true);
    unsub();
  });
});
```

- [ ] **Step 2: Run the new tests**

```bash
cd frontend && npm run test -- events.svelte.test
```

Expected: all three tests pass.

- [ ] **Step 3: Run the full frontend test suite**

```bash
cd frontend && npm run test
```

Expected: all pass. No regressions.

- [ ] **Step 4: Run frontend checks**

```bash
cd frontend && npm run lint && npm run format:check && npm run check && npm run build
```

Expected: all pass.

- [ ] **Step 5: Commit**

```bash
cd frontend && git commit --only src/lib/stores/events.svelte.test.ts -m "test(frontend): add surfaces_changed debounce and loadSurfaceRegistry call tests"
```

---

## Task 8: Final quality gate

- [ ] **Step 1: Backend format and check**

```bash
cargo fmt --all
cargo check --no-default-features --features db-sqlite
cargo check --all-features
```

Fix any errors before continuing.

- [ ] **Step 2: Backend lint**

```bash
cargo clippy --all-targets --no-default-features --features db-sqlite
cargo clippy --all-targets --all-features
```

Fix any warnings.

- [ ] **Step 3: Full backend test suite**

```bash
cargo test --no-default-features --features db-sqlite
cargo test --all-features
```

Expected: all pass.

- [ ] **Step 4: Markdownlint**

```bash
markdownlint --config .markdownlint.json '**/*.md'
```

- [ ] **Step 5: Final commit (if any fmt/lint fixes were needed)**

```bash
git add -p
git commit -m "chore: fmt and clippy fixes for surfaces-dynamic-update"
```

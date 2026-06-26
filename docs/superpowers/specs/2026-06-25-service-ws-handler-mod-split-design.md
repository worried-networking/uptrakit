# service_ws/handler/mod.rs split — design

- Status: Draft
- Date: 2026-06-25
- Author: Andrey Yantsen (with Claude)
- Scope: `crates/ui/web-api/src/routes/service_ws/handler/mod.rs`
- Related: ADR-0023 (controller boot-phase decomposition — same pattern),
  `2026-06-22-controller-boot-decomposition-design.md`

## Problem

CodeScene flags `service_ws/handler/mod.rs` as the worst hotspot in the repo:

- Code Health **2.10 / 10** (red — severe technical debt).
- **6508 LOC** in one file (~52% is the `#[cfg(test)] mod tests`).
- 136 revisions; friction 0.49; hotspot health for the project trending **-0.13/month**.
- Primary driver: **Low Cohesion (LCOM4)** — the module holds ≥7 unrelated
  responsibilities (audit emission, message dispatch, surface wire conversion,
  connection setup, embedded handler, authenticated-session lifecycle,
  enrolled-session lifecycle).
- Secondary: Code Duplication across `emit_service_*_audit_event` (3 near-identical)
  and the `run_embedded_*_handler` pair; plus brain methods, arg-bags, and
  duplicated test fixtures.

The handler directory **already uses a flat multi-module layout** (12 sibling
modules: `cert`, `credentials`, `messages`, `updates`, `workload`,
`service_config`, `update_tracking`, …). `mod.rs` is the leftover god module that
never got carved up.

In `codebase-design` terms: `mod.rs` is a **shallow grab-bag** — wide interface,
no single responsibility. Splitting along the existing cohesive **seams** raises
**locality** (each concept lives in one place) and removes the Low-Cohesion
penalty, which is the single biggest lever on the 2.10 score.

## Goals

- Move each cohesive cluster into its own flat sibling module under `handler/`.
- Collapse the two CodeScene-flagged duplication groups (safe extract-helper only).
- Preserve **exact behavior** and the **public surface** — no caller outside
  `handler/` changes.
- Co-locate each module's tests; lift shared fixtures into one test-support module.

## Non-goals (deferred — see "Deferred")

- Breaking up brain methods (`MessageProcessor::dispatch` cc=16,
  `ingest_service_audit_event` cc=25, `handle_surface_action_request`, …).
- Reducing excess-argument functions to context structs.
- Any logic change beyond the two named dedup collapses.
- Touching `updates.rs` / `messages.rs` (also red, separate follow-ups).

## Decisions (from grilling)

| Decision       | Choice                                                                                                                                                                                                                                                                                                                                                                                                                 |
| -------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Aggressiveness | **Split + dedup only.** Mechanical module moves + collapse the two flagged duplication groups. No cyclomatic-complexity reductions.                                                                                                                                                                                                                                                                                    |
| Structure      | **Flat sibling files** under `handler/` — matches existing convention.                                                                                                                                                                                                                                                                                                                                                 |
| Tests          | **Hybrid:** single-cluster tests co-locate with their module; cross-cluster end-to-end tests (`surface_action_*`/`surface_registration_*` that drive `MessageProcessor` AND assert `audit_*` rows) → one `handler/tests.rs`. Shared fixtures → `handler/test_support.rs`. (Revised from pure co-location after review flagged cross-cluster import wiring as the bulk of the risk; tests don't drive the LCOM4 score.) |
| Verification   | **Full quality gates + Docker WS/service-lifecycle integration tests.**                                                                                                                                                                                                                                                                                                                                                |

## Target module map

Production code (lines 1–3384 of current `mod.rs`) splits as follows. Line ranges
are approximate (file edited 2026-06-25); implementation maps exact symbols.

### `handler/audit_service.rs`

Service-lifecycle audit: ingest/validate forwarded service audit events + emit
cert/enrollment events. ~450 LOC prod. (Split out from a single `audit.rs` because
the cluster holds two distinct concepts — see `audit_surface.rs` — totaling ~965 LOC.)

- Types: `ServiceAuditCtx`, `AuditEventScope`
- `audit_event_scope`, `validate_audit_event_payload`, `resolve_service_audit_identity`,
  `resolve_service_target_display`, `ingest_service_audit_event`,
  `emit_service_enrollment_completed_audit_event`,
  `emit_service_certificate_issue_audit_event`,
  `emit_service_certificate_renew_audit_event` **(deduped — see below)**
- Tests: `forwarded_*_audit_event_*`, `local_*_audit_event_*`,
  `*_certificate_*_audit_event_writes_*`, `*_enrollment_completed_audit_event_writes_*`,
  `invalid_forwarded_service_audit_event_*`, `forwarded_stateful_audit_event_*`

### `handler/audit_surface.rs`

Surface-interaction audit: classify surface registration/action/proxy/lookup outcomes
and emit the corresponding audit events. ~430 LOC prod. Pairs with `surface_wire.rs`
and `message_processor.rs`'s surface handling.

- `surface_action_target_display`, `surface_provider_kind_name`,
  `truncate_surface_registration_audit_value`,
  `classify_surface_registration_validation_error`,
  `surface_registration_rejection_reason_code`,
  `classify_surface_registration_error_for_audit`,
  `emit_surface_registration_audit_event`,
  `emit_surface_action_scope_denied_audit_event`, `emit_surface_action_invoke_audit_event`,
  `classify_surface_action_response_for_audit`, `classify_surface_proxy_error_for_audit`,
  `classify_surface_lookup_error_for_audit`,
  `classify_surface_action_request_validation_error`,
  `resolve_surface_action_audit_tenant_id`
- Tests: `surface_action_*_emits_*_audit_row`, `*surface_registration*_emits_*`,
  `surface_action_target_display_includes_*`

> Note: `ServiceAuditCtx` (positioned near the surface fns in current `mod.rs`) goes
> with `audit_service.rs`. Confirm at implementation: if `emit_surface_*` also consume
> it, lift the shared ctx to `shared_types.rs` instead.

### `handler/message_processor.rs`

The message dispatch engine.

- Types: `LoopAction` (+ impl), `ProcessorMessage`, `ProcessorChannels`
- `MessageProcessor` + impl: `run`, `dispatch`, `dispatch_update_hooks`,
  `dispatch_update_tracking`, `dispatch_surfaces`, `handle_surface_registration`,
  `handle_surface_action_request`
- `spawn_message_processor`
- Tests: single-cluster processor unit tests co-locate here; the `surface_action_*`/`surface_registration_*`
  end-to-end tests that also assert `audit_*` rows go to `handler/tests.rs` (cross-cluster).
- **Consumes** `HandlerError` / `HandlerResult` from `shared_types.rs` (does not own them —
  they are cross-cutting, also used by `shared_types` and `updates`; relocating them here would
  cycle. See `shared_types.rs` note below).
- Tests: `surface_action_*` (success/denied/failure/lookup/payload/tenant),
  `surface_registration_*`

### `handler/surface_wire.rs`

Pure surface ↔ wire conversions (no I/O).

- `register_surface_provider`, `surface_registration_error_message`,
  `surface_proxy_error_to_wire`, `surface_registry_lookup_error_to_wire`,
  `action_error_code`
- Tests: `surface_registration_error_message_serializes_*`

### `handler/embedded.rs`

Embedded-mode message handling.

- Type: `EmbeddedHandlerSession`
- `run_embedded_message_handler` _(pub(crate))_, `run_embedded_system_message_handler`
  _(pub(crate))_ **(deduped — see below)**, `run_embedded_message_handler_inner`,
  `cleanup_embedded_service_session`
- Tests: `embedded_system_handler_cleanup_*`, `cleanup_embedded_session_*`

### `handler/session_authenticated.rs`

Authenticated (post-enrollment) session lifecycle.

- Types: `AuthenticatedSessionState`, `AuthenticatedSessionOwnership`, `TextAction`
- `register_connection`, `load_service_capabilities`,
  `cancellation_token_from_connection_handle`, `load_session_host_ids`,
  `prepare_reconnect_updates_on_connect`, `receive_register_message`,
  `setup_authenticated_session`, `cleanup_authenticated_session`,
  `handle_cancelled_authenticated_session_after_close`,
  `authenticated_session_ownership`, `finalize_authenticated_session`,
  `handle_incoming_text`, `handle_authenticated_loop` _(pub(crate))_
- Tests: `cleanup_authenticated_session_*`, `finalize_replaced_session_*`,
  `reconnect_cleanup_*`, `cancelled/finalized_authenticated_session_*`, `connect_phase_*`

### `handler/session_enrolled.rs`

Enrolled (pre-/during-enrollment) session lifecycle.

- Type: `EnrolledSessionState`
- `upgrade_service_capabilities`, `setup_enrolled_session`, `cleanup_enrolled_session`,
  `handle_enrolled_loop` _(pub(crate))_
- Tests: `setup_enrolled_session_emits_*`

### `handler/test_support.rs` (`#[cfg(test)]`)

Shared test fixtures used by ≥2 module test blocks.

- `build_handler_test_state`, `build_db_audited_state`, `test_authenticated_session`,
  `register_test_runtime_state`, `register_test_connection`, `test_surface_registration`,
  `sign_agent_csr`, `active_ca_fingerprint`, `insert_test_service_row`,
  `insert_test_system_service_row`, `tenant_audit_row_for_action`,
  `system_audit_row_for_action`, `MockEmbeddedNotifier`, `insert_service_row`,
  `insert_linked_host_and_item`, `relink_service_host`, `insert_owned_in_progress_update`,
  `run_embedded_register_once`
- Declared in `mod.rs` as `#[cfg(test)] pub(super) mod test_support;`; helpers
  `pub(super)` (not `pub(crate)` — over-broad for test-only fixtures); sibling tests import via
  `use crate::routes::service_ws::handler::test_support::*;`.

### `handler/tests.rs` (`#[cfg(test)]`)

Home for **cross-cluster** end-to-end tests — those that drive `MessageProcessor` dispatch AND assert
`emit_*_audit_event` rows (the `surface_action_*_emits_*` / `surface_registration_*_emits_*` /
`*_config_scope_violation_emits_denied_*` families). Keeping them here avoids wiring multiple clusters'
`pub(super)` symbols into a single module's test block. Single-cluster tests stay co-located with their module.

### Cross-cutting items → `shared_types.rs`

These are consumed by multiple new modules; co-locating them in the existing
`handler/shared_types.rs` (which already holds `ProcessorResponse`/`ProcessorAction`/
`load_linked_host_ids`) avoids sibling import cycles. All `pub(super)`:

- `HandlerError` (enum), `HandlerResult<T>` (alias), and the
  `impl_report_conversion!(sea_orm::DbErr => HandlerError::Database)` — currently in `mod.rs`,
  already imported by `shared_types` and `updates`. **They must NOT go in `message_processor.rs`**
  (would cycle with `shared_types`).
- `MAX_UPDATE_OUTPUT_BYTES` const (imported by `updates`).
- Micro-helpers `system_service_tenant_binding`, `is_valid_service_config_scope` (a few lines each).

### `handler/mod.rs` after the split (thin facade, ~50 LOC)

- `mod` declarations for all submodules (new + existing).
- `pub(crate) use` re-exports preserving the public surface:
  - `run_embedded_message_handler`, `run_embedded_system_message_handler`
    (from `embedded`)
  - `handle_authenticated_loop` (from `session_authenticated`)
  - `handle_enrolled_loop` (from `session_enrolled`)
  - existing: `discovery::trigger_discovery_for_agent_host`,
    `updates::dispatch_next_batch_update`
- Nothing else. No logic.

## Dedup tasks (the only logic changes)

1. **`emit_service_*_audit_event` (3 near-identical):** collapse
   `emit_service_enrollment_completed_audit_event`,
   `emit_service_certificate_issue_audit_event`,
   `emit_service_certificate_renew_audit_event` into one private helper
   parameterized by audit action kind + payload, with three thin
   (or removed) call sites. Lands in `audit.rs`.
2. **`run_embedded_message_handler` / `run_embedded_system_message_handler`:**
   both are thin wrappers over `run_embedded_message_handler_inner` differing by a
   "system service" flag. Collapse the duplicated wrapper body into a single shared
   private path; keep both `pub(crate)` entry points (public surface) as
   one-line delegations. Lands in `embedded.rs`.

Both dedups are behavior-preserving extract-method/parameterize refactors. **Commit
the dedups separately from the pure file moves** so reviewers can verify moves as
zero-diff relocations.

## Visibility note

Several free functions are currently bare `fn` (module-private). After the move,
cross-module callers need `pub(super)` (visible within `handler` and therefore to
all sibling submodules). Bump visibility minimally — prefer `pub(super)` over
`pub(crate)` unless the symbol is part of the preserved public surface. This is the
only widespread mechanical edit beyond cut/paste.

## Implementation order (suggested commits)

1. `surface_wire.rs` — smallest, no I/O, low risk. Establish the pattern.
2. `audit_surface.rs` (moves).
3. `audit_service.rs` (moves) → then dedup `emit_service_*` (separate commit).
4. `message_processor.rs`.
5. `embedded.rs` (moves) → then dedup `run_embedded_*` (separate commit).
6. `session_authenticated.rs`.
7. `session_enrolled.rs`.
8. `test_support.rs` + co-locate tests (can interleave per module above).
9. Reduce `mod.rs` to the facade; verify external callers unchanged.

After each module move: `cargo check` + that module's tests green before the next.

## Verification (per docs/development/quality-gates.md)

Full gate set — this touches the WS enrollment/service-lifecycle path:

```bash
cargo fmt --all
cargo check --no-default-features --features db-sqlite
cargo check --all-features
cargo clippy --all-targets --no-default-features --features db-sqlite
cargo clippy --all-targets --all-features
cargo test --all-features
cargo deny check
```

Plus, because wire/service-lifecycle code is in scope:

```bash
docker build -f docker/Dockerfile.test -t uptrakit-test:latest .
cargo test -p uptrakit-integration-tests -- --ignored
```

Workspace lints (`unwrap_used`, `expect_used`, `unreachable_pub`,
`unfulfilled_lint_expectations` = deny) must stay clean — pure moves should not
trip these; watch `unreachable_pub` when adjusting visibility.

## Documentation deliverables

- **None required for behavior** — internal refactor, no externally observable
  change, no API/config/wire surface change.
- **AGENTS.md** — if it enumerates `service_ws/handler/` module files in the
  codebase-layout section, update the listing. Verify during implementation; update
  only if the file list is present.
- **No CONTEXT.md change** — all new modules named after existing domain/architecture
  concepts (audit, message processing, session, embedded mode); no new term.
- **No new ADR** — the split introduces no new architectural decision: it follows the
  existing flat-module convention and the precedent set by ADR-0023 (controller boot
  decomposition). A one-paragraph ADR for symmetry with ADR-0023 is _optional_;
  recommendation is to **skip** (nothing to re-litigate later).

## Risks & mitigations

| Risk                                                   | Mitigation                                                                                     |
| ------------------------------------------------------ | ---------------------------------------------------------------------------------------------- |
| Regression on hot WS auth/enrollment path              | Full test suite + Docker integration tests; behavior-preserving moves; dedup commits isolated. |
| `pub(super)`/`pub(crate)` visibility errors after move | `unreachable_pub` lint is deny — compiler catches over-broad vis; bump minimally.              |
| Test fixtures tangled across modules                   | `test_support.rs` extracted first/early; sibling tests import from it.                         |
| Hidden coupling between clusters surfaces mid-split    | Suggested commit order does smallest/leaf modules first; `cargo check` after each.             |
| Dedup changes behavior subtly                          | Keep dedups as separate, reviewable commits; the moved tests must pass unchanged.              |

## Deferred / out of scope

- **Brain-method decomposition (tracked follow-up):** break up
  `MessageProcessor::dispatch` (cc=16), `ingest_service_audit_event` (cc=25),
  `handle_surface_action_request`, `cleanup_authenticated_session` (cc=11),
  `validate_audit_event_payload` (cc=11); convert 5–7-arg functions to context
  structs. To be specced separately once the split lands and the per-module
  CodeScene scores are re-measured.
- `updates.rs` (health 2.80) and `messages.rs` (health 3.13) splits — separate
  follow-ups; benefit from abstractions this split establishes.
- Test assertion-block deduplication / table-driven test conversion.

## Open questions

- None blocking. (Audit is pre-split into `audit_service.rs` / `audit_surface.rs`,
  so no oversized leftover module is expected. `ingest_service_audit_event`'s cc=25
  brain method still lands red in `audit_service.rs` — that reduction is the deferred
  follow-up, not this split.)

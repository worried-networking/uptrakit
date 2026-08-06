# Mutating-Request-Validation Stage-1 Hardening — Design

Date: 2026-08-06. Status: approved (owner decisions confirmed in spec Q&A).

## Context

Stage 1 of the mutating-request-validation work landed on `main` via ff-merge at `9b3b46de3`: `Unvalidated<T>`/`UnvalidatedForm<T>`
type-state extractors in `crates/ui/web-api/src/extract.rs`, 23 converted handlers, 16 new `Validate` impls,
`ci/verify_no_raw_body_extractors.sh` with a shrink-only allowlist, and ADR-0038. Design:
[2026-07-12-mutating-request-validation-design.md](2026-07-12-mutating-request-validation-design.md).

This spec resolves the plan-mandated parked owner decisions and closes minor deferrals. Eight items; every owner decision below was
confirmed during grilling on 2026-08-06.

## Goals

- Fix the one live regression Stage 1 introduced (item 1).
- Restore audit parity on validation-reject paths that lost or never had it (items 2, 3).
- Close two latent traps in the surface-dispatch validation path before a real validation rule lands (items 4, 5).
- Make the gate's "frozen allowlist" claim mechanically true and close two tripwire gaps (items 6, 7).
- Document the intentional `reason_code` split (item 8).

## Non-goals

- Converting any Bucket-B1 allowlisted handler (Stage 2, separate spec).
- Retiring or extending `Validated<T>` (Stage 3).
- The 8 unconditional `Ok(())` `Validate` impls (accepted as documented in Stage 1).
- Adding real validation rules to the 5 surface request types (their impls stay `Ok(())`; see item 5's testability note).

## Item 1 — `UpdateHostAssignmentRequest`: relax to at-most-one source (live regression)

**Problem.** `impl Validate for UpdateHostAssignmentRequest` (`crates/shared/web-api-types/src/software_items.rs:696`) requires
exactly one of `plugin_config_id` / `plugin_config` / `plugin_type`. The query layer
(`crates/ui/web-api-queries/src/queries/software_items/host_assignments.rs:556`, `req.plugin_config_id.or(existing_pcid)`)
deliberately supports zero-source updates that change only `package_identifier` / `execution_site` / `config_override` and fall back
to the existing assignment's config. Such requests now fail validation with 400 — a live regression.

**Decision (owner-confirmed).** Relax the rule to **at most one** source. Zero sources = "keep the existing plugin source." Two or
more sources remain rejected with the existing error shape (message updated to "at most one of …"). The non-empty
`package_identifier` check and nested `plugin_config.validate()` call are unchanged. The **creation** path
(`HostSoftwareAssignment`) keeps its own validation untouched — creation has no existing row to fall back to.

**Changes.**

- `software_items.rs`: `sources != 1` → `sources > 1`; message text; update the field doc comments on `plugin_config`
  and `plugin_type` (currently "mutually exclusive with …") to state the at-most-one-for-update semantics. Doc comments flow into
  the OpenAPI schema, so `./scripts/regen-api.sh` runs in the same commit and both artifacts
  (`crates/ui/web-api/openapi.json`, `frontend/src/lib/api/generated/`) are committed with it. Pre-declared artifact impact:
  `openapi.json` changes (description strings only); the generated TS client changes only if descriptions are emitted into it —
  commit whatever the regen produces.
- Unit tests (same file's test module): zero sources → `Ok`, one source → `Ok`, two sources → `Err` naming `plugin_config_id`.
- Handler test (TestApp harness, `crates/ui/web-api/src/routes/software_items/tests.rs`): PATCH an existing assignment with only
  `package_identifier` set → success, and the assignment's `plugin_config_id` is unchanged. This test is RED on current `main`
  (400) — it is the regression's pin.

## Item 2 — audit-mirror the `require_valid()` Err arms in `update_software_item` and `trigger_update`

**Problem.** Both handlers return an unaudited 400 on `require_valid()` Err while their handler families audit other rejections.

**Decision (owner-confirmed).** Both handlers emit an audit event on the Err arm; upgraded from the "document where none exists"
default for `trigger_update` — rejected trigger attempts become auditable.

**Changes.**

- `update_software_item` (`crates/ui/web-api/src/routes/software_items/crud.rs:270`): mirror the established pattern at
  `host_assignments.rs:437-455` — hoist the `api_token_id` / `authenticated_user_audit_actor` / `tenant_id` derivations above the
  `require_valid()` match, then on Err emit
  `AuditEntry::<Event>::builder_event(SOFTWARE_ITEM_UPDATE_AUDIT_ACTION)` with `.tenant_scope(tenant_id)`,
  `.actor(actor_type, actor_id)`, `.outcome(AuditOutcome::ValidationFailed)`, and
  `.details(json!({ "software_item_id": item_id, "reason_code": "invalid_request" }))`, then return the existing 400.
  `SOFTWARE_ITEM_UPDATE` is registered Stateful, but the `builder_event` failure-outcome emit for pre-transaction rejections is the
  file family's established precedent (same-file sibling `SOFTWARE_ITEM_UPDATE_HOST_ASSIGNMENT`, also Stateful, at
  `host_assignments.rs:440`; and the merge-execute mirror in commit `9b3b46de3`, which needed no `audit-catalog.toml` change
  because the handler site already carries the action).
- `trigger_update` (`crates/ui/web-api/src/routes/software_items/updates.rs:46`): on Err emit
  `SOFTWARE_UPDATE_TRIGGERED` (registered Event) with `.outcome(ValidationFailed)` and
  `.details(json!({ "software_item_id": item_id, "host_id": host_id, "reason_code": "invalid_request" }))`, keep the existing
  `ApiError` 400 with code `validation_error`. This is a **new emit site** (the action's success emission lives downstream in
  `controller-core/src/update/controller.rs`, not the handler), so it gets a new `audit-catalog.toml` row
  (`crates/shared/audit-log/audit-catalog.toml`) in the same commit, and `cargo xtask audit-coverage-check` joins that task's gate
  list. Any catalog-counting golden that trips (e.g. under `xtask/tests/`) is updated in the same commit.
- Tests (TestApp): for each handler, send an invalid body (icon URL failing `validate_https_icon_url` for
  `update_software_item`; over-length `to_version` failing `validate_command_length` for `trigger_update`) → assert 400 **and**
  assert the audit row by discriminating fields (`action_type` + `outcome == ValidationFailed`), not by "newest row".

## Item 3 — `device_auth_approve`: restore audit on validation reject

**Problem.** A garbage `user_code` now 400s at the `require_valid()` boundary (`crates/ui/web-api/src/routes/device_auth.rs:74`)
with no audit row; previously the handler emitted a device-auth decision audit event and returned 404.

**Decision (owner-confirmed).** Keep the 400; emit `AUTH_DEVICE_APPROVE` (registered Event) with `outcome ValidationFailed` and
`reason_code "invalid_request"` on the Err arm. No `device_flow_id` detail — the code is rejected before normalization/hashing, so
none exists. Emit via a direct `builder_event` chain (the existing `emit_device_auth_decision_audit` helper requires a
`device_flow_id`; do not widen its signature for one call site). Same handler site already emits this action on both existing
arms, so no `audit-catalog.toml` change is expected; `cargo xtask audit-coverage-check` confirms.

**Test.** TestApp: empty/whitespace `user_code` → 400 + `AUTH_DEVICE_APPROVE` audit row with `outcome ValidationFailed`.

## Items 4 + 5 — surface dispatch: permission check before validation, actor attribution threaded

**Problem.**

- Item 5: the five method-mapped surface handlers call `require_valid()` (`surfaces.rs:742` and siblings) **before**
  `dispatch_surface_interaction` runs its permission enforcement (steps 2–3, `surfaces.rs:411-495`). All five surface request
  types currently validate `Ok(())`, so nothing is reachable today — but the first real rule turns this into an input-validity
  leak: unauthorized callers get 400 instead of 403.
- Item 4: `validation_failed_response` (`surfaces.rs:624`) hardcodes `auth_user.audit_actor(None)`, dropping `api_token_id`
  attribution.

**Decision (owner-confirmed).** One structural fix: handlers stop validating and pass the body through as
`Unvalidated<InvokeSurfaceInteractionRequest>` (or `Option<…>` for the optional-body methods); `dispatch_surface_interaction`
calls `require_valid()` at a single choke point **after** the step-3 permission checks. `validation_failed_response` moves to that
site and takes the actor attribution from `InteractionCallCtx` (`ctx.api_token_id`), fixing item 4 in the same motion.

**Envelope-peek seam.** Dispatch step 1 reads `target_provider_id` / `timeout_seconds` before resolution — resolution and the
permission lookup need the provider id, so envelope extraction cannot move after the checks. `Unvalidated<T>`'s inner value is
deliberately private (`extract.rs:324`), so the design adds a narrow, opt-in projection to `extract.rs`:

```rust
/// Routing metadata a dispatcher may read from a body before validation.
/// Implement only for types whose envelope fields are pure routing inputs
/// (they select a target; they are never business payload).
pub trait RoutingEnvelope {
    type Envelope;
    fn routing_envelope(&self) -> Self::Envelope;
}

impl<T: RoutingEnvelope> Unvalidated<T> {
    /// Project the routing envelope without unlocking the payload.
    pub fn peek_envelope(&self) -> T::Envelope {
        self.0.routing_envelope()
    }
}
```

`InvokeSurfaceInteractionRequest` implements `RoutingEnvelope` with
`Envelope = (Option<String>, Option<u16>)` (`target_provider_id`, `timeout_seconds`). The type-state guarantee is preserved for
everything not explicitly declared routing metadata; payload fields (`params`, `item_id`, idempotency key, …) remain reachable
only through `require_valid()`. Envelope values invalid enough to matter already produce uniform resolution errors for authorized
and unauthorized callers alike, so reading them pre-permission leaks nothing.

**Ordering after the change** (dispatch): split envelope → resolve → descriptor/interaction permission checks →
`require_valid()` → schema/param handling → provider dispatch. The GET path has no body and is untouched.

**Testability note (explicit deviation).** With all five `Validate` impls unconditionally `Ok(())`, no HTTP-level test can make
`require_valid()` fail, so the 403-before-400 ordering has no constructible discriminating test today — a test asserting 403 for
an unauthorized caller passes identically before and after this change (vacuous). Coverage provided instead:

- A direct unit test of the relocated `validation_failed_response` asserting the emitted audit row carries
  `actor_type ApiToken` when an `api_token_id` is present (item 4's observable behavior).
- Existing surface tests stay green (regression net).
- A code comment at the dispatch choke point states the mandated ordering and why it exists, so the first real validation rule's
  author inherits the invariant and can then write the discriminating test.

## Item 6 — gate ratchet: frozen entry set, not a row count

**Problem.** `ci/verify_no_raw_body_extractors.sh` enforces a row-count ceiling (`MAX_ALLOWLIST_ENTRIES=34`) plus a staleness
check. A delete-one-add-one swap keeps the count flat, matches a real flagged signature, and evades both — the allowlist header's
CAVEAT block documents exactly this hole, and ADR-0038's Consequences section admits the set is not frozen.

**Decision (owner-confirmed).** Compare against the exact frozen entry set. The script embeds the frozen 2026-08-05 rows (the
current allowlist's non-comment rows at freeze time) in a `FROZEN_ENTRIES` heredoc; the check requires every current allowlist row
to appear verbatim in the frozen set (current ⊆ frozen). Additions and swaps fail with a "frozen set" violation; deletions pass
(the existing STALE check keeps forcing them as Stage 2 converts sites). `MAX_ALLOWLIST_ENTRIES` and the count check are deleted —
one mechanism, no redundant belt. The set lives in the script rather than a second data file: editing the gate itself is the most
conspicuous possible diff, and one fewer file can drift.

**RED probes (run during implementation, per the gate-authoring discipline).**

1. Append a new `raw_extractor` row for a real raw-body handler → gate fails (frozen-set violation).
2. Delete one frozen row **and** add one new matching row (the swap) → gate fails.
3. Delete one row only → gate passes (shrink still allowed).
4. Unmodified tree → gate passes.

**Same-commit doc updates.** Allowlist header CAVEAT rewritten (the hole is now closed mechanically); ADR-0038's Consequences
caveat and `MAX_ALLOWLIST_ENTRIES` mentions updated to claim exactly what the script now checks; `docs/development/quality-gates.md`
gate description updated. The AGENTS.md quick-start line describes the gate generically ("frozen shrink-only allowlist") and needs
no text change — verified against the current line, noted here so the "update both in the same commit" constraint is discharged
explicitly.

## Item 7 — gate tripwire: `String` and `Multipart` body params

**Problem.** The third-door tripwire misses handlers that take `body: String` or `Multipart` — both implement `FromRequest` and
consume the body, bypassing extractor-keyed enforcement.

**Decision (owner-confirmed).** Extend the scan with two patterns:

- `:\s*(axum::extract::)?Multipart\s*(<|[,)])` — all scanned signatures (zero current uses; pure tripwire).
- `:\s*String\s*[,)]` — **only** signatures with a `pub` visibility prefix. Handlers must be `pub` to be referenced from
  `router.rs`; private column-0 async fns in `routes/` are internal helpers that legitimately take `String` args
  (e.g. `dispatch_surface_interaction(surface_id: String, …)`), and flagging them would poison the gate. Residual risk — a
  private routed handler taking `String` would evade — is documented in the script comment; none exists today and `router.rs`
  cannot reference private fns from other modules.

**Pre-check.** Run the new patterns over the current tree before landing: expected zero hits, so no allowlist rows are added. If a
hit appears, it is triaged (convert or allowlist with justification) in the same change, never silently absorbed.

**RED probes.** A scratch `pub async fn` with `body: String` → flagged; the same fn without `pub` → not flagged; a scratch
`pub async fn` with `mp: Multipart` → flagged; unmodified tree → passes.

## Item 8 — `reason_code` split: documented as intentional

`"validation_error"` is an **HTTP error-envelope code** (`ApiError` paths); `"invalid_request"` is an **audit details
reason_code**. Different namespaces with different consumers (API clients vs audit review); renaming either would churn recorded
audit rows or the API error contract for zero information gain. Decision (owner-confirmed): no rename. A short paragraph is added
to the Request Type Validation section of `docs/development/coding-standards.md` naming both values and their namespaces so the
split stops reading as drift.

## Alternatives considered

- **Item 1 — keep exactly-one and drop the query-layer fallback**: breaking for any client doing partial updates; deletes working
  behavior to satisfy a validator. Rejected.
- **Item 2 — document-only for `trigger_update`**: was the recommended default; owner chose handler-level emission so rejected
  trigger attempts are auditable. The cost (one catalog row) is small.
- **Item 5 — keep per-handler validation, hoist a pre-resolution permission check into each handler**: duplicates permission
  logic five times and still needs resolution for the action lookup. Rejected.
- **Item 6 — frozen set in a separate committed file**: workable, but a second data file is a second thing to tamper with and a
  less conspicuous diff than the gate script itself. Rejected.
- **Item 7 — key the `String` tripwire on the param name `body`**: an unenforced naming convention — the exact failure class this
  gate family exists to close. Rejected.

## Deliverables

Code:

- `crates/shared/web-api-types/src/software_items.rs` — item 1 validate + doc comments + unit tests.
- `crates/ui/web-api/src/routes/software_items/crud.rs`, `updates.rs` — item 2 Err-arm mirrors.
- `crates/ui/web-api/src/routes/device_auth.rs` — item 3 Err-arm emit.
- `crates/ui/web-api/src/routes/surfaces.rs` — items 4+5 restructure.
- `crates/ui/web-api/src/extract.rs` — `RoutingEnvelope` trait + `peek_envelope`.
- `crates/shared/audit-log/audit-catalog.toml` — item 2 `trigger_update` site row.
- `ci/verify_no_raw_body_extractors.sh` + `ci/verify_no_raw_body_extractors_allowlist.txt` — items 6, 7.
- Tests: TestApp handler tests (items 1, 2, 3), validate unit tests (item 1), `validation_failed_response` unit test (item 4).

Docs (non-optional):

- `docs/adr/0038-type-state-request-body-validation-via-unvalidated-extractor.md` — Consequences caveat rewritten to match the
  frozen-set mechanics (existing ADR amended in place; no new ADR — no new architectural decision is being made).
- `docs/development/quality-gates.md` — gate description (same commit as the gate change).
- `docs/development/coding-standards.md` — item 8 paragraph in the Request Type Validation section.
- `docs/superpowers/specs/2026-07-12-mutating-request-validation-design.md` — parked-decisions section annotated as resolved by
  this spec.
- Regenerated `crates/ui/web-api/openapi.json` + `frontend/src/lib/api/generated/` (item 1 doc-comment change), committed with it.
- AGENTS.md: no change required (verified; quick-start line already generic).

No new external dependencies; no wire-protocol, migration, or frontend-behavior changes.

## Constraints and conformance

- Typed errors, no `#[allow]`, TestApp harness for all new endpoint tests (snapshot Binding Rules).
- Audit changes go through the typed emit API (`builder_event` / `emit_event`) and `audit-catalog.toml`;
  `cargo xtask audit-coverage-check` gates items 2–3.
- Gate changes are RED-probed end-to-end (items 6–7 probe lists above) and land with their doc updates in the same commit.
- Audit assertions in tests key on discriminating fields, never "newest row" ordering.
- Docs/ADR claims about the gate state only what the script mechanically checks.
- Commits follow Conventional Commits; each commit leaves the whole tree green.

## Deferred / out of scope

Bucket-B1 handler conversions (Stage 2, separate spec); `Validated<T>` retirement (Stage 3); the 8 unconditional `Ok(())`
`Validate` impls (accepted as documented); real validation rules for the surface request types (first such rule must add the
403-before-400 discriminating test per item 5's note).

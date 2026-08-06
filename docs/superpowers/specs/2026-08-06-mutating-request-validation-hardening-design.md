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

**Zero-source routing (query layer).** The relaxation alone is a half-fix: `update_host_assignment_in_tx` branches on
`if let Some(pt) = req.plugin_type` (`host_assignments.rs:498`), so a zero-source request always falls into the config-based
branch — correct for config-backed rows (`existing_pcid` fallback), but for a row created via the **type-only inline path**
(`plugin_config_id` NULL, `plugin_type` set) `resolve_plugin_config_txn` hits its `_ =>` arm and errors `PluginConfigNotFound`.
The query layer therefore also changes: zero-source requests route to the type-only branch when **an existing row is present
and is type-only** — predicate `existing_plugin.as_ref().is_some_and(|ep| ep.plugin_config_id.is_none())`, NOT a bare
"existing_pcid is NULL" (that is also true when no row exists at the `role`/`ordinal`, a real reachable path that must keep
rejecting `PluginConfigNotFound`). Because the type-only branch head is `if let Some(pt) = req.plugin_type`, this is a small
restructure, not a bolted-on `||`: compute the effective plugin-type **string** (`req.plugin_type` as string, else the stored
row's `plugin_type` when the row is type-only) before the branch, and branch on that (`validate_assignment` takes `&str`; the
reuse is a plain simplification — `PluginTypeId` parsing is infallible, so no error-avoidance is claimed for it). Known
limitation, accepted: a zero-source edit on a row whose plugin type is no longer compiled in still hard-errors inside
`validate_assignment` (catalog lookup) — pre-existing behavior for all update paths, unchanged here. "Keep the existing plugin
source" must hold for both row shapes, not just config-backed ones.

**Changes.**

- `software_items.rs`: `sources != 1` → `sources > 1`; message text; update the field doc comments on `plugin_config`
  and `plugin_type` (currently "mutually exclusive with …") to state the at-most-one-for-update semantics.
- `crates/ui/web-api-queries/src/queries/software_items/host_assignments.rs`: the zero-source routing fix above (branch
  predicate becomes "request names a type, or the existing row is type-only"). Doc comments flow into
  the OpenAPI schema, so `./scripts/regen-api.sh` runs in the same commit and both artifacts
  (`crates/ui/web-api/openapi.json`, `frontend/src/lib/api/generated/`) are committed with it. Pre-declared artifact impact:
  `openapi.json` changes (description strings only); the generated TS client changes only if descriptions are emitted into it —
  commit whatever the regen produces.
- Unit tests (same file's test module): zero sources → `Ok`, one source → `Ok`, two sources → `Err` naming `plugin_config_id`.
- Handler tests (TestApp harness, `crates/ui/web-api/src/routes/software_items/tests.rs`), parameterized over **both** existing
  row shapes — a config-backed assignment AND a type-only inline assignment: PATCH with only `package_identifier` set →
  success, and the row's plugin source (`plugin_config_id` / `plugin_type`) is unchanged. The config-backed leg is RED on
  current `main` (400 from the validator); the type-only leg is RED even after the validator relaxation alone (it errors
  `PluginConfigNotFound` without the query-layer routing fix) — a single-fixture test would certify the half-fix. A third leg
  pins the negative path: zero-source PATCH naming a `role`/`ordinal` with **no existing row** → still rejected
  (`PluginConfigNotFound`), guarding the routing predicate against the no-row case.

## Item 2 — audit-mirror the `require_valid()` Err arms in `update_software_item` and `trigger_update`

**Problem.** Both handlers return an unaudited 400 on `require_valid()` Err while their handler families audit other rejections.

**Decision (owner-confirmed).** Both handlers emit an audit event on the Err arm; upgraded from the "document where none exists"
default for `trigger_update` — rejected trigger attempts become auditable.

**Changes.**

- `update_software_item` (`crates/ui/web-api/src/routes/software_items/crud.rs:270`): mirror the established pattern at
  `crates/ui/web-api/src/routes/software_items/host_assignments.rs:437-455` (the routes file — distinct from the identically
  named query-layer file cited in item 1) — hoist the `api_token_id` / `authenticated_user_audit_actor` / `tenant_id` derivations
  above the `require_valid()` match, then on Err build
  `AuditEntry::<Event>::builder_event(SOFTWARE_ITEM_UPDATE_AUDIT_ACTION)` with `.tenant_scope(tenant_id)`,
  `.actor(actor_type, actor_id)`, `.outcome(AuditOutcome::ValidationFailed)`, and
  `.details(json!({ "software_item_id": item_id, "reason_code": "invalid_request" }))`, emit it via
  `state.audit_emitter.emit_event(entry)` (build-then-emit — the Event class's async emit path), then return the existing 400.
  `SOFTWARE_ITEM_UPDATE` is registered Stateful, but the `builder_event`/`emit_event` failure-outcome emit for pre-transaction
  rejections is the file family's established precedent (same-file sibling `SOFTWARE_ITEM_UPDATE_HOST_ASSIGNMENT_AUDIT_ACTION`,
  also Stateful, at `host_assignments.rs:440`; and the merge-execute mirror in commit `9b3b46de3`).
- `trigger_update` (`crates/ui/web-api/src/routes/software_items/updates.rs:46`): on Err build-and-emit
  `SOFTWARE_UPDATE_TRIGGERED` (registered Event) the same way, with `.outcome(ValidationFailed)`,
  `.target("software_item", item_id.to_string(), None)`, and
  `.details(json!({ "host_id": host_id, "reason_code": "trigger_update.invalid_request" }))`, keep the existing `ApiError` 400
  with code `validation_error`. Field shape follows the **action family**, not the generic mirror family: every existing
  `SOFTWARE_UPDATE_TRIGGERED` row sets the `software_item` target (`controller-core/src/update/controller.rs:551`,
  `service_ws/handler/update_tracking.rs:30`) and uses site-namespaced reason codes (`trigger_update.host_not_assigned`,
  `trigger_update.software_item_not_found`) — a bare `invalid_request` with the item id buried in `details` would make these
  the only target-less rows for the action and collide with every subsystem's generic reject code. The family's codes are
  produced by one classifier (`TriggerUpdateError::trigger_audit_classification`,
  `crates/ui/web-api-queries/src/queries/update_dispatch.rs:99`, which returns inline literals — there is no const table);
  the new code lands as a `pub const` beside that classifier and the handler references the const. This buys naming proximity
  plus one mechanical tie: a unit test beside the classifier asserts the const carries the family's `trigger_update.` prefix.
  Known field-shape divergence, stated
  deliberately: the family's success/failure rows also carry `to_version`/`interactive` in details; the validation-reject row
  cannot (those fields sit behind the failed validation). The action's success emission lives downstream in
  `controller-core/src/update/controller.rs`, but the catalog keys coverage at function granularity and
  `audit-catalog.toml` already carries the `…::updates::trigger_update` site with `software.update.triggered`
  (`crates/shared/audit-log/audit-catalog.toml:619`), so **no new catalog row is expected**;
  `cargo xtask audit-coverage-check` runs in that task's gate list to confirm.
- `trigger_update`'s audit actor derives via `authenticated_user_audit_actor(&user, api_token_id)` — the same helper
  `update_software_item` uses — NOT via the handler's existing dispatch-oriented `ActorType` match (`updates.rs:58-61`), which
  feeds `UpdateDispatchParams` and is a separate concern.
- Tests (TestApp): for each handler, send an invalid body (icon URL failing `validate_https_icon_url` for
  `update_software_item`; over-length `to_version` failing `validate_command_length` for `trigger_update`) → assert 400 **and**
  assert the audit row via the existing `tenant_audit_row_for_action_and_outcome` helper (`tests.rs:344`) — never the
  action-only/newest-row sibling helper.

## Item 3 — `device_auth_approve`: restore audit on validation reject

**Problem.** A garbage `user_code` now 400s at the `require_valid()` boundary (`crates/ui/web-api/src/routes/device_auth.rs:74`)
with no audit row; previously the handler emitted a device-auth decision audit event and returned 404.

**Decision (owner-confirmed).** Keep the 400; emit `AUTH_DEVICE_APPROVE` (registered Event) with `outcome ValidationFailed` and
`reason_code "invalid_request"` on the Err arm. No `device_flow_id` detail — the code is rejected before normalization/hashing, so
none exists. Build via a direct `AuditEntry::<Event>::builder_event` chain and emit via `state.audit_emitter.emit_event(entry)`
(the existing `emit_device_auth_decision_audit` helper requires a `device_flow_id`; do not widen its signature for one call
site). Same handler site already emits this action on both existing
arms, so no `audit-catalog.toml` change is expected; `cargo xtask audit-coverage-check` confirms.

**Test.** TestApp: empty/whitespace `user_code` → 400 + `AUTH_DEVICE_APPROVE` audit row with `outcome ValidationFailed`.

## Items 4 + 5 — surface dispatch: permission check before validation, actor attribution threaded

**Problem.**

- Item 5: the five method-mapped surface handlers call `require_valid()` (`surfaces.rs:740` and siblings at 799, 859, 1003,
  1064) **before**
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
deliberately private (`extract.rs:324`), so the design adds a narrow, opt-in projection. The `RoutingEnvelope` trait is declared
in `crates/shared/web-api-types/src/validation.rs` beside `Validate` (the types crate cannot see `web-api`, so declaring it in
`extract.rs` would force the impl away from the struct); the concrete impl co-locates with the existing
`impl Validate for InvokeSurfaceInteractionRequest` in `crates/shared/web-api-types/src/surfaces.rs`; only the inherent
projection method on `Unvalidated<T>` lands in `extract.rs`:

```rust
// crates/shared/web-api-types/src/validation.rs (beside Validate):

pub(crate) mod sealed {
    pub trait Sealed {}
}
// pub(crate): the Sealed impl lives in the sibling surfaces.rs module (a
// private `mod sealed` would E0603 there); still unimplementable outside
// the crate, so the seal holds. No private_bounds warning fires.

/// Routing metadata a dispatcher may read from a body before validation.
/// Sealed: implementable only inside web-api-types, so declaring a type's
/// envelope is a reviewed change in the crate that owns the request types —
/// a doc-comment convention alone would be an ungated escape hatch from the
/// Unvalidated<T> type-state guarantee. Envelope fields are pure routing
/// inputs (they select a target; they are never business payload).
pub trait RoutingEnvelope: sealed::Sealed {
    type Envelope;
    fn routing_envelope(&self) -> Self::Envelope;
}

// crates/ui/web-api/src/extract.rs (imports RoutingEnvelope from web-api-types):

impl<T: RoutingEnvelope> Unvalidated<T> {
    /// Project the routing envelope without unlocking the payload.
    pub fn peek_envelope(&self) -> T::Envelope {
        self.0.routing_envelope()
    }
}
```

`InvokeSurfaceInteractionRequest` implements `RoutingEnvelope` with a **named** envelope struct — mirroring the GET path's
existing `GetInvokeEnvelope` precedent (`surfaces.rs:1200`, which cannot be reused directly since it lives in `web-api` and the
types crate cannot depend back on it):

```rust
// crates/shared/web-api-types/src/surfaces.rs (beside the RoutingEnvelope impl):

/// Body-path routing envelope; field-for-field twin of web-api's GetInvokeEnvelope.
#[derive(Debug, Clone)]
pub struct InvokeRoutingEnvelope {
    pub target_provider_id: Option<String>,
    pub timeout_seconds: Option<u16>,
}
```

with `type Envelope = InvokeRoutingEnvelope` (plus the one-line `impl sealed::Sealed for InvokeSurfaceInteractionRequest`) —
never an anonymous tuple (positional `Option`s invite swapped-field bugs the named struct exists to prevent). The type-state guarantee is preserved for
everything not explicitly declared routing metadata; payload fields (`params`, `item_id`, idempotency key, …) remain reachable
only through `require_valid()`. Envelope values invalid enough to matter already produce uniform resolution errors for authorized
and unauthorized callers alike, so reading them pre-permission leaks nothing. `Unvalidated<T>`'s struct doc comment ("the only
way to reach the fields is `require_valid`") is amended in the same change to name the `RoutingEnvelope` carve-out — the doc
states the type's core invariant and must not go stale.

**Ordering after the change** (dispatch): split envelope → resolve → descriptor/interaction permission checks →
`require_valid()` → schema/param handling → provider dispatch. The GET path has no body and is untouched.

**Precision of the invariant.** The reorder covers **semantic** validation (`Validate` rules) only. Deserialization failures
(malformed JSON, wrong field types) are rejected by `Unvalidated<T>`'s `FromRequest` impl before any handler code runs and
cannot move after the permission checks — an unauthorized caller retains a shape-level 400 oracle regardless. The choke-point
comment (and any future test) must state exactly this: "semantic validation after permission enforcement; deserialization is
structurally earlier and out of scope" — never the overclaim "unauthorized callers always get 403 before any 400".

**Testability note (explicit deviation).** With all five `Validate` impls unconditionally `Ok(())`, no HTTP-level test can make
`require_valid()` fail, so the 403-before-400 ordering has no constructible discriminating test today — a test asserting 403 for
an unauthorized caller passes identically before and after this change (vacuous). Coverage provided instead:

- A direct unit test of the relocated `validation_failed_response` asserting the emitted audit row carries
  `actor_type ApiToken` when an `api_token_id` is present (item 4's observable behavior).
- Existing surface tests stay green (regression net).
- A code comment at the dispatch choke point states the mandated ordering and why it exists, so the first real validation rule's
  author inherits the invariant and can then write the discriminating test.

## Item 6 — gate ratchet: shrink-only enforced against history, not a row count

**Problem.** `ci/verify_no_raw_body_extractors.sh` enforces a row-count ceiling (`MAX_ALLOWLIST_ENTRIES=34`) plus a staleness
check. A delete-one-add-one swap keeps the count flat, matches a real flagged signature, and evades both — the allowlist header's
CAVEAT block documents exactly this hole, and ADR-0038's Consequences section admits the set is not frozen.

**Decision (owner-confirmed mechanism, refined during review).** Shrink-only is a **history** property, so it is enforced
against history directly: the script compares the live allowlist's non-comment rows against the same file's content at
`$(git merge-base HEAD origin/main)` and requires `current ⊆ base` — every row must already exist at the merge-base. No
snapshot companion files: an in-tree frozen/retired pair was considered and discarded because any in-tree baseline is
direction-free (a working tree can move rows between its own files and satisfy every set relation; only the merge-base copy
is outside the commit's control). This closes additions, swaps, delete-then-re-add, and the one-commit trade (convert handler
A, regress handler B) in one check, with zero extra files and no ratchet constant. `MAX_ALLOWLIST_ENTRIES` and the count check
are deleted.

Two refinements, stated so the docs claim exactly what the script checks:

- **Rename support**: a row addition is permitted iff a base row with the same `class` and fn-regex exists at a different
  path and that base row is simultaneously absent from `current` — supporting file moves/facade splits (a documented activity
  in this repo) for still-allowlisted handlers, without admitting any new handler. A regressed handler's row cannot enter this
  way: its base row is gone from the baseline.
- **Baseline availability**: when `origin/main` or the merge-base is unavailable (offline local run, shallow clone), the
  history sub-check emits a loud warning and is skipped; the STALE and violation checks still run. CI and pre-push always
  have the baseline, so the enforcement surface is the one that matters. This residual is stated in the script comment.

**RED probes (run during implementation, per the gate-authoring discipline).**

1. Append a new `raw_extractor` row for a real raw-body handler → gate fails (row absent at merge-base).
2. Delete one row **and** add one new matching row (the swap) → gate fails.
3. Delete one row only → gate passes (shrink still allowed, no side bookkeeping).
4. Commit a deletion, then in a follow-up working tree re-add the same row → gate fails (row no longer at merge-base) — the
   delete-then-re-add hole, one-commit trade variant included.
5. Move a live row's `path` field (same class+regex), removing the old-path row → gate passes (rename support).
6. Same as probe 5 but the old-path row kept → gate fails (addition, not a rename).
7. Unmodified tree → gate passes; tree with baseline unavailable → warning + remaining checks still run.

**Same-commit doc updates.** Allowlist header CAVEAT rewritten (the hole is now closed mechanically); ADR-0038's Consequences
caveat and `MAX_ALLOWLIST_ENTRIES` mentions updated to claim exactly what the script now checks; the second copy of the same
count-ceiling caveat in `docs/development/coding-standards.md` § Raw body extractors are banned (line 1070) rewritten identically;
`docs/development/quality-gates.md` gate description updated. The AGENTS.md quick-start line ("Request bodies go through
Unvalidated&lt;T&gt;/Validated&lt;T&gt;; raw Json/Form banned") does not reference the ratchet mechanics and needs no text change —
verified against the current line, noted here so the "update both in the same commit" constraint is discharged explicitly.

## Item 7 — gate tripwire: `String` and `Multipart` body params

**Problem.** The third-door tripwire misses handlers that take `body: String` or `Multipart` — both implement `FromRequest` and
consume the body, bypassing extractor-keyed enforcement.

**Decision (owner-confirmed).** Extend the scan with two patterns:

- `:\s*(axum::extract::)?Multipart\s*(<|[,)])` — all scanned signatures (zero current uses; pure tripwire).
- `String` in **last-parameter position** (the only position axum treats as a body extractor), **only** in signatures with a
  **bare `pub`** prefix — deliberately narrower than the script's existing any-visibility `pub(…)` matcher. Rationale:
  `routes/service_ws/` carries `pub(super) async fn` helpers with `String` params that are ordinary arguments, not body
  extractors (`connection.rs::handle_authenticated(cert_serial: String, …)`,
  `handler/session_authenticated.rs::send_ws_with_timeout(…, json: String, …)`,
  `handler/updates/result.rs::select_best_output(…, agent_output: String)`), and private helpers like
  `dispatch_surface_interaction(surface_id: String, …)` are equally innocent; flagging them would poison the gate. Every
  routed handler today is bare `pub`. Documented residuals in the script comment: a `pub(crate)`-visible routed handler, or a
  handler taking `String` in non-last position, would evade — neither exists today and both are review-visible.

**Pre-check.** Run the new patterns over the current tree before landing: expected zero hits with the narrowed anchors, so no
allowlist rows are added. If a hit appears, it is triaged (convert or allowlist with justification) in the same change, never
silently absorbed.

**RED probes.** A scratch `pub async fn` with trailing `body: String` → flagged; the same fn as `pub(super)` → not flagged;
the same fn with `String` in non-last position → not flagged; a scratch `pub async fn` with `mp: Multipart` → flagged;
unmodified tree → passes (in particular, the three `pub(super)` service_ws helpers above stay unflagged).

## Item 8 — `reason_code` split: documented as intentional

`"validation_error"` is an **HTTP error-envelope code** (`ApiError` paths); `"invalid_request"` is an **audit details
reason_code**. Different namespaces with different consumers (API clients vs audit review); renaming either would churn recorded
audit rows or the API error contract for zero information gain. Decision (owner-confirmed): no rename. A short paragraph is added
to the Request Type Validation section of `docs/development/coding-standards.md` naming both values and their namespaces so the
split stops reading as drift. The same paragraph notes the second axis: reason codes on the `SOFTWARE_UPDATE_TRIGGERED` action
family are **site-namespaced** (`trigger_update.*`), while the generic `require_valid()` mirror family uses bare
`invalid_request` — item 2's `trigger_update` emit follows its action family (see item 2), not the mirror convention.

## Alternatives considered

- **Item 1 — keep exactly-one and drop the query-layer fallback**: breaking for any client doing partial updates; deletes working
  behavior to satisfy a validator. Rejected.
- **Item 2 — document-only for `trigger_update`**: was the recommended default; owner chose handler-level emission so rejected
  trigger attempts are auditable. The cost is one emit block plus a reason-code const — no catalog change (the site is
  already registered; see item 2).
- **Item 5 — keep per-handler validation, hoist a pre-resolution permission check into each handler**: duplicates permission
  logic five times and still needs resolution for the action lookup. Rejected.
- **Item 6 — in-tree frozen baseline** (script heredoc, `.frozen.txt` companion, or a frozen+retired partition — all three
  were iterated through review): every in-tree variant is direction-free — the working tree controls all sides of the
  comparison, so a commit can move rows between its own files and satisfy any set relation; and a count ceiling (kept or
  tightened to equality) is evaded by any equal-size trade. Rejected for the merge-base history check, where the baseline is
  outside the commit's control.
- **Item 7 — key the `String` tripwire on the param name `body`**: an unenforced naming convention — the exact failure class this
  gate family exists to close. Rejected.

## Deliverables

Code:

- `crates/shared/web-api-types/src/software_items.rs` — item 1 validate + doc comments + unit tests.
- `crates/ui/web-api-queries/src/queries/software_items/host_assignments.rs` — item 1 zero-source routing to the type-only
  branch for type-only rows.
- `crates/ui/web-api/src/routes/software_items/crud.rs`, `updates.rs` — item 2 Err-arm mirrors.
- `crates/ui/web-api/src/routes/device_auth.rs` — item 3 Err-arm emit.
- `crates/ui/web-api/src/routes/surfaces.rs` — items 4+5 restructure.
- `crates/shared/web-api-types/src/validation.rs` — `RoutingEnvelope` trait declaration (beside `Validate`).
- `crates/shared/web-api-types/src/surfaces.rs` — `InvokeRoutingEnvelope` struct +
  `impl RoutingEnvelope for InvokeSurfaceInteractionRequest` (beside its `Validate` impl).
- `crates/ui/web-api/src/extract.rs` — `peek_envelope` inherent method + amended `Unvalidated<T>` doc comment.
- `crates/ui/web-api-queries/src/queries/update_dispatch.rs` — `pub const` for `trigger_update.invalid_request` beside the
  family classifier (item 2).
- `crates/shared/audit-log/audit-catalog.toml` — no row changes expected (all three emit sites already cataloged at function
  granularity); `cargo xtask audit-coverage-check` confirms.
- `ci/verify_no_raw_body_extractors.sh` + `ci/verify_no_raw_body_extractors_allowlist.txt` — items 6, 7.
- Tests: TestApp handler tests (items 1, 2, 3), validate unit tests (item 1), `validation_failed_response` unit test (item 4).

Docs (non-optional):

- `docs/adr/0038-type-state-request-body-validation-via-unvalidated-extractor.md` — Consequences caveat rewritten to match the
  merge-base shrink-only mechanics (existing ADR amended in place; no new ADR — no new architectural decision is being made).
- `docs/development/quality-gates.md` — gate description (same commit as the gate change).
- `docs/development/coding-standards.md` — item 8 paragraph in the Request Type Validation section, plus the item 6
  count-ceiling caveat rewrite at line 1070.
- `docs/superpowers/specs/2026-07-12-mutating-request-validation-design.md` — the doc has no dedicated parked-decisions
  section (its deferral notes are scattered inline); add a short "Resolved by" pointer near the top referencing this spec for
  the formerly-parked owner decisions.
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
discriminating test per item 5's note: 403 before the *semantic* 400 — a well-formed body violating a `Validate` rule from an
unauthorized caller; deserialization 400s are structurally earlier and exempt).

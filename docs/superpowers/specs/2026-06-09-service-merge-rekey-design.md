# Service Merge Re-key

**Date:** 2026-06-09
**Status:** Approved

## Problem

Merging a freshly-enrolled pending Service into an existing approved Service breaks
the running Agent's reconnect with `401 Unauthorized`.

Observed (agent-ssh, `--features reset-data`):

```text
INFO uptrakit_service_sdk::ws: connecting to controller url=wss://…/api/v1/ws/service?service_id=019eadd3-011a-7460-a754-8260d201db3f
INFO uptrakit_service_sdk::lifecycle: transient enrollment error, reconnecting in 2.046s error=
 ● WebSocket error: HTTP error: 401 Unauthorized
   ╰ crates/shared/service-sdk/src/ws.rs:129
```

DB at `~/Library/Application Support/org.uptrakit.controller/uptrakit.db` confirms:

| id (hex)                           | status   | deactivated_at              | note   |
| ---------------------------------- | -------- | --------------------------- | ------ |
| `019eadd3011a7460a7548260d201db3f` | pending  | `2026-06-09T19:26:49.188Z`  | source |
| `019d76d30aa67d13bfc3986e921939bc` | approved | _null_                      | target |

## Root Cause

`merge_service` in `crates/ui/web-api-queries/src/queries/services.rs:639` moves the
source row's `enrollment_secret_hash`, `hostname`, `friendly_name`, `ip_address` onto
the target row and deactivates the source. The running Agent's `service.json` still
holds `service_id = source_uuid`.

On reconnect the Agent calls
`resume_enrollment` (`crates/shared/service-sdk/src/ws.rs:523`) which appends
`?service_id=<source_uuid>` to the WS URL. The controller's `lookup_by_secret`
(`crates/ui/web-api/src/routes/service_ws/mod.rs:193`) narrows by:

```text
secret_hash == hash AND deactivated_at IS NULL AND id == query_service_id
```

The source row is deactivated, the target row's id does not equal the query hint, so
no row matches. The controller returns 401.

## Desired Behaviour

| Scenario | Result |
| --- | --- |
| Agent reconnects with source enrollment secret + source service_id after merge | Controller resolves to target via redirect, accepts auth, pushes `Approved { service_id: target }`, issues cert under target. |
| Agent persists target service_id locally after first re-keyed reconnect | All subsequent connections use target_uuid (bearer until cert installed, mTLS after). |
| Operator merges B into C while A→B redirect already exists | A→B redirect rewritten to A→C in the same transaction. |
| Operator attempts to merge into (or from) an embedded service | API returns 400 `service.embedded`. CLI / internal callers receive typed `ServiceQueryError::TargetEmbedded` / `SourceEmbedded`. Frontend hides embedded services from merge-target picker. |
| Target service deleted while a redirect still points at it | Cascade deletes the redirect row (FK `ON DELETE CASCADE`). |
| Merge re-key resolves on connect | New `AUTH_SERVICE_REKEY_RESOLVED` audit event written; details `{source_id, target_id}`. |
| Existing `SERVICE_MERGE` audit at merge time | Unchanged. |

## Solution

### 1. New table `service_merge_redirect`

Migration `crates/shared/db/src/migration/m20260610_000001_service_merge_redirect.rs`:

```text
service_merge_redirect
- source_id     UUID NOT NULL PRIMARY KEY   -- old service uuid agent persists locally
- target_id     UUID NOT NULL               -- active service uuid
- redirected_at TIMESTAMPTZ NOT NULL

FK target_id → services(id) ON DELETE CASCADE
INDEX idx_smr_target (target_id)
```

SeaORM `.uuid()` is used for the UUID columns (BLOB-backed on SQLite, native on
Postgres) to match every other UUID column in the workspace. `helpers::timestamp(...)`
wraps `.timestamp_with_time_zone().not_null()` per established migration convention.

No `tenant_id` column. Tenant is inferred via FK→`services` (same pattern as
`service_host`; CLAUDE.md "Tenant Isolation for Join Tables"). Cross-tenant rows are
impossible by construction because `merge_service` runs inside a single `TenantDb`.

New SeaORM entity at `crates/shared/db/src/entity/service_merge_redirect.rs`.

### 2. `merge_service` query — extend in-tx logic

`crates/ui/web-api-queries/src/queries/services.rs:639`:

1. **Embedded guard** (early bail, inside the existing `BEGIN IMMEDIATE` txn so the
   read is consistent with the rest of the merge):
   - If `target.is_embedded` → `ServiceQueryError::TargetEmbedded`.
   - If `source.is_embedded` → `ServiceQueryError::SourceEmbedded`.
2. **Chain invariant assertion**: chains A→B→C cannot form by construction —
   `merge_service` requires `source.status == Pending` and `target.status == Approved`,
   and a previously-deactivated row is neither. Therefore no `service_merge_redirect`
   row may have `target_id == source_uuid` at merge time. Encode this as a
   debug-assertion query (read-only, expected count = 0); fail the merge with
   `ServiceQueryError::Db` if violated so the invariant breach surfaces loudly
   rather than silently corrupting state:

   ```rust
   let dangling = ServiceMergeRedirect::find()
       .filter(service_merge_redirect::Column::TargetId.eq(source_uuid))
       .count(&txn)
       .await
       .context_to::<ServiceQueryError>()?;
   debug_assert_eq!(dangling, 0, "redirect chain invariant violated: source is a redirect target");
   if dangling != 0 {
       bail!(ServiceQueryError::RedirectChainInvariantViolated);
   }
   ```

3. **Redirect insert** via SeaORM `on_conflict` upsert (no raw SQL):

   ```rust
   let model = service_merge_redirect::ActiveModel {
       source_id: Set(source_uuid),
       target_id: Set(target_uuid),
       redirected_at: Set(now),
   };
   ServiceMergeRedirect::insert(model)
       .on_conflict(
           OnConflict::column(service_merge_redirect::Column::SourceId)
               .update_columns([
                   service_merge_redirect::Column::TargetId,
                   service_merge_redirect::Column::RedirectedAt,
               ])
               .to_owned(),
       )
       .exec(&txn)
       .await
       .context_to::<ServiceQueryError>()?;
   ```

   Upsert handles the (extremely rare) re-merge of a previously-deactivated source.

**Transitive-chain reasoning**: chains cannot pre-exist because (a) merge requires
`source.status == Pending` so a previously-deactivated row cannot be a future
source, and (b) merge requires `target.status == Approved` and not deactivated, so a
deactivated row cannot be a future target either. The new error variant
`ServiceQueryError::RedirectChainInvariantViolated` surfaces the (impossible-by-design)
breach instead of silently rewriting state.

New typed errors on `ServiceQueryError`: `TargetEmbedded`, `SourceEmbedded`,
`RedirectChainInvariantViolated`. `TargetEmbedded` and `SourceEmbedded` map to
HTTP 400 with reason codes `service.embedded_target` / `service.embedded_source` in
`crates/ui/web-api/src/actions/services.rs`. `RedirectChainInvariantViolated` maps
to HTTP 500 with reason `service.merge_invariant` — it indicates a corrupted
redirect table and warrants operator investigation. All variants `#[non_exhaustive]`-free
because `ServiceQueryError` is internal (per coding-standards.md).

### 3. Route handler — fast 400

`crates/ui/web-api/src/routes/services.rs:1089` (`merge_service`):

Before calling `state.service_connections.is_connected(...)`, load both services and
short-circuit:

```rust
if target.is_embedded || source.is_embedded {
    emit_service_lifecycle_audit(... ValidationFailed, reason "service.embedded_target" | "service.embedded_source");
    return Ok(error_response(StatusCode::BAD_REQUEST, "Cannot merge embedded services"));
}
```

This is defence-in-depth — the query-layer guard is the single source of truth, but
the route layer turns the error into a clear 400 with a focused message and a
ValidationFailed audit entry, avoiding the generic 500-to-400 mapping in
`ApiError::from`.

### 4. WS auth — redirect fallback + audit

`crates/ui/web-api/src/routes/service_ws/mod.rs`, `lookup_by_secret` (`:193`):

Current narrow lookup is preserved. After both tenant and system lookups miss, and
only when `query_service_id` is `Some(hint)`, try one additional lookup via
redirect:

```rust
let Some(hint) = service_id else {
    return Err(report!(ServiceWsError::InvalidSecret));
};
let Some(redirect) = ServiceMergeRedirect::find_by_id(hint)
    .one(db)
    .await
    .context_to::<ServiceWsError>()?
else {
    return Err(report!(ServiceWsError::InvalidSecret));
};
let Some(target) = service_entity::Entity::find_by_id(redirect.target_id)
    .filter(service_entity::Column::DeactivatedAt.is_null())
    .filter(service_entity::Column::EnrollmentSecretHash.eq(&secret_hash))
    .one(db)
    .await
    .context_to::<ServiceWsError>()?
else {
    return Err(report!(ServiceWsError::InvalidSecret));
};
emit_rekey_resolved_audit(state, hint, target.id, target.tenant_id).await;
Ok((target.id, /* is_system = */ false))
```

The fallback is gated on `query_service_id.is_some()` — anonymous bearer attempts
without a hint are never redirected, preserving the comment-block defence-in-depth
about cross-service collisions.

System-services path (`SystemService::find()`) is unchanged — embedded/system
services never participate in redirects.

New audit emission helper `emit_rekey_resolved_audit` writes
`AUTH_SERVICE_REKEY_RESOLVED` with actor `Service(target_id)`, tenant scope from the
target row, details `{ "source_id": hint, "target_id": target.id,
"reason_code": "merge_redirect" }`, outcome `Success`.

Emission must use the non-blocking `audit_emitter.emit_event(entry)` path (same as
`emit_bearer_service_auth_failure_audit`) so the WS upgrade is not delayed by audit
serialization or persistence.

**Miss-path observability**: the existing `emit_bearer_service_auth_failure_audit`
already fires on `InvalidSecret`. Enrich its `details` JSON to include
`"redirect_checked": true` and `"redirect_present": <bool>` whenever the redirect
fallback was attempted. On-call grep for "redirect_checked: true, redirect_present:
false" then surfaces every stuck rebind — without this, on-call cannot
distinguish "wrong secret" from "merge happened but agent still pinned to source".

### 5. Audit catalog

`crates/shared/audit-log/src/action_type.rs` — three coupled additions
(missing any one will make emission fail at runtime via `is_registered()`):

1. **Const declaration** alongside `AUTH_SERVICE_AUTHENTICATE` (line 88):

   ```rust
   pub const AUTH_SERVICE_REKEY_RESOLVED: RegisteredAuditAction =
       RegisteredAuditAction::new("auth.service.rekey_resolved", AuditActionKind::Event);
   ```

2. **Entry in `V1_ACTIONS`** static slice (line 422 onward).
3. **Entry in `audit_actions!` macro block** (line 678 onward):
   `auth_service_rekey_resolved => AUTH_SERVICE_REKEY_RESOLVED, Event;`

No schema migration — `AuditActionType` is a `const` namespace.

### 6. Wire / SDK — re-key on resume

No protocol change. `ApprovedPayload.service_id` is already authoritative.

`crates/shared/service-sdk/src/ws.rs:222` — change `wait_for_approval` return type to
`Result<Uuid>` (returns `payload.service_id`).

`crates/shared/service-sdk/src/ws.rs:523` — `resume_enrollment`:

```rust
let approved_id = wait_for_approval(&mut ws, &mut in_seq).await?;
if approved_id.is_nil() {
    bail!(EnrollmentError::Protocol(ProtocolError::Enrollment(
        "controller sent nil service_id in Approved".to_string()
    )));
}
if Some(approved_id) != identity.service_id() {
    let old_id = identity.service_id();
    let secret = identity.enrollment_secret()
        .ok_or_else(|| report!(EnrollmentError::Identity(IdentityError::NotEnrolled)))?
        .to_string();
    identity.save_enrollment(approved_id, &secret).await?;
    tracing::info!(?old_id, new_id = %approved_id,
                   "service identity rebound via merge redirect");
}
identity.ensure_keypair().await?;
let csr_pem = identity.generate_csr_for_self()?;   // CN = approved_id
```

The `is_nil()` guard prevents silent corruption of `service.json` if any future
controller code path forgets to populate `ApprovedPayload.service_id` correctly —
the agent refuses the message instead of overwriting its identity with `Uuid::nil()`.

`run_enrollment` (`crates/shared/service-sdk/src/ws.rs:456`, the fresh-enrollment
path) also calls `wait_for_approval` at line 503. The returned id always matches
`identity.service_id()` for the fresh-enrollment path (controller assigned the id
in the immediately-preceding `Enrolled` response which was already persisted via
`save_enrollment`), so the rebind branch is a no-op. The call site must still
handle the new return type — use `let _approved_id = wait_for_approval(...).await?;`
to discard the value without triggering `clippy::let_underscore_must_use`.

**`SecretString` note** (pre-existing pattern): `ServiceIdentityState::enrollment_secret()`
returns `Option<&str>` and `save_enrollment(&mut self, service_id, &str)` takes a
plain `&str`. This is unchanged by this spec — the credential is already exposed
as `&str` at this API boundary in the existing SDK. Tightening the SDK API to
`SecretString` is out of scope.

`asyncapi.yaml` documentation: add a sentence to `ApprovedPayload` noting that
`service_id` is authoritative and may differ from the connection's
`?service_id=` hint when a server-side merge has occurred since enrollment.

### 7. Frontend

The current merge UI lives in `frontend/src/routes/services/+page.svelte` (no
dedicated dialog component yet). The existing `mergeTargetOptions` `$derived` at
`:121` filters by `status === 'approved' && capabilities.includes('software_discovery')
&& id !== mergeSource?.id`. Required changes:

- **Target filter (`:124`)** — add `!s.is_embedded` to the existing predicate; PRESERVE
  the existing `capabilities.includes('software_discovery')` and self-exclusion
  checks. Final form:

  ```ts
  s.status === 'approved'
    && s.capabilities.includes('software_discovery')
    && !s.is_embedded
    && s.id !== mergeSource?.id
  ```

- **Source picker** — the source list (pending services available for merge) must
  also filter `!s.is_embedded`. Embedded services cannot enter the Pending state via
  enrollment, but the filter is added as belt-and-braces against any future code
  path that could surface them.
- On 400 with reason_code `service.embedded_*` surface inline form error using the
  existing form-error pattern in the page.

`ServiceResponse.is_embedded` is already exposed
(`crates/shared/web-api-types/src/services.rs:20`) — no API surface change needed.

## Schema migration

Single up migration `m20260610_000001_service_merge_redirect`:

```rust
manager
    .create_table(
        Table::create()
            .table(ServiceMergeRedirect::Table)
            .if_not_exists()
            .col(ColumnDef::new(ServiceMergeRedirect::SourceId).uuid().not_null().primary_key())
            .col(ColumnDef::new(ServiceMergeRedirect::TargetId).uuid().not_null())
            .col(helpers::timestamp(ServiceMergeRedirect::RedirectedAt))
            .foreign_key(
                ForeignKey::create()
                    .from(ServiceMergeRedirect::Table, ServiceMergeRedirect::TargetId)
                    .to(Service::Table, Service::Id)
                    .on_delete(ForeignKeyAction::Cascade),
            )
            .to_owned(),
    )
    .await?;
manager
    .create_index(
        Index::create()
            .table(ServiceMergeRedirect::Table)
            .name("idx_smr_target")
            .col(ServiceMergeRedirect::TargetId)
            .to_owned(),
    )
    .await?;
```

`down`: drop index + table.

## Files Touched

| Path | Change |
| --- | --- |
| `crates/shared/db/src/migration/m20260610_000001_service_merge_redirect.rs` | new — migration |
| `crates/shared/db/src/migration/mod.rs` | register new migration |
| `crates/shared/db/src/entity/service_merge_redirect.rs` | new — SeaORM entity |
| `crates/shared/db/src/entity/prelude.rs` | export entity alias |
| `crates/shared/db/src/entity/mod.rs` | `pub mod service_merge_redirect;` |
| `crates/ui/web-api-queries/src/queries/services.rs:639` | embedded guards, redirect collapse + insert in txn, new error variants |
| `crates/ui/web-api/src/routes/services.rs:1089` | route-level 400 for embedded |
| `crates/ui/web-api/src/actions/services.rs` | map new error variants → 400 / audit reason codes |
| `crates/ui/web-api/src/routes/service_ws/mod.rs` | `lookup_by_secret` redirect fallback; new `emit_rekey_resolved_audit` |
| `crates/shared/audit-log/src/action_type.rs` | new `AUTH_SERVICE_REKEY_RESOLVED` constant |
| `crates/shared/service-sdk/src/ws.rs:222` | `wait_for_approval -> Result<Uuid>` |
| `crates/shared/service-sdk/src/ws.rs:523` | re-key branch in `resume_enrollment` |
| `crates/shared/service-sdk/src/ws.rs:503` (`run_enrollment`) | discard returned id via `let _approved_id = ...` (no-op for fresh path) |
| `crates/shared/wire/asyncapi.yaml` | doc note on `ApprovedPayload.service_id` rebind semantics |
| `frontend/src/routes/services/+page.svelte` | filter `is_embedded` from merge source + target lists; surface 400 reason_codes |
| `docs/adr/0020-service-merge-redirect.md` | new ADR |
| `CONTEXT.md` | glossary entry for "Service Merge Redirect" |

## Tests

Unit / query tests (`crates/ui/web-api-queries`):

- `merge_inserts_redirect_row` — source_id → target_id present after merge.
- `merge_collapses_existing_redirect_chain` — A→B exists, merge B→C, row becomes A→C; new row B→C also present.
- `merge_rejects_embedded_target` — returns `TargetEmbedded`.
- `merge_rejects_embedded_source` — returns `SourceEmbedded`.
- `merge_redirect_cascades_on_target_delete` — delete target → redirect row gone.

WS auth tests (`crates/ui/web-api/src/routes/service_ws`):

- `lookup_by_secret_uses_redirect_when_source_deactivated` — bearer + source_uuid
  hint resolves to target.
- `lookup_by_secret_rejects_redirect_when_hash_mismatch` — redirect exists but secret
  hash on target differs → `InvalidSecret`.
- `lookup_by_secret_skips_redirect_when_no_hint` — bare bearer without
  `?service_id=` does not consult redirects (defence-in-depth).
- `service_ws_emits_rekey_resolved_audit_on_redirect` — audit row with action
  `AUTH_SERVICE_REKEY_RESOLVED`, success outcome, expected detail keys.

Route tests (`crates/ui/web-api/src/routes/services.rs`):

- `merge_returns_400_when_target_embedded` — body shape: reason_code
  `service.embedded_target`.
- `merge_returns_400_when_source_embedded` — reason_code `service.embedded_source`.

SDK tests (`crates/shared/service-sdk`):

- `wait_for_approval_returns_payload_service_id` — driven by an in-process mock
  WebSocket stream. Pure transport test; no tokio time API → no `start_paused`.
- `resume_enrollment_rebinds_identity_on_id_mismatch` — `identity.service_id()`
  changes from source to target; `service.json` on disk reflects target after the
  call. Skip the cert-issuance leg (mock controller responds with a pre-baked
  cert) to keep the test deterministic.
- `resume_enrollment_noop_when_ids_match` — no extra write to `service.json`.
- `resume_enrollment_rejects_nil_service_id` — controller sends
  `Approved { service_id: Uuid::nil() }`; SDK errors out without touching
  `service.json`.

Integration test (`crates/core/integration-tests`, gated on Docker, `--ignored`):

- `service_merge_rekey_end_to_end` — fresh enrollment, operator merge, re-key
  resume, cert issued under target id, subsequent mTLS connect succeeds. This
  test asserts only the second-connect succeeds with the target id; it does NOT
  paused-time the cert-issuance path (real cert clocks too brittle under
  `start_paused`).

Frontend test:

- Component test for `MergeServiceDialog` filtering `is_embedded` from both
  source and target option lists.

## Quality Gates

Per `docs/development/quality-gates.md` and the standards snapshot:

```text
cargo fmt --all
cargo check --no-default-features --features db-sqlite
cargo check --all-features
cargo clippy --all-targets --no-default-features --features db-sqlite
cargo clippy --all-targets --all-features
cargo test --all-features
cargo deny check
markdownlint --config .markdownlint.json '**/*.md'
# DB/migration changes
cargo test -p uptrakit-integration-tests --test database -- --ignored
# Enrollment / wire / service changes (Docker required)
docker build -f docker/Dockerfile.test -t uptrakit-test:latest .
cargo test -p uptrakit-integration-tests -- --ignored
# Frontend
cd frontend && npm run lint && npm run format:check && npm run check && npm run test && npm run build
```

## Documentation Deliverables

- **`docs/adr/0020-service-merge-redirect.md`** (new, REQUIRED) — records the choice
  of an explicit redirect table over (a) inferring rebind from
  `RevocationReason::ServiceMerged` and (b) rewriting the `services.id` primary key
  on merge. Trade-offs: per-merge row vs. inferred state, fresh ID vs. preserving
  Agent state for an existing approved Service. Hard to reverse (PK semantics +
  WS auth fallback would have to change), surprising without context, real
  alternatives existed → satisfies ADR criteria.
- **`CONTEXT.md`** — add **Service Merge Redirect** glossary entry: "Row mapping a
  deactivated Service's UUID to the merge-target Service's UUID so an Agent that
  enrolled against the deactivated row can reconnect and re-key without operator
  intervention."
- **`asyncapi.yaml`** — short note on `ApprovedPayload.service_id` semantics on
  resume (already in §6).
- **`docs/development/coding-standards.md`** — no new pattern surfaced.
- **`README.md`** — no surface-area change (operator-visible behaviour is a bug fix,
  not a new capability).
- **`docs/api/services-operations.md`** — note 400 `service.embedded_*` reason
  codes on `POST /services/{id}/merge`. The OpenAPI utoipa annotation on the route
  must list the new 400 reason codes.

## Rollout / Rollback

- **Controller-before-agent**: new controller can serve old (pre-rekey-aware) agents
  unchanged — old agents continue to receive the same `service_id` they enrolled
  with on the fresh-enrollment path, and old agents that have already certified
  authenticate via mTLS (rekey path is bearer-only). Old SDKs hitting a merged
  target row will stay broken until the agent binary is upgraded; operator
  recovery is to re-enroll the affected host (wipe `service.json` + re-bootstrap).
- **Rollback**: removing the redirect fallback from `lookup_by_secret` reverts
  controller behaviour to pre-fix state. The `service_merge_redirect` table can be
  left in place — rows are harmless without the fallback path. Agents that already
  rebound their `service.json` remain bound to the target identity and do NOT need
  to re-enroll; their `service.json` now matches the target row's identity directly.
  Forward migration is one-way at the agent layer.
- **Pre-existing secret-clearing**: `identity.save_certificate` (`identity.rs`,
  test `certificate_save_clears_enrollment_secret` line 910) already zeroes the
  enrollment secret on disk once the client certificate is installed. The post-rekey
  cert issuance therefore narrows the bearer-leak window to the same envelope as
  fresh enrollment — no extra rotation is added here.

## Invariants Preserved

- `BEGIN IMMEDIATE` read-then-write semantics (CLAUDE.md SQLite Transaction Rule 1)
  — embedded guard, redirect collapse, and redirect insert all execute inside the
  existing `BEGIN IMMEDIATE` block.
- `#[non_exhaustive]` rules — `ServiceQueryError` is internal; no `#[non_exhaustive]`
  required. No new wire enum variants → no wire-side `Other(String)` work.
- Tenant isolation — redirect rows never cross tenants (constructed only within
  `TenantDb`-scoped `merge_service`); WS lookup re-acquires tenant context from the
  resolved target row.
- `parking_lot` / async-lock rule — no new locks introduced.
- No raw SQL — all changes go through SeaORM builders.
- Defence-in-depth on bearer narrowing — redirect fallback only triggers when a
  `?service_id=` hint was supplied; bare bearer attempts retain the original
  cross-service-collision protection.
- mTLS authenticated path (`ConnectionType::Authenticated`) is untouched —
  re-key only affects the bearer / resume flow used before a cert is issued.

## Out of Scope / Deferred

- **Operator-driven force re-key without merge** — no UI to manually rebind an
  agent identity; merge is the only trigger.
- **Notification of running Agents on merge** — Agents discover the rebind on next
  reconnect. A push notification ("Agent X has been merged, restart pending") is
  deferred.
- **Garbage collection of stale redirect rows** — kept forever per Q4 decision; a
  retention policy can be added later if the table grows unexpectedly.
- **Cross-tenant merge** — out of scope; tenant scope is preserved by current
  `TenantDb` constraints.
- **System-service redirects** — system services do not participate (embedded ban
  covers the analogous case for tenant-scoped embedded services).

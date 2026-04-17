# Semantic Audit Logs Design

## Goal

Replace the current request-shaped audit logging with mutation-first semantic
audit logging so Uptrakit records what security-relevant action happened, who
performed it, what it targeted, and how it finished.

## Scope

### V1

- redefine `audit_logs` and `system_audit_logs` as semantic action logs
- unify database-backed audit logs and ad hoc `security_audit` tracing into one
  canonical audit event pipeline
- keep the existing product surface names:
  - Audit Logs UI page
  - `GET /api/v1/audit-logs`
  - `GET /api/v1/system-audit-logs`
- focus on mutation-first coverage:
  - state-changing API handlers
  - auth outcomes
  - WebSocket and service lifecycle mutations
  - selected runtime and scheduler mutations
- use light structured details payloads, not generic entity diffs
- document the new audit model for operators, developers, and AI agents

### Explicitly out of scope for V1

- generic old/new field diffs
- full entity snapshots
- exhaustive coverage of every mutation path in the codebase
- transport-wide automatic inference of semantic actions
- multi-event workflow correlation graphs
- retaining or migrating currently stored request-style audit rows

## Current Codebase Baseline

### Current audit subsystem

The existing audit log subsystem is request-centric:

- `crates/shared/audit-log/src/entry.rs` defines `AuditEntry` as authenticated
  HTTP request metadata:
  - method
  - path
  - route pattern
  - status
  - client IP
  - user agent
  - duration
- `crates/ui/web-api/src/middleware/audit_log.rs` emits one audit entry per
  authenticated request after `require_auth`
- `crates/shared/audit-log/src/backend.rs` fans those entries out to:
  - `audit_logs`
  - `system_audit_logs`
  - journald

This model captures transport activity, not business actions.

### Separate security log stream

The codebase also has a second, fragmented mechanism:

- many mutation paths emit `tracing::warn!(target: "security_audit", ...)`
- representative call sites already exist in:
  - `crates/ui/web-api/src/routes/plugin_configs.rs`
  - `crates/ui/web-api/src/routes/plugin_type_settings.rs`
  - `crates/ui/web-api/src/routes/services.rs`
  - `crates/ui/web-api/src/surface_proxy.rs`
  - `crates/core/agent-runtime/src/lib.rs`
  - `crates/core/agent-ssh-runtime/src/lib.rs`

Those events often capture better semantics than the current audit tables, but
they are not queryable through the Audit Logs product surface and are not part
of one canonical contract.

### Product surface today

The current Audit Logs UI, API, CLI, and docs are all request-shaped:

- filters use HTTP method and status
- table rows describe requests, not actions
- docs explicitly say Uptrakit records authenticated HTTP requests

This means the current product contract and the current storage model must both
change together.

## Design Principles

- **Semantic over transport**: audit records describe the action that occurred,
  not the request envelope that carried it.
- **One pipeline**: database rows, journald output, and former `security_audit`
  events all come from the same canonical audit event.
- **Mutation-first**: V1 focuses on state changes and security-relevant
  outcomes, not routine reads.
- **Emit where meaning is known**: handlers and runtime components emit audit
  events explicitly after the outcome is known.
- **Safe details only**: V1 carries curated, intentionally small detail payloads.
- **No backward-compatibility burden**: old request-style audit rows can be
  dropped.

## Proposed Design

### Canonical audit event model

Redefine `uptrakit_audit_log::AuditEntry` as a semantic action record with the
following V1 contract:

- `id`
- `tenant_id: Option<Uuid>`
- `occurred_at`
- `actor_type`
- `actor_id: Option<Uuid>`
- `actor_display: Option<String>`
- `action_type: AuditActionType`
- `target_type: Option<String>`
- `target_id: Option<String>`
- `target_display: Option<String>`
- `outcome`
- `details_json: Option<serde_json::Value>`
- `request_id: Option<String>`

Required meaning:

- `tenant_id = Some(...)` routes to tenant audit logs
- `tenant_id = None` routes to system audit logs
- `action_type` is the stable machine-readable primary identifier and is stored
  as a string at DB/API boundaries
- `occurred_at` is always produced and stored in UTC across all backends
- `action_type` names the operation only and never encodes the result
- `outcome` captures whether the action succeeded, was denied, failed
  validation, failed during execution, or partially completed
- `details_json` is present only for explicitly safe, purpose-built metadata
- `request_id` preserves the current request-id behavior, including
  client-provided non-UUID `x-request-id` values

Required V1 bounds:

- `action_type`: max 128 bytes
- `request_id`: max 255 bytes
- `actor_display`: max 255 bytes
- `target_display`: max 255 bytes
- serialized `details_json`: max 4096 bytes

The emitter API must enforce these bounds for all locally produced events
before dispatch, and the controller must re-validate them on forwarded
`ServiceMessage::AuditEvent` ingress.

On local bound violations, the emitter returns a validation error to the
producer, the audit event is dropped, and the producer logs a warning. The
business operation itself continues.

### Actor model

The request-era actor enum is too narrow. V1 should support:

- `user`
- `api_token`
- `oidc`
- `service`
- `system`

Notes:

- `actor_id` is nullable because `system` actions may have no persisted actor
- `actor_display` provides a human-readable label for UI and journald output
- service-originated mutations should use the actual service UUID when known
- actions performed by a user who authenticated through OIDC use
  `actor_type = user`; `actor_type = oidc` is reserved for OIDC flow events
  before or during account/session establishment
- pre-auth OIDC flow events use `actor_type = oidc`, `actor_id = None`, and
  `actor_display = None`

### Outcome model

V1 should use a small closed outcome set:

- `success`
- `denied`
- `validation_failed`
- `failed`
- `partial`

This keeps filtering simple and makes docs, UI badges, and journald output
consistent.

V1 intentionally keeps both enums closed. Adding a new `actor_type` or
`outcome` value is a deliberate contract change that requires coordinated API
and client updates rather than an `Other(String)` catch-all.

API DTOs may still expose `actor_type` and `outcome` as strings at the REST
boundary to stay compatible with the project's wire-safety conventions, even if
controller-side code uses closed internal enums.

Expected V1 use of `partial`:

- `software.batch_update.triggered` when some requested targets are accepted
  and some are rejected during the same batch-trigger operation

### Action taxonomy

`action_type` values must be stable, namespaced, and intentionally boring.
Storage and API transport still use strings, but producer code must not use raw
string literals. V1 should add a single canonical registry in
`uptrakit-audit-log` as a typed `AuditActionType` newtype plus constructor
constants.

`AuditActionType` must be a newtype wrapper over a normalized string value, not
a closed enum, so persisted rows and future action additions remain DB-round-
trip safe.

Raw ad hoc `action_type` literals should be forbidden outside tests, fixtures,
and migrations.

Format:

- `<domain>.<verb>`
- `<domain>.<subdomain>.<verb>` when the extra segment adds clarity

Examples:

- `plugin_config.create`
- `plugin_config.update`
- `plugin_type_settings.upsert`
- `service.update_freeze.enable`
- `service.merge`
- `auth.login`
- `auth.api_token.authenticate`
- `auth.oidc.callback`
- `notification_channel.test`
- `software.batch_update.triggered`
- `software.batch_update.started`
- `system.scheduler.audit_log_cleanup`

Do not encode free-form prose or outcome state in `action_type`.
Examples:

- `auth.login` + `outcome = success`
- `auth.login` + `outcome = denied`
- `auth.oidc.callback` + `outcome = failed`

### Target model

Targets should be described independently from the actor:

- `target_type`: stable kind such as `plugin_config`, `service`,
  `notification_channel`, `user`, `global_setting`
- `target_id`: text so it can carry UUIDs or other stable identifiers without
  schema churn
- `target_display`: short human label such as config name or service name

`target_*` is optional because some audit entries describe actor-centric events
like login success/failure.

### Details payload

`details_json` is intentionally lightweight in V1. It should hold small, safe,
curated metadata that improves investigation without turning the log into a
generic snapshot system.

Representative V1 payloads:

- plugin config create/update:
  - `plugin_type`
  - `config_name`
  - `contains_command_fields`
- update freeze:
  - `enabled`
  - `reason_present`
- auth failure:
  - `reason_code`
- notification channel test:
  - `channel_type`
- batch update trigger:
  - `host_count`
  - `item_count`

V1 must not put secrets, credentials, request bodies, or generic old/new
snapshots into `details_json`.

## Storage And Backends

### Tables

Keep the existing two-table product split:

- `audit_logs`
- `system_audit_logs`

But replace their columns with the semantic action contract.

Recommended tenant-scoped columns:

- `id`
- `tenant_id`
- `occurred_at`
- `actor_type`
- `actor_id`
- `actor_display`
- `action_type`
- `target_type`
- `target_id`
- `target_display`
- `outcome`
- `details_json`
- `request_id`

`system_audit_logs` uses the same shape minus `tenant_id`.

Keep the existing compliance-oriented rule:

- no foreign key from `audit_logs.tenant_id` to `tenants`

Required V1 indexes:

- tenant table:
  - `(tenant_id, occurred_at desc)`
  - `(tenant_id, action_type, occurred_at desc)`
  - `(tenant_id, actor_type, occurred_at desc)`
  - `(tenant_id, outcome, occurred_at desc)`
  - `(tenant_id, target_type, occurred_at desc)`
  - `(tenant_id, target_id, occurred_at desc)`
  - `(tenant_id, actor_id, occurred_at desc)`
- system table:
  - `(occurred_at desc)`
  - `(action_type, occurred_at desc)`
  - `(actor_type, occurred_at desc)`
  - `(outcome, occurred_at desc)`
  - `(target_type, occurred_at desc)`
  - `(target_id, occurred_at desc)`
  - `(actor_id, occurred_at desc)`

`actor_display` is display-only in V1 and does not get a search index.

### Backends

Preserve the current backend model:

- `DatabaseBackend`
- `JournaldBackend`
- `MultiplexBackend`
- `NoopBackend`

But all of them now receive semantic action events.

This means journald becomes a first-class audit backend for the same canonical
event, not a separate `security_audit` side channel.

`JournaldBackend` must use a fixed field contract mirroring the canonical
`AuditEntry` keys so journald consumers do not depend on unstable ad hoc field
names.

Required journald fields:

- `audit_id`
- `tenant_id`
- `occurred_at`
- `actor_type`
- `actor_id`
- `actor_display`
- `action_type`
- `target_type`
- `target_id`
- `target_display`
- `outcome`
- `request_id`
- `details_json`

### Tracing unification

Retire `target: "security_audit"` as an independent producer contract.

After this change:

- code emits semantic `AuditEntry` values via the audit subsystem
- those events may still be written to journald through `JournaldBackend`
- no new mutation path should emit ad hoc `security_audit` tracing instead of a
  canonical audit event

## Emission Model

### Core rule

Emit audit events where business meaning is known, not in transport middleware.

### HTTP handlers

Mutation handlers should emit audit events after the action outcome is known.

Examples:

- plugin config created
- plugin config update rejected by dangerous-command policy
- plugin type settings deleted
- service merge denied
- notification rule deleted
- global setting updated

The event should include:

- actor context from the authenticated user or token
- scope from the tenant/system route
- action type
- target metadata
- final outcome
- optional light details

### Auth flows

Audit emission must no longer depend on `require_auth`, because important auth
events happen before or outside successful authentication.

Required auth coverage in V1:

- login succeeded
- login failed
- API token or JWT rejected
- token refresh failure where applicable
- OIDC initiation and callback outcomes
- device authorization approval/denial outcomes

### WebSocket and service flows

Non-HTTP service operations must emit directly through the same audit API.

V1 coverage should include representative mutations such as:

- service approval or enrollment completion
- service merge
- update freeze enable/disable
- batch update trigger acceptance/rejection
- certificate or lifecycle mutations already treated as security-relevant

For standalone services and agents, V1 must add an additive
`ServiceMessage::AuditEvent(AuditEventPayload)` wire message so service-side
canonical audit events can be forwarded to the controller.

`AuditEventPayload` fields:

- `action_type: AuditActionType`
- `tenant_id: Option<Uuid>`
- `target_type: Option<String>`
- `target_id: Option<String>`
- `target_display: Option<String>`
- `outcome: String`
- `details_json: Option<serde_json::Value>`
- `request_id: Option<String>`

Rules:

- the payload carries action, target, request-correlation, and detail data, but
  does not own persisted `id` or `occurred_at`
- the controller assigns persisted `id` and `occurred_at` when accepting the
  forwarded event
- V1 uses controller-ingestion time as canonical `occurred_at` for forwarded
  events; preserving original service-side emission timestamps across offline
  replay is intentionally deferred
- for forwarded `ServiceMessage::AuditEvent` messages, actor attribution is
  determined by the action registry, not by service input:
  - `service.*` actions are attributed as `actor_type = service`
  - `software.update.started` and `software.batch_update.started` are
    attributed as `actor_type = service`
  - `system.service.*` actions are attributed as `actor_type = system`
- for `service.*` actions, the controller overwrites:
  - `actor_id` from the authenticated service identity
  - `actor_display` from controller-known service metadata
- for `system.service.*` actions, the controller sets:
  - `actor_id = None`
  - `actor_display` to a controller-generated system label
  - the authenticated service identity in target or detail fields as needed for
    investigation
- for tenant-bound services, the controller overwrites `tenant_id` from the
  authenticated connection context
- for tenant-agnostic system services, the payload may include `tenant_id` only
  for tenant-targeted actions; the controller must validate that scope against
  the action's referenced entities before accepting it, otherwise the event is
  rejected
- `action_type` must validate against the canonical action registry
- `outcome` is a string at the wire boundary and must validate against the
  controller's closed internal outcome set
- `target_type`, `target_id`, `target_display`, and `request_id` must be
  rejected if they exceed their canonical size bounds
- `target_*` and `details_json` are accepted only after those size checks and
  any action-specific validation the controller can enforce
- forwarded `details_json` must be rejected if its serialized size exceeds the
  4096-byte canonical bound
- invalid forwarded events must be dropped with a warning and optional
  controller-side error response to the originating service, but they must not
  disconnect the service connection
- old services remain compatible because the new wire message is additive
- old controllers may ignore the new message during mixed service/controller
  rollouts, so controller upgrade comes first

### Runtime and scheduler components

Runtime components that currently write `security_audit` warnings should be
converted into canonical audit producers.

The V1 conversion set is not subjective. It must include these currently
existing runtime-denial and runtime-mutation call sites:

- `crates/core/agent-runtime/src/lib.rs`
  - machine-id mismatch rejection
  - update rejected because execution is frozen
  - update rejected because cooldown is active
  - update freeze enabled via controller
  - update freeze disabled via controller
- `crates/core/agent-ssh-runtime/src/lib.rs`
  - host update rejected because execution is frozen
  - host update rejected because cooldown is active
  - update freeze enabled via remote command
  - update freeze disabled via remote command

Scheduler and internal maintenance tasks that perform audited mutations should
emit with:

- `actor_type = system`
- `actor_id = None`
- a stable `action_type`

## Audit Emitter API

Add a small explicit emission API in `uptrakit-audit-log` so producers do not
hand-roll event construction at every call site.

The API should provide:

- a canonical `AuditEntry` type
- helper constructors or builders for:
  - actor context
  - tenant vs system scope
  - target metadata
  - outcomes
  - optional details
- a dispatcher-facing emitter object suitable for:
  - Axum handlers
  - service/WebSocket handlers
  - runtime components
  - scheduler executors

Injection contract:

- controller HTTP and WebSocket producers access the emitter through
  `AppState`, matching the existing state-sharing pattern
- scheduler executors receive the emitter through their executor context or
  constructor wiring
- agent and service runtimes receive a cloned emitter handle in their runtime
  context structs and use it both for local journald output and
  controller-forwarded `ServiceMessage::AuditEvent` emission when connected

The design goal is not hidden magic. It is explicit, low-friction, consistent
event creation.

## Product Surface Changes

### API

Keep the endpoint names:

- `GET /api/v1/audit-logs`
- `GET /api/v1/system-audit-logs`

But replace the response and query model with action-centric fields.

Recommended filters:

- `action_type`
- `actor_type`
- `outcome`
- `target_type`
- `target_id`
- `actor_id`
- `from`
- `to`

V1 does not provide free-text filtering by `actor_display`. Rows with
`actor_id = None` are filtered via `actor_type = system` or the action/target
dimensions.

Remove request-era filters like HTTP method and status from the primary API
contract.

### UI

Keep the Audit Logs page and tenant/system split, but update the main table to
show actions instead of requests.

Recommended columns:

- occurred at
- action
- target
- outcome
- actor
- scope
- details summary

Recommended filter controls:

- action type
- actor type
- outcome
- target type
- actor id
- time range

`actor_display` is a rendered column, not a filterable search field in V1.

### CLI

Keep the existing CLI area but update the output model from request fields to
semantic action fields.

Human output should optimize for:

- action
- actor
- target
- outcome
- timestamp

JSON output should mirror the new API contract exactly.

## Migration Strategy

Backward compatibility is intentionally not required.

Recommended migration posture:

- replace the existing audit log table schemas with the semantic action schema
- delete old request-shaped audit rows instead of transforming them
- remove `audit_log` HTTP middleware from semantic audit event production in V1;
  it may remain only for non-audit transport helpers, but it must not write
  audit rows
- update queries, API DTOs, CLI output, UI filters, and docs in the same change

Rollout contract:

- this is a coordinated controller cutover, not a rolling controller schema
  migration
- all controller instances must be stopped or drained before the schema
  migration runs, because old controller binaries write the request-era column
  set
- only new controller binaries may start after the migration completes
- service rollout may remain rolling because `ServiceMessage::AuditEvent` is
  additive; old services simply will not emit forwarded semantic audit events
  until upgraded
- controller upgrade is required before upgrading services that emit
  `ServiceMessage::AuditEvent`
- the migration and schema representation must remain compatible with both
  PostgreSQL and the workspace's SQLite quality-gate path; V1 must not rely on
  Postgres-only DDL for the semantic audit tables

This keeps the system conceptually clean and avoids hybrid rows with mixed
request/action semantics.

## Required V1 Audited-Action Catalog

V1 should define an explicit audited-action catalog so coverage is deliberate
instead of accidental.

For this spec, the lists below are the required V1 action set for the scoped
coverage areas. They are not illustrative examples. If implementation adds or
removes a V1 audited action, the catalog must be updated in the same change.

### Category 1: Auth outcomes

Code areas:

- `crates/ui/web-api/src/routes/auth.rs`
- `crates/ui/web-api/src/routes/oidc_auth.rs`
- `crates/ui/web-api/src/routes/device_auth.rs`
- `crates/ui/web-api/src/middleware/require_auth.rs`

Required V1 actions:

- `auth.login`
- `auth.api_token.authenticate`
- `auth.jwt.authenticate`
- `auth.token_refresh`
- `auth.oidc.authorize`
- `auth.oidc.callback`
- `auth.device.approve`
- `auth.device.deny`

Semantics:

- `auth.jwt.authenticate` is failure-only in V1: it covers JWT rejections in
  `require_auth` and similar middleware-side token validation failures, and it
  does not emit on successful token validation for every authenticated request
- `auth.api_token.authenticate` is also failure-only in V1 for the same
  high-volume per-request reason
- `auth.token_refresh` emits on both success and failure because refresh is a
  discrete low-volume mutation, not a per-request validation path
- `auth.device.approve` and `auth.device.deny` are attributed to the approving
  or denying admin user, not to `system`

### Category 2: User and token management

Code areas:

- user-management routes and queries
- API token management routes and queries

Required V1 actions:

- `user.create`
- `user.update`
- `user.delete`
- `api_token.create`
- `api_token.revoke`
- `enrollment_token.create`
- `enrollment_token.revoke`

### Category 3: Global and tenant settings mutations

Code areas:

- global settings routes
- tenant settings or policy routes

Required V1 actions:

- `global_setting.update`
- `tenant_setting.update`
- `oidc_provider.create`
- `oidc_provider.update`
- `oidc_provider.delete`

### Category 4: Plugin config and plugin type settings mutations

Code areas:

- `crates/ui/web-api/src/routes/plugin_configs.rs`
- `crates/ui/web-api/src/routes/plugin_type_settings.rs`
- `crates/ui/web-api/src/surface_proxy.rs`

Required V1 actions:

- `plugin_config.create`
- `plugin_config.update`
- `plugin_config.delete`
- `plugin_type_settings.upsert`
- `plugin_type_settings.delete`

### Category 5: Notification mutations and test actions

Code areas:

- notification channel routes
- notification rule routes
- callback and test-action routes where relevant

Required V1 actions:

- `notification_channel.create`
- `notification_channel.update`
- `notification_channel.delete`
- `notification_channel.test`
- `notification_rule.create`
- `notification_rule.update`
- `notification_rule.delete`

### Category 6: Service lifecycle mutations

Code areas:

- `crates/ui/web-api/src/routes/services.rs`
- `crates/ui/web-api/src/routes/service_ws/handler/`

Required V1 actions:

- `service.approve`
- `service.reject`
- `service.merge`
- `service.certificate.issue`
- `service.certificate.renew`
- `service.update_freeze.enable`
- `service.update_freeze.disable`
- `service.enrollment.completed`
- `service.deactivate`

### Category 7: Software and update-trigger actions

Code areas:

- software-item routes and queries
- update trigger and batch dispatch queries
- `crates/ui/web-api/src/routes/autodiscovery.rs`
- `crates/ui/web-api-queries/src/queries/autodiscovery/ignore_rules.rs`

Required V1 actions:

- `software.update.triggered`
- `software.update.started`
- `software.batch_update.triggered`
- `software.batch_update.started`
- `software.ignore.create`
- `software.ignore.delete`

Semantics:

- `.triggered` means the controller accepted and dispatched or queued the
  operation
- `.started` means the responsible service reported that execution actually
  began
- `outcome = partial` on `software.batch_update.triggered` means one aggregate
  batch-trigger event was emitted for the request and the request both accepted
  and rejected targets; V1 does not emit per-target audit rows at trigger time

### Category 8: System-initiated audited mutations

Code areas:

- scheduler executors
- runtime components currently using `security_audit`

Required V1 actions:

- `system.scheduler.audit_log_cleanup`
- `system.service.update_freeze.apply`
- `system.service.machine_id.validate`
- `system.service.update_gate`

Semantics:

- `system.service.update_freeze.apply` is emitted by the runtime when a
  previously accepted freeze-enable or freeze-disable command is actually
  applied on the service side
- `system.service.update_freeze.apply` uses
  `details_json = {"enabled": true|false}`
- `system.service.machine_id.validate` uses `outcome = denied` when a
  service/runtime message is rejected because the machine identity does not
  match expectations
- `system.service.machine_id.validate` does not emit on successful validation
- `system.service.update_gate` uses:
  - `outcome = denied`
  - `details_json = {"reason": "frozen" | "cooldown"}`
  for runtime-side self-protection rejections

The catalog should live in the development docs and be updated whenever a new
audited mutation class is introduced.

## Reliability And Failure Semantics

- emission remains fire-and-forget from the caller perspective
- backend failures must not fail the user operation
- DB and journald may both receive the same canonical event
- dispatcher/backpressure behavior may remain as-is unless a separate design
  changes it
- audit events are at-least-once from a workflow perspective; retries or late
  failures may create multiple rows for one user-visible operation

V1 does not attempt storage-level deduplication. UI, CLI, and operators must
not treat audit rows as exactly-once facts.

The main reliability risk in V1 is missing producer coverage, not storage
plumbing. Missing coverage must be treated as a product bug.

## Testing Strategy

### Unit tests

- `AuditEntry` serialization and validation helpers
- actor and outcome enum conversions
- database backend persistence mapping
- journald backend field mapping

### Query and API tests

- list endpoints return action-shaped rows
- filters work for:
  - `action_type`
  - `actor_type`
  - `actor_id`
  - `outcome`
  - `target_type`
  - `target_id`
  - time range

### Producer tests

Representative route and service tests must assert that audited mutations write
the expected canonical event.

Minimum representative coverage:

- plugin config create or update
- plugin type settings mutation
- service update freeze
- one auth success path
- one auth failure path outside `require_auth`
- one explicit `ServiceMessage::AuditEvent` forwarding path
- one additional non-HTTP producer path
- one secret-bearing mutation asserting `details_json` excludes known secret
  values

### Migration tests

- schema migration produces usable semantic audit tables
- request-era rows are not preserved
- fresh writes through the new emitter succeed after migration

### Guardrail tests

Add a lightweight guard so new code does not reintroduce ad hoc
`target: "security_audit"` mutation logging as a separate audit mechanism.

V1 should implement this as CI grep/script checks so the guardrails exist
outside normal code-review discipline.

Required guardrails:

- fail CI on any `target: "security_audit"` mutation logging that is not listed
  in one explicit repo allowlist file for temporary migrations
- fail CI on raw `action_type` string literals outside the canonical registry,
  tests, fixtures, and migrations

## Documentation Deliverables

### Operator-facing docs

Update:

- `docs/end-user/audit-logs.md`
- `docs/security/audit-logs.md`
- `docs/api/audit-logs.md`

These docs must explain:

- audit logs are semantic action logs, not HTTP request logs
- the tenant vs system split
- what is captured in V1
- what is intentionally excluded
- how outcomes and action types should be interpreted by operators

### Developer and agent-facing docs

Rewrite or update:

- `docs/development/audit-logs.md`
- `AGENTS.md`
- `ARCHITECTURE.md`

These docs must explain:

- the canonical `AuditEntry` contract
- where audit events must be emitted
- the required audited-action catalog
- the ban on ad hoc `security_audit` as a parallel audit mechanism
- the canonical `action_type` registry pattern
- the `ServiceMessage::AuditEvent` propagation path for standalone services and
  agents

Minimum content by file:

- `docs/development/audit-logs.md`
  - producer rules
  - emitter injection pattern
  - audited-action catalog
  - test expectations
- `AGENTS.md`
  - audit subsystem summary rewritten around semantic actions
  - action-type registry rule
  - explicit note that request middleware is no longer the canonical source
- `ARCHITECTURE.md`
  - end-to-end flow from producer to dispatcher to DB/journald
  - controller/service propagation path for service-originated audit events

## V2 / Intentionally Deferred

These items are intentionally deferred and should be called out explicitly in
every design and implementation document for this feature:

- generic field-level old/new diffs
- generic entity snapshots
- exhaustive mutation coverage across the entire codebase
- automatic semantic interception across all transport layers
- richer request-to-event and multi-step workflow correlation
- broader audit analytics and search beyond the current product surface

The point of V1 is to establish one clean semantic audit contract and unify the
existing fragmented logging story without dragging diff engines and generic
change capture into the first rollout.

## Recommended Implementation Shape

At a high level, the implementation should proceed in this order:

1. redefine the audit entry type and DB schema
2. update backends and journald output to the new event shape
3. add the explicit emitter API
4. migrate every producer required by the V1 audited-action catalog away from
   middleware-only and `security_audit` tracing
5. update list queries, API DTOs, CLI output, and UI
6. rewrite operator and developer docs
7. add guardrails so the unified model stays unified

This preserves one conceptual migration path:

- first redefine what an audit log is
- then make producers emit it
- then make every consumer render it

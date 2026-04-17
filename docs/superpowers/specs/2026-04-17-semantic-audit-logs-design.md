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
- `action_type: String`
- `target_type: Option<String>`
- `target_id: Option<String>`
- `target_display: Option<String>`
- `outcome`
- `details_json: Option<serde_json::Value>`
- `request_id: Option<Uuid>`

Required meaning:

- `tenant_id = Some(...)` routes to tenant audit logs
- `tenant_id = None` routes to system audit logs
- `action_type` is the stable machine-readable primary identifier
- `outcome` captures whether the action succeeded, was denied, failed
  validation, failed during execution, or partially completed
- `details_json` is present only for explicitly safe, purpose-built metadata

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

### Outcome model

V1 should use a small closed outcome set:

- `success`
- `denied`
- `validation_failed`
- `failed`
- `partial`

This keeps filtering simple and makes docs, UI badges, and journald output
consistent.

### Action taxonomy

`action_type` values must be stable, namespaced, and intentionally boring.
Format:

- `<domain>.<verb>`
- `<domain>.<subdomain>.<verb>` when the extra segment adds clarity

Examples:

- `plugin_config.create`
- `plugin_config.update`
- `plugin_type_settings.upsert`
- `service.update_freeze.enable`
- `service.merge`
- `auth.login.succeeded`
- `auth.login.failed`
- `auth.oidc.callback_failed`
- `notification_channel.test`
- `software.batch_update.started`
- `system.scheduler.audit_log_cleanup`

Do not encode free-form prose in `action_type`.

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

### Backends

Preserve the current backend model:

- `DatabaseBackend`
- `JournaldBackend`
- `MultiplexBackend`
- `NoopBackend`

But all of them now receive semantic action events.

This means journald becomes a first-class audit backend for the same canonical
event, not a separate `security_audit` side channel.

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
- OIDC initiation and callback outcomes where feasible in V1
- device authorization approval/denial outcomes where feasible in V1

### WebSocket and service flows

Non-HTTP service operations must emit directly through the same audit API.

V1 coverage should include representative mutations such as:

- service approval or enrollment completion
- service merge
- update freeze enable/disable
- batch update trigger acceptance/rejection
- certificate or lifecycle mutations already treated as security-relevant

### Runtime and scheduler components

Runtime components that currently write `security_audit` warnings should be
converted into canonical audit producers where the event is truly an auditable
mutation or denial.

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
- `actor_id`
- `from`
- `to`

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
- actor
- time range

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
- remove or sharply reduce the role of `audit_log` HTTP middleware
- update queries, API DTOs, CLI output, UI filters, and docs in the same change

This keeps the system conceptually clean and avoids hybrid rows with mixed
request/action semantics.

## Required V1 Audited-Action Catalog

V1 should define an explicit audited-action catalog so coverage is deliberate
instead of accidental.

### Category 1: Auth outcomes

Code areas:

- `crates/ui/web-api/src/routes/auth.rs`
- `crates/ui/web-api/src/routes/oidc_auth.rs`
- `crates/ui/web-api/src/routes/device_auth.rs`
- `crates/ui/web-api/src/middleware/require_auth.rs`

Examples:

- `auth.login.succeeded`
- `auth.login.failed`
- `auth.api_token.rejected`
- `auth.oidc.callback_failed`

### Category 2: User and token management

Code areas:

- user-management routes and queries
- API token management routes and queries

Examples:

- `user.create`
- `user.update`
- `user.delete`
- `api_token.create`
- `api_token.revoke`

### Category 3: Global and tenant settings mutations

Code areas:

- global settings routes
- tenant settings or policy routes

Examples:

- `global_setting.update`
- `tenant_setting.update`

### Category 4: Plugin config and plugin type settings mutations

Code areas:

- `crates/ui/web-api/src/routes/plugin_configs.rs`
- `crates/ui/web-api/src/routes/plugin_type_settings.rs`
- `crates/ui/web-api/src/surface_proxy.rs`

Examples:

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

Examples:

- `notification_channel.create`
- `notification_channel.update`
- `notification_channel.test`
- `notification_rule.delete`

### Category 6: Service lifecycle mutations

Code areas:

- `crates/ui/web-api/src/routes/services.rs`
- `crates/ui/web-api/src/routes/service_ws/handler/`

Examples:

- `service.approve`
- `service.merge`
- `service.update_freeze.enable`
- `service.update_freeze.disable`
- `service.enrollment.completed`

### Category 7: Software and update-trigger actions

Code areas:

- software-item routes and queries
- update trigger and batch dispatch queries

Examples:

- `software.update.triggered`
- `software.batch_update.started`
- `software.ignore.created`

### Category 8: System-initiated audited mutations

Code areas:

- scheduler executors
- runtime components currently using `security_audit`

Examples:

- `system.scheduler.audit_log_cleanup`
- `system.service.update_freeze.applied`

The catalog should live in the development docs and be updated whenever a new
audited mutation class is introduced.

## Reliability And Failure Semantics

- emission remains fire-and-forget from the caller perspective
- backend failures must not fail the user operation
- DB and journald may both receive the same canonical event
- dispatcher/backpressure behavior may remain as-is unless a separate design
  changes it

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
  - `outcome`
  - `target_type`
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
- one non-HTTP producer path

### Migration tests

- schema migration produces usable semantic audit tables
- request-era rows are not preserved
- fresh writes through the new emitter succeed after migration

### Guardrail tests

Add a lightweight guard so new code does not reintroduce ad hoc
`target: "security_audit"` mutation logging as a separate audit mechanism.

This can be a CI grep/script check or a targeted test, whichever best fits the
existing repo conventions.

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
4. migrate representative high-value producers away from middleware-only and
   `security_audit` tracing
5. update list queries, API DTOs, CLI output, and UI
6. rewrite operator and developer docs
7. add guardrails so the unified model stays unified

This preserves one conceptual migration path:

- first redefine what an audit log is
- then make producers emit it
- then make every consumer render it

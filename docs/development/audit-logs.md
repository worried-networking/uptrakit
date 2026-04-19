# Audit Log Subsystem

Development guide for the semantic, mutation-first audit-log model.

## Model

Uptrakit audit logs are semantic events, not request transcripts.

- Producers emit `AuditEntry` for meaningful state changes (for example
  `plugin_config.create`, `host.update`, `service_config.store`).
- Each entry includes `action_type`, `outcome`, actor metadata, optional target metadata,
  optional `details_json`, optional `request_id`, and `occurred_at`.
- `AuditActionType` is a validated canonical action registry in
  `crates/shared/audit-log/src/action_type.rs`.
- `AuditOutcome` is separate from action name (`success`, `denied`,
  `validation_failed`, `failed`, `partial`).

## Pipeline

```text
Mutation / runtime event producer
            │
            ▼
AuditEntry::builder(...) + AuditEmitter::emit_best_effort(...)
            │
            ▼
AuditLogDispatcher (unbounded channel, fire-and-forget)
            │
            ▼
DatabaseBackend and/or JournaldBackend (optional MultiplexBackend fan-out)
```

Notes:

- `crates/ui/web-api/src/middleware/audit_log.rs` is intentionally a no-op after semantic cutover.
- `audit_context_from_parts()` in that middleware still provides request context helpers.
- Do not add new `target: "security_audit"` producers; use semantic emitters.

## Core crates

| Path | Purpose |
| --- | --- |
| `crates/shared/audit-log/` | `AuditActionType`, `AuditEntry`, `AuditOutcome`, `AuditEmitter`, `RuntimeAuditEmitter`, dispatcher, backends |
| `crates/shared/db/src/entity/audit_log.rs` | Tenant-scoped semantic rows |
| `crates/shared/db/src/entity/system_audit_log.rs` | System-scoped semantic rows |
| `crates/ui/web-api/src/routes/*` | HTTP mutation producers |
| `crates/ui/web-api/src/routes/service_ws/handler/mod.rs` | Service-forwarded audit event ingestion + scope validation |
| `crates/shared/scheduler-engine/src/executors/audit_log_cleanup.rs` | Retention cleanup + runtime audit emission |

## Persistence model

`DatabaseBackend` routes by scope:

- `tenant_id = Some(...)` -> `audit_logs`
- `tenant_id = None` -> `system_audit_logs`

Both tables now store semantic fields:

- `actor_type`, `actor_id`, `actor_display`
- `action_type`
- `target_type`, `target_id`, `target_display`
- `outcome`
- `details_json` (optional JSON)
- `request_id` (optional string)
- `occurred_at`

`audit_logs.tenant_id` intentionally has no FK, so audit history survives tenant deletion.

## Runtime / service-forwarded events

Services can forward `AuditEventPayload` over wire messages. The controller re-validates:

- `action_type` must be a valid canonical action.
- `outcome` must be a supported enum value.
- Scope rules are enforced (tenant-only, service-bound, system-only allowlists).
- Length and JSON constraints are enforced before persistence.

Invalid or out-of-scope events are dropped with warning logs.

## CLI and backend configuration

Controller flags are unchanged:

- `--audit-log-backend` (`db`, `journald`, `none`, repeatable)
- `--audit-log-db-url` (optional separate DB)
- `--audit-log-filter` (`all`, `mutations`, `none`)

`journald` backend emits structured events to `target: "uptrakit_audit"`.

## V2 Deferrals

- `AuditLogFilter` / `audit_log.filter` is still wired in config/state, but semantic
  producer-side enforcement is not centralized yet.
- `AuditLogRetentionDays` (`audit_log.retention_days`) exists as a setting key, but cleanup
  currently uses the global retention policy (default 90 days).

## Adding a new audit event

1. Reuse an existing `AuditActionType` constant, or add a new canonical constant.
2. Build an `AuditEntry` with explicit scope (`tenant_scope` or `system_scope`).
3. Set actor metadata (`actor(...)`, `actor_service(...)`, or `actor_system()`).
4. Set `outcome` explicitly and include minimal, non-secret `details_json` if needed.
5. Emit via `state.audit_emitter.emit_best_effort(entry)`.
6. If service-forwarded, update the allowlist/scope checks in service WS handler.

## Read surface

Read APIs and CLI now expose semantic filters:

- `actor_type`, `action_type`, `outcome`, `target_type`, `target_id`
- `from`, `to`, `actor_id`, plus pagination

See:

- [Audit Logs API Reference](../api/audit-logs.md)
- [Audit Logs Security](../security/audit-logs.md)
- [Audit Logs End-User Guide](../end-user/audit-logs.md)

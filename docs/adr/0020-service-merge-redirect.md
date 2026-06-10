# ADR 0020 — Service Merge Redirect

Date: 2026-06-09
Status: Accepted

## Context

`merge_service` consolidates a freshly-enrolled pending Service into an existing approved Service
by moving the source's `enrollment_secret_hash` + identity fields onto the target row and
deactivating the source. The running Agent on the host still holds the source UUID in its
`service.json` and reconnects with `?service_id=<source>` + Bearer secret. With the source row
deactivated and the bearer-lookup narrowed by `service_id`, the auth path failed with
`401 Unauthorized` even though the secret was correct on the target row.

Three approaches were considered:

1. **Inferring rebind from `RevocationReason::ServiceMerged`** — the merge transaction already
   revokes both source and target certs with this reason. The WS auth path could detect a merge
   by walking the cert table for the supplied source_id and following the reason. This couples
   authentication to cert-lifecycle bookkeeping that may legitimately change in the future and
   exposes private cert state to the bearer flow.
2. **Rewriting `services.id` (primary key replacement)** — keep the source row's PK; copy the
   target row's identity onto it; preserve every FK by virtue of the PK reuse. This preserves
   Agent state but rewrites identity semantics for the rest of the system; every downstream
   consumer (audit logs, certs, host links) would need to be re-keyed in the same txn. PK
   rewrite is also harder to reason about under FK cascade interactions and difficult to
   reverse.
3. **Explicit redirect table** _(chosen)_ — a thin mapping from old (deactivated) source UUID to
   current target UUID, written in the same `BEGIN IMMEDIATE` txn as the merge. The auth path
   consults it on a hint mismatch; the SDK reads the canonical id back from the `ApprovedPayload`
   and overwrites `service.json` on disk.

## Decision

Adopt approach (3). Schema:

| Column          | Type        | Notes                               |
| --------------- | ----------- | ----------------------------------- |
| `source_id`     | UUID PK     | the Agent's persisted service_id    |
| `target_id`     | UUID FK     | FK→`services(id)` ON DELETE CASCADE |
| `redirected_at` | TIMESTAMPTZ |                                     |

The table is tenant-scoped via FK, not via a denormalized `tenant_id` column, matching the
`service_host` join-table pattern. Cross-tenant rows are impossible by construction (the merge
query runs inside a `TenantDb`).

The WS auth fallback triggers ONLY when `?service_id=<hint>` is supplied in the connection URL,
preserving the existing cross-service-collision defence-in-depth for the hint-less bearer path.

Chains cannot pre-exist: `merge_service` requires `source.status == Pending` and
`target.status == Approved`, and a previously-deactivated row satisfies neither. A
debug-assertion query inside the merge txn enforces this invariant; a violation surfaces as
`ServiceQueryError::RedirectChainInvariantViolated` (HTTP 500) rather than silent state rewrite.

The Agent reads the canonical id from `ControllerMessage::Approved(ApprovedPayload {
service_id })` — no new wire-message variant, no protocol-version bump.

## Consequences

- Adds one tiny table; rows are retained forever (UUID payload is negligible; supports Agents that
  disconnect for long periods and reappear after merges).
- Rollback path: removing the WS auth fallback reverts behaviour to pre-fix; redirect rows become
  inert. Agents that already rebound `service.json` remain bound to the target id.
- Embedded Services are explicitly excluded from merge at both query and route layers
  (`ServiceQueryError::TargetEmbedded` / `SourceEmbedded`, HTTP 400 with reason codes
  `service.embedded_target` / `service.embedded_source`).
- A new audit action `AUTH_SERVICE_REKEY_RESOLVED` surfaces every successful re-key for on-call
  discoverability; the existing bearer-miss audit is enriched with `redirect_checked` /
  `redirect_present` flags so a stuck-rebind state is greppable.

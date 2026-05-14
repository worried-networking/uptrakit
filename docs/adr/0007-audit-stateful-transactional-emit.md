# 0007 — Audit Stateful Transactional Emit

**Date:** 2026-05-14
**Status:** Accepted

## Context

V1 introduced a single `emit_best_effort` path for all audit events: entries were enqueued to an
async dispatcher, written to the database and the journald multiplex in the background, and
failures were logged but never propagated to the caller. This gave every audit action the same
fire-and-forget semantics regardless of whether the action described an entity transition or a
workflow fact.

V2 must guarantee that stateful audit rows — rows recording before/after snapshots of a mutated
entity — commit or roll back atomically with the mutation they describe. An async best-effort path
cannot provide that guarantee: if the process crashes between the mutation commit and the deferred
audit write, the mutation is durable but the audit row is absent. For security-relevant entity
transitions (plugin config updates, service approvals, user modifications) the resulting gap is not
a degraded-but-acceptable outcome — it undermines the audit trail's trustworthiness as evidence.

V2 also introduces a second coverage requirement: every state-changing site in the codebase must
have an explicit catalog decision (audited or justified-skip). The async V1 design treated missing
coverage as a code-review observation; V2 makes it a build failure. This changes the
failure-detection point from human review to CI.

## Decision

1. **Compile-time action classification.** Every registered audit action is classified at
   definition time as either `Stateful` (the action describes an entity transition; a before/after
   snapshot pair is required) or `Event` (the action describes a discrete workflow fact; snapshots
   are forbidden). This classification is enforced by a typestate builder:
   `AuditEntry<K>` where `K` is a phantom type (`Stateful` or `Event`). `Builder<Stateful>`
   exposes `.before(&impl AuditView)` and `.after(&impl AuditView)`; `.build()` is only callable
   once both snapshots are supplied. `Builder<Event>` does not expose snapshot methods. Calling
   `.before()` on an event builder, calling `.build()` on an incomplete stateful builder, or
   passing an `AuditEntry<Event>` to `emit_stateful` (or vice versa) are all compile errors, not
   runtime panics. A `CHECK` constraint on the database table enforces the same invariant as a
   second line of defence after the typestate builder.

2. **Stateful emission inside the mutation transaction.** `AuditEmitter::emit_stateful(&tx, entry)`
   writes the audit row directly onto the caller's `DatabaseTransaction`. The DB row is canonical:
   the mutation and its audit row share a single `COMMIT`. No separate write, no deferred flush, no
   window for the row to be absent after the mutation is durable. The journald multiplex for
   stateful events uses a caller-supplied `AuditCommitHook` obtained from
   `AuditEmitter::commit_hook()`. The hook buffers the entry in memory; the caller calls
   `hook.flush_after_commit()` immediately after `tx.commit().await?` succeeds. If the transaction
   rolls back, or if the caller returns an error before reaching `tx.commit()`, the hook is dropped
   without flushing — no journald entry is emitted for a rolled-back mutation. Journald flush
   failures are fire-and-forget (consistent with the rest of the journald path); the DB row stands.

3. **Event emission keeps V1's fire-and-forget path.** `AuditEmitter::emit_event(entry)` enqueues
   the entry onto the V1 async dispatcher unchanged. Both the database backend and the journald
   backend handle event entries asynchronously. Failures are logged at `error!` and never
   propagated to the caller.

### Operator-observable reliability difference

| Emission path   | DB row guarantee                                                        | Journald guarantee                                              |
| --------------- | ----------------------------------------------------------------------- | --------------------------------------------------------------- |
| `emit_stateful` | Committed-or-not-present. Mutation and audit row share one transaction. | At-least-once after commit; may be delayed or missing on crash. |
| `emit_event`    | Best-effort. May be delayed or missing on crash.                        | At-least-once; same as V1.                                      |

This difference is documented in `docs/security/audit-logs.md` so operators can reason about
evidence coverage when reviewing audit trails after an incident.

### Per-transaction latency budget

The target is that a stateful audit INSERT adds at most 5 ms to the P99 duration of a typical
mutation transaction on Postgres. The INSERT is a single row write (≤32 KB) inside an already-open
transaction; no additional round-trip to acquire a lock or begin a new transaction is required.

On SQLite the budget concern is different: mutation transactions are already gated on a
`BEGIN IMMEDIATE` write lock. The audit INSERT is folded into the same lock window, so the
additional latency is bounded by the single serialized write path that SQLite already imposes.
The budget applies to the Postgres path where concurrent writers are possible.

A criterion benchmark in `crates/shared/audit-log/benches/` measures the per-transaction overhead
of `emit_stateful` against a baseline mutation-only transaction. The measured P99 regression is
recorded in the implementation plan notes. A regression beyond 5 ms is a signal to investigate
INSERT path optimizations (batching, async pre-serialization); it is not a CI gate but is reviewed
during the landing.

## Consequences

**Positive:**

- Stateful audit rows are committed-or-not-present. There is no window in which a mutation is
  durable but its audit row is absent. This property is unconditional, not dependent on process
  uptime after commit.
- Event actions retain zero-overhead fire-and-forget semantics. The latency impact of V2 falls
  only on stateful paths, which are the ones that require the guarantee.
- The typestate builder makes misclassification a compile error, not a production incident. A
  caller that omits a snapshot for a stateful action cannot produce a binary.

**Negative:**

- Every V1 producer call site (~100 sites) must be reshaped to use the new API. The action-kind
  classification and constructor naming convention are chosen to make the migration mechanical, but
  the change is coordinated and non-incremental — no green-CI midway state exists during the
  migration step.
- `BEGIN IMMEDIATE` becomes load-bearing for snapshot capture, not merely a best-practice
  recommendation. Any stateful transaction that reads the before-snapshot must be opened with
  `begin_with_options(TransactionOptions { sqlite_transaction_mode: Some(SqliteTransactionMode::Immediate), ..Default::default() })`
  to prevent `SQLITE_BUSY_SNAPSHOT` (error code 5, bypasses `busy_timeout`) on SQLite when another
  writer commits between the snapshot SELECT and the audit INSERT. This rule already exists in
  `docs/development/coding-standards.md`; V2 makes non-compliance a correctness bug rather than a
  performance hint.
- V1 audit rows are dropped at migration time. The V2 migration drops the V1 audit tables and
  creates V2 tables; no transformation or back-population is performed. `docs/security/audit-logs.md`
  documents an optional pre-migration export step so deployments with compliance requirements can
  preserve V1 history out-of-band before running the migration.
- Wire-forwarded Stateful action types are rejected at controller ingress. Services may only
  forward Event-class actions. Any service-originated workflow that triggers a stateful audit row
  must do so through the controller-side handler that owns the authoritative entity state. This
  is a trust-boundary rule, not a convenience trade-off: service-supplied snapshots could be
  fabricated by a compromised service.

## Alternatives Considered

**All-async (V1 default).** Rejected. An async path cannot give the committed-or-not-present
guarantee that stateful audit requires. The gap between mutation commit and async audit write is
unbounded and non-recoverable on crash.

**SeaORM lifecycle hook auto-emit.** Rejected. A model-layer hook has no access to the HTTP
handler's outcome, the authenticated actor identity, the request correlation ID, or the
action-type classification that gives an audit row its semantic meaning. Automatic interception
at the ORM layer produces rows with missing context; the catalog gate is the substitute for
coverage enforcement.

**Optional snapshots everywhere.** Rejected. A single builder shape with optional snapshot
fields makes the presence of snapshots a runtime configuration choice, not a compile-time
contract. The V2 coverage promise — "every stateful action has a snapshot pair" — is hollow if
the producer can silently omit snapshots. The typestate builder makes the promise enforceable.

**Per-action-kind retention.** Deferred to V3. Compliance-driven retention distinctions
(e.g. keeping stateful rows for 7 years and event rows for 90 days) require a per-action-kind
policy engine that is out of scope for the V2 data-model release.

## References

- Spec: `docs/superpowers/specs/2026-05-11-semantic-audit-logs-v2-design.md`
- V1 spec: `docs/superpowers/specs/2026-04-17-semantic-audit-logs-design.md`
- Coding standard for `BEGIN IMMEDIATE`: `docs/development/coding-standards.md`
- Related ADR: `docs/adr/0006-instance-scoped-plugins.md`

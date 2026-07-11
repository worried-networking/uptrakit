# SQLite Transaction-Mode Conformance + CI Gate — Design

**Date:** 2026-07-11
**Status:** Draft
**Source:** `.superpowers/audit-2026-07-11.md` — HIGH "append_update_output_if_owned uses BEGIN DEFERRED for a
read-then-write transaction on the hottest streaming path" + MEDIUM "deactivate_system_service: read-then-write
transaction opened with BEGIN DEFERRED" + MEDIUM "execute_merge_software_items: read-then-write transaction
opened with BEGIN DEFERRED" + MEDIUM "Protection policy upserts are non-transactional read-then-write (TOCTOU
insert race)". The audit's executive summary explicitly calls this a recurring pattern "worth a CI grep gate,
not just point fixes" — this spec is both.

## Problem

The project's documented SQLite rule: any transaction that reads rows before writing must open with
`begin_with_options(TransactionOptions { sqlite_transaction_mode: Some(SqliteTransactionMode::Immediate), … })`;
`BEGIN DEFERRED` (plain `db.begin()`) fails with `SQLITE_BUSY_SNAPSHOT` — bypassing `busy_timeout`, erroring in
~2ms — whenever another connection commits between the read and the write. Four confirmed violations remain
(the fifth, proxmox `apply_match`, is covered by the tenant-isolation spec):

1. **`append_update_output_if_owned`** (`update_batches/dispatch.rs:781`, HIGH): plain `begin()` → SELECT the
   `update_history` row for the output budget → UPDATE + INSERT. Runs once per streamed output line from every
   connected agent, concurrent with heartbeats and status transitions — the exact multi-writer scenario the rule
   exists for. Failure drops the output line (`TriggerUpdateError::Database`). Same file already does it right
   in `maybe_complete_batch`.
2. **`deactivate_system_service`** (`system_services.rs:320`, MEDIUM): plain `begin()` → SELECT the service →
   UPDATE + cert revocation. Concurrent commit leaves the service active with an operator-facing error. The
   sibling `batch_deactivate_system_services` is fine **for this rule** — its SELECT happens *outside* the
   transaction, so its transaction is write-only and cannot hit BUSY_SNAPSHOT (the read-outside-tx staleness it
   carries instead is a different, benign-here class: the UPDATE is a keyed state transition).
3. **`execute_merge_software_items`** (`software_items/merge.rs:758`, MEDIUM): plain `begin()` →
   `build_merge_plan` multi-SELECT → link moves/deletes/soft-deletes. Merge runs on a live controller with
   constant background writes; intermittent 500s under load.
4. **Proxmox protection-policy upserts** (`policy_store.rs:177`, MEDIUM): `upsert_global_default` /
   `upsert_item_override` SELECT-then-INSERT/UPDATE as separate **auto-commit** statements — no transaction at
   all; two concurrent saves both see "no row", second INSERT hits the composite-PK violation, surfaced to the
   UI as a raw DB error. Sibling stores (`protection_store.upsert_audit`, scaling_store) wrap the identical
   pattern in BEGIN IMMEDIATE.

Point-fixing recurrences of a documented rule four audits in a row is process failure; the durable half of this
spec is the gate.

## Approach

### 1. Convert the four sites

Sites 1–3: replace `begin()` with `begin_immediate()` (§3) + the crate-standard one-line comment ("BEGIN
IMMEDIATE prevents SQLITE_BUSY_SNAPSHOT …"), matching what their in-file/in-crate siblings do today with the
long form. Site 4: wrap both upserts in a BEGIN IMMEDIATE transaction matching `scaling_store`'s shape (the
audit's alternative — SeaORM `on_conflict` upsert — is also acceptable if the composite-PK expression is clean;
implementer picks whichever reads closer to the sibling stores, consistency being the point). No behavior
change beyond eliminating the failure mode; no signature changes.

Perf note on site 1, stated because it is the hot path: Immediate holds the write lock across the SELECT too,
so the per-output-line append serializes on a slightly wider lock window than DEFERRED — but DEFERRED wasn't
buying concurrency here, it was buying dropped lines under exactly the multi-writer load that matters. Correct
trade. If the append path ever shows up in profiling, the real lever is batching multiple output lines per
transaction, not the tx mode.

### 2. Ban plain `begin()` outright: all-Immediate + clippy gate

**Direction change during review** (two earlier gate designs killed): an inline-marker gate had a false premise
(the raw-SQL comment convention is documentation-only — no `ci/verify_*` script enforces it) and no in-tree
precedent; the allowlist-file replacement had a structural flaw the contrarian pass nailed — a `path|regex`
row asserting "write-only tx" is keyed to the *call site* but the invariant lives in the *body*, so the row
survives exactly the refactor it exists to catch (someone moves a SELECT inside the tx; the gate stays green;
BUSY_SNAPSHOT ships pre-approved). Detached approval is false assurance.

Meanwhile the fast-path being protected is worth ~nothing: the coding-standards doc itself states Immediate's
only cost on a write-only transaction is acquiring the write lock at `BEGIN` instead of at the first write —
microseconds later for these bodies — and it is a no-op on Postgres. Protecting that with a bespoke script, an
allowlist file, and a per-site classification judgment is negative-sum. Therefore:

- **Every** `.begin()` call in the workspace (~31 production + ~9 test sites; the conversion is mechanical, no
  read-vs-write classification needed — Immediate is always correct) converts to the new
  `uptrakit_shared_db::begin_immediate(conn)` helper.
- `clippy.toml` gains a `disallowed-methods` entry for `sea_orm::TransactionTrait::begin` (the file already
  carries a disallowed-methods entry with rationale — house mechanism, zero new tooling; verify the trait-path
  form resolves during implementation, including any `begin()` on `DatabaseTransaction` savepoint call sites —
  convert or escape those the same way). The escape hatch for a future genuinely-hot write-only loop is
  `#[expect(clippy::disallowed_methods, reason = "write-only tx: …")]` — co-located with the code, moves with
  the body, visible to every reviewer touching it, and consistent with the workspace's expect-with-reason rule.
  No such site exists today; the hatch starts unused.
- This replaces the entire verify-script apparatus: no `ci/verify_sqlite_tx_mode.sh`, no allowlist file, no
  classification pass, no pre-push wiring, no quality-gates.md command addition — clippy already runs
  everywhere the gate would have.

### 3. `begin_immediate()` helper in shared-db

The mechanism of §2, and independently justified: the workspace has ~91 `begin_with_options` sites, every one
hand-rolling the identical
`TransactionOptions { sqlite_transaction_mode: Some(SqliteTransactionMode::Immediate), ..Default::default() }`
block — ceremony that caused the drift being fixed. Placement verified: both web-api-queries and the proxmox
plugin depend on shared-db directly (neither can reach the other). Migrating the ~91 existing
`begin_with_options` sites to the helper is **not** in scope (they're compliant; mechanical churn, separate
cleanup if ever wanted — the clippy ban targets bare `begin` only).

**Rejected alternatives:** inline `// tx: write-only` markers (false premise + no precedent, above);
allowlist-file bash gate (detached-approval flaw, above); keeping the write-only DEFERRED fast-path at all
(microseconds of value against a recurring data-affecting bug class).

## Tests

1. Each converted site keeps its existing behavior tests; where a site had none covering the transactional
   path (policy upserts concurrent-save), add the double-upsert test: two sequential upserts (insert-then-update
   path) succeed and produce one row — the TOCTOU itself is not deterministically testable without fault
   injection; the transaction + sibling-consistency is the verifiable part.
2. Gate validation: after the conversion, `cargo clippy --all-targets --all-features` over the tree is the
   standing test; prove the ban fires once by hand (a synthetic bare `begin()` must fail clippy) before
   landing.
3. No `start_paused`, no tokio-time APIs (DB-backed tests — snapshot rule).

## Documentation deliverables

- `docs/development/coding-standards.md` "Database Query Patterns" section: rewrite the BEGIN IMMEDIATE rule
  around `begin_immediate()` + the clippy ban + the `#[expect]` escape-hatch contract (reason string must state
  either a write-only rationale or "savepoint on an existing transaction" — the two legitimate escapes); retire
  the "write-only transactions may use plain begin()" allowance text.
- `docs/development/quality-gates.md` + AGENTS.md quick-start: no new command to add (clippy already listed);
  while in the area, fix the pre-existing drift found during review — `ci/verify_no_inline_query_params.sh`
  runs in CI and pre-push but is missing from both enumerations.
- Inline comments at converted sites (crate-standard BUSY_SNAPSHOT comment).
- No new ADR: enforcement of an existing documented rule.

## Out of scope / deferred

- proxmox `apply_match` DEFERRED fix (covered by
  `docs/superpowers/specs/2026-07-11-proxmox-match-tenant-isolation-design.md`).
- Postgres transaction-mode tuning (Immediate is a SQLite-only pragma concern; no-op elsewhere).
- Retry-on-BUSY_SNAPSHOT machinery (the rule prevents the error class; retrying it is treating the symptom).
- Migrating the ~91 already-compliant `begin_with_options` sites to the `begin_immediate()` helper (mechanical
  churn; separate cleanup if ever wanted).

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

1. **`append_update_output_if_owned`** (`crates/ui/web-api-queries/src/queries/update_batches/dispatch.rs:781`,
   HIGH): plain `begin()` → SELECT the
   `update_history` row for the output budget → UPDATE + INSERT. Runs once per streamed output line from every
   connected agent, concurrent with heartbeats and status transitions — the exact multi-writer scenario the rule
   exists for. Failure drops the output line (`TriggerUpdateError::Database`). Same file already does it right
   in `maybe_complete_batch`.
2. **`deactivate_system_service`** (`crates/ui/web-api-queries/src/queries/system_services.rs:320`, MEDIUM):
   plain `begin()` → SELECT the service →
   UPDATE + cert revocation. Concurrent commit leaves the service active with an operator-facing error. The
   sibling `batch_deactivate_system_services` is fine **for this rule** — its SELECT happens *outside* the
   transaction, so its transaction is write-only and cannot hit BUSY_SNAPSHOT (the read-outside-tx staleness it
   carries instead is a different, benign-here class: the UPDATE is a keyed state transition).
3. **`execute_merge_software_items`** (`crates/ui/web-api-queries/src/queries/software_items/merge.rs:758`,
   MEDIUM): plain `begin()` →
   `build_merge_plan` multi-SELECT → link moves/deletes/soft-deletes. Merge runs on a live controller with
   constant background writes; intermittent 500s under load.
4. **Proxmox protection-policy upserts** (`crates/plugins/infrastructure/proxmox/src/policy_store.rs:177`,
   MEDIUM): `upsert_global_default` /
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

Stated generally (contrarian pass): under all-Immediate, the 5s `busy_timeout` is the budget for the read phase
of the longest read-then-write transaction — e.g. site 3's `build_merge_plan` multi-SELECT now holds the write
lock while it reads, and concurrent writers queue behind it. On this deployment's table sizes that phase is
milliseconds; risk to monitor, not a flaw. If it ever matters, the cheap fix is hoisting `build_merge_plan`'s
pure reads outside the transaction (staleness re-validated by the writes' own keyed filters — the same argument
§Problem already makes for `batch_deactivate_system_services`), not reverting the tx mode.

Commit shape (contrarian round 3): two commits in one PR — (1) new crate + helper + canary + clippy bans +
mechanical conversion of all ~148 sites; (2) the four behavior fixes as their own reviewable commit (sites 1–3
are absorbed into commit 1 automatically; site 4's proxmox upsert transaction-wrapping is genuinely new code
and belongs in commit 2). Keeps the semantic fixes visible inside the mechanical diff and independently
revertable.

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

- **Every** `.begin()` call in the workspace converts to the new `uptrakit_shared_db::begin_immediate(conn)`
  helper. Recount at review time (2026-08-11): **49 call sites — 28 production + 13 test + 8 in migration
  code** (`crates/shared/db/src/migration/`, agent-ssh-runtime's migration, proxmox `controller_migration.rs`).
  Migration sites are in scope and convert identically. Mechanism note (corrected twice during contrarian
  review — stated from in-tree facts only): the shared-db runner wraps the entire migrator in one top-level
  `db.begin()` at `crates/shared/db/src/migration/mod.rs:301/309/439`, and agent-ssh-runtime's runner calls
  `Migrator::up(db, None)` on a bare connection with **no** wrapping transaction, making
  `m20260308_000003_ssh_host_uuid_columns.rs:124` a fourth **top-level** begin — those four sites carry the
  read-then-write invariant. The remaining in-migration `begin()`s (proxmox `controller_migration.rs`) execute
  as savepoints under the shared-db wrapper, where the Immediate mode is silently ignored by SeaORM ("the mode
  only applies to the top-level BEGIN", sea-orm 2.0.1 `transaction.rs`) — converting them is
  cosmetic-but-harmless. Caution for reuse: the agent-ssh migration is also re-exported via
  `service_migrations()` (`handler.rs:172`), so the same migration can run wrapped *or* unwrapped depending on
  caller. The safety claim rests on the strong premise, not on classifying contexts: **Immediate is correct in
  both** — at top level it is the fix; on a savepoint it is a no-op. Hence blanket conversion, no
  classification needed.
- The ~99 compliant `begin_with_options` sites migrate to the helper **in the same change** (contrarian
  finding, round 2): with only bare `begin` banned, the surviving legal shape is
  `begin_with_options(TransactionOptions::default())` — and `TransactionOptions` derives `Default` with
  `sqlite_transaction_mode: None`, i.e. **DEFERRED**. A developer copying one of 99 in-tree blocks and dropping
  the mode line re-creates the exact bug shape with a green clippy, less visually suspicious than `begin()`
  ever was. The gate must close the successor shape too, and it cannot while 99 legitimate callers exist. The
  sites are byte-identical 4-line blocks modulo receiver (contrarian round 3 parsed all 99 programmatically:
  0 carry any other option; no Postgres-divergent shape exists) — the migration is sed-able. Any future site
  needing a non-default option keeps `begin_with_options` under `#[expect]` with a reason naming the option.
  Expected sed collateral: ~40 files lose their last `TransactionOptions`/`SqliteTransactionMode` import —
  self-correcting (`warnings = "deny"` makes unused imports errors); plan a second mechanical pass, don't read
  the error wall as a failed migration.
- `clippy.toml` gains `disallowed-methods` entries for **all five** `sea_orm::TransactionTrait`
  transaction-opening methods — `begin`, `begin_with_config`, `begin_with_options`, `transaction`,
  `transaction_with_config` — plus the two inherent `DatabaseTransaction::transaction_async` /
  `transaction_with_config_async` paths (contrarian round 3: `begin_with_config(None, None)` and the
  closure-style `transaction()` — the shape sea-orm's own docs lead with — all open DEFERRED and would pass a
  two-method ban green; in-tree usage of the five extra methods is **zero**, so this costs clippy.toml lines
  and no migration). The file already carries a disallowed-methods entry with rationale — house mechanism,
  zero new tooling. Result: exactly one way to open a transaction in the workspace, `begin_immediate()`.
  Enforceability verified empirically during review against sea-orm 2.0.1's
  actual trait shape (`#[async_trait]`, per-impl override): the ban fires on concrete
  `DatabaseConnection`/`DatabaseTransaction` calls, through `T: TransactionTrait` generic bounds, on savepoint
  call sites (`DatabaseTransaction` implements `TransactionTrait`, so nested `txn.begin()` resolves through the
  same disallowed path), and on UFCS/fn-pointer forms. The helper's own internal `begin_with_options` call
  carries the one legitimate `#[expect]`.
- **Inert-gate canary** (contrarian finding, empirically verified): if a future sea-orm upgrade renames or
  relocates the banned methods, the unresolvable `disallowed-methods` path degrades to a *config warning* that
  `-D warnings` does **not** deny — clippy exits 0 and the ban silently disarms. Counter: a `#[cfg(test)]` fn
  in the helper's crate calling bare `.begin()` (and `begin_with_options`) under
  `#[expect(clippy::disallowed_methods, reason = "canary: proves the TransactionTrait ban still resolves")]`.
  When a ban goes inert the expectation is unfulfilled, and `unfulfilled_lint_expectations = "deny"` (already
  in workspace lints) turns that into a hard compile error — verified to hold even against clippy's
  `allow-invalid = true`. The fn is never called, so it additionally needs
  `#[expect(dead_code, reason = "canary is never called")]` stacked on top (verified: without it,
  `warnings = "deny"` rejects the uncalled fn). Zero new tooling. Canary coverage: `begin`,
  `begin_with_options`, `begin_with_config` (same receiver shape, cheap). Explicit trade: a *trait relocation*
  is caught by any single canary entry; a per-method *rename* is caught only by a canary on that method — the
  closure-style methods are left uncovered by choice, not oversight.
- The escape hatch for a future genuinely-hot write-only loop is
  `#[expect(clippy::disallowed_methods, reason = "write-only tx: …")]` — co-located with the code, moves with
  the body, visible to every reviewer touching it, and consistent with the workspace's expect-with-reason rule.
  No such production site exists today; the helper internals and the canary are the hatch's sole, deliberate
  first users.
- This replaces the entire verify-script apparatus: no `ci/verify_sqlite_tx_mode.sh`, no allowlist file, no
  classification pass, no pre-push wiring, no quality-gates.md command addition — clippy already runs
  everywhere the gate would have.

### 3. `begin_immediate()` helper in a new `uptrakit-db-tx` leaf crate, re-exported from shared-db

The mechanism of §2, and independently justified: the workspace has ~99 `begin_with_options` sites, every one
hand-rolling the identical
`TransactionOptions { sqlite_transaction_mode: Some(SqliteTransactionMode::Immediate), ..Default::default() }`
block — ceremony that caused the drift being fixed. Signature: generic over the connection —
`pub async fn begin_immediate<C: TransactionTrait<Transaction = DatabaseTransaction>>(conn: &C) ->
Result<DatabaseTransaction, DbErr>` — the associated-type bound is required (an unconstrained
`C: TransactionTrait` fails E0308: `C::Transaction` doesn't unify with the declared return type; both
`DatabaseConnection` and `DatabaseTransaction` set `type Transaction = DatabaseTransaction` in sea-orm 2.0.1,
so top-level and savepoint call sites both compile through the same helper; verified in a throwaway crate
during review). Free function matches the crate's existing idiom, cf. `db_error::is_unique_constraint_violation`.

**Placement** (settled over three contrarian rounds): a new leaf crate `uptrakit-db-tx` (single dependency:
`sea-orm`), registered in `[workspace.dependencies]` first per dependency policy, re-exported from shared-db
alongside the existing tenant-db re-exports (`crates/shared/db/src/lib.rs:10`) so shared-db consumers see no
call-site difference. The crate manifest **must** carry `[lints] workspace = true` (cf.
`crates/shared/db/Cargo.toml:41`) — without it, `unfulfilled_lint_expectations = "deny"` never reaches the
canary and the inert-gate detector is itself silently inert; this is an explicit line item, not an implicit
convention. Honest caveat: the crate exists to serve **one consumer today** — agent-ssh-runtime's 3 sites;
every other affected crate could take the helper from shared-db directly. Stated so it isn't re-litigated.
Why not shared-db itself: the ban covers all 49 sites, and `agent-ssh-runtime` (3 sites,
own separate SQLite DB) depends on neither shared-db nor tenant-db — pulling shared-db (controller entities,
tenant-db, crypto) into an agent-side crate for one function is an architecture-boundary regression. Why not
tenant-db (round-1 proposal, killed in round 2): its charter is "tenant-scoped database access primitives";
overloading it puts `TenantDb`/`TenantScoped` — a multi-tenant controller abstraction — into the import
namespace of an agent-side crate with no tenants, inviting exactly the misuse the boundary rules exist to
prevent. Why not inlining `begin_with_options` at the 3 agent-ssh sites: §2 now bans that method too.

**Rejected alternatives:** inline `// tx: write-only` markers (false premise + no precedent, above);
allowlist-file bash gate (detached-approval flaw, above); keeping the write-only DEFERRED fast-path at all
(microseconds of value against a recurring data-affecting bug class); banning bare `begin` only while
deferring the 99-site migration (leaves the `begin_with_options(Default)` DEFERRED hole open permanently —
§2); placing the helper in shared-db or tenant-db (§3 boundary reasoning).

## Tests

1. Each converted site keeps its existing behavior tests; where a site had none covering the transactional
   path (policy upserts concurrent-save), add the double-upsert test: two sequential upserts (insert-then-update
   path) succeed and produce one row — the TOCTOU itself is not deterministically testable without fault
   injection; the transaction + sibling-consistency is the verifiable part.
2. Gate validation: after the conversion, `cargo clippy --all-targets --all-features` over the tree is the
   standing test; the §2 canary (`#[expect]`-guarded `begin()` + `begin_with_options()` calls in a
   `#[cfg(test)]` fn) is the *permanent* proof both bans still resolve — an inert ban fails the build via
   `unfulfilled_lint_expectations`. Requires `--all-targets` (already the gate command). Before landing, run
   the single experiment that validates ban resolution, canary wiring, and lint inheritance at once: delete one
   `disallowed-methods` entry locally and confirm `cargo clippy --all-targets` **fails** (via the canary's
   unfulfilled expectation); restore it and confirm a synthetic unguarded call fails clippy directly.
3. No `start_paused`, no tokio-time APIs (DB-backed tests — snapshot rule).

## Documentation deliverables

- `docs/development/coding-standards.md` "Database Query Patterns" section: rewrite the BEGIN IMMEDIATE rule
  around `begin_immediate()` as the sole way to open a transaction + the two clippy bans (`begin`,
  `begin_with_options`) + the `#[expect]` escape-hatch contract (reason string must state either a write-only
  rationale or name the non-default `TransactionOptions` field the site needs — the two legitimate escapes
  besides the helper internals and canary); retire the "write-only transactions may use plain begin()"
  allowance text.
- AGENTS.md codebase-layout tree: one line for the new `uptrakit-db-tx` crate (per the "update when a crate is
  added" maintenance rule).
- `docs/development/quality-gates.md` + AGENTS.md quick-start: no new command to add (clippy already listed);
  while in the area, fix the pre-existing drift found during review — `ci/verify_no_inline_query_params.sh`
  runs in CI and pre-push but is absent from `quality-gates.md` entirely and from the AGENTS.md quick-start
  command block (AGENTS.md mentions it only in the OpenAPI-params rule prose).
- Inline comments at converted sites (crate-standard BUSY_SNAPSHOT comment).
- No new ADR: enforcement of an existing documented rule.

## Out of scope / deferred

- proxmox `apply_match` DEFERRED fix (covered by
  `docs/superpowers/specs/2026-07-11-proxmox-match-tenant-isolation-design.md`).
- Postgres transaction-mode tuning (Immediate is a SQLite-only pragma concern; no-op elsewhere).
- Retry-on-BUSY_SNAPSHOT machinery (the rule prevents the error class; retrying it is treating the symptom).

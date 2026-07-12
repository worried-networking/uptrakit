# Developer-Docs Drift Sweep — Design

Merge five verified docs-drift findings from `.superpowers/audit-2026-07-11.md` into one realignment pass.
All five are the same problem class — a developer guide **duplicates a code/CI fact and has drifted from
it** — and they live in a cluster of `docs/development/` files distinct from the `plugin-guidelines.md`
sweep already specced ([`2026-07-12-plugin-guidelines-realignment-design.md`](./2026-07-12-plugin-guidelines-realignment-design.md),
which foreshadowed this as the follow-up "developer-docs drift sweep"). One coherent editing pass is more
maintainable than five piecemeal ones.

## Problem

1. **`quality-gates.md` full-suite + hook tables drift (MEDIUM, audit L837; doc L19–L62).** The documented
   pre-commit table, pre-push table, and "Full quality gate suite" list omit gates that CI and the hooks
   actually enforce, so a contributor who follows the doc verbatim still fails CI.
2. **`quality-gates.md` pre-push table lags `.husky/pre-push` (MEDIUM, audit L974; doc L19).** Same file,
   overlapping content — the pre-push tier is missing roughly half of what the hook runs, and the
   "across all 28 crates" count is wrong. **Merged with finding 1** (one file, one edit).
3. **`scheduler-engine.md` documents a deleted crate (MEDIUM, audit L921; doc L3).** The guide points at
   `crates/shared/scheduler-engine/` (does not exist); the code is `crates/core/scheduler-runtime/`. Module
   tree, executor inventory, `TaskExecutor` signature, `SchedulerConfig` fields, and a function name are all
   stale.
4. **`error-handling.md` + `testing.md` example paths point at deleted files (MEDIUM, audit L937; doc
   error-handling L210, testing L219).** Two `error-handling.md` "real example" paths and one `testing.md`
   path reference files the runtime/binary split moved or turned into directories.
5. **`coding-standards.md` `#[non_exhaustive]` inventory drifted (MEDIUM, audit L1413; doc L189).** The
   hand-maintained enum inventory misattributes `PluginCapability` to the wrong crate and omits five enums
   that carry the attribute today.

## Verified current reality

Confirmed byte-accurate against the tree (2026-07-12) by an Explore subagent. Every claim below is the
authoritative current source the docs must be corrected to.

### `quality-gates.md`

- The doc's pre-commit table **already lists** `cargo fmt --all -- --check` (rust staged), `markdownlint`
  (md staged), and frontend `npm run lint` + `npm run format:check` — those rows are correct. What it
  **omits** are the other path-gated pre-commit gates the hook actually runs: `python3 ci/verify_db_access_policy.py`
  and `./scripts/check_legacy_error_matches.sh` (when `crates/ui/web-api/src/routes/*.rs` is staged),
  `actionlint` (when `.github/workflows/*` is staged), and `bash ci/verify_agents_md_budget.sh` (when an
  `AGENTS.md` is staged). (There is also a website-docs symlink check; niche, mention optionally.)
- The doc's pre-push table **omits** gates `.husky/pre-push` runs, notably
  `bash ci/verify_no_inline_query_params.sh`, `cargo xtask audit-coverage-check`, and
  `cargo xtask openapi-client-check`. (`cargo deny` runs via `scripts/check_deny.sh` with a computed base
  branch; `cargo fmt --check` and full-tree markdownlint run at pre-push, not only pre-commit.) Do not pin a
  row count — diff the table against the hook and add every missing gate.
- The "Full quality gate suite" list **omits** `bash ci/verify_no_inline_query_params.sh`,
  `cargo xtask audit-coverage-check`, `cargo xtask openapi-client-check` (all in `ci.yml`).
- `bash ci/verify_handler_state_contract.sh` **is a real gate** (listed in root `AGENTS.md` Quick-start and
  the doc's full-suite list) even though it is not in `.husky/pre-push` — **do not delete it**; it is
  CI/manual, not obsolete.
- "across all 28 crates" (doc ~L62) is wrong: `cargo metadata --no-deps` reports **76** workspace packages.
- None of the target docs are in `.markdownlintignore`; all four are markdownlint-gated.

### `scheduler-engine.md`

- Crate is `crates/core/scheduler-runtime/` (not `crates/shared/scheduler-engine/`).
- `TaskExecutor::execute` returns `error::Result<()>` (`= Result<(), Report<SchedulerError>>`,
  `executor.rs:11`), **not** `Result<(), String>`.
- `SchedulerConfig` (`scheduler.rs:26–44`) fields: `poll_interval`, `controller_id`, `task_execution_timeout`
  — there is **no** `tenant_id` field.
- The `TaskExecutor` implementations live in `crates/core/scheduler-runtime/src/executors/` (verified: the
  `impl TaskExecutor` sites there are auth_cleanup, stale_lease_cleanup, fetch_releases, detect_version,
  service_cert_check, crl_renewal, audit_log_cleanup, discover_software) — more than the doc's "six". Do not
  re-pin an exact count/list in the doc (see finding-3 approach); the point is only that the directory is the
  authoritative set.
- **`tick_executor` is NOT a `TaskExecutor` and NOT in `executors/`.** It is a **distinct trait**,
  `TickExecutor` (`crates/core/scheduler-runtime/src/tick_executor.rs:13`), registered separately via
  `Scheduler::register_tick_executor` (`scheduler.rs:107`, stored in `tick_executors:
  Vec<Arc<dyn TickExecutor>>` at `scheduler.rs:68`). Do not fold it into the `TaskExecutor`/`executors/` list,
  and scope the doc's directory pointer to "the `TaskExecutor` implementations" so it is not silently
  incomplete. (`awaiting_restart.rs` also sits in `executors/` but carries no `impl TaskExecutor` — treat the
  directory, not a hand-list, as truth.)
- The claim-recovery function is `recover_stale_claims` (`claim.rs:110`), not `recover_stale`.

### `error-handling.md` + `testing.md`

- `DbError` lives at `crates/core/controller-runtime/src/db/error.rs` (Pattern 1 cites the deleted
  `crates/core/controller/src/db/error.rs`).
- `is_receive_closed()` / `is_cert_expired()` live on `EnrollmentError` in
  `crates/shared/service-sdk/src/error.rs` (Pattern 11 cites `crates/core/agent/src/error.rs`; the agent
  crate `src/` now holds only `cli.rs` + `main.rs`). The doc's own Transport/Error Contract table (~L85)
  **already** references service-sdk correctly — the doc contradicts itself.
- `testing.md` (~L219) cites `crates/ui/web-api-queries/src/queries/autodiscovery.rs`; it is now a directory
  `queries/autodiscovery/` (`mod.rs`, `discovery_items.rs`, `ignore_rules.rs`, `reconcile.rs`,
  `default_configs.rs`).

### `coding-standards.md`

- `PluginCapability` lives in `uptrakit-shared-types` at `crates/shared/types/src/plugin_capability.rs:13`
  (carries `#[non_exhaustive]`); the doc lists it under `uptrakit-plugin-infrastructure-core`.
- Five enums in `uptrakit-shared-types` carry `#[non_exhaustive]` today but are omitted from the doc's
  shared-types list: `ConfigTestKind` (`config_test_kind.rs`), `OsFamily` (`os_family.rs`), `PluginRole`
  (`plugin_role.rs`), `SessionTokenType` (`session_token_type.rs`), `UpdateCategory` (`update_category.rs`).

## Goal

Make these five developer-docs describe the code and CI that exist, and — where a doc mirrors a greppable
source that has *already* rotted — replace the hand-maintained copy with a pointer to the source, so the
same drift cannot recur. This mirrors the repo's own anti-drift philosophy (`AGENTS.md` § *Maintaining this
file*: "no hardcoded counts", "no inventory tables that mirror code", "one canonical home; link, don't
copy") and the fix philosophy of the plugin-guidelines sweep.

Non-goals: restructuring any guide, changing code or CI wiring, adding a shared gate-runner script, or
touching docs-drift findings in files outside this five-finding cluster.

## Approach (primary recommendation)

Pure documentation correction + de-duplication, one editing pass, per-file. For each file: correct every
false fact against the verified reality above; where a rotted list mirrors a greppable source, delete the
copy and point at the source.

**Locator convention.** Line numbers in this spec are informational hints (the docs shift as edits land);
locate each correction by its section heading and/or a grep-defined string, and edit each file
independently.

### `quality-gates.md` (canonical doc — correctness first)

`quality-gates.md` is declared **canonical for Rust command definitions** (root `AGENTS.md`: "Canonical
source for Rust command definitions"), so unlike the other four it is *not* a pure mirror to be deleted —
its job is to be the accurate human-facing gate reference. Fix:

- **Correct the three lists** (pre-commit table, pre-push table, full-suite block) to match `.husky/pre-commit`,
  `.husky/pre-push`, and `ci.yml` per the verified deltas above — the correction is purely **additive** (the
  rows the doc already lists are in the right tier; add the omitted path-gated pre-commit gates and the
  omitted pre-push gates). No tier-move is needed.
- **Frame the tables honestly — they are a non-authoritative summary, not a second source of truth.** The
  doc owns **command definitions** (what each gate command is — stable regardless of tier). The tier tables
  (which gate runs at pre-commit vs pre-push, under what path trigger) still enumerate *tier membership*,
  which is exactly the volatile fact that drifted in findings 1–2; keeping them means the doc keeps a copy of
  wiring. Do not pretend otherwise: state in the doc that `.husky/*` is **authoritative for tier membership**
  and the tables are a **readability summary**, with **CI as the drift backstop**. This corrects the
  *omission* drift that occurred (gates missing entirely) but does not structurally prevent *mis-tiering*
  drift (a gate listed under the wrong tier) — the only structural cure for that is the shared runner deferred
  below, and it is out of scope here. So: fix the tables now, label them non-authoritative, and stop inviting
  a future reader to trust them as canonical (the very trust that caused this finding). Do not transcribe the
  hooks' per-path triggers or internal ordering.
- **Drop the "28 crates" number entirely** (do not re-pin "76" — it rots the same way; per the anti-count
  rule say "all workspace crates" / point at `cargo metadata`).
- **Keep `verify_handler_state_contract.sh`** — it is a real gate, not obsolete.
- **Same-commit sync check (repo rule):** root `AGENTS.md` requires the Quick-start block to move with
  `quality-gates.md` when a command/flag changes. No command is *changing* here (we are documenting gates
  that already exist), and the `AGENTS.md` Quick-start is explicitly a "run the gates relevant to what you
  touched" subset, not an exhaustive list — so no mandatory `AGENTS.md` edit. The implementer must still
  eyeball the Quick-start for any now-contradicted line and reconcile only if one exists (respecting the
  AGENTS.md size budget). Do not bulk-import every gate into the subset.

### `scheduler-engine.md` (rewrite against `crates/core/scheduler-runtime/`)

- Correct the crate path, module tree, `TaskExecutor` signature (`error::Result<()>`), `SchedulerConfig`
  fields (drop `tenant_id`; add `task_execution_timeout`), and `recover_stale` → `recover_stale_claims`.
- **Executor inventory:** do not hand-list the executors with a count ("six built-in executors" already
  rotted). Describe the executor concept + role, point at `crates/core/scheduler-runtime/src/executors/` as
  the authoritative list **of `TaskExecutor` implementations**, and give 2–3 representative examples. No count.
- **Do not conflate `TickExecutor` with `TaskExecutor`.** They are two distinct traits with separate
  registration paths; the directory pointer must name which trait it covers. Since the current doc omits
  `TickExecutor` entirely, the lower-risk correctness-only move is to scope the pointer precisely ("the
  `TaskExecutor` implementations live in `.../executors/`") and add one sentence that a separate `TickExecutor`
  trait (`src/tick_executor.rs`, `register_tick_executor`) also exists — so the pointer is complete-by-scope,
  not silently missing a sibling. Do not expand into documenting `TickExecutor` mechanics.
- Per ledger row 7 (merge-and-delete risks silent invariant loss): where the current page carries still-true
  *conceptual* guidance (how claims/leases work, executor contract), correct the mechanism reference and
  preserve the concept — do not trim explanation along with the stale specifics.

### `error-handling.md` + `testing.md` (path corrections)

- `error-handling.md` Pattern 1 → `crates/core/controller-runtime/src/db/error.rs`.
- `error-handling.md` Pattern 11 → `crates/shared/service-sdk/src/error.rs` (`EnrollmentError`), resolving
  the doc's self-contradiction with its own ~L85 table.
- `testing.md` (~L219) → `crates/ui/web-api-queries/src/queries/autodiscovery/` (directory).
- These are targeted reference fixes; no prose restructuring.

### `coding-standards.md` (delete the rotted inventory, keep the rule)

The `#[non_exhaustive]` inventory has now rotted twice (wrong crate for `PluginCapability`, five omissions)
and the attribute is trivially greppable. Per the audit's own "better" recommendation and the repo
anti-inventory philosophy:

- **Delete the per-crate per-variant inventory.** Keep the binding rule ("apply `#[non_exhaustive]` by
  default to extensible public enums/structs; external match sites need a wildcard arm") and its
  rationale, plus **one illustrative example**, and point at
  `grep -rn '#\[non_exhaustive\]' crates/` as the authoritative live list. Pick the retained example from a
  **stable, central crate** (e.g. a long-lived `uptrakit-shared-types` enum) so the one surviving reference
  does not itself rot — the enumerated list carried no teaching value a contributor needs (which types carry
  the attribute is trivia); the rule + rationale + one example is the entire pedagogical payload.
- This removes the copy that keeps drifting rather than re-syncing it into a third rotted state. (Same move
  the repo already made when it slimmed `AGENTS.md` by deleting inventory tables.)

### Rejected alternative — shared `scripts/quality-gates.sh` (YAGNI)

Audit finding 1 suggests extracting the gate list into a `scripts/quality-gates.sh` invoked by both the hook
and the doc "so the three copies cannot drift independently." The shared runner is, honestly, the **correct
eventual fix for tier-drift specifically** — a single sourced gate list is the only thing that makes
mis-tiering structurally impossible (see the ownership caveat above). But it is **deferred, not dismissed**,
on three grounds: (1) **scope** — a portable `sh` runner sourced by the husky hooks *and* CI *and* referenced
by the doc, with per-tier selection and macOS-dev/Linux-CI concerns, is a CI-tooling change with its own test
and review surface; folding it into a documentation-correctness pass is scope creep and forfeits this spec's
clean "no code/CI impact" property. (2) **reversibility** — the doc correction is independently valuable and a
prerequisite either way (the runner still needs accurate command definitions to source), so it is not wasted
work. (3) **evidence** — the runner is justified once drift *recurs after* a clean correction; two rots
against an *uncorrected* doc is consistent with "nobody fixed it," not yet proof that fixing it fails to hold.
If gate-list drift recurs after this sweep, the shared-runner refactor is a separate, deliberately-scoped
spec — not folded into a docs pass.

## Testing / verification

No code, so no unit tests. Verification is mechanical, per file, and belongs in the implementation plan:

- Run `markdownlint --config .markdownlint.json` over the five edited files (`quality-gates.md`,
  `scheduler-engine.md`, `error-handling.md`, `testing.md`, `coding-standards.md`) — all are markdownlint-gated
  (none excluded). Do **not** run `npx prettier`: this repo scopes
  Prettier to `frontend/` only (no root Prettier config; the pre-commit path check never touches `docs/`).
- **Grep each corrected file for the deleted/stale references** → 0 hits:
  `crates/shared/scheduler-engine`, `Result<(), String>` (in scheduler-engine.md), `tenant_id` (as a
  `SchedulerConfig` field), `recover_stale\b` (without `_claims`), `crates/core/controller/src/db/error.rs`,
  `crates/core/agent/src/error.rs`, `queries/autodiscovery.rs`, and the "28 crates" phrase.
- **Cross-check each corrected fact against its source** (ledger row 44 — verify against the real symbol,
  not memory): `grep` the cited signatures/fields/paths in `crates/core/scheduler-runtime/`,
  `crates/core/controller-runtime/src/db/error.rs`, `crates/shared/service-sdk/src/error.rs`,
  `crates/shared/types/src/*.rs` before writing them into the doc.
- **`TaskExecutor` vs `TickExecutor` categorization check:** grep the rewritten `scheduler-engine.md` for
  `tick_executor` / `TickExecutor`; confirm any mention is NOT described as a `TaskExecutor` or a member of
  `executors/`. Ground truth: `grep -rn 'impl TaskExecutor\|trait TickExecutor' crates/core/scheduler-runtime/`
  (`TaskExecutor` impls under `executors/`; the `TickExecutor` trait at `src/tick_executor.rs`).
- **quality-gates.md gate lists:** diff the doc's three lists against `.husky/pre-commit`, `.husky/pre-push`,
  and `ci.yml` after editing. This is a **completeness** check, not a line-for-line-sync requirement (the doc
  owns command definitions, `.husky/*` owns exact wiring): assert no *gate* the hooks/CI run is absent from
  the doc — either listed or covered by the "see `.husky/*` for exact wiring" pointer — and no listed command
  is dead. Per-condition path triggers and hook-internal ordering are deliberately not transcribed.
- **coding-standards.md:** after deleting the inventory, `grep -rn '#\[non_exhaustive\]' crates/ | wc -l` is
  the live source the doc now points at; confirm the retained example compiles conceptually (matches a real
  annotated enum).

## Documentation deliverables

This spec **is** a documentation change; the deliverables are the edited docs:

- `docs/development/quality-gates.md` — finding 1 + 2 (gate lists + crate count).
- `docs/development/scheduler-engine.md` — finding 3 (full rewrite against `scheduler-runtime`).
- `docs/development/error-handling.md` — finding 4 (Pattern 1 + Pattern 11 paths).
- `docs/development/testing.md` — finding 4 (autodiscovery directory path).
- `docs/development/coding-standards.md` — finding 5 (delete inventory, keep rule + pointer).
- Root `AGENTS.md` Quick-start — **conditional**: reconcile only a now-contradicted line if one exists (see
  quality-gates.md § same-commit sync check); not a mandatory edit.

No ADR (documentation accuracy is not an architectural decision). No README/CONTEXT change. No
wire/OpenAPI/frontend/API-doc impact. No code or CI change.

**Commit granularity.** The five file edits are **independent** — no edit changes a fact another edit depends
on (findings 1+2 are the only ones sharing a file, `quality-gates.md`). That independence, not the shared
theme, is what makes bundling them into one spec low-risk. The `scheduler-engine.md` rewrite (finding 3) is
the only non-trivial edit; the implementation plan should land it as its **own commit**, separate from the
four mechanical one-to-two-line corrections, so a problem in the rewrite does not hold the trivial path fixes
hostage (and vice versa).

## Out of scope / deferred

- All other unspecced Medium+ audit findings (code-stability MEDIUMs — silent error swallowing, data-loss
  paths, large-file refactors, etc.); each is its own spec or merge in a later iteration.
- The `scripts/quality-gates.sh` shared-runner refactor (rejected above; separate spec if drift recurs).
- Any restructuring of the five guides beyond the corrections named here.
- Any code, CI-wiring, or hook change.

# Spec: `software_items/mod.rs` → topical-submodule facade split

- **Date:** 2026-06-27
- **Status:** Approved, ready for planning
- **Target file:** `crates/ui/web-api/src/routes/software_items/mod.rs`
- **Measured on:** `main` @ `7036b3c96` (CodeScene score + live structure verified)
- **Template:** sibling `crates/ui/web-api/src/routes/service_ws/handler/messages/` split
  (`messages.rs` 2.81/RED → none-red) and `.../handler/updates/` (3.3/RED → none-red). This dir is
  ALREADY a partial facade — `controller_fetch.rs` and `version_check_dispatch.rs` exist alongside
  `mod.rs`. Extend that pattern; do NOT introduce a competing structure.

## Problem

`software_items/mod.rs` is **3928 lines**, CodeScene code health **6.63 / YELLOW**. It is the
highest-churn large file in the repo (141 revisions, friction 0.27 — top of the undecomposed
hotspots). This is **friction reduction, not a red rescue**: the file is yellow, not red. The
dominant drags are file-aggregate smells (low cohesion across ~35 top-level production items, a
~1319-LoC inline test footprint) that splitting relieves directly.

## Goal & success metric

- **Primary goal — churn/cohesion on the #1 hotspot.** The file is YELLOW (6.63), so the
  red-floor below is ALREADY met — "no red" cannot justify this work. The actual justification is
  **friction reduction on the repo's highest-churn undecomposed file** (141 revisions): smaller,
  single-responsibility files that a future maintainer and CI re-touch with less cognitive load.
  Treat this as a high-value cleanup on the worst hotspot, not a required red rescue.
- **Optimized metric (secondary):** per-file CodeScene `code_health_score` — aim each submodule
  green (≥ 9.0); a **yellow** production submodule is acceptable. `version_check.rs` inherits the only
  genuine function-level smells (bumpy `load_agent_service`/`classify_role_assignments`, the two
  precondition enums) and is the EXPECTED yellow one — its yellow is not a failure.
- **Enforceable floor (guardrail):** no resulting file is red (every file ≥ 4.0). `tests.rs` is the
  single most likely floor risk (the precedent `messages/tests.rs` lands at **4.76**) — score it
  explicitly (see Step 4 / re-score gate).
- **Hard constraint — behavior-preserving:** no change to route semantics, request/response JSON,
  DB queries, audit outcomes, SSE/MQTT payloads. Mechanical decomposition only.

## Live-code verification (corrections vs. original brief)

Verified against `main` @ `7036b3c96`:

| Claim in brief                                     | Verified reality                                                                                         |
| -------------------------------------------------- | -------------------------------------------------------------------------------------------------------- |
| Test module ~line 2151 → EOF, ~1777 lines          | **TWO** cfg-gated modules: `audit_tests` (2151–2246, 96 LoC) + `tests` (2706–3928, 1223 LoC) = ~1319 LoC |
| ~87 top-level items                                | **~35** production items (11 consts, 2 structs, 2 enums, 16 pub async fns, sync+async helpers)           |
| 6.63 / YELLOW                                      | Confirmed `6.63`                                                                                         |
| `fire_software_item_lifecycle` is update-path      | **create-only** — sole caller is `create_software_item` (mod.rs:297). Belongs in `crud.rs`, NOT updates  |
| file-level `#![expect]` to relocate                | **none** in production; `#![expect]` exist only INSIDE the two test modules (move with tests)            |
| siblings controller_fetch / version_check_dispatch | Confirmed present (291 / 311 LoC), untouched by this work                                                |

**External consumers** (drive visibility) — all `router.rs` references via
`crate::routes::software_items::`. Confirmed against live `router.rs`: **16** route handlers are
externally named and wired (`.routes(...)` + OpenAPI block):
`create_software_item`, `list_software_items`, `get_software_item`, `update_software_item`,
`delete_software_item`, `approve_software_item`, `assign_hosts`, `unassign_host`,
`update_host_assignment`, `delete_plugin_assignment`, `trigger_update`, `check_versions`,
`check_versions_host`, `batch_software_items`, **`preview_software_item_merge`**,
**`execute_software_item_merge`** (the merge pair routed at `router.rs:635`/`638`, OpenAPI at
`110`/`111`).

> All **16** handlers get `pub(in super::super)` in their destination submodule + `pub(super) use` in
> the facade. The implementer should still re-grep `router.rs` (and any sub-router) to confirm the set
> is unchanged at implementation time, but no handler beyond these 16 is expected.

All re-exported request/response types (`CreateSoftwareItemRequest`, `SoftwareItemResponse`,
`MergeSoftwareItems*`, `TriggerUpdate*`, `BatchAction*`, etc.) are re-exported from
`uptrakit-web-api-types` and are NOT defined in this dir — the facade's existing `pub use` of those
type names is preserved verbatim; this split does not move them.

## Chosen approach

Convert `mod.rs` into a thin facade plus per-responsibility-family production submodules and a moved
test module. (User decisions, recorded in grilling: 7-module family cut; dedicated `audit.rs` with
error enums colocated next to their consumer; harmless dedups; conditional extractions gated on
CodeScene re-score.)

### Step 1 — Facade conversion

- `git mv mod.rs mod.rs` is a no-op — `mod.rs` STAYS as the facade (the dir already exists). Do NOT
  rename. New submodules are NEW files created beside it. (History for moved code follows via the
  per-family commits; there is no single-file rename to preserve.)
- `mod.rs` becomes a thin facade containing, in this order:
  1. Module doc-comment mapping each submodule to its responsibility.
  2. `mod <name>;` declarations for each new submodule (`audit`, `crud`, `merge`,
     `host_assignments`, `version_check`, `updates`, `batch`) plus the existing
     `controller_fetch` / `version_check_dispatch` declarations (unchanged).
  3. Visibility-correct re-exports of externally-consumed handlers (see Visibility).
  4. The preserved `pub use` of `uptrakit-web-api-types` request/response names (verbatim).
  5. `#[cfg(all(test, feature = "db-sqlite"))] mod tests;`

### Step 2 — Move the test modules FIRST (biggest cheap win)

- Move BOTH inline cfg-gated modules into `software_items/tests.rs`:
  - `audit_tests` (lines 2151–2246, 96 LoC)
  - `tests` (lines 2706–3928, 1223 LoC)
- **Structure (prescribed):** in `tests.rs`, put the `tests` module's contents at file scope and keep
  `audit_tests` as a NESTED `mod audit_tests { ... }` inside it. Do NOT flatten the two into one
  module. Each module carries a DIFFERENT `#![expect]` set (below); nesting preserves each set in its
  own scope and avoids a same-scope duplicate-`#![expect]` collision under
  `unfulfilled_lint_expectations = "deny"`. Preserve every test's behavior.
- **`#![expect]` relocation (exact attrs — verified against live code):**
  - `audit_tests` (inner attrs at mod.rs ~2153/2154) carries:
    `#![expect(clippy::expect_used, reason = "test code: panics on failure are acceptable")]` and
    `#![expect(clippy::string_slice, reason = "test code: slice indexes are at validated boundaries")]`
    — note **`clippy::string_slice`** (triggered by `format!("host-{}", &id.to_string()[..8])`), NOT
    `clippy::panic`. Keep both on the nested `mod audit_tests`.
  - `tests` (inner attrs at mod.rs ~2708/2709) carries:
    `#![expect(clippy::expect_used, reason = "test code: panics on failure are acceptable")]` and
    `#![expect(clippy::panic, reason = "test code: panics on failure are acceptable")]`. These become
    the file-level `#![expect(...)]` of `tests.rs`.
  - Workspace sets `unfulfilled_lint_expectations = "deny"`, so a missing OR stale expectation is a
    hard build error. Verify by compiling `--all-features`.
- **Restore private-helper visibility for moved tests.** The test block reaches production helpers via
  same-file scope today. After the move, `use super::*` resolves to the facade, which does NOT
  re-export private helpers. For EACH private production item the tests call (e.g.
  `DeleteHostAssignmentParams`, audit consts, `AuditContext`, any private fn): promote to `pub(super)`
  in its destination submodule and add an explicit `use super::<submodule>::<item>;` in `tests.rs`.
  **Grep the moved test bodies for every direct call** and promote exactly those — an unused `use` in
  `tests.rs` is a `warnings = deny` failure; over-promotion is equally a failure. Do not over-promote.
- This alone removes the ~1319-line test footprint from the production-scored file.

> **Re-score checkpoint (after Step 2).** Run CodeScene `code_health_score` on `mod.rs` once the test
> footprint is gone — most of the file-aggregate drag (declarations, file LoC, the test-duplication
> groups) lives in the moved tests, so `mod.rs` may already land green here. The per-family production
> split (Step 3) still proceeds — it is justified by **churn/cohesion on the hotspot**, not by the
> score alone — but record this number: it isolates how much the production split actually buys, and if
> any later submodule extraction would REGRESS a then-green `mod.rs`, prefer leaving that code in place.

### Step 3 — Per-family production submodules

| Submodule             | Items (top-level)                                                                                                                                                                                                                                                                                              |
| --------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `audit.rs`            | 11 `SOFTWARE_ITEM_*` / `SOFTWARE_VERSION_CHECK_*` audit-action consts, `AuditContext<'a>` struct, `emit_software_item_mutation_audit`, `emit_software_version_check_audit` (all sync; all `pub(super)`)                                                                                                        |
| `crud.rs`             | `create_software_item`, `list_software_items`, `get_software_item`, `update_software_item`, `delete_software_item`, `approve_software_item`, `fire_software_item_lifecycle` (private — create-only)                                                                                                            |
| `merge.rs`            | `preview_software_item_merge`, `execute_software_item_merge`                                                                                                                                                                                                                                                   |
| `host_assignments.rs` | `assign_hosts`, `unassign_host`, `update_host_assignment`, `delete_plugin_assignment`, `DeleteHostAssignmentParams` (`pub(super)` — tests consume it)                                                                                                                                                          |
| `version_check.rs`    | `check_versions`, `check_versions_host`, `version_check_dispatch_mode` (private), `classify_version_check_context_load_failure` (private), `CheckVersionsHostPreconditionError` + `LoadAgentServiceError` (enums, private), `verify_software_item_and_host`, `load_agent_service`, `classify_role_assignments` |
| `updates.rs`          | `trigger_update`                                                                                                                                                                                                                                                                                               |
| `batch.rs`            | `batch_software_items`                                                                                                                                                                                                                                                                                         |
| `tests.rs`            | moved test modules (Step 2)                                                                                                                                                                                                                                                                                    |

Aim: each fn < 70 LoC where reasonable, cc < 9 where reasonable, each file well under brain-class
size. Large handlers (`check_versions`, `check_versions_host`, `batch_software_items`,
`assign_hosts`) remain intact within their family file — splitting them is NOT required to meet the
goal and risks regression (Step 5).

> **Audit-const safety (when moving `audit.rs`).** The 11 `SOFTWARE_ITEM_*` / `SOFTWARE_VERSION_CHECK_*`
> action consts are persisted/wire-checked audit-outcome strings — a one-character typo during the move
> compiles clean and silently changes an audit outcome. Do NOT eyeball: after moving, `git diff` (or
> `grep`-extract + `diff`) the const string LITERALS byte-for-byte against the original `mod.rs`, and
> confirm `bash ci/verify_typed_audit_actions.sh` passes.

### Step 4 — Harmless de-dups

**Test-setup boilerplate (≈19 instances):** the moved test fns repeat the same four-line setup:

```rust
let db = setup_migrated_db().await;
let tenant_id = insert_default_tenant(&db).await;
let state = build_test_state_without_real_protection(db.clone(), tenant_id).await;
let tenant_db = TenantDb::new_for_test(state.db().clone(), tenant_id);
```

Collapse to ONE shared helper (e.g. `async fn setup_state() -> (Db, Uuid, AppState, TenantDb)`) in
`tests.rs`. Behavior identical.

- **Any genuinely-identical production dup pairs** surfaced during the split — collapse only when
  behavior is provably identical (move shared code into the owning submodule, do not invent a
  cross-cutting util).

> **`tests.rs` is the floor-risk file — score it explicitly.** The setup-dedup above is NOT the
> dominant duplication driver; CodeScene flags ~14 near-identical **audit-assertion bodies** as the
> duplication groups, and the precedent `messages/tests.rs` lands at **4.76** (lowest of all split
> files). After the test move + setup-dedup, re-score `tests.rs`. If it dips toward red, the lever is a
> table-driven assertion helper for those audit-assertion bodies (NOT more setup-dedup) — apply it only
> if it keeps the file none-red without changing any assertion's meaning.

### Step 5 — Conditional, re-score-gated extraction

- Candidates (attempt AFTER Steps 1–4, AFTER confirming none-red):
  - The two precondition error enums `CheckVersionsHostPreconditionError` / `LoadAgentServiceError`
    have near-identical `impl` blocks (`into_response()` + `audit()`). A macro or shared trait could
    DRY them. **Caution:** the two enums emit DISTINCT audit actions — any shared abstraction MUST
    keep each enum's action distinct. Gate this extraction on the moved unit tests that assert each
    enum's specific action still passing (`cargo test --all-features`); if no such test exists, do not
    attempt the dedup (the safety net is absent).
  - Any remaining bumpy-road handler the post-split re-score flags.
- **Gate (apply verbatim):** keep an extraction ONLY if a CodeScene `code_health_score` re-score of
  the affected submodule shows the score improves (or at minimum does not regress) AND the extraction
  introduces **no** new excess-args helper and **no** new large method. If it redistributes
  complexity or spawns 5–7-arg helpers, **revert it**. Precedent: the analogous
  `build_enriched_display_overrides` extraction REGRESSED the messages.rs score 7.8 → 7.09 and was
  reverted. Do not repeat that.
- Stop at yellow if green is not free.

### Step 6 — Bookkeeping (non-optional gates)

- **`crates/ui/web-api/db_access_policy.toml`** — replace the
  `[routes."software_items/mod.rs"]` section with one section per new file path. The checker tracks
  **only `async fn`** (top-level AND nested async fns); sync fns get NO entry. Values preserved:
  - `[routes."software_items/crud.rs"]`:
    `create_software_item = "full-state"`, `list_software_items = "tenant-scoped"`,
    `get_software_item = "tenant-scoped"`, `update_software_item = "full-state"`,
    `delete_software_item = "full-state"`, `approve_software_item = "full-state"`,
    `fire_software_item_lifecycle = "ignore"`
  - `[routes."software_items/merge.rs"]`:
    `preview_software_item_merge = "tenant-scoped"`, `execute_software_item_merge = "tenant-scoped"`
  - `[routes."software_items/host_assignments.rs"]`:
    `assign_hosts = "full-state"`, `unassign_host = "full-state"`,
    `update_host_assignment = "full-state"`, `delete_plugin_assignment = "full-state"`
  - `[routes."software_items/version_check.rs"]`:
    `check_versions = "full-state"`, `check_versions_host = "full-state"`,
    `verify_software_item_and_host = "ignore"`, `load_agent_service = "ignore"`,
    `classify_role_assignments = "ignore"`
  - `[routes."software_items/updates.rs"]`: `trigger_update = "full-state"`
  - `[routes."software_items/batch.rs"]`: `batch_software_items = "full-state"`
  - **No `audit.rs` section** — both its fns (`emit_software_item_mutation_audit`,
    `emit_software_version_check_audit`) are sync. Likewise, the two sync helpers moving to
    `version_check.rs` (`version_check_dispatch_mode` at mod.rs:146, `classify_version_check_context_load_failure`
    at mod.rs:155 — both verified `fn`, not `async fn`) get NO policy entry. The checker tracks only
    `async fn`. The canonical async list above is derived from the existing `software_items/mod.rs`
    section (which tracks exactly these 20 async fns); re-grep `async fn` in the original `mod.rs` before
    migrating to confirm nothing changed.
  - The existing `[routes."software_items/controller_fetch.rs"]` and
    `[routes."software_items/version_check_dispatch.rs"]` sections stay unchanged.
  - Add `[routes."software_items/tests.rs"]` listing the moved async test fns/helpers as `"ignore"`
    (convention: `updates/tests.rs` / `messages/tests.rs` already do this).
  - Verify with `python3 ci/verify_db_access_policy.py`.
- **`AGENTS.md`** — replace the single `software_items/mod.rs` row in the route/handler-module table
  (lines ≈1336–1338) with one row per new submodule. MD060 aligned style — pad columns so
  markdownlint passes; run `npx prettier --write AGENTS.md` then markdownlint. Suggested purposes:
  - `software_items/mod.rs` — Software-item route facade (module wiring + handler re-exports)
  - `software_items/audit.rs` — Software-item audit-action consts + audit-emit helpers
  - `software_items/crud.rs` — CRUD handlers (create/list/get/update/delete/approve) + create lifecycle hook
  - `software_items/merge.rs` — Merge preview/execute handlers
  - `software_items/host_assignments.rs` — Host + plugin assignment handlers
  - `software_items/version_check.rs` — Version-check handlers + precondition/agent-service helpers
  - `software_items/updates.rs` — Update-trigger handler
  - `software_items/batch.rs` — Batch-action handler
  - `software_items/tests.rs` — Unit tests for the software_items submodules
  - Keep the existing `controller_fetch.rs` / `version_check_dispatch.rs` rows.

## Import & visibility conventions (apply verbatim — hard-won from messages.rs/updates.rs)

**Import convention:**

- Submodules reach handler-level / crate-level siblings via **relative** `super::super::<sibling>`
  paths (or `super::super::super::` as depth requires), NOT absolute `crate::routes::...`. Route any
  needed `crate::routes::<other>` name through the facade as a private
  `use super::super[::super]::<mod>::{...}` re-import, and have submodules consume `super::<Name>`.
  (A `/verify` pass caught exactly this `crate::routes::` leak on the prior job.)
- **Intra-dir cross-submodule** sharing (e.g. `crud.rs`/`merge.rs`/`batch.rs` using `audit.rs`
  items): the item is `pub(super)` in its submodule and consumers `use super::audit::{AuditContext,
emit_software_item_mutation_audit, SOFTWARE_ITEM_CREATE_AUDIT_ACTION, ...};` — `super` from a
  submodule resolves to the facade, so `super::audit::Foo` reaches a `pub(super)` item directly. No
  facade re-export is needed for intra-dir sharing.
- When moving a body that calls `super::<sibling>::<fn>(...)`, rewrite the now-broken qualified path:
  bare name imported via `use super::{...}` if the facade re-exports it, else `super::super::<sibling>`.
  `cargo check` names every broken path.
- As each family extracts, **PRUNE now-orphaned DIRECT imports from `mod.rs`** (move them into the
  destination submodule; don't delete names still used). `unused_imports` is `warnings = deny`. Keep
  `use super::<sibling>` BRIDGE re-imports in the facade where children consume them via `super::`.

**Visibility:**

- A handler consumed OUTSIDE `software_items` (named by `router.rs`) must be `pub(in super::super)` in
  its submodule **and** re-exported by the facade with `pub(super) use`. Applies to every handler the
  router-grep finds (the 14 listed above, plus the two merge handlers if confirmed routed).
- A `pub(super)` item + `pub(super) use` re-export does **NOT** compile (can't widen). Intra-dir
  helpers and test-consumed items stay `pub(super)` (facade pulls them with a private `use` only if
  the facade itself needs them); everything same-file stays private (no keyword).
- Verify actual call sites before promoting — grep `tests.rs` and `router.rs` for direct references.
  Do not over-promote.

## Idiom & standards conformance (from `.superpowers/standards-snapshot.md`)

- Lint suppressions stay `#[expect(lint, reason="...")]` — never bare `#[allow]`, never a NEW
  suppression to dodge a gate. Preserve existing `#[expect]` attrs on moved items verbatim; relocate
  the test-module file-level `#![expect]` to `tests.rs` (Step 2).
- No `unwrap()` in production (parking_lot/RwLock excepted); errors via `rootcause::Report` /
  `report!` / `bail!`. This refactor moves code, does not author new error paths — preserve patterns.
- `parking_lot::Mutex`, drop guards before `.await` — preserve as-is in moved code.
- All HTTP request types keep their `Validate` impls and handler-entry `validate()` calls verbatim.
- Conventional Commits at workspace level. Suggested commit sequence (each commit compiles + passes
  fmt/clippy): facade skeleton; move tests; extract each family submodule; harmless dedups;
  bookkeeping (db_access_policy + AGENTS.md); (optional) gated extraction.

> ⚠ **Commit hygiene (hard-won):** `git commit --only <dir>` DROPS new untracked files (commits a
> broken tree). Use `git add <exact paths>` then a PLAIN `git commit`. After EVERY commit verify with
> `git status --short` (clean) AND `git ls-tree HEAD software_items/` (new file present).

## Quality gates (run BOTH feature permutations)

```bash
cargo fmt --all
cargo check  --no-default-features --features db-sqlite
cargo check  --all-features
cargo clippy --all-targets --no-default-features --features db-sqlite
cargo clippy --all-targets --all-features
cargo test   --all-features
cargo deny check
python3 ci/verify_db_access_policy.py
bash   ci/verify_no_security_audit.sh        # audit-emit code moves across submodules
bash   ci/verify_typed_audit_actions.sh      # typed audit actions in moved code
bash   ci/verify_handler_state_contract.sh   # split restructures handler module layout
python3 ci/check_plugin_semantic_boundary.py # blocking gate; production code path
markdownlint --config .markdownlint.json '**/*.md'   # AGENTS.md + this spec
# REQUIRED — software_items touches version-check / update-dispatch / controller-fetch /
# service-lifecycle flows (version_check_dispatch.rs sibling confirms it):
docker build -f docker/Dockerfile.test -t uptrakit-test:latest . \
  && cargo test -p uptrakit-integration-tests -- --ignored
```

Re-score gate: after the split (Steps 1–4), run CodeScene `code_health_score` on `mod.rs` and every
new production submodule; confirm none-red. Only then attempt Step 5, re-scoring the affected
submodule to gate each extraction.

## Documentation deliverables

- **`AGENTS.md`** — handler-module table: replace the `software_items/mod.rs` row with per-submodule
  rows (Step 6). **Required.**
- **`db_access_policy.toml`** — per-file policy sections migrated (Step 6). Tracked gate. **Required.**
- **No ADR** — pure mechanical decomposition under the existing web-api decomposition strategy; no new
  architectural decision. Behavior-preserving file split following an established pattern.
- **No `README.md` / `CONTEXT.md` change** — no externally observable behavior, surface, config, or
  architecture change.

## Sequencing

1. Facade skeleton in `mod.rs` (module decls + preserved re-exports).
2. Move both test modules → `tests.rs`; relocate file-level `#![expect]`; collapse ≈19 dup setup
   blocks into one helper; restore private-helper visibility for moved tests.
3. Extract per-family production submodules (`audit`, `crud`, `merge`, `host_assignments`,
   `version_check`, `updates`, `batch`); wire facade re-exports + visibility; prune orphaned imports.
4. Apply harmless production dedups.
5. Migrate `db_access_policy.toml` sections; update `AGENTS.md` rows (`prettier` + markdownlint).
6. **Re-score** `mod.rs` + every production submodule → confirm none-red.
7. **Only then** attempt gated extractions (error-enum dedup macro etc.); keep each only if the
   re-score improves or holds with no new excess-args/large-method; otherwise revert.
8. Full quality gates incl. Docker integration suite.

Execution: subagent-driven in a git worktree; commit per task; review each task (spec compliance +
quality); final whole-branch review + `/verify` before merge.

## Out of scope / deferred

- Decomposing the large intact handlers (`check_versions`, `check_versions_host`,
  `batch_software_items`, `assign_hosts`) beyond gated dedup attempts — high regression risk for no
  required score benefit (the messages.rs/updates.rs precedents reverted analogous work).
- Any change to route semantics, request/response JSON, DB queries, audit JSON, SSE/MQTT payloads.
- Touching `controller_fetch.rs` / `version_check_dispatch.rs` internals, or sibling route modules
  beyond the import/visibility wiring needed to keep them compiling.
- Pursuing green at the cost of redistributed complexity or new excess-args helpers.

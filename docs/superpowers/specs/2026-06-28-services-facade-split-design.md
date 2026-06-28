# Spec: `routes/services.rs` → topical-submodule facade split

- **Date:** 2026-06-28
- **Status:** Approved, ready for planning
- **Target file:** `crates/ui/web-api/src/routes/services.rs` (single file — no `services/` dir yet)
- **Measured on:** `main` @ HEAD (`020801e8e`); CodeScene score + live structure verified
- **Template:** sibling `crates/ui/web-api/src/routes/software_items/` split
  (`software_items/mod.rs` 6.63/YELLOW → none-red); same web-api route-module shape, same
  `AuditContext` / merge / batch / status-lifecycle families. Precedent design:
  `docs/superpowers/specs/2026-06-27-software-items-facade-split-design.md`. The RED-rescue precedents
  are `messages-facade-split` and `updates-handler-decomposition` (same dir family).

## Problem

`services.rs` is **2721 lines** (`wc -l`), CodeScene code health **5.97 / YELLOW**, **128 revisions**,
friction **0.231** — the highest-friction file in the repo and CodeScene's **#1 recommended
refactoring target** now that `software_items` is split.

> **Stale-hotspot note (verified).** `list_technical_debt_hotspots_for_project` (project `81357`)
> still reports `services.rs` at **3816 LoC** and ranks `software_items/mod.rs` (6.63) on top. That
> ranking is **stale** — it predates the un-pushed `software_items` merge and counts the pre-split
> `services.rs` test footprint. The live `code_health_score` on `services.rs` is **5.97**; treat that
> as ground truth.

This is **friction reduction, not a red rescue**: the file is yellow, not red. The dominant drags are
file-aggregate smells — low cohesion across 9 route handlers + an audit-helper cluster, plus a
**~1311-line inline `#[cfg(test)]` footprint** (`mod tests` at L1411 → EOF) — that splitting relieves
directly.

## Goal & success metric

- **Primary goal — churn/cohesion on the #1 hotspot.** The file is YELLOW (5.97); the red-floor below
  is already met. Justification is **friction reduction on the repo's highest-churn undecomposed file
  (128 revisions)**: smaller, single-responsibility files that a maintainer and CI re-touch with less
  cognitive load.
- **Optimized metric (secondary):** per-file CodeScene `code_health_score` — aim each submodule green
  (≥ 9.0); a **yellow** production submodule is acceptable. **`code_health_review` on the live file
  shows ~95% of the 5.97 drag is in the test module** (12 duplication findings, a 209-LoC `test_state`,
  5 large/duplicated assertion blocks — all `tests.*`). The ONLY flagged production smell is
  `emit_service_lifecycle_audit` (6 args → Excess Function Arguments); none of the 9 handlers is flagged
  Large Method. So the real score risks are **`tests.rs`** (heavy duplication) and possibly **`audit.rs`**
  (the 6-arg helper) — NOT `lifecycle.rs`, which despite ~715 LoC has no flagged handler smells and is
  expected green/none-red.
- **Enforceable floor (guardrail):** no resulting file is red (every file ≥ 4.0). `tests.rs` is the
  single most likely floor risk (precedent `messages/tests.rs` landed at 4.76; the services test module
  is heavier — 1311 LoC, 12 dup findings — so plausibly 5–6) — score it explicitly (Step 2 / re-score
  gate). This is where the score gain actually lives; gate hard on it.
- **Hard constraint — behavior-preserving:** no change to route semantics, request/response JSON, DB
  queries, audit outcomes/strings, SSE/MQTT payloads. Mechanical decomposition only.

## Live-code verification (vs. brief)

Verified against `main` @ `020801e8e`:

| Claim in brief                              | Verified reality                                                                                                                                                                                |
| ------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| ~2721 LoC, 5.98/YELLOW                      | `wc -l` = **2721**; `code_health_score` = **5.97**                                                                                                                                              |
| 9 pub async fn handlers                     | Confirmed: `list_services`, `get_service`, `update_service`, `approve_service`, `reject_service`, `deactivate_service`, `set_update_freeze`, `merge_service`, `batch_services`                  |
| `AuditContext<'a>` + emit helpers ~L42      | `AuditContext` L42; `emit_service_lifecycle_audit` L49, `emit_service_batch_audit` L72, `batch_action_to_audit_action` L92 — **all sync**                                                       |
| inline test module ~L1411 → EOF, ~1310 LoC  | Confirmed: **single** `#[cfg(test)] mod tests` at L1411 → EOF (~1311 LoC). NOT two cfg modules (software_items had two)                                                                         |
| nested `#[cfg(feature="test-utils")]` block | It is a `cfg`-gated block **inside** `test_state` helper (alongside `oidc`/`interactive` cfg blocks), NOT a separate test module. Moves verbatim with `tests.rs`                                |
| file-level `#![expect]` to relocate         | **none** in production. The `mod tests` carries exactly ONE mod-level attr: `#![expect(clippy::let_underscore_must_use, reason = "fire-and-forget sends in tests drop results intentionally")]` |

**External consumers** (drive visibility) — `router.rs` references all **9** handlers via
`crate::routes::services::<handler>` (`.routes(routes!(...))` at L535–553 + the `routes!()` OpenAPI
list L49–57). The OpenAPI `components` block names **10 types via `crate::routes::services::`**
(L213–222) — `ServiceStatus`, `ServiceResponse`, `UpdateServiceRequest`, `MessageResponse`,
`MergeAgentRequest`, `SetUpdateFreezeRequest`, `BatchActionRequest`, `BatchActionResponse`,
`BatchActionSuccess`, `BatchActionFailure` — plus `PaginatedResponse<ServiceResponse>` referenced at
L303 via `uptrakit_web_api_types::pagination::` (the `PaginatedResponse` wrapper is NOT pathed through
`crate::routes::services`, only its `ServiceResponse` type arg is).

> All these type names are already re-exported in the current `services.rs` head (L33–40) **from
> `uptrakit-web-api-types`** (`pub use uptrakit_web_api_types::{batch_actions::*, pagination::*, services::*}`).
> They are NOT defined in this file. The facade preserves those three `pub use` blocks
> **verbatim** — this split does not move or redefine any request/response type. Re-grep `router.rs`
> at implementation time to confirm the handler/type set is unchanged.

**Audit-helper consumer map** (verified by grep — narrows `audit.rs` coupling):

- `AuditContext` + `emit_service_lifecycle_audit` (`emit_event`, fire-and-forget): used by
  `set_update_freeze` (L943+), `merge_service` (L1098+), `batch_services` (L1248+).
- `emit_service_batch_audit` + `batch_action_to_audit_action`: used by `batch_services` only.
- `approve_service` / `reject_service` / `deactivate_service` / `update_service` emit **inline
  `emit_stateful`** (transactional, in-tx) — they do **NOT** call the `audit.rs` helpers.

So `audit.rs` is consumed by `lifecycle.rs` (only `set_update_freeze`), `merge.rs`, and `batch.rs` —
all via `super::audit::{...}`. `crud.rs` does not consume `audit.rs`.

## Chosen approach

Convert `services.rs` into a thin facade plus per-responsibility-family production submodules and a
moved test module. **User decision (grilling): 5-module production cut** —
`crud` / `lifecycle` / `merge` / `batch` / `audit`, plus `tests.rs`. (Alternative considered: split
`lifecycle` into `status.rs` + `freeze.rs` — rejected; the four handlers are one status-lifecycle
family and `set_update_freeze` at ~155 LoC does not warrant its own file. The `emit_stateful` vs
`emit_event` mechanism difference is an internal detail, not a module boundary.)

### Step 1 — Facade creation (`git mv`)

- **`git mv crates/ui/web-api/src/routes/services.rs crates/ui/web-api/src/routes/services/mod.rs`**
  FIRST (history follows the largest body of code; this is a fresh facade, unlike `software_items`
  which was already a dir). New submodules are NEW files created beside `mod.rs`.
- `mod.rs` becomes a thin facade containing, in this order:
  1. Module doc-comment mapping each submodule to its responsibility.
  2. `mod audit; mod crud; mod lifecycle; mod merge; mod batch;` declarations.
  3. Visibility-correct **plain `pub use`** re-exports of the 9 handlers + their `__path_*` types
     (see Visibility).
  4. The preserved `pub use uptrakit_web_api_types::{...}` request/response re-exports (L33–40, verbatim).
  5. `#[cfg(test)] mod tests;`

### Step 2 — Move the test module FIRST (biggest cheap win)

- Move the single inline `#[cfg(test)] mod tests` (L1411 → EOF, ~1311 LoC) into
  `crates/ui/web-api/src/routes/services/tests.rs` as the file body.
- **`#![expect]` relocation (exact, verified):** the `mod tests` carries exactly ONE mod-level
  attribute —
  `#![expect(clippy::let_underscore_must_use, reason = "fire-and-forget sends in tests drop results intentionally")]`.
  It becomes the file-level `#![expect(...)]` of `tests.rs`. The workspace sets
  `unfulfilled_lint_expectations = "deny"`, so a missing OR stale expectation is a hard build error —
  verify by compiling `--all-features`. If `--all-features` surfaces an additional needed
  test-context `#![expect]` (e.g. `clippy::expect_used`/`panic`/`string_slice`) that previously
  resolved via the inline-module context, add the EXACT lint+reason the compiler names — do not
  invent suppressions.
- **No separate `audit_tests.rs` needed (verified).** Unlike `software_items` (whose `audit_tests`
  module called facade-private items, forcing a top-level split), `services.rs` tests reach audit
  state only by reading DB audit rows (`latest_tenant_audit_row`) after invoking handlers — they make
  **zero** direct calls to `AuditContext` / `emit_service_lifecycle_audit` / `emit_service_batch_audit`
  / `batch_action_to_audit_action` (grep-confirmed: all refs live in the production region). The single
  `tests.rs` is sufficient.
- **Restore glob-resolved imports for the moved tests.** The test block keeps `use super::*;` plus its
  ~15 explicit `use crate::...` imports (which move verbatim). Today `use super::*` also pulls the
  production-level imports of `services.rs` (`std::sync::Arc`, `crate::AppState`, `uuid::Uuid`,
  `serde_json`, axum extractors, etc.). After the split, `super` is the facade, which re-exports only
  the handlers + the `uptrakit-web-api-types` names — the production-level imports move OUT into
  submodules. So for every name the tests previously resolved through the old glob (compile errors
  will name each), add an explicit `use` to `tests.rs` (e.g. `use std::sync::Arc;`,
  `use crate::AppState;`, `use uuid::Uuid;`). Do NOT add per-handler imports (handlers resolve via the
  facade `pub use` glob — an extra import is `unused_imports` → `warnings = deny`). Grep + `cargo check`
  drive the exact set; do not over-import.
- This alone removes the ~1311-line test footprint from the production-scored file.

> **Re-score checkpoint (after Step 2) — a genuine decision point.** Run CodeScene `code_health_score`
> on `mod.rs` once the test footprint is gone. Per the live `code_health_review`, ~95% of the drag is
> the moved tests, so `mod.rs` will **almost certainly land green** here — Step 2 alone recovers nearly
> the entire score. **The per-family production split (Step 3) therefore buys ≈ zero CodeScene score**
> (the handlers carry no flagged smells to redistribute); it is justified PURELY as a churn/cohesion
> judgment on this 128-revision file, NOT by the score. State it as such. Record both numbers.
> **The full 5-module split is the committed deliverable (user decision, grilling) and is NOT gated on
> the score** — proceed with all 5 families even when `mod.rs` re-scores green; a green score does not
> license shipping fewer modules. The ONLY guardrail: if extracting a SPECIFIC family would measurably
> REGRESS a then-green `mod.rs`, leave that one family's code in `mod.rs` and note it. (The genuinely
> optional, score-gated work is Step 6, not Step 3.)
> **Also re-score `tests.rs`** here — it is THE floor-risk file (precedent `messages/tests.rs` = 4.76;
> services' test module is heavier). The `test_state` helper (~209 LoC) is a Large Method but is mock
> setup — acceptable as test code unless `tests.rs` trends red, in which case the table-driven
> assertion lever (Step 4) is the mitigation, not shrinking `test_state`.

### Step 3 — Per-family production submodules

| Submodule      | Items (top-level)                                                                                                                                  | db_access_policy    |
| -------------- | -------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------- |
| `audit.rs`     | `AuditContext<'a>` struct, `emit_service_lifecycle_audit`, `emit_service_batch_audit`, `batch_action_to_audit_action` (all sync; all `pub(super)`) | **none** (all sync) |
| `crud.rs`      | `list_services`, `get_service`, `update_service`                                                                                                   | 3 entries           |
| `lifecycle.rs` | `approve_service`, `reject_service`, `deactivate_service`, `set_update_freeze`                                                                     | 4 entries           |
| `merge.rs`     | `merge_service`                                                                                                                                    | 1 entry             |
| `batch.rs`     | `batch_services`                                                                                                                                   | 1 entry             |
| `tests.rs`     | moved `mod tests` (Step 2)                                                                                                                         | helpers `ignore`    |

Aim: each fn < 70 LoC where reasonable, cc < 9 where reasonable, each file well under brain-class
size. The large handlers (`approve`/`reject`/`deactivate` ~185 LoC each, `update_service` ~168,
`batch_services` ~170, `merge_service` ~151) remain **intact** within their family file — splitting
them is NOT required to meet the goal and risks regression (see Step 6).

> **Audit-action safety.** `services.rs` uses inline `uptrakit_audit_log::AuditActionType::SERVICE_*`
> constants (no local audit-string consts to move), but `batch_action_to_audit_action` maps the string
> literals `"approve"`/`"reject"`/`"deactivate"` → typed actions. Moving it must preserve those literal
> strings byte-for-byte (a one-char typo compiles clean and silently changes batch audit routing).
> `git diff` the moved body and confirm `bash ci/verify_typed_audit_actions.sh` passes.

### Step 4 — Harmless de-dups

- **Test setup helpers are ALREADY factored** (`setup_test_db`, `insert_tenant`, `test_state`,
  `insert_target_and_source`, `latest_tenant_audit_row`, `agent_caps_json`, `insert_service_embedded`)
  — there is no ~19× repeated inline setup block to collapse (that was the `software_items` shape, not
  this one). Do NOT manufacture a dedup that isn't there. If, while moving, any test fn repeats an
  identical multi-line setup verbatim, collapse it into the existing helper — only when behavior is
  provably identical.
- **Audit-assertion bodies (floor lever, conditional):** CodeScene's `tests.rs` floor risk is driven
  by ~14 near-identical audit-assertion bodies (`latest_tenant_audit_row` + assert `action`/`outcome`/
  `details`). If the Step-2 re-score shows `tests.rs` dipping toward red, introduce ONE table-driven
  assertion helper for those bodies — apply only if it keeps `tests.rs` none-red **without changing any
  assertion's meaning**. This is the lever, NOT more setup-dedup.
- **Any genuinely-identical production dup pair** surfaced during the split — collapse only when
  behavior is provably identical (move shared code into the owning submodule; do not invent a
  cross-cutting util).

### Step 5 — Bookkeeping (non-optional gates)

- **`crates/ui/web-api/db_access_policy.toml`** — replace `[routes."services.rs"]` (L588–597) with one
  section per new file path (values preserved verbatim; checker tracks **only `async fn`**):
  - `[routes."services/crud.rs"]`: `list_services = "full-state"`, `get_service = "full-state"`,
    `update_service = "full-state"`
  - `[routes."services/lifecycle.rs"]`: `approve_service = "full-state"`, `reject_service = "full-state"`,
    `deactivate_service = "full-state"`, `set_update_freeze = "full-state"`
  - `[routes."services/merge.rs"]`: `merge_service = "full-state"`
  - `[routes."services/batch.rs"]`: `batch_services = "full-state"`
  - **No `[routes."services/audit.rs"]` section** — all three `audit.rs` fns are sync.
  - After migration, the `[routes."services.rs"]` header must be empty → **DELETE it**.
  - Add `[routes."services/tests.rs"]` listing the moved async test fns/helpers as `"ignore"`
    (convention: `software_items/tests.rs`, `messages/tests.rs` already do this). Re-grep `async fn` in
    `tests.rs` to build the exact list.
  - Verify with `python3 ci/verify_db_access_policy.py`.
- **`AGENTS.md`** — replace the single `crates/ui/web-api/src/routes/services.rs` row (L1335; its
  current description `` `batch_services` handler `` is stale) with one row per new submodule, MD060
  aligned style. Mirror the `software_items` block immediately below (L1337–1346). Suggested rows:
  - `crates/ui/web-api/src/routes/services/mod.rs` — Service route facade (module wiring + handler re-exports)
  - `crates/ui/web-api/src/routes/services/audit.rs` — `AuditContext` + service audit-emit helpers
  - `crates/ui/web-api/src/routes/services/crud.rs` — List / get / update service handlers
  - `crates/ui/web-api/src/routes/services/lifecycle.rs` — Approve / reject / deactivate / set-update-freeze handlers
  - `crates/ui/web-api/src/routes/services/merge.rs` — Service merge handler
  - `crates/ui/web-api/src/routes/services/batch.rs` — Batch service-action handler
  - `crates/ui/web-api/src/routes/services/tests.rs` — Unit tests for the services submodules
  - Run `npx prettier --write AGENTS.md` then `markdownlint --config .markdownlint.json AGENTS.md`.

### Step 6 — Conditional, re-score-gated extraction (LAST — after Step 5 bookkeeping + none-red re-score)

- **Candidate:** `approve_service` / `reject_service` / `deactivate_service` share a near-identical
  skeleton (load service via `TenantDb` → "missing service" denied-audit branch → connectivity
  precondition → `BEGIN IMMEDIATE` tx → status update + inline `emit_stateful` → send
  `ControllerMessage` → `flush_after_commit`). A shared precondition/dispatch helper could DRY them.
  **Caution:** each emits a DISTINCT `SERVICE_*` audit action and sends a DISTINCT `ControllerMessage`
  payload — verified: `approve_service` → `ApprovedPayload`, `reject_service` → `RejectedPayload`,
  `deactivate_service` → `RequestCrlRenewalPayload` (NOT `set_update_freeze`, whose `SetUpdateFreezePayload`
  lives in `lifecycle.rs` but is OUTSIDE this candidate group). Any shared abstraction MUST keep each
  action + payload distinct. Gate on the moved unit tests that assert each handler's specific audit
  action (`approve_service_writes_service_approve_audit_event`, etc.) still passing.
- **Candidate (`audit.rs`):** `emit_service_lifecycle_audit` takes 6 args — the sole flagged production
  smell (CodeScene Excess Function Arguments). It is UNDER clippy's `too_many_arguments` threshold (> 7),
  so no lint forces a change and the snapshot's param-struct rule (`> 7 args`) does not mandate one. IF
  `audit.rs` re-scores yellow on this, bundle the 4–5 call-specific args (`action_type` / `service_id` /
  `service_display` / `outcome` / `details`) into a small param struct — behavior-preserving (same values
  passed), mirrors the existing `AuditContext` pattern. Gate on an `audit.rs` re-score; skip if `audit.rs`
  is already none-red (do not introduce a struct purely to chase green).
- **Gate (apply verbatim):** keep an extraction ONLY if a CodeScene `code_health_score` re-score of the
  affected submodule shows the score improves (or at minimum does not regress) AND the extraction
  introduces **no** new excess-args helper (> 7 args → would need a param struct) and **no** new large
  method. If it redistributes complexity or spawns 5–7-arg helpers, **revert it**. Precedent: the
  analogous `build_enriched_display_overrides` extraction REGRESSED `messages.rs` 7.8 → 7.09 and was
  reverted. Do not repeat that.
- Stop at yellow if green is not free.

## Import & visibility conventions (apply verbatim — hard-won from messages/updates/software_items)

**Handler visibility (CRITICAL — corrected vs. the software_items design text):**

- `crate::router` is a crate-level SIBLING of `crate::routes` and references each handler by full path
  `crate::routes::services::<handler>`. Keep each router-named handler **`pub async fn`** in its
  submodule, and re-export from the facade with a **plain `pub use services_submod::<handler>;`**.
  This reproduces the original public path `crate::routes::services::<handler>`.
- **Do NOT** use `pub(in super::super)` / `pub(super) use` for these 9 handlers — that resolves to
  `pub(in crate::routes)`, invisible to `crate::router`, and yields **E0603**. (The `software_items`
  design _prose_ said `pub(super) use`, but the _shipped_ `software_items/mod.rs` uses plain
  `pub use batch::{__path_batch_software_items, batch_software_items};` — match the shipped code, not
  the prose.)
- **`__path_<handler>` re-export (required):** utoipa `routes!()` generates a `__path_<handler>` type
  per handler; the facade MUST re-export both names:
  `pub use crud::{__path_list_services, list_services, __path_get_service, get_service, __path_update_service, update_service};`
  and likewise for `lifecycle`, `merge`, `batch`. Forgetting any `__path_*` breaks router's
  `routes!(...)` / OpenAPI block. Verify exact names via `cargo check`.
- The re-exported request/response **types** (`ServiceResponse`, `BatchActionRequest`, etc.) stay in
  the facade as the existing `pub use uptrakit_web_api_types::{...}` blocks — they are router-named and
  already maximally public; do not move or narrow them.

**Intra-dir sharing & imports:**

- `audit.rs` items (`AuditContext`, `emit_service_lifecycle_audit`, `emit_service_batch_audit`,
  `batch_action_to_audit_action`) are `pub(super)`; consumers do
  `use super::audit::{AuditContext, emit_service_lifecycle_audit, ...};` (`super` from a submodule
  resolves to the facade, so `super::audit::Foo` reaches a `pub(super)` item directly — no facade
  re-export needed for intra-dir sharing).
- **Never** absolute `crate::routes::...` inside a submodule. Route any needed `crate::routes::<other>`
  name through the facade as a private `use super::super[::super]::<mod>::{...}` re-import; submodules
  consume `super::<Name>`. (A `/verify` pass caught exactly this `crate::routes::` leak on the prior
  job.)
- When moving a body that calls `super::<sibling>::<fn>(...)`, rewrite the now-broken qualified path to
  a bare name imported via `use super::{...}` (or `super::super::<sibling>`). `cargo check` names every
  broken path.
- As each family extracts, **PRUNE now-orphaned direct imports from `mod.rs`** (move them into the
  destination submodule). `unused_imports` is `warnings = deny`. Remember: pruning a facade-private
  `use` can break a moved test's `use super::*` glob — when that happens, add the explicit import to
  `tests.rs` (Step 2).
- A type appearing in a `pub` handler signature (e.g. a `Query<ListServicesQuery>` / `Json<...>`
  extractor, a returned error type) must be ≥ as visible as the handler — `private_interfaces` deny-lint
  forbids narrowing. Keep such types as visible as the compiler forces; do not over-narrow. (Here the
  relevant extractor/response types are all `uptrakit-web-api-types` re-exports, already `pub`.)

## Idiom & standards conformance (`.superpowers/standards-snapshot.md`)

- Lint suppressions stay `#[expect(lint, reason = "...")]` — never bare `#[allow]`, never a NEW
  suppression to dodge a gate. Preserve existing `#[expect]` attrs on moved items verbatim; relocate
  the `mod tests` file-level `#![expect(clippy::let_underscore_must_use, ...)]` to `tests.rs` (Step 2).
- `unreachable_pub = "deny"`: do not leave a handler `pub` if the facade does not re-export it on the
  public chain — the 9 handlers ARE on the chain (router-consumed), so `pub` is correct for them;
  internal helpers stay `pub(super)`/private.
- No `unwrap()`/`expect()`/`panic!()` in production (parking_lot/RwLock + test code excepted); errors
  via `rootcause::Report` / `report!` / `bail!`. This refactor MOVES code, does not author new error
  paths — preserve patterns verbatim.
- `parking_lot::Mutex`, `BEGIN IMMEDIATE` read-then-write txns, `emit_stateful`/`emit_event` audit
  contract, `Validate` impls + handler-entry `validate()` calls, typed permission extractors — all
  preserved as-is in moved code.
- Conventional Commits at workspace level, scope `web-api`. Suggested sequence (each commit compiles +
  passes fmt/clippy): `git mv` + facade skeleton; move tests; extract `audit`; extract `crud`; extract
  `lifecycle`; extract `merge` + `batch`; bookkeeping (db_access_policy + AGENTS.md); (optional) gated
  extraction.

> ⚠ **Commit hygiene (hard-won — supersedes the general `git commit --only <paths>` subagent
> convention FOR THIS JOB):** every commit here introduces NEW untracked submodule files, and
> `git commit --only <dir>` DROPS new untracked files (commits a broken tree). Use `git add <exact
paths>` then a PLAIN `git commit` instead. After EVERY commit verify with `git status --short`
> (clean) AND `git ls-tree HEAD crates/ui/web-api/src/routes/services/` (new file present). The
> `--only` convention still applies to commits that touch only already-tracked files (e.g. the
> bookkeeping-only commit).

## Quality gates (run BOTH feature permutations)

```bash
cargo fmt --all
cargo check  --no-default-features --features db-sqlite
cargo check  --all-features
cargo clippy --all-targets --no-default-features --features db-sqlite
cargo clippy --all-targets --all-features
cargo test   -p uptrakit-web-api --all-features      # 931+ baseline — count must not drop
cargo test   --all-features                          # full workspace — final pre-merge run (canonical gate)
cargo deny check
python3 ci/verify_db_access_policy.py
bash   ci/verify_no_security_audit.sh                # audit-emit code moves across submodules
bash   ci/verify_typed_audit_actions.sh              # batch_action_to_audit_action moved
bash   ci/verify_handler_state_contract.sh           # split restructures handler module layout
python3 ci/check_plugin_semantic_boundary.py         # blocking gate; production code path
markdownlint --config .markdownlint.json '**/*.md'   # AGENTS.md + this spec
# REQUIRED — services approve/reject/deactivate/merge touch service-lifecycle / enrollment /
# merge-rekey / update-freeze / SSE-MQTT flows:
docker build -f docker/Dockerfile.test -t uptrakit-test:latest . \
  && cargo test -p uptrakit-integration-tests -- --ignored
```

Re-score gate: after the split (Steps 1–4), run CodeScene `code_health_score` on `mod.rs` and every
new production submodule + `tests.rs`; confirm none-red. Only then attempt Step 6, re-scoring the
affected submodule to gate each extraction.

## Documentation deliverables

- **`AGENTS.md`** — handler-module table: replace the `services.rs` row with per-submodule rows
  (Step 5). **Required.**
- **`db_access_policy.toml`** — per-file policy sections migrated; empty `[routes."services.rs"]`
  header deleted (Step 5). Tracked gate. **Required.**
- **No ADR** — pure mechanical decomposition under the existing web-api decomposition strategy
  (ADR-0001); no new architectural decision. Behavior-preserving file split following an established
  pattern.
- **No `README.md` / `CONTEXT.md` change** — no externally observable behavior, surface, config, or
  architecture change.

## Sequencing

1. `git mv services.rs services/mod.rs`; facade skeleton (module decls + preserved type re-exports).
2. Move `mod tests` → `tests.rs`; relocate the `#![expect(let_underscore_must_use)]`; restore
   glob-resolved imports; **re-score `mod.rs` + `tests.rs`**.
3. Extract per-family submodules (`audit`, `crud`, `lifecycle`, `merge`, `batch`); wire facade
   re-exports (plain `pub use` of handlers + `__path_*`) + `pub(super)` audit items; prune orphaned
   imports.
4. Apply harmless dedups (only if genuinely present; `tests.rs` assertion-helper lever only if
   floor-risk).
5. Migrate `db_access_policy.toml`; update `AGENTS.md` (`prettier` + markdownlint).
6. **Re-score** `mod.rs` + every production submodule + `tests.rs` → confirm none-red.
7. **Only then** attempt the gated `approve/reject/deactivate` skeleton dedup; keep only if the
   re-score improves or holds with no new excess-args/large-method; otherwise revert.
8. Full quality gates incl. the **required** Docker integration suite.

Execution: subagent-driven in a git worktree; commit per task; review each task (spec compliance +
quality); final whole-branch review + `/verify` before merge.

## Out of scope / deferred

- Decomposing the large intact handlers (`approve`/`reject`/`deactivate`/`update_service`/
  `merge_service`/`batch_services`) beyond the gated Step-6 dedup — high regression risk for no
  required score benefit (messages/updates precedents reverted analogous work).
- Any change to route semantics, request/response JSON, DB queries, audit JSON/outcomes, SSE/MQTT
  payloads.
- Touching sibling route modules beyond the import/visibility wiring needed to keep them compiling.
- Pursuing green at the cost of redistributed complexity or new excess-args helpers.

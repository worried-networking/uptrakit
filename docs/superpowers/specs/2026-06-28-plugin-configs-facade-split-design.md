# Spec: `routes/plugin_configs.rs` → topical-submodule facade split

Status: ready for planning
Date: 2026-06-28
Author: spec workflow (mirrors the just-merged `services.rs` split)
ADR: ADR-0001 (web-api decomposition) — pure mechanical decomposition, no new ADR

## Problem

`crates/ui/web-api/src/routes/plugin_configs.rs` is a single 2981-LoC file carrying
9 route handlers, an audit context + semantic-audit emit helper, command-safety
detection helpers, an async agent-service loader, and a ~1450-LoC inline `#[cfg(test)]`
module. With `services.rs` and `software_items` now split, it is the **highest-friction
remaining web-api route module** and the #1 facade-split target.

Live-verified metrics (main @ `5ccee2c26`, codescene project **81357**):

- `wc -l` = **2981 LoC** (CodeScene's stale hotspot list reports ~3973 / ranks
  software_items+services on top — IGNORE; those merges are unpushed).
- `code_health_score` = **6.16 / YELLOW** (single file).
- Brief's 91 revisions / friction 0.147-rising is consistent with the YELLOW score.

## Goal & success metric

- **Optimized metric**: per-file `code_health_score` (codescene MCP).
- **Floor (hard)**: no resulting file RED (`< 4.0`). A YELLOW production file is acceptable.
- **Aspiration**: facade `mod.rs` + small submodules land GREEN (`>= 9.0`), mirroring
  services (`5.97 -> 9.68`, none-red).
- **Behavior-preserving**: zero change to route semantics, request/response JSON, DB
  queries, audit outcomes/action-strings (plugin-config semantic audit), secret-masking,
  SSE/MQTT payloads. Mechanical decomposition only.
- **Baseline guard**: `cargo test -p uptrakit-web-api --all-features` test count must not
  drop — capture it BEFORE starting.

## Live-code verification (vs. brief)

Confirmed against HEAD:

| Brief claim | Verified |
| --- | --- |
| Single file, no `plugin_configs/` dir | ✅ fresh facade — `git mv` first |
| 9 `pub async fn` handlers | ✅ `list_plugin_types`, `create_plugin_config`, `list_plugin_configs`, `get_plugin_config`, `update_plugin_config`, `delete_plugin_config`, `discover_plugin_config`, `batch_plugin_configs`, `test_plugin_config` |
| `AuditContext<'a>` + `emit_plugin_config_semantic_audit` (~L170/177) | ✅ |
| `load_active_agent_service_for_host` async, policy `ignore` (~L98) | ✅ |
| inline `#[cfg(test)]` module (~L1534 → EOF) | ✅ L1534–2981 |
| TWO production file-level `#![expect]` + one in test mod | ✅ L1 `clippy::expect_used`, L5 `clippy::indexing_slicing`, L1536 `clippy::assertions_on_result_states` (test mod) |

**Correction to brief — `mask_config_secrets` is NOT defined in this file.** It is a
`PluginConfigOps` trait method (`crates/plugins/infrastructure/core/src/plugin_ops.rs`,
carries `#[must_use]`). Tests call it via the local `catalog()` helper. There is therefore
**no facade-private `mask_config_secrets` production item to `pub(super)`-promote** — only
the test-local `catalog()` and the `pub(crate) plugin_field_to_api_field` matter. The
`#[must_use]` attribute lives in the plugins crate and is untouched by this refactor.

**Lint-trigger map (load-bearing for relocating the two production `#![expect]`):**

- `clippy::expect_used` fires at **L62/L64** (`plugin_field_to_api_field`, a `pub(crate)`
  conversion helper) and **L1175** (`format_dangerous_pattern_rejection`,
  `write!(String).expect(...)`). Two different families → two homes.
- `clippy::indexing_slicing` fires at **L800–803** (`update_plugin_config`,
  `serde_json::Value` `["key"]` Index). One family (crud).

**External coupling confirmed:**

- `plugin_field_to_api_field` is `pub(crate)` and consumed by
  `routes/instance_plugins.rs` via `crate::routes::plugin_configs::plugin_field_to_api_field`
  → the facade MUST re-export it `pub(crate) use crud::plugin_field_to_api_field;` to keep
  that path stable.
- `crate::router` references all 9 handlers by full path
  `crate::routes::plugin_configs::<handler>` (utoipa OpenAPI block + `routes!(...)`) and the
  DTOs `PluginTypeInfo / CreatePluginConfigRequest / UpdatePluginConfigRequest /
  PluginConfigResponse` (already facade `pub use` re-exports of `uptrakit-web-api-types`).
- `db_access_policy.toml` `[routes."plugin_configs.rs"]` (L225) lists **10 async-fn
  entries**: 9 handlers + `load_active_agent_service_for_host = "ignore"`.

## Chosen approach

Mirror the `services.rs` split exactly (same route-module shape, same AuditContext/CRUD/batch
families, same incremental-policy migration, same handler-visibility rules). **6 production
submodules + `tests.rs`** (granularity confirmed with user — keeps `audit.rs` small and
isolates the command-safety detection logic, which is one of the two `expect_used` triggers):

```text
routes/plugin_configs/
  mod.rs            facade: module decls + re-exports ONLY (wiring)
  audit.rs          AuditContext<'a>, emit_plugin_config_semantic_audit,
                    dangerous_pattern_matches_to_json, CommandRiskSummary (+ impl)
  command_safety.rs DangerousPatternMatch, collect_dangerous_patterns,
                    format_dangerous_pattern_rejection, detect_command_fields,
                    COMMAND_FIELD_NAMES   [holds expect_used #[expect] for L1175]
  crud.rs           list_plugin_types, create/list/get/update/delete_plugin_config,
                    plugin_field_to_api_field, ListPluginConfigsParams + From impl,
                    descriptor_is_config_model_none, reject_config_model_none_plugin_type
                    [holds expect_used (L62/64) + indexing_slicing (L800) #[expect]]
  discover.rs       discover_plugin_config, AgentHostRow
  batch.rs          batch_plugin_configs
  test_action.rs    test_plugin_config + load_active_agent_service_for_host (its sole caller)
  tests.rs          the #[cfg(test)] module  [holds assertions_on_result_states #[expect]]
```

Placement notes (call-graph **verified against HEAD**):

- `reject_config_model_none_plugin_type` (sync) → `crud.rs`, **`pub(super)`** (certain, not
  conditional): called by `create_plugin_config` L373 + `update_plugin_config` L664 (crud) AND
  by `test_plugin_config` L1376 (test_action). `test_action.rs` consumes it via
  `super::crud::reject_config_model_none_plugin_type`. `test_action.rs` ALSO calls
  `collect_dangerous_patterns` (L1424) + `format_dangerous_pattern_rejection` (L1428) →
  `use super::command_safety::{collect_dangerous_patterns, format_dangerous_pattern_rejection};`.
  So `test_action.rs` imports from TWO siblings (crud + command_safety).
- `descriptor_is_config_model_none` (sync) → `crud.rs`, **private** (crud-internal only):
  callers are `reject_config_model_none_plugin_type` L85 + `list_plugin_types` L308/L315, all
  inside `crud.rs`.
- `CommandRiskSummary::from_config` calls `detect_command_fields` + `collect_dangerous_patterns`
  (command_safety) → `audit.rs` consumes them via `super::command_safety::{…}`; those fns are
  `pub(super)`. `details_fragment` calls `dangerous_pattern_matches_to_json`, which lives WITH
  `CommandRiskSummary` in `audit.rs` (same-module, no promotion).
- **`DangerousPatternMatch` (command_safety.rs): the TYPE and BOTH fields (`field`,
  `description`) need `pub(super)`** — `tests.rs` builds it by struct literal (L2158,
  sibling-module access, private fields insufficient). Same parallel rule as `AuditContext`.
- `ListPluginConfigsParams` is the `Query<…>` extractor type in `list_plugin_configs`'s
  signature → it must be ≥ as visible as the handler (`private_interfaces` deny). Keep its
  current visibility; if `pub`, facade `pub use crud::ListPluginConfigsParams;`.
- `load_active_agent_service_for_host` (async, policy `ignore`) → **`test_action.rs`, private**:
  its sole caller is `test_plugin_config` (L1471), NOT `discover_plugin_config` (which has its own
  inline `AgentHostRow` query). It moves with its caller — no cross-module visibility, no facade
  re-export. Its `db_access_policy` `"ignore"` entry lands in the `test_action.rs` section.

### Step 1 — Facade creation (`git mv`, single commit)

1. `git mv routes/plugin_configs.rs routes/plugin_configs/mod.rs` (history follows).
2. In the SAME commit, rename `db_access_policy.toml` `[routes."plugin_configs.rs"]` →
   `[routes."plugin_configs/mod.rs"]`, keeping all 10 entries verbatim (deferred policy
   migration is REJECTED at the `git mv` commit — services proved this).
3. `cargo fmt --all`; verify build both feature permutations; commit.

### Step 2 — Move the test module FIRST (biggest cheap win)

The ~1450-LoC `#[cfg(test)] mod tests` is the single largest chunk. Move it to
`plugin_configs/tests.rs` as `#[cfg(test)] mod tests;` declared from `mod.rs`.

- Relocate the test module's own `#![expect(clippy::assertions_on_result_states, reason=…)]`
  (L1536) verbatim into `tests.rs`.
- Tests reference handlers (`create_plugin_config`, `update_plugin_config`,
  `delete_plugin_config`, `batch_plugin_configs`) + `plugin_field_to_api_field`. Because the
  facade `pub use`-re-exports the handlers, `use super::*` resolves them — **do not** add
  per-handler imports (redundant import = `warnings=deny`).
- BUT once `mod.rs` becomes wiring-only, the parent's PRIVATE `use` items vanish, so
  `use super::*` no longer pulls them. The moved test file needs explicit imports for exactly
  what `cargo check` reports unresolved (services' `tests.rs` needed `use crate::AppState; use
  std::sync::Arc;` etc.). Add **only** the names the compiler names — no more.
- This test module probes private internals across the sibling boundary (unlike services'
  `use super::*`). It has explicit `use super::<helper>` lines that must be **repathed** to the
  owning submodule once split: `super::detect_command_fields` (L2015), `super::collect_dangerous_
    patterns` (L2110), `super::format_dangerous_pattern_rejection` (L2111), and the `super::
    DangerousPatternMatch` struct literal (L2158) all become `super::command_safety::…`. Those
  four items (+ `DangerousPatternMatch` fields) are the `pub(super)` promotions this requires.
- `plugin_field_to_api_field` is `pub(crate)` re-exported by the facade → test's
  `super::plugin_field_to_api_field` keeps resolving; keep it.
- **In the SAME (Step 2) commit**, add `[routes."plugin_configs/tests.rs"]` to
  `db_access_policy.toml` listing every async test fn/helper as `"ignore"` (generate the list
  mechanically). `verify_db_access_policy.py` runs in the pre-commit hook and catches nested
  async fns — deferring this section would trip the hook on the Step-2 commit AND every
  subsequent commit until it's added. The test-fn list is stable; extraction Steps 3.x do not
  add async fns to `tests.rs`.
- Re-score `mod.rs` AND `tests.rs` (tests.rs is the floor-risk file — score it explicitly).
- The current per-`use` `#[cfg(feature = "db-sqlite")]` gating (L1541–1586) moves verbatim.

### Step 3 — Per-family production submodules

Extract the 6 modules above, ONE submodule per commit. For each:

1. Move the items; add `mod <name>;` to `mod.rs`.
2. Facade re-exports for that module's handlers: **plain** `pub use <name>::{<handler>,
   __path_<handler>};` (both names — utoipa `routes!()`/OpenAPI needs `__path_*`). Verify
   exact `__path_*` names via `cargo check`.
3. Cross-submodule shared items (`AuditContext` + its fields, the emit helper,
   `command_safety` detection fns) → `pub(super)`, consumed as `super::<mod>::<item>`.
   `AuditContext` struct fields need `pub(super)` (certain): `batch.rs` is the sole constructor
   (`AuditContext { … }` L1259) and the sole caller of `emit_plugin_config_semantic_audit`
   (L1271/1288/1308), building it by struct literal across the sibling boundary (identical to
   `services/audit.rs`). `CommandRiskSummary` + `dangerous_pattern_matches_to_json` (also in
   `audit.rs`) are instead consumed by `crud.rs` (create L369/397/482, update L668/686, delete
   L919/935) → both `pub(super)`.
4. Relocate the two production `#![expect]`s by **narrowing to function scope** (idiomatic,
   snapshot rule "smallest scope"; preserve lint + `reason` string verbatim — improving the
   reason text is out of scope):
   - `#[expect(clippy::expect_used, reason="…")]` on `plugin_field_to_api_field` (crud.rs)
     and on `format_dangerous_pattern_rejection` (command_safety.rs).
   - `#[expect(clippy::indexing_slicing, reason="…")]` on `update_plugin_config` (crud.rs),
     OR scoped to the `risk_details[…]` block if narrower fits.
   - Delete both file-level `#![expect]`s from the facade once the triggering code leaves.
   - If a moved module has multiple trigger sites for one lint, annotate **each triggering
     function individually** with `#[expect]` (finest scope that works — a module-level
     `#![expect]` would hide a later-stale suppression because `unfulfilled_lint_expectations`
     only fires when the lint hits nowhere in the module). For this file the trigger map is one
     site per lint per module, so per-function is always achievable. `cargo check --all-features`
     names every stale/missing one.
5. Rewrite any inline `super::<sibling>::<fn>` qualified paths that break once `super`
   rebinds → bare names imported via `use super::{…}` or `super::super::<sibling>`.
6. **Never** `crate::routes::…` from inside a submodule (route through the facade). Other
   `crate::` paths (`crate::AppState`, `crate::queries`, `crate::middleware`, …) are fine.
7. In the SAME commit, MOVE that submodule's async-fn entries from the `mod.rs` policy
   section into a new `[routes."plugin_configs/<name>.rs"]` section. `load_active_agent_
   service_for_host` (async) → `test_action.rs` section as `"ignore"` (moves with its sole
   caller, NOT discover). Sync helpers / `AuditContext` get no entry (checker tracks only
   async fns).
8. After the last extraction the `[routes."plugin_configs/mod.rs"]` section is empty →
   DELETE the header.
9. (The `[routes."plugin_configs/tests.rs"]` section was already added in Step 2 — just
   confirm it stays complete; no async test fns are added during extraction.)

### Step 4 — Harmless de-dups (only provable ones)

- Test-helper dup pairs (e.g. repeated `mask_config_secrets(&PluginTypeId::from_static(...))`
  setup, `latest_tenant_audit_row` / `audit_details` / `create_seed_plugin_config` already
  shared — verify no accidental copies). De-dup ONLY provably-identical pairs.
- Any provably-identical production helper pairs. **Do NOT manufacture a dedup that isn't
  there** (messages/updates lesson).

### Step 5 — Conditional, re-score-gated extraction (LAST)

After the pure split + all bookkeeping pass none-red, attempt faithful extraction on the
worst bumpy roads — chiefly the **225-LoC `update_plugin_config`** brain method (L627–852),
secondarily `create_plugin_config` (~172 LoC) and `test_plugin_config` (~165 LoC).

**Gate each extraction on a codescene `code_health_score` re-score**: KEEP only if the score
improves (or at least holds) WITHOUT adding excess-args (`> 7` → param struct) or new
large methods. On messages.rs and updates.rs the analogous extractions REGRESSED and were
REVERTED — do not repeat. This step is optional; a YELLOW `crud.rs` with the pre-existing
brain method intact is an acceptable outcome if extraction doesn't help. **Do not iterate**:
if a single faithful extraction attempt does not improve the re-score, declare Step 5 done and
accept YELLOW. (`update_plugin_config` threads `tenant_db`/`state`/`existing_config`/`req`/`txn`/
`risk_summary`/`audit_ctx` — far past 7 args — so factoring sub-routines tends to force a param
struct and net-add complexity, exactly the messages/updates regression. The floor — `crud.rs`
~820–870 LoC with one 225-LoC method lands ~5–7 YELLOW, never RED — holds without Step 5.)

## Import & visibility conventions (apply verbatim — hard-won)

- **Handlers**: `pub async fn` in submodule + facade **plain** `pub use sub::<handler>;`.
  Never `pub(super) use` / `pub(in super::super) use` for handlers → resolves to
  `pub(in crate::routes)`, invisible to `crate::router` → **E0603**. (Match shipped
  `services/mod.rs`, which uses plain `pub use`.)
- **`__path_<handler>`**: facade re-exports alongside each handler.
- **Pub-signature types** (`Query` extractor, returned error enum) ≥ handler visibility —
  `private_interfaces` deny forbids narrowing.
- **Intra-dir shared items** → `pub(super)`, via `super::<mod>::<item>`.
- **`plugin_field_to_api_field`** → `pub(crate) use crud::plugin_field_to_api_field;` in the
  facade (external `instance_plugins.rs` consumer).
- Lint attrs stay `#[expect(…, reason=…)]` — never bare `#[allow]`, never a NEW suppression
  to dodge a gate. No `#![allow(unreachable_pub)]`; downgrade unreachable `pub` to
  `pub(crate)`/`pub(super)` as the compiler forces.

## Idiom & standards conformance (`.superpowers/standards-snapshot.md`)

- Facade-split structure, handler `pub use` + `__path_*`, E0603 avoidance, per-submodule
  policy migration, test-glob private-import rule — all directly from
  `project_route_facade_split_gotchas` (snapshot §facade-split-structure). ✅
- Audit action strings unchanged byte-for-byte (snapshot §audit-emit; `verify_typed_audit_
    actions.sh` + `verify_no_security_audit.sh` stay green). ✅
- Lint suppression via `#[expect(reason=…)]`, smallest scope (snapshot §lint-suppression). ✅
- `db_access_policy.toml` per-handler classification preserved (snapshot §db-access). ✅
- No new deps; pure refactor under ADR-0001 (snapshot §facade-split). ✅
- Commit format `refactor(web-api): …`, small granular commits (snapshot §commit-format). ✅

## Quality gates (run BOTH feature permutations; capture test baseline first)

```bash
cargo fmt --all
cargo check       --no-default-features --features db-sqlite
cargo check       --all-features
cargo clippy --all-targets --no-default-features --features db-sqlite
cargo clippy --all-targets --all-features
cargo test  -p uptrakit-web-api --all-features        # count must not drop vs baseline
cargo test  --all-features                            # full workspace (snapshot gate; nothing downstream breaks)
cargo deny check
python3 ci/verify_db_access_policy.py
bash    ci/verify_no_security_audit.sh
bash    ci/verify_typed_audit_actions.sh
bash    ci/verify_handler_state_contract.sh
python3 ci/check_plugin_semantic_boundary.py
markdownlint --config .markdownlint.json '**/*.md'    # only for the AGENTS.md edit
```

**REQUIRED — Docker integration suite.** Moved code touches plugin-config discovery
(`discover_plugin_config` + `load_active_agent_service_for_host`), agent-service dispatch,
secret-masking, and semantic-audit flows → integration suite is non-optional:

```bash
docker build -f docker/Dockerfile.test -t uptrakit-test:latest .
cargo test -p uptrakit-integration-tests -- --ignored
```

## Commit hygiene (verbatim)

- `git commit --only <dir>` DROPS new untracked files. Use `git add <exact paths>` then a
  PLAIN `git commit`. After EVERY commit verify `git status --short` (clean) AND
  `git ls-tree HEAD routes/plugin_configs` (new file present). (This supersedes the general
  `git commit --only` memory rule, which assumes all touched files are already tracked — false
  here, where each extraction adds a NEW submodule file.)
- `cargo fmt --all` BEFORE `git add` (pre-commit hook runs `rustfmt --check` on staged
  routes files; pre-commit ALSO runs `verify_db_access_policy.py` keyed on the file's
  CURRENT path — hence policy moves in the same commit as each code move).
- Per-commit scope: Step 1 (git mv + policy rename); Step 2 (tests.rs); one commit per
  Step-3 submodule (code + policy section together); de-dup commit(s); conditional-extraction
  commit(s) iff re-score passes.

## Documentation deliverables

- **`AGENTS.md`** (root) — the ONLY doc change. Two tables reference plugin_configs:
  1. Replace the `routes/plugin_configs.rs` row in the route/handler-module table with 8 rows
     (`plugin_configs/{mod,audit,command_safety,crud,discover,batch,test_action,tests}.rs`),
     MD060-aligned — mirror the just-merged services rows (L1335–1341).
  2. The batch-handler-registry table row
     `crates/ui/web-api/src/routes/plugin_configs.rs | batch_plugin_configs handler` (L1357)
     updates its path to `routes/plugin_configs/batch.rs`.
  Run `npx prettier --write AGENTS.md` then `markdownlint` after editing.
- **No ADR / no README / no CONTEXT change** — pure mechanical decomposition under ADR-0001.
  Surface a deviation only if something non-obvious arises during impl.

## Sequencing

1. Capture `cargo test -p uptrakit-web-api --all-features` baseline count + `code_health_
   score` of the pre-split file (6.16).
2. Step 1 — facade `git mv` + policy rename (commit).
3. Step 2 — `tests.rs` (commit) → re-score `mod.rs` + `tests.rs`.
4. Step 3 — 6 submodule extractions, one commit each, policy section moved in-commit; delete
   empty `mod.rs` policy header after the last.
5. Step 4 — harmless de-dups (commit, only if provable).
6. Step 5 — conditional re-score-gated extraction (commit only if score holds/improves).
7. AGENTS.md doc update (commit).
8. Full gate sweep incl. Docker integration suite; re-score every resulting file (floor: none
   RED; target: GREEN facade + small modules).

## Out of scope / deferred

- Refactoring query-layer `crates/ui/web-api-queries/src/queries/plugin_configs.rs` (different
  file, not in scope).
- Changing any audit action string, request/response DTO, masking behavior, or DB query.
- Improving the verbatim-relocated `#![expect]` reason strings.
- Any non-`update_plugin_config` brain-method extraction beyond what the re-score gate keeps.
- New ADR / CONTEXT / README content.

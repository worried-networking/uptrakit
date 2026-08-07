# M1.8 — Delete `Permission` — Design

Date: 2026-08-07. Verified against HEAD `8ffad6032` (branch `main`).

Milestone context: `.superpowers/authn-and-authz-refactoring/` — [10-milestones.md](../../../.superpowers/authn-and-authz-refactoring/10-milestones.md)
§Milestone 1 internal staging step (4), [11-task-breakdown.md](../../../.superpowers/authn-and-authz-refactoring/11-task-breakdown.md) §M1.8.
Depends on M1.4a/b–M1.7, all landed (verified below). M1.9 (docs + ADR) follows this task and owns every
`docs/`-tree rewrite.

## Goal

One commit that removes the last compiled remnants of the legacy `Permission` authorization model:
the `Permission` enum (incl. `Other(String)`), its re-export chain, the temporary `Permission` → `Action`
mapping shim, the legacy test fixtures that seed `permissions`/`role_permissions` rows, and a migration
dropping the `permissions` and `role_permissions` tables. Plus one durability improvement approved by the
owner: the `ci/verify_action_security_declarations.py` leniency for "unconverted" operations becomes a
hard error so the deleted `x-required-permission` extension cannot silently reappear.

Non-goal: any `docs/` rewrite, the root `AGENTS.md` MUST-FOLLOW update, the ADR — all M1.9.

## Verified current state (what M1.4–M1.7 already removed)

Re-derived by grep on 2026-08-07 — the plan must re-run these at plan-write time (tree may move):

- `permission_extractor!` — one Rust hit left: the shim's own doc comment
  (`crates/ui/controller-core/src/access/shim.rs:13`), which dies with the file in §B. Remaining
  occurrences are `docs/`, `website/`, and the scoped `crates/ui/web-api/AGENTS.md` narrative
  (M1.9 territory except the one tense fix below).
- `get_user_permissions` — **zero Rust hits**; only docs + `CHANGELOG.md` (excluded as history).
- `x-required-permission` — zero route/handler sites. Remaining code-tree hits: a retirement-narrative
  doc comment in `crates/ui/web-api/src/middleware/action.rs:4-7` (reworded here) and the CI checker +
  its test (`ci/verify_action_security_declarations.py`, `ci/test_verify_action_security_declarations.py`)
  which legitimately carry the literal as ban enforcement (precedent: `ci/verify_no_security_audit.sh`).
- Generated artifacts are already clean: `crates/ui/web-api/openapi.json`, `crates/shared/wire/asyncapi.yaml`,
  and `frontend/src/lib/api/generated/` contain **zero** `Permission` occurrences. This task therefore
  changes **no** generated artifact; `./scripts/regen-api.sh` is run once as a **no-op verification**
  (empty `git status` afterwards), never as an expected-red.
- The code-defined preset permission lists named in the task breakdown are already gone (M1.6b);
  `crates/shared/types/src/role_bundle.rs` is role-name-based and contains no `Permission` reference —
  it stays.

## Deletion inventory (site classes, per repo-wide grep)

Every range below starts at the item's doc comment and includes attributes. The plan re-runs the
inventory greps and diffs against this list before writing tasks.

### A. The enum and its re-export chain

| Site | Action |
| --- | --- |
| `crates/shared/types/src/permissions.rs` (335 lines: enum, `Other`, serde, `PartialSchema`/`ToSchema`) | delete file |
| `crates/shared/types/src/lib.rs` — `pub mod permissions;` + `pub use permissions::Permission;` | delete lines |
| `crates/shared/web-api-types/src/permissions.rs` (3-line re-export) | delete file |
| `crates/shared/web-api-types/src/lib.rs` — `pub mod permissions;` + test-module block (§1 "Permission enum round-trip", ~lines 92–210) + `let _ = Permission::ViewSettings;` in the compile-use test (~line 785) | delete lines/blocks |
| `crates/shared/web-api-types/src/prelude.rs:59` — `pub use crate::permissions::Permission;` | delete line |
| `crates/ui/web-api-auth/src/auth/permissions.rs` (1-line re-export) + `auth/mod.rs:11` `pub mod permissions;` | delete file + decl |

`strum` stays in both `shared/types` and `web-api-types` manifests (other live consumers verified:
`plugin_role.rs`, `registration.rs`, etc.). No `Cargo.toml` changes anywhere.

### B. The mapping shim

`crates/ui/controller-core/src/access/shim.rs` (whole file, incl. its strum guard test) +
`access/mod.rs:30` `pub mod shim;`. Verified: **zero** non-test consumers of `actions_for_permission`.

### C. DB entities and core-table registration

| Site | Action |
| --- | --- |
| `crates/shared/db/src/entity/permission.rs`, `entity/role_permission.rs` | delete files |
| `entity/mod.rs:36,42` — module decls | delete lines |
| `entity/prelude.rs:69,77` — `Permission`/`PermissionModel`, `RolePermission`/`RolePermissionModel` re-exports | delete lines |
| `entity/role.rs` — delete exactly three items, by symbol: the `Relation::RolePermissions` variant (with its `#[sea_orm(has_many)]` attr), the `impl Related<super::role_permission::Entity> for Entity`, and the `impl Related<super::permission::Entity> for Entity` (the via/via_rev one). The `Related<super::user_role::Entity>` and `Related<super::user::Entity>` impls sit between/around them and **must survive** | delete items |
| `crates/shared/db/src/migrate_core_tables.rs:184,187` — the two `CoreTableDescriptor` rows | delete lines |

### D. Legacy test fixtures and their call sites

`seed_permissions_for_owner` is a vestige: enforcement reads `access_grants`; seeding
`permissions`/`role_permissions` rows has no authorization effect since M1.4–M1.7. Deleting the calls
leaves tests green because the first registered user holds all seed roles/grants (owner bootstrap).

- `crates/ui/web-api/src/test_harness/fixtures.rs` — the fn (~lines 234–290) + `permission`,
  `role_permission` entries in the import at line 19.
- Call sites in `crates/ui/web-api/src/routes/`: `update_batches.rs` (8), `autodiscovery.rs` (11),
  `discovery_allowlist.rs` (5), and `crates/ui/web-api/src/integration_tests/notifications.rs` (1) —
  counts as of today; the plan deletes **every** hit of a fresh
  `rg -n 'seed_permissions_for_owner' crates/` plus the now-unused import names, never a line-range copy
  of this table.
- Twin in `crates/core/integration-tests/tests/database_helpers/fixtures.rs` (fn at ~line 151 + import
  line 8 pruning) and its one call in `tests/database/notifications.rs` (+ import).

### E. Migration-adjacent test surgery

Historical migrations under `crates/shared/db/src/migration/` are **frozen** — no seed migration is
edited, renamed, or deleted. Only test code around them changes:

1. `m20260728_000001_access_grants_and_role_scope.rs`
   - `rebuild_preserves_role_ids_and_assignments`: currently ends with `Migrator::up(&db, None)` — after
     this task that would run the drop and red the `row_count("role_permissions") == perms_before`
     assertion. Do **not** hardcode `Some(2)`: the test's own "(this one + the seed migration)" comment
     is stale (`up(None)` applies three today — `m20260728_000002` and `m20260803` included), and the
     sibling round-trip test's comment explicitly bans hardcoded step counts. Instead compute the bound
     by name, mirroring the file's `migration_index()` idiom: look up the drop migration's position and
     `Migrator::up(&db, Some(drop_index - migration_index()))` — "run everything except the drop",
     drift-proof against future insertions. Assertions stay byte-identical; fix the stale comment in
     the same edit.
   - `up_down_up_round_trips`: the two `row_count(&db, "role_permissions") > 0` assertions (post-down
     ~line 701, post-re-up ~line 708) are re-pointed — and the test must not silently lose its
     down-direction child-parking **data-survival** guard (post-drop, `role_permissions` is empty on the
     down chain, so parking it proves nothing). New shape: after the initial full `up`, seed one user +
     `user_roles` assignment (mirror the insert idiom of `rebuild_preserves_role_ids_and_assignments` —
     a fresh migrated DB has **zero** `user_roles` rows, so seeding is required for the assertion to be
     non-vacuous); post-down assert `role_permissions` **exists** (sqlite_master lookup, same idiom as
     the `access_grants` absence check above it — schema-only-recreated by the new migration's `down()`)
     **and** `row_count("user_roles")` still equals 1 (`user_roles` is the surviving
     `child_parking_plan()` entry, so it now carries the parking data-survival guard); post-re-up assert
     `permissions` **and** `role_permissions` are **absent** (pins the drop at tip) and the `user_roles`
     row survives.
2. `migration/mod.rs` tests — **retired guards deleted** (owner decision 2026-08-07: a guard over frozen
   seed literals whose tip-state invariant is retired guards nothing that can drift):
   - `role_permissions_entity_query_succeeds` (~line 988) — delete.
   - `manage_commands_assigned_to_command_manager` (~line 935) and
     `manage_commands_not_assigned_to_viewer_role` (~line 1568) — delete.
   - `migrations_run_on_empty_sqlite` — delete only the **three** permissions-table assertion blocks
     (manage_commands created-and-assigned; granular permission names exist; old coarse names absent —
     ~lines 810–935); the rest of the test survives (assert a sentinel neighbor assertion survives
     after the edit).
   - Repair tests **kept, re-pointed** (the repairs still run on fresh installs): in
     `repair_migration_fixes_text_uuid_storage` drop the trailing `role_permission::Entity::find()`
     decode (the existing `typeof(id) = 'blob'` asserts already prove the repair); in
     `repair_migration_fixes_created_at_format` replace the `entity::permission` decode with a
     sea_query-typed `Query::select()` of `created_at` (mirror the `owner_role_id` select idiom in the
     same test — **not** `query_one_raw`, which is reserved for SQLite-specific functions with an
     inline justification) + a `time::OffsetDateTime` RFC 3339 parse assertion (the entity API dies
     with the entity; the format guarantee is what the repair provides).
   - Prune the now-unused `role_permission` name from the test-module import at ~line 461.
3. `crates/core/controller-runtime/src/db_migrate/mod.rs` — descriptor rows removed in C make the copy
   loop skip the dropped tables; update the stale seed-inventory comment at ~line 221
   ("… 9 permissions, role_permission links …").
4. `crates/core/integration-tests/tests/database/migrations.rs` — delete the "built-in permissions
   should exist after migrations" block (lines ~29–35) and the two `assert_queryable!` lines (61–62);
   no import pruning here — the only import in play is the blanket `use uptrakit_shared_db::entity::*;`
   (line 56), shared by the surviving entities. Cross-backend absence pinning lives in the new
   migration's own tests plus
   the round-trip re-point; the Docker suite's job here is proving the drop migration **runs** on
   Postgres.

### F. Comment/prose rewords (attached prose is in scope; grep cleanliness)

- `crates/ui/web-api/src/middleware/action.rs:4-7` — reword so the module doc no longer carries the
  literal `x-required-permission` (describe the native-security model in present tense).
- `crates/shared/types/src/access/action.rs:130` — "Divergence from the `Permission` schema" comment:
  reword **removing the capitalized token** (e.g. "the legacy permission schema") — done-when grep 1
  demands zero, so a faithful reword keeping the backticked name would red it.
- `crates/ui/web-api/src/integration_tests/access_management.rs:41-42` — module-doc reference to "the
  wire `Permission` vocabulary retirement": reword without the capitalized symbol.
- `frontend/tests/e2e/auth.test.ts:12-15` — fixture comment "none of which were real `Permission`
  variants": reword to lowercase prose ("legacy permission variants").
- `frontend/openapi-ts.config.ts:7` — inline comment "the old Permission-union rationale is gone
  (M1.7)": reword to lowercase prose (config behavior unchanged).
- `crates/ui/mcp/tests/get_current_user_mcp.rs:693` — the **one** discriminator-test doc comment
  ("legacy role_permissions still grant access_mcp…") describes tables that no longer exist at tip;
  reword to current semantics (grant row deleted → engine denies). The sibling OAuth-variant comment
  references the test by name only and needs nothing. Test bodies unchanged — they stage via
  `user_roles` + `access_grants` only.
- `crates/ui/web-api/AGENTS.md` — two edits: line 73 tense fix ("`Permission` itself and its backing
  tables are removed in M1.8" → past tense), and line 120's fixture-helper example list drops
  `seed_permissions_for_owner` (deleted in §D; the scoped guide must not cite a dead helper — it is
  outside M1.9's named doc list and outside the done-when greps, so nothing else catches it).
  (Root `AGENTS.md` MUST-FOLLOW rewrite stays in M1.9.)

## The drop migration

New file `crates/shared/db/src/migration/m20260807_000001_drop_permissions_tables.rs`, registered at
the end of the migrator list.

- **up()**: `drop_table(role_permissions)` first (FK child), then `drop_table(permissions)`. sea_query
  builders only; backend-agnostic.
- **down()**: schema-only recreation of both tables, **`permissions` first, then `role_permissions`**
  (parent before FK child — the reverse of `up()`; wrong order fails FK creation on PostgreSQL, and
  nothing else would catch it: every down-chain test is `sqlite::memory:` with `foreign_keys=false`).
  Copy the column/FK shape from the initial migration `m20260209_000001_initial.rs` —
  `permissions(id uuid PK, name UNIQUE, description NULL, created_at)`; `role_permissions(role_id,
  permission_id)` composite PK, FKs to `roles.id`/`permissions.id` both `ON DELETE CASCADE`. Neither
  table has any index beyond PK/UNIQUE — do not invent one. The `role_id`/`permission_id` column names
  are a **contract**: m20260728's `copy_columns` parking selects them by literal name. Doc comment
  states data is **not** restored (destructive up, documented per the migration rules).
  Schema-only recreation is **forced**, not stylistic: the m20260728 down-path parks `role_permissions`
  rows via `role_permissions_mig_bak` during its **SQLite** roles-table recreation — a pure no-op
  `down()` here would break every down-chain crossing it (`up_down_up_round_trips` runs exactly that
  chain). The forcing constraint is SQLite-only (`rescope_roles_postgres` uses plain `ALTER TABLE` and
  never touches `role_permissions`); recreation is merely harmless on PG.
- **In-file tests** (mirror the `migration_index()` idiom from
  `m20260513_000006_oauth_controller_instances.rs`, copied verbatim at plan time): after full `up` both
  tables absent (sqlite_master); after `down(Some(1))` both present and empty; FK check passes.
- **Postgres `down()` coverage** (owner-requested follow-up, 2026-08-08): the in-file tests are
  SQLite-only, and PostgreSQL validates FK targets at creation time — so the `down()` recreation order
  is additionally proven by a Docker-suite test (`crates/core/integration-tests/tests/database/
  migrations.rs`, `db_test!` both backends: tip → `down(Some(1))` → tables exist empty → re-up →
  dropped). Lands as its own `test(db):` commit after the single M1.8 commit — the one-commit rule
  scopes the deletion itself.
- Fresh installs run the full chain: create → seed → … → drop. The three seed migrations write into
  tables that exist at their point in the sequence; the backfill migration (`m20260803`) reads only
  `roles` + `access_grants` (verified) — no ordering hazard.
- Deployment note: the single live deployment (SQLite, single owner) loses the legacy rows by design —
  sanctioned by the milestone's hard-cutover rules; `access_grants` has been the enforcement source
  since M1.4–M1.7. The real downgrade cost is stronger than lost rows: sea-orm-migration errors on an
  applied-but-missing migration file, so **any pre-M1.8 controller binary refuses to boot** against an
  upgraded database (recovery = restore the DB file). Copy the SQLite file (controller stopped) before
  deploying this release; the `BREAKING CHANGE:` footer states the refuses-to-boot consequence, not
  just the data loss.

## CI checker hardening (owner-approved 2026-08-07)

`ci/verify_action_security_declarations.py` currently **ignores** "unconverted" operations
(`bearer_token` + `x-required-permission`) — a leniency branch that is dead after M1.4b and would let
the extension silently reappear. The leniency's actual shape (verified): inside the per-operation loop,
`if not uses_action_module:` only checks the R3 oauth2-mix case then `continue`s — so an operation
carrying the extension in a file with **no** action-module import passes today. Change: ban any
`x-required-permission` occurrence **inside the `_iter_operations` loop, before/regardless of the
`uses_action_module` gate** (a new rule id or a widened R3 — placement is the load-bearing part; a
check folded into the post-gate branch misses exactly the regression shape that matters). Scope
honesty: the checker sees utoipa path attrs under the routes tree — the extension cannot reappear on
any utoipa operation; a repo-wide literal grep gate stays deferred (see Deferred). Update the
checker's module docstring — it currently documents the leniency at length ("Unconverted operations …
are ignored") and would misdescribe the new behavior. Rule-id choice is **pinned**: widen R3 by
**replacing** the pre-gate branch (`verify_action_security_declarations.py:231-234`), not adding a new
rule beside it. Three existing tests carry no-action-import fixtures with the extension and must be
re-dispositioned in the same edit — improvising this mid-implementation is not left open:
`test_legacy_file_using_can_update_hosts_without_action_import_is_clean` (line 303, asserts clean)
**inverts into the RED case** (its `middleware::permission` parser-trap rationale died with M1.7 —
rewrite its comment); `test_zero_converted_operations_in_legacy_only_file` (line 279, asserts clean)
inverts likewise; `test_mixed_x_required_permission_and_oauth2_in_legacy_file` (line 223, asserts
exactly one R3 violation) stays at exactly one R3-prefixed violation under the widened-replace shape.
Empty-input stays green. Keep the new rule message and fixtures to the lowercase
`x-required-permission` literal — `ci/` currently has zero `\bPermission\b` hits and done-when grep 1
scans `ci/`. Gate:
`python3 -m unittest ci/test_verify_action_security_declarations.py` in-task — the checker's own test
suite runs in CI, so an arity/behavior change must land with its tests in the same commit.

## Done-when (exit criteria)

Grep criteria — mechanical commands, residual classes enumerated:

1. `rg -n '\bPermission\b' crates/ frontend/ scripts/ xtask/ ci/ -g '!**/CHANGELOG.md' -g '!**/CODEREVIEW.md' -g '!**/AGENTS.md' -g '!crates/shared/db/src/migration/*'`
   → remaining hits only in the three POSIX file-permission files
   (`crates/shared/directories/src/lib.rs`, `crates/shared/service-sdk/src/dirs.rs` — "Permission
   hardening" prose; `crates/core/agent-ssh-runtime/src/operations/bootstrap.rs` — the `"Permission
   denied"` stderr match). Excluding those three paths as well, the count is **zero**. Historical
   migrations are excluded as frozen history; `docs/`, `website/`, and `AGENTS.md` files are excluded
   as documentation (the scoped `crates/ui/web-api/AGENTS.md` keeps a past-tense historical mention
   after this task's tense fix; M1.9 owns the full doc sweep). The excluded `CODEREVIEW.md` hits
   (`crates/ui/`, `crates/ui/web-api/`) are frozen review artifacts — never edited by this task or
   M1.9.
2. `rg -n 'permission_extractor!' crates/ frontend/ scripts/ xtask/ ci/ -g '!**/AGENTS.md'` → zero
   (the scoped `crates/ui/web-api/AGENTS.md` keeps its historical narrative until M1.9; the shim's
   doc-comment hit dies with the shim).
3. `rg -n 'x-required-permission' crates/ frontend/ scripts/ xtask/ -g '!**/AGENTS.md' -g '!**/CHANGELOG.md'`
   → zero (the `ci/` checker + its test keep the literal as ban enforcement and are excluded by scope;
   the scoped AGENTS.md narrative is M1.9's; the two controller `CHANGELOG.md`s carry it as release
   history and are never edited).

Build/test gates (single commit, tree green):

- `cargo fmt --all`
- `cargo clippy --all-targets --no-default-features --features db-sqlite`
- `cd frontend && npm run build`, then `cargo clippy --all-targets --all-features`
- `cargo test --all-features` (the canonical full-suite gate) plus
  `cargo test --no-default-features --features db-sqlite` (the pre-push hook's leaner world — run
  both to satisfy the milestone's "green on both canonical feature sets" exit line; `--all-targets`
  clippy is mandatory — plain `cargo check` skips `#[cfg(test)]` modules where most of this task's
  edits live)
- `python3 -m unittest ci/test_verify_action_security_declarations.py`, then
  `python3 ci/verify_action_security_declarations.py` against the real route tree (direct proof the
  hardened rule does not false-positive on the converted routes)
- `python3 ci/verify_db_access_policy.py` (no handler changes — expected unchanged)
- `cargo deny check` (unconditional in CI and pre-push; no manifest changes expected)
- `cd frontend && npm run lint && npm run format:check && npm run check` (the two edited `.ts` files
  are outside the vite build; these are their canonical gates)
- `./scripts/regen-api.sh` as no-op verification: `git status` clean afterwards (requires
  `frontend/node_modules`; run `npm ci` first in a fresh checkout)
- Docker DB integration suite (migration change trigger, both backends):
  `cargo test -p uptrakit-integration-tests --test database -- --ignored` — export `DOCKER_HOST` from
  `docker context inspect --format '{{.Endpoints.docker.Host}}'` first (colima)
- `markdownlint --config .markdownlint.json` scoped to the Markdown files this task edits (the repo
  glob is chronically red on gitignored scratch — flake-triage guidance; the pre-commit hook runs the
  staged-file-scoped equivalent) and `bash ci/verify_agents_md_budget.sh` (the scoped AGENTS.md edit
  is under the size-budget gate)

Commit: one commit, `refactor(auth)!: delete the legacy Permission model and its tables` — body names
the dropped tables + deleted enum/shim/fixtures and the checker hardening; `BREAKING CHANGE:` footer
stating that controller binaries older than this release refuse to boot against an upgraded database
(applied-but-missing migration error; recovery = DB-file restore) in addition to the schema drop; plus
the standard co-author/session trailers.

## Alternatives considered

- **No-op `down()`** for the drop migration — rejected: breaks the m20260728 down-chain child-parking
  (evidence above); schema-only recreation is the minimal working reversal.
- **Rebind retired seed-guard tests to bounded `up(Some(N))`** instead of deleting — rejected by owner.
  The load-bearing argument is coverage-class survival, not file frozenness: the "after all migrations,
  role X holds capability Y" class stays covered at tip for the **new** model by
  `m20260728_000002::seed_content_matches_expected_table` and
  `m20260803_000001::backfill_grants_mcp_use_to_access_mcp_roles_only` (both run `up(&db, None)` and
  assert grant content verbatim); the deleted guards asserted the same class for the retired model.
- **Defer checker hardening to a later cleanup** — rejected by owner: the rg-clean exit criterion would
  be one-shot instead of CI-durable.
- **Splitting into staged commits** — excluded by the milestone: steps 1–3 of the staging choreography
  already landed (M1.3–M1.7); M1.8 is defined as the single final deletion commit.

## Doc deliverables

- In this task: the comment/prose rewords of §F (including the one-line tense fix in
  `crates/ui/web-api/AGENTS.md`) and the new migration's doc comments. No other doc changes —
  explicitly deferred: `docs/security/auth-and-authorization.md`, `docs/end-user/user-management.md`,
  `docs/api/user-management.md`, root `AGENTS.md` MUST-FOLLOW rules, the model-replacement ADR, and
  every remaining `docs/`/`website/` mention of `Permission`/`permission_extractor!`/
  `x-required-permission` — all assigned to M1.9 by the task breakdown; touching them here would
  collide with that task.
- No OpenAPI/AsyncAPI/frontend-artifact changes (verified zero occurrences; regen is a no-op check).

## Deferred / out of scope

- M1.9 in full (docs rewrite + ADR + root `AGENTS.md`).
- A durable grep gate against reintroduction of `Permission`/`permission_extractor!`/the entities
  (ten-line addition to an existing `ci/verify_*` script, `verify_no_security_audit.sh` precedent).
  The in-task checker hardening covers only the OpenAPI extension; done-when criteria 1–2 stay
  one-shot greps. Candidate for M1.9 alongside its AGENTS.md rule rewrite — recorded, not silently
  dropped.
- Any `access_grants`/engine behavior change — this task deletes dead code and dead tables only; every
  enforcement path already reads the engine.
- New dependencies: none.

## Snapshot conformance

Checked against `.superpowers/standards-snapshot.md` (2026-08-07): no raw SQL (sea_query builders in
the migration; the one raw `typeof()` probe pre-exists in a frozen test context and is not touched
beyond deletion); migration naming + `down()` + no-entity-imports rules respected; no `#[allow()]`;
no new dependencies; batch/tenant rules untouched (dropped tables were global); Conventional Commit
with `!` + footer; tests cover success and failure paths (migration in-file tests + round-trip
re-point pin both directions). Ledger rows applied: 20 (site-class sweep incl. data literals and
`.txt`/`.py` surfaces — none found keyed to deleted paths), 21 (doc-comment-inclusive delete ranges;
`--all-targets` compile sweep), 30 (down-chain/FK parking analysis; core-table deregistration;
sibling-test inventory across crates), 35 (checker change lands with its own unittest + RED case),
51 (artifact presence verified before any regen claim — zero occurrences, so no expected-red exists).

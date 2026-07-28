# M1.2 — Grant Storage and Seeds (`uptrakit-shared-db`)

Date: 2026-07-28. Status: approved design, pending plan.

Second task of the authn/authz refactoring Milestone 1 (sources of truth:
`.superpowers/authn-and-authz-refactoring/`, esp. `06-grant-model.md` §Storage schema / §Seed roles /
§Validation summary, `09-resolved-questions.md` §Grant model, `11-task-breakdown.md` §M1.2,
`12-test-plan.md` §B; and the M1.1 spec
`docs/superpowers/specs/2026-07-28-access-types-core-design.md`, whose type surface this spec consumes).
Owner-settled decisions are applied, not reopened. **Implementation sequencing**: the M1.1 module must
be landed first — this spec's query module and migration consume `uptrakit_shared_types::access`
(`ActionPattern`, `Selector`, `validate_patterns`, `actions::*_STR`).

## Problem / goal

Give the new access model its storage: the `access_grants` table + an engine-owned query module in
`uptrakit-shared-db`, `roles.tenant_id` with per-scope uniqueness, and a migration attaching the seed
roles' grants to the existing role rows. Behavior-invisible until M1.3+ (nothing reads the table yet);
`permissions`/`role_permissions` stay untouched (drop is M1.8).

## Decisions locked during grilling (owner, 2026-07-28)

1. **Seed widenings confirmed (both).** `settings_manager` gains `settings:read` and `operator` gains
   `services:read` — the visibility-trap fixes 06 flags for owner confirmation ("not silently added").
   Confirmed here; test E11's assertions are in scope.
2. **Query-module scope: grants only.** Role-CRUD query helpers land with their consumer (M1.6a
   management API). M1.2 ships validated grant writes + the batched resolution read.
3. **Validation lives in the query module.** Rules 1–2 of `06-grant-model.md` §Validation summary plus
   the M1 phase gate (test row B9: any non-`All` selector rejected until M2) are enforced inside the
   engine-owned module on every write — single choke point; M1.6a handlers and any other caller route
   through it and cannot bypass.

## Findings that amend the upstream text

- **06's uniqueness phrasing is unimplementable as written.** "`(tenant_id, name)` with NULL
  participating as the global scope" — NULL ≠ NULL in unique indexes on both SQLite and Postgres, so a
  single composite unique index admits duplicate global role names. Implemented instead as a partial
  unique index **pair** (see §Migrations).
- **The courtesy `user_roles` name remap is vacuous.** The live `roles` table already contains exactly
  the eight target role names (`viewer`, `operator`, `service_manager`, `software_manager`,
  `host_manager`, `settings_manager`, `command_manager`, `system_administrator` — seeded by
  `m20260310_000002_granular_permissions.rs`, verified 2026-07-28). Role rows are reused in place: ids
  stable, `user_roles` untouched, "unmatched assignments dropped" has no members. A test asserts
  assignment preservation instead of remapping machinery.
- **`subject_type` is a typed enum column, not an open string.** 06 says "string in DB like
  `channel_type`", but `channel_type` is deliberately open (plugin-extensible) while `user`/`role` is a
  closed set — the coding-standards fixed-string-enum rule wins: `DeriveActiveEnum`
  (`rs_type = "String"`, `db_type = "Text"`; precedent `crates/shared/db/src/entity/scheduled_task.rs`).
  Storage form is still TEXT, satisfying 06's intent.

## Scope

In: `uptrakit-shared-db` entity + query module + two migrations + tests. Out (deferred to the named
tasks): `AccessEngine`/cache/wire invalidation (M1.3), grant/role management API + role-CRUD query
fns + Stateful audit actions + lockout guard (M1.6a), catalog endpoint (M1.6b), selector rules 3–5 and
non-`All` acceptance (M2.1), `permissions`/`role_permissions` drop + `Permission` deletion (M1.8),
canonical docs + ADR (M1.9).

## Schema

### `access_grants` (new table)

Per `06-grant-model.md` §Storage schema:

| Column | Type | Notes |
| --- | --- | --- |
| `id` | UUID PK | `Uuid::now_v7()`, `auto_increment = false` |
| `tenant_id` | UUID NULL | FK → `tenants.id`; NULL = global grant (see encoding rules) |
| `subject_type` | TEXT | `GrantSubjectType` enum: `user` \| `role` (`DeriveActiveEnum`) |
| `subject_id` | UUID | **no FK** — polymorphic across `users.id`/`roles.id` by `subject_type`; documented in rustdoc |
| `patterns` | JSON | array of pattern strings; entity field `serde_json::Value`, typed at the query-module boundary |
| `selector` | JSON | `Selector` tagged JSON (`{"type":"all"}`, …); entity field `serde_json::Value`, same boundary rule |
| `description` | TEXT NULL | operator-facing "why", ≤ `bounds::MAX_GRANT_DESCRIPTION_LEN` (enforced in module) |
| `created_at` / `updated_at` | timestamptz | `time::OffsetDateTime` |
| `created_by` | UUID NULL | NULL for seed rows |

Indexes: `(subject_type, subject_id)` and `tenant_id` (plain, non-unique). JSON columns follow the
repo idiom (raw `serde_json::Value` on the entity — `FromJsonQueryResult` has zero in-repo usage and
is not introduced); the engine-owned module is the only reader/writer and converts to/from
`Vec<ActionPattern>` / `Selector` fail-closed.

Entity `crates/shared/db/src/entity/access_grant.rs` deliberately does **not** implement
`TenantScoped` (mixed NULL/tenant rows break the non-null tenant filter contract) — rustdoc states the
engine-owned access rule from 06: no generic entity access; all reads/writes via the query module.

### `roles` changes

- Add `tenant_id` UUID NULL (FK → `tenants.id`): NULL = global (the eight built-ins), non-NULL =
  tenant-defined custom role (creatable from M1.6a).
- Replace the column-level `UNIQUE(name)` (from `string_uniq(Roles::Name)` in the initial migration)
  with the per-scope pair:
  - `CREATE UNIQUE INDEX uix_roles_global_name ON roles (name) WHERE tenant_id IS NULL`
  - `CREATE UNIQUE INDEX uix_roles_tenant_name ON roles (tenant_id, name) WHERE tenant_id IS NOT NULL`
- Entity `role.rs`: `tenant_id: Option<Uuid>` added, `#[sea_orm(unique)]` dropped from `name`.
  `is_built_in` and existing relations survive unchanged.

## Migrations (two, both in `crates/shared/db/src/migration/`)

Naming follows the `mYYYYMMDD_NNNNNN_<slug>.rs` convention, registered in the migration `mod.rs`
Migrator list after `m20260727_000001_plugin_type_id_grammar`.

**Migration 1 — schema** (`…_access_grants_and_role_scope`):

1. Create `access_grants` via sea_query builders (no raw SQL).
2. Rebuild `roles` per the table-recreation guide in
   `docs/development/database-migrations.md` (SQLite cannot drop a column-level UNIQUE constraint):
   create `roles_new` without the name constraint and with `tenant_id` NULL, copy rows with typed
   round-trips (ids/timestamps preserved — never via `String`), swap, recreate FKs/indexes. Copy uses
   one accumulated `Query::insert()` guarded by `!rows.is_empty()` (never per-row inserts — batch
   invariant; empty `INSERT` is a syntax error on both backends).
3. Create the two partial unique indexes via `execute_unprepared` raw SQL with the documented
   limitation comment — the sanctioned idiom
   (`m20260309_000003_host_tags.rs`: "SQLite does not support partial indexes via sea_query's
   `.and_where()`").
4. `down()`: drop `access_grants`, rebuild `roles` back (recreating `UNIQUE(name)`), drop the partial
   indexes.

**Migration 2 — seeds** (`…_seed_access_grants`):

One grant row per (role, 06 seed-table line), all with `subject_type = role`, `tenant_id = NULL`,
`selector = {"type":"all"}`, `created_by = NULL`. Pattern strings:

| Role | Patterns |
| --- | --- |
| `viewer` | `*:read` |
| `operator` | `services:read`, `services:approve`, `services:reject`, `hosts:read`, `checks:trigger`, `updates:trigger` |
| `service_manager` | `services:*` |
| `software_manager` | `software:*`, `hosts:read`, `checks:trigger`, `updates:trigger`, `scheduler:manage`, `discovery.ignores:manage`, `plugin-configs:trigger` |
| `host_manager` | `hosts:*`, `hosts.tags:manage` |
| `settings_manager` | `settings:read`, `settings.*:manage`, `notifications:*`, `audit:read`, `users:manage`, `access:manage` |
| `command_manager` | `commands:manage`, `plugin-configs:trigger` |
| `system_administrator` | `system.*:*` |

(The table restates `06-grant-model.md` §Seed roles verbatim, including the two owner-confirmed
widenings; that section stays normative — re-diff at plan time.)

Mechanics:

- Concrete-action strings reference `uptrakit_shared_types::access::actions::*_STR` consts
  (compile-checked, per `09-resolved-questions.md` §Action model); wildcard/subtree patterns
  (`*:read`, `services:*`, `settings.*:manage`, `system.*:*`, …) are string literals — no consts
  exist for patterns by design.
- **Self-validating**: before any insert, every seed string is parsed via
  `ActionPattern::from_str` and the per-role lists run through `validate_patterns`; any failure
  aborts the migration with a `DbErr` naming the offending string. Fail-closed at migration time,
  independent of the guard test.
- Role ids resolved by name lookup against the live `roles` table (check-then-insert idempotency,
  `m20260310_000002_granular_permissions.rs` idiom — ON CONFLICT is avoided there for backend
  parity); a missing role name aborts (cannot happen in sequence — Migration 1 of this pair follows
  `m20260310`, which seeds all eight — but fail-closed anyway).
- `down()`: delete the seeded grant rows (identified by `subject_type = 'role'` +
  `created_by IS NULL` + the eight role ids).
- No audit emission (migrations are not handlers; `audit-catalog.toml` untouched). No
  OpenAPI/asyncapi impact (no endpoints, no wire types).

## Engine-owned query module

`crates/shared/db/src/access_grants.rs` (crate-root module beside `provider_settings.rs`), with typed
error + alias per the error-handling boundary rule:

```rust
pub enum AccessGrantError { /* thiserror variants: Validation, TenantEncoding,
    SelectorPhaseGate, CorruptRow, Db(...), NotFound, ... */ }
pub type Result<T> = std::result::Result<T, rootcause::Report<AccessGrantError>>;
```

(exact variant set at implementer's discretion; every fn in the module returns this alias, including
reads.)

Types:

```rust
pub struct ResolvedGrant {
    pub id: Uuid,
    pub tenant_id: Option<Uuid>,
    pub subject: GrantSubject,          // User(Uuid) | Role(Uuid)
    pub patterns: Vec<ActionPattern>,   // typed, parsed fail-closed
    pub selector: Selector,
}

pub struct NewGrant<'a> { /* subject, tenant_id, patterns: &'a [ActionPattern],
    selector: Selector, description: Option<String>, created_by: Option<Uuid> */ }
```

Functions (M1.2 surface — M1.6a adds nothing to storage, only handlers):

- `insert_grant(db, NewGrant) -> Result<Uuid>` / `update_grant(db, id, …)` / `delete_grant(db, id)`.
  Every write enforces, in order:
  1. **Rule 1** — `validate_patterns` (count bound + per-pattern `can_match_any`).
  2. **Rule 2 (single encoding per subject type)** — any `system.`-reaching pattern ⇒
     `tenant_id IS NULL` (and tenant grants contain no `system.` patterns); **role**-subject ⇒
     `tenant_id IS NULL` always; **user**-subject tenant-plane ⇒ `tenant_id` non-NULL. A
     `system.`-reaching pattern is one whose resource side is `Exact`/`Subtree` literally starting
     with `system` (the `*` wildcard never reaches the system plane — M1.1 matching semantics).
  3. **B9 phase gate** — `selector != Selector::All` ⇒ typed error (`SelectorPhaseGate`), until M2.1
     replaces this arm with rules 3–5. The arm carries a comment naming M2.1 as the removal site.
  4. Bounds — description length (`MAX_GRANT_DESCRIPTION_LEN`), selector `validate()` (inert while
     B9 holds, kept so M2.1 only deletes the gate arm), `MAX_GRANTS_PER_SUBJECT` on insert (count
     query per subject).
- `load_grants_for_principal(db, tenant_id: Uuid, user_id: Uuid, role_ids: &[Uuid]) -> Result<Vec<ResolvedGrant>>`
  — **one** query (batch invariant, no N+1): `Condition::any()` over
  (`subject_type = user` ∧ `subject_id = user_id` ∧ (`tenant_id = ?` ∨ `tenant_id IS NULL`)) ∪
  (`subject_type = role` ∧ `subject_id IS IN role_ids`). Role-subject rows are always
  `tenant_id NULL` by rule 2, and tenant scoping comes from the caller's `user_roles`-derived
  `role_ids` — restating 06's safety argument in the fn's rustdoc.
  Rows parse fail-closed: an unparseable `patterns`/`selector` JSON or unknown pattern string maps to
  `CorruptRow` (the whole call errors — a deny — never a silent row skip).
- `load_grant(db, id)` for M1.6a's read-before-update; nothing else.

No generic entity access anywhere: the module doc states the 06 engine-owned contract. Structural
enforcement is **convention + rustdoc, not visibility** — `lib.rs` declares `pub mod entity;`, so
`access_grant::Entity` is public like every sibling (verified 2026-07-28; hiding it would fight the
crate's structure and SeaORM's needs). The entity's rustdoc carries a prominent "engine-owned — use
`access_grants::…`, never `Entity::find()`" warning; the mechanical gate for this rule is the
deferred `verify_db_access_policy.py` extension, whose trigger 09 already records
(`09-resolved-questions.md` §Decision engine #3).

## Tests (SQLite in-crate; Postgres via the existing integration harness)

Migration + schema:

- Fresh `Migrator::up`: both migrations apply; `roles` keeps eight rows with original ids
  (capture ids pre/post rebuild); a pre-seeded `user_roles` row survives untouched (assignment
  preservation — the "remap" test).
- Uniqueness both directions per index: duplicate global name rejected; same name global + tenant
  accepted; duplicate `(tenant, name)` rejected; two different tenants may reuse a name.
- `down()` round-trip: `up → down → up` green (guards the table-recreation reversal).

Seeds (assert content, not counts):

- Every 06 seed-table row present verbatim (per-role pattern-set equality against the spec table);
  `settings_manager` contains `settings:read` and `operator` contains `services:read` (E11's
  assertions, owner-confirmed); all seed rows `subject_type = role`, `tenant_id NULL`,
  `selector = All`.
- Every seeded pattern string round-trips through `ActionPattern::from_str` + `can_match_any`
  (guard against catalog drift).

Query module (B rows owned by M1.2):

- B1 (All-selector subset): valid user-subject tenant grant and role-subject global grant insert +
  load back typed.
- B2: `system.*` patterns with `tenant_id = NULL` accepted.
- B3: `system.` pattern with non-NULL tenant rejected; tenant-plane user-subject grant with NULL
  tenant rejected — both typed errors.
- B9: each non-`All` selector variant rejected with `SelectorPhaseGate`.
- B11: global role-subject grant with tenant-plane patterns legal; `load_grants_for_principal`
  returns it when the role id is in `role_ids` and not otherwise; user-subject NULL-tenant
  tenant-plane still rejected.
- Resolution: one-query shape (assert via result correctness across the union cases — direct user +
  role + global user rows in one call; foreign user's/role's rows absent); `CorruptRow` on a
  hand-inserted bad JSON row (test writes malformed JSON via the entity directly — in-crate test may
  bypass the module precisely to prove the module's fail-closed read).
- `MAX_GRANTS_PER_SUBJECT` enforced at the boundary (insert #201 for one subject rejected;
  count-1 at bound accepted — seed via batch insert, not a 200-iteration loop of module calls).

Postgres: schema + seed + uniqueness coverage rides
`cargo test -p uptrakit-integration-tests --test database -- --ignored` (the migration/REST Docker
suite — 11's "migration runs on SQLite + Postgres" done-when). Time-dependent tests: none expected;
if any test touches `tokio::time`, `start_paused` per the standing rule — DB tests must NOT use
`start_paused`.

## Verification gates

Crate-scoped lanes **must be derived at plan time by running them at baseline** (ledger discipline:
shared-db's test code is feature-gated on `migration`, and scoped `-p … --all-targets` invocations
have historically mis-fired on this crate — name the exact green commands in the plan, don't
pattern-guess). Expected shape: clippy + test on `-p uptrakit-shared-db --features migration,db-sqlite` (the crate
declares its own `db-sqlite`/`db-postgres` features — `db_error.rs` cfg-gates on them, and the
ledger records exactly this crate mis-firing under partial feature sets) plus the crate's full
feature union, then the canonical workspace lanes (db-sqlite pair, `--all-features`
check/clippy/test with `frontend/build` prerequisite), `cargo deny check` (no manifest change
anticipated — `rootcause` is already the crate's error idiom, verified 2026-07-28:
`provider_settings.rs`/`raw_settings.rs`/`migrate_core_tables.rs` all use `Report<E>` aliases;
sea-orm/serde_json/uuid/time all present), `verify_db_access_policy.py` (routes-scoped; no route
changes — must pass untouched), `cargo xtask audit-coverage-check` (no new emit sites — passes
untouched).
Commit scope: copy from `git log --oneline -- crates/shared/db` at plan time.

## Documentation deliverables

- Rustdoc: entity (engine-owned contract, no-FK rationale, single-encoding rule), query module
  (resolution safety argument, B9 gate + M2.1 removal pointer), both migrations (limitation comments
  on the raw partial-index SQL; table-recreation rationale).
- **No canonical doc/ADR/CONTEXT.md updates in M1.2 — deliberate deferral**: storage is invisible
  until M1.3+ reads it; `docs/security/auth-and-authorization.md` + ADR + vocabulary land in M1.9 per
  the milestone plan. `docs/development/database-migrations.md` needs no edit (both raw-SQL uses fall
  under its existing documented exception class).

## Alternatives considered

- **Single composite unique `(tenant_id, name)`** — broken on both backends (NULL ≠ NULL); partial
  pair chosen.
- **New role rows + `user_roles` remap** (09's literal phrasing) — pointless churn: live table already
  has the exact eight names; in-place reuse keeps ids and assignments.
- **Open-string `subject_type`** (06's "like channel_type") — closed two-value set; typed
  `DeriveActiveEnum` per coding standards.
- **Typed JSON columns via `FromJsonQueryResult`** — zero in-repo usage; repo idiom is
  `serde_json::Value` + boundary typing, and the engine-owned module is already the single boundary.
- **Validation in M1.6a handlers** — rejected (grilling decision 3): second writers could bypass;
  the module is the choke point.
- **Seed strings all as consts** — impossible: wildcard/subtree patterns have no consts; the
  migration self-validates every string instead.

## Deferred / out of scope (verbatim carriers)

`AccessEngine` + resolution cache + `AccessInvalidated` wire variant (M1.3), grant/role management
API + role-CRUD query fns + Stateful grant/role audit actions + lockout guard re-target (M1.6a),
catalog endpoint + preset retirement (M1.6b), selector validation rules 3–5 + non-`All` write-path
acceptance + `TargetRef` matching (M2.1), `permissions`/`role_permissions` drop + `Permission`
deletion + shim removal (M1.8), canonical docs + ADR + CONTEXT.md vocabulary (M1.9),
`verify_db_access_policy.py` engine-owned-exemption extension (deferred upstream with its trigger,
`09-resolved-questions.md` §Decision engine).

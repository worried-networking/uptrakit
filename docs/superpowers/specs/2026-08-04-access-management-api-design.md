# M1.6a — Grant/Role Management API — Design

Task M1.6a of the authn/authz refactoring
(`.superpowers/authn-and-authz-refactoring/11-task-breakdown.md`). Delivers the management surface for
the greenfield authorization model: grant CRUD, role CRUD, role assignment re-gating, the
`users:manage`/`access:manage` split, the re-targeted lockout guard, and the Stateful audit actions.
Depends on M1.3 (landed); coordinates with M1.4b/M1.5 by owning `users.rs`/`roles.rs` conversion.

Tree state this design was verified against (2026-08-04): M1.1–M1.4a landed and M1.4b/M1.5 in
progress — `uptrakit-shared-types::access` (catalog, patterns, selectors, bounds), engine-owned
`uptrakit-shared-db::access_grants` query module with write validation (incl. the B9 non-`All`
selector phase gate), `AccessEngine` + `ControllerMessage::AccessInvalidated`, `action_extractor!` +
`ci/verify_action_security_declarations.py`, 44 route families converted, and the surfaces resolution
path already enforcing through an inline `engine.authorize` call (`routes/surfaces.rs`). M1.5's
remaining inline conversions (`interactive_ws.rs`, `plugin_type_settings.rs`, `system_services.rs`)
are not M1.6a's scope and run in parallel. `users.rs`, `roles.rs`, `access_presets.rs` still on
`permission_extractor!`. No grant routes exist; `roles.rs` is read-only and embeds legacy permission
names in `RoleResponse`.

## Owner decisions (grilling round, 2026-08-04) and corpus deviations

All decided with the owner on 2026-08-04; each deviation from the design corpus is listed here so the
corpus files stay untouched (the M1.9 ADR captures the final state):

1. **Role list/get gate = `access:manage` only** (not `users:manage`-OR). The whole roles + grants
   surface is one authority-administration domain.
2. **No batch endpoints in M1.6a** — deviation from task text and test row E1's "batch" leg. Deferred
   until a consumer exists. E1 is satisfied minus its batch clause.
3. **CLI commands deferred to M1.6b**; M1.6a ships API + `uptrakit-openapi-client` modules only
   (client sync is a standing invariant).
4. **`access_presets.rs` legacy lockout-guard copy stays untouched** — accepted inconsistent-guard
   window until M1.6b deletes the preset endpoints. Documented gap, owner sign-off.
5. **OIDC role sync is guarded** (see below) — closes a verified lockout bypass.
6. **New catalog action `system.access:manage`** — closes a verified tenant→system escalation and makes
   E6 well-defined. Vocabulary addition inside the still-open M1 freeze window.
7. **Lockout guard mechanics amended** versus `06-grant-model.md` §Lockout prevention — the doc's
   JSON-`LIKE` pre-filter is a Postgres runtime error on the `JsonBinary` (JSONB) column, and
   `lock_exclusive()` on the candidate-grant query serializes the wrong rows (concurrent deactivations
   contend on `users`, not `access_grants`). Amended shape below, owner-approved.

## API surface

All new/changed operations declare native security requirements
(`security(("oauth2" = ["<action>"]), ("developer_token" = []))`) and use `action_extractor!` structs.
New extractors: `CanManageAccess` (`access:manage`), `CanManageUsers` (`users:manage` — same struct name
as the legacy permission extractor, different module; no file imports both). The conditional
system-plane requirement (below) is a handler-body fine check via plane classification +
`AccessEngine::authorize` on the request path. Rule basis: the design corpus restates the
authorization invariant as "coarse gate via extractor always; fine checks only via engine /
visible-query calls, never hand-rolled" (`07-decision-and-enforcement.md` §REST route extractors);
a request-body-dependent requirement cannot be a `FromRequestParts` extractor, and the landed
precedent is the surfaces resolution path's inline `engine.authorize` (`routes/surfaces.rs`). Every
fine-check site carries a greppable marker comment (`// APPROVED: body-dependent fine check (corpus
07, restated invariant)`) extending the documented custom-auth-path exception convention
(`docs/development/coding-standards.md` §"Approved exception: custom authentication paths" — which
mandates a `// APPROVED:` marker no shipped code instantiates verbatim yet) until M1.9 restates the
rule for the engine model. **CI-gate visibility, decided**: the fine-check action is intentionally absent from
the operation's static `oauth2` scope list (`ci/verify_action_security_declarations.py` matches
extractors in signatures only); the conditional requirement is stated in the operation description —
the same documented runtime-valued exception class as `x-action-dynamic`, carried into the M1.9
security-doc rewrite.

### Grant CRUD — new routes `/api/v1/access/grants`

| Op     | Path                                | Gate                                                       |
| ------ | ----------------------------------- | ---------------------------------------------------------- |
| Create | `POST /api/v1/access/grants`        | `access:manage` (+ system-plane fine check)                |
| List   | `GET /api/v1/access/grants`         | `access:manage`                                            |
| Get    | `GET /api/v1/access/grants/{id}`    | `access:manage`                                            |
| Update | `PUT /api/v1/access/grants/{id}`    | `access:manage` (+ system-plane fine check, lockout guard) |
| Delete | `DELETE /api/v1/access/grants/{id}` | `access:manage` (+ system-plane fine check, lockout guard) |

- Request/response types in `uptrakit-web-api-types`, `Validate` impls bounded by the existing
  `access::bounds` constants; handlers use the `Validated<T>` extractor.
- All persistence through the engine-owned `uptrakit-shared-db::access_grants` module (the entity is
  `pub(crate)` and CI-guarded; the table's tenant-mixed rows are the documented engine-owned exemption
  from the `TenantDb` invariant — `06-grant-model.md` §Storage schema). Create maps to `insert_grant`; update to `update_grant`
  (`GrantUpdate` — subject and `tenant_id` immutable; re-subject/re-scope is delete + create, stated in
  the API docs); the module's existing validation (pattern parse + matrix, plane purity, tenant-encoding
  rule 2, B9 selector phase gate, bounds) is the write-path validation — handlers add nothing on top.
- Tenant scope: user-subject tenant-plane grants get `tenant_id = active tenant` from `AccessContext`;
  system-plane and role-subject grants are `tenant_id NULL` per encoding rule 2. The request carries no
  free-form tenant id.
- List returns the active tenant's rows plus global (`tenant_id NULL`) rows, optionally filtered by
  `subject_type`/`subject_id` query params (`IntoParams` struct per ADR-0025). No pagination — bounded
  by ≤ 200 rows/subject and the deployment's scale; revisit with M2 tooling.

### Role CRUD — extend `/api/v1/roles`

| Op     | Path                        | Gate                                                       |
| ------ | --------------------------- | ---------------------------------------------------------- |
| List   | `GET /api/v1/roles`         | `access:manage` (was `users:manage`)                       |
| Get    | `GET /api/v1/roles/{id}`    | `access:manage` (was `users:manage`)                       |
| Create | `POST /api/v1/roles`        | `access:manage`                                            |
| Update | `PUT /api/v1/roles/{id}`    | `access:manage`                                            |
| Delete | `DELETE /api/v1/roles/{id}` | `access:manage` (+ system-plane fine check, lockout guard) |

- Create: custom role with `tenant_id = active tenant`, `is_built_in = false`; fields `name`
  (1–64 chars, kebab/word validation consistent with existing role names) and optional `description`
  (≤ 500 chars, mirroring `MAX_GRANT_DESCRIPTION_LEN`).
- Update: `name`/`description` only; `tenant_id` and `is_built_in` immutable.
- Built-in immutability (test B10): update/delete of an `is_built_in` role → **409** with reason code
  `built_in_role_immutable` (assignment stays allowed). 409-with-reason-code matches the lockout-denial
  idiom; 403 would misreport an authorization failure.
- **Name-shadowing rejection**: create/rename rejects a name equal to any global (`tenant_id NULL`)
  role's name — **409** with reason code `role_name_shadows_global` — on top of the per-scope
  uniqueness indexes. See the shadowing fix below for why.
- Delete (custom roles only): one transaction deletes the role row, its `user_roles` rows, and its
  role-subject `access_grants` rows (**no FK cascades from `access_grants.subject_id`** — the entity
  doc names M1.6a as the owner of this cleanup). Shrinking mutation → lockout guard.
- `RoleResponse` re-shaped: `id`, `name`, `description`, `is_built_in`, `tenant_id`, `created_at` —
  the legacy `permissions` name list is dropped (verified: no non-generated frontend consumer; plan
  re-verifies before landing). Role grants are visible through the grant list filtered by subject.

### Role assignment and the `users:manage`/`access:manage` split (`users.rs`)

`users.rs` converts off `permission_extractor!` wholesale (M1.4b hands this file to M1.6a):

| Handler                                        | New gate                                                                               |
| ---------------------------------------------- | -------------------------------------------------------------------------------------- |
| `list_users`, `get_user`                       | `users:manage`                                                                         |
| `update_user_active`                           | `users:manage` (+ lockout guard on deactivate, both planes)                            |
| `update_profile`                               | self-or-`users:manage` — path-dependent, so inline `engine.authorize` + marker comment |
| `list_permissions` (`GET /api/v1/permissions`) | `users:manage`; endpoint dies in M1.8 (pointer note)                                   |
| `update_user_roles` (`PUT /users/{id}/roles`)  | **`access:manage`** (+ system-plane fine check, lockout guard)                         |

- `update_profile` keeps its existing self-or-admin semantics (the inline `ManageUsers` check is
  live code today — no widening); its operation declares **empty** scope lists (authenticated-only
  shape), which `ci/verify_action_security_declarations.py` accepts for a handler with no action
  extractor, with the self-or-`users:manage` rule stated in the operation description.
- Assignment keeps full-replace semantics. The lockout guard evaluates the **post-state** role set —
  never per-removal (a request swapping covering role A for covering role B is legal). No
  "new ⊇ old" short-circuit: the guard always runs on this endpoint; it is cheap.
- Assignment gains the missing invalidation (verified gap: today the endpoint invalidates nothing).

## New catalog action: `system.access:manage`

Closes a verified escalation: `access:manage` alone would let any tenant admin `POST` a grant
`{patterns: ["system.*:*"], tenant_id: null}` for themselves — the plane boundary bypassed in one
request. K3's A12 wildcard exclusion blocks _using_ `*:*` against system endpoints, not _minting_
system-plane authority.

- Catalog entry: resource `system.access` (`SystemAccess`), verb `manage`, `SelectorSupport::None`,
  description "Manage system-plane grants and role assignments conferring system-plane authority".
  The `system_administrator` seed (`system.*:*`) covers it automatically — no seed change (subtree
  `system.*` matches the dotted resource `system.access`).
- **Enforcement (fine check after the `CanManageAccess` coarse gate)**, required when:
  - grant create/update where the written patterns classify system-plane (the query module's
    plane classifier already exists);
  - grant delete/update where the **existing** row's patterns classify system-plane;
  - role delete where the role holds system-plane grants;
  - role assignment (`update_user_roles` and OIDC sync) where the post-state **adds** a role whose
    grants reach the system plane (assigning `system_administrator` is conferring system authority).
- Not required for: tenant-plane grant/role operations, role create (no grants yet), rename/description
  updates.
- Known accepted gap (pre-existing, not introduced here): OIDC role _mapping_ config
  (`settings.auth:manage`) can map an IdP group to a system-plane role — config-time escalation
  direction. Recorded as a future K-row; unchanged behavior today.

## Lockout guard (amended shape)

Replaces `count_other_manage_users_holders` + `roles_grant_manage_users` in `users.rs` (the
`access_presets.rs` copies stay per decision 4). Invariant: no mutation may leave zero active users
whose resolved authority covers `access:manage` @ `All` in the affected tenant scope (tenant plane),
nor zero active users whose global authority covers `system.access:manage` (global plane, E6).

**Guarded (shrinking) mutations**: grant update, grant delete, role delete, `update_user_roles`
(always, post-state), `update_user_active` when deactivating, OIDC role sync (whenever the
post-state role set differs from the pre-state set — only an exact no-op sync skips the guard and
takes **no lock**, keeping the common login path lock-free; see the OIDC section below for why the
pure-add exemption was dropped). **Every guarded mutation
evaluates both planes** — grant delete/update and role delete can drop the last system-plane holder
just as deactivation can; the pre-filter is cheap at this scale.
**Skipped (adding-only)**: grant create, role create, role rename/description update, user activation —
under allow-only union these cannot shrink authority (E5).

**Placement**: the guard (sentinel lock helper, candidate loaders, post-state confirm) lives in the
engine-owned `uptrakit-shared-db::access_grants` module — `ConnectionTrait`-generic, beside the
validation it complements, and reachable from both consumers (`uptrakit-web-api` handlers and the
`uptrakit-web-api-auth` OIDC sync; `web-api-auth` depends on `shared-db` but not on `web-api`).

Mechanics, in order, inside one transaction with the mutation and its Stateful audit emit:

1. **Serialize**: `begin_with_options` + `SqliteTransactionMode::Immediate` (house idiom,
   `device_flow.rs`), then `SELECT … FOR UPDATE` via `lock_exclusive()` — the workspace's only prior
   row-lock use is `queries/services.rs::merge_service`; the mechanism is reused here but aimed at
   **one global sentinel row**, not at the mutated rows (the corpus's candidate-query locking is
   exactly what decision 7 rejects). The sentinel is the **default tenant's `tenants` row for every
   guarded mutation, both planes** — not a per-tenant row: role-subject grants are `tenant_id NULL`
   and a role can be assigned in multiple tenants, so per-tenant sentinels would let a role-grant
   delete and a same-role unassignment in another tenant slip past each other; a single row also
   erases every lock-ordering question. Guarded mutations are rare admin operations — contention on
   one row is irrelevant. Concurrent shrinks genuinely serialize on Postgres READ COMMITTED (the
   corpus shape locked `access_grants` rows while deactivation races contend on `users` — verified
   wrong). **A sentinel lookup returning no row is a hard error (500), never a pass-through**: on
   Postgres a zero-row `FOR UPDATE` locks nothing (and on SQLite sea_query drops the lock clause
   entirely — `Immediate` alone serializes there, so only the Postgres leg ever exercises the row
   lock).
2. **SQL pre-filter, typed columns only** (no JSON operators): two non-outer-join queries loading
   candidate grant rows — (a) user-subject grants in the affected scope joined to active users,
   (b) role-subject grants joined through `user_roles` (affected tenant scope; every assigned tenant
   for role-subject mutations) to active users. Engine-owned
   queries in the `access_grants` module, generic over `ConnectionTrait`, running on the guard's
   transaction.
3. **In-memory candidate filter + confirm**: keep rows with selector `All` whose patterns match the
   closed candidate set — tenant plane `{access:manage, access:*, *:manage, *:*}`, global plane
   `{system.access:manage, system.access:*, system.*:manage, system.*:*}` (`*` never matches the
   system plane) — then simulate the mutation's post-state (drop the deleted row / substitute the
   updated row's content / apply the post-state role set / exclude the deactivating user) and
   confirm coverage via `ActionPattern::matches`. **The tenant-plane invariant is per tenant**:
   group post-state candidates by tenant scope and require ≥ 1 covering active user in **every**
   affected tenant (a role-subject mutation affects each tenant holding an assignment — a global
   "≥ 1 anywhere" would let tenant B lose its last holder while tenant A still has one). The global
   plane is a single check.
   **Prohibition: the guard never calls `AccessEngine`** — its moka cache, pool-connection reads, and
   scope intersection all escape the transaction and under-count holders.
4. On violation: rollback, audit `Denied` entry, **409** with reason code only
   (`lockout_access_manage` / `lockout_system_access`) — never holder identities or counts (a
   `users:manage`-only caller receives this 409 and must not learn access-plane state).

Completeness tripwires: the tenant set is closed because no `Subtree` pattern matches a root
resource and `access` is dot-free; the global set is closed because `system.access` has exactly one
subtree stem (`system.*`) and `*` never matches the system plane. Rather than trusting that prose,
the unit test **derives** each candidate set from the guarded action's resource string (exact
resource + every dot-prefix stem's subtree form + `Any` where plane-admissible, crossed with
exact-verb/`*`) and asserts equality with the hardcoded sets, with a comment naming the guard as the
dependent — a future resource rename or grammar extension fails loudly.

Guard-scope future obligations (rustdoc on the guard): user hard-delete, tenant deactivation, and any
admin credential-reset endpoints do not exist today (verified); each becomes a guarded (or
account-takeover-relevant) mutation the day it is built. The `users:manage` catalog description is
trimmed to the capabilities that exist (activate/deactivate, lifecycle reads).

## OIDC role-sync guard

`sync_oidc_roles` (`web-api-auth`, full-replace of `user_roles` on every OIDC login) is a verified
guard bypass: an IdP group change could strip the last `access:manage` holder at next login, with no
recovery path. Decision: **guard the sync**.

- The sync runs the same post-state check on its transaction **whenever the post-state role set
  differs from the pre-state set**; only an exact-match (no-op) sync — the overwhelmingly common
  login — takes no guard, no sentinel lock, and stays lock-free. (An earlier draft of this bullet
  also exempted pure-add syncs. That shortcut was dropped during implementation: deciding
  whether-to-lock from a read taken _before_ the sentinel lock is an unguarded-shrink hole the
  moment the serialization property is the deployment's rather than the code's. A pure-add set
  now reaches the guard, which re-reads authority state under the lock and returns `Permitted`.)
  On violation the sync **keeps the existing
  assignment unchanged**, lets the login complete, and an audit **Event**
  (`user_role.sync_lockout_prevented`) is emitted naming the provider and the attempted role set —
  the login never fails.
- The system-plane fine check does not apply here (no principal to hold `system.access:manage` — the
  mapping config is the trust anchor; see the accepted-gap note above).
- Layering: `sync_oidc_roles` returns a typed outcome (`Applied`/`SkippedLockout`/`NoChange`); the
  HTTP-layer caller in `uptrakit-web-api` emits the audit Event and, on `Applied`, performs the
  post-commit invalidation + `AccessInvalidated` publish (`web-api-auth` has no audit/NATS
  dependency — the outcome-driven split keeps it that way). Two plumbing facts the implementation
  must honor: the sync's signature carries only the active `tenant_id`, so the **default tenant id
  (sentinel key) must be passed in**; and when the guard does engage, the pre-state role set is
  **re-read inside the transaction** — the lock-free skip decision's read is advisory only.
- **Audit scope, explicit**: sync-**applied** role changes remain un-audited in v1, as today — a
  Stateful `user_role.update` here would require in-transaction emission from `web-api-auth`, which
  has no audit-log dependency, and the pre-existing sync has never audited. Documented gap; trigger
  to close: any move of the sync into a crate with the audit emitter, or an operator need for
  IdP-driven assignment history. Only the lockout Event and the invalidation are new.

## Role-name shadowing fix

Verified defect class triggered by custom-role creation: four resolvers look roles up **by name with
no scope filter** — `assign_viewer_role` (`routes/auth.rs`), the role lookup in `sync_oidc_roles`
(`authentication.rs`), `assign_owner_roles` (`routes/auth.rs` — the actual `Role::find()` site behind
`handle_first_user_setup`, which itself contains no role query; patch the callee, not the caller),
and the default-role lookup in `resolve_oidc_user` (`authentication.rs`, `Name.eq("user")`). A tenant role legally named `viewer` under per-scope
uniqueness would be silently assigned to every self-registered user (`.one()` on two rows is
arbitrary), or doubly assigned by the sync. The `resolve_oidc_user` case is nastier: `"user"` is a
**deleted** legacy role name (dropped by the granular-permissions migration), so the lookup is dead
today — but the shadow-rejection below only blocks collisions with **existing global** names, so a
custom tenant role named `user` would resurrect the lookup and auto-assign that role to every new
OIDC user. Fix in the same task, defense in depth:

1. Role create/rename rejects names colliding with global role names (validation above).
2. All four resolvers gain `tenant_id.is_null()` filters — the resolvers are the actual defect;
   validation alone is not relied on. The scoped `resolve_oidc_user` lookup stays permanently inert
   (no global `user` role exists), preserving today's behavior.

## Engine-owned query additions (`uptrakit-shared-db::access_grants`)

The module's surface today is insert/update/delete/load/load-for-principal. New exports, all
`&impl ConnectionTrait`:

- `list_grants(scope, subject filter)` — management list (tenant rows + global rows).
- Guard candidate loaders — the two typed pre-filter queries above.
- `delete_grants_for_role(role_id)` — role-delete orphan cleanup.
- A **public plane classifier** — `pattern_plane`/`grant_plane` and the `Plane` enum are private free
  functions today; the system-plane fine check in handlers needs them exported (pub re-export or a
  pub wrapper like `patterns_reach_system_plane(&[ActionPattern])`), not duplicated.

Role CRUD queries: new `queries/roles.rs` module in `uptrakit-web-api-queries` (typed error enum +
module-wide `Result` alias per the error-handling standard), `ConnectionTrait`-generic so role delete
runs inside the guard transaction together with `delete_grants_for_role`. The tenant-mixed `roles`
table stays outside `TenantDb` helpers (like `access_grants`), with explicit scope filters.

## Audit

New actions in `audit-catalog.toml` + `audit_actions!` (all snapshot views derive `AuditView`,
secret-free):

| Action                              | Kind     | Snapshot view                                                               |
| ----------------------------------- | -------- | --------------------------------------------------------------------------- |
| `access_grant.create/update/delete` | Stateful | `AccessGrantView` (id, tenant_id, subject, patterns, selector, description) |
| `role.create/update/delete`         | Stateful | `RoleView` (id, name, description, is_built_in, tenant_id)                  |
| `user_role.update`                  | Stateful | `UserRolesView` (user_id, role_ids)                                         |
| `user_role.sync_lockout_prevented`  | Event    | —                                                                           |

- `update_user_roles` re-points from `user.update` to `user_role.update`. Same commit must move: the
  `audit-catalog.toml` row for that site, and the asserting tests. Blast radius (verified): the shared
  helper `latest_user_update_audit_row_for_target` (`users.rs`) hardcodes an `ActionType == USER_UPDATE`
  filter and has **four** callers — only `update_user_roles_writes_user_update_audit_event` re-points;
  the three `update_user_active` sibling tests must keep asserting `user.update`. **Parameterize the
  helper by action type** — never re-point its hardcoded filter (that breaks the three siblings).
- Stateful emits run via `emit_stateful` **inside the guard transaction** + `flush_after_commit()`
  (house pattern, `routes/hosts.rs`). Lockout denials keep the existing `Denied`-outcome audit idiom
  with the new reason codes.
- Deny **Events** for `access:manage` etc. are M1.6b, not here.
- `cargo xtask audit-coverage-check` green is a done-when criterion; new handlers get catalog entries.

## Invalidation

Strict order at every mutation site (grant CRUD, role delete, assignment, deactivation, OIDC sync):
**commit → `engine.invalidate_subjects` → publish `AccessInvalidated`** — never inside the
transaction (SQLite single-writer; and a pre-commit publish lets peers re-cache stale state for the
full TTL). Role-delete/role-grant mutations invalidate by `role_ids`; user-level mutations by
`user_ids`. Document on the grant-delete endpoint: cross-instance revocation latency is bounded by the
60 s TTL backstop, and an in-flight load can briefly re-insert pre-mutation authority (engine-doc'd);
acceptable for v1, no generation counter.

## Testing

Rows from `12-test-plan.md` §E owned here: **E1** (minus the waived batch leg), **E2, E3, E4, E5, E6,
E7, E10, E11**, plus **B10** (built-in immutability lands with role CRUD). E8/E9 are M1.6b.
`TestApp` harness for all endpoint tests; both success and failure paths.

- E2 both directions: `users:manage`-only principal → 403 on every grants/roles/assignment op;
  lifecycle ops succeed. `access:manage`-only principal: lifecycle 403, management 200.
- E3 per shrinking kind: delete last covering grant; unassign last covering role (full-replace);
  deactivate last covering user; narrow the covering pattern via grant update. Each → 409 with reason
  code, state unchanged, audit `Denied` row.
- E4: positive half via API; the "only with selector `All`" half is **unassertable through the API**
  (B9 phase gate rejects non-`All` writes until M2.1) — asserted at query-module level against a
  directly inserted non-`All` row, noted in the test.
- E5: behavioral — grant create / role create / assignment-add / activate never 409 with a lockout
  reason code, even when zero covering subjects exist beforehand (the state that would trip a
  wrongly-run guard); no instrumentation-based "no queries ran" assertion.
- E6: global plane — deleting the last `system.*:*` grant / deactivating its holder → 409
  independently of tenant-plane holders.
- E7: grant mutations produce Stateful rows with before/after `AuditView` snapshots.
- **E10 runs on Postgres too**: the concurrency claim is untestable on SQLite (single-writer hides
  the race the sentinel lock exists to close, and sea_query drops `FOR UPDATE` on SQLite anyway).
  Harness leg in the SQLite suite for the serialized 409; a Postgres leg in
  `uptrakit-integration-tests` (`--ignored`, database suite) drives two concurrent shrinks and also
  covers the missing-sentinel hard-error path.
- E11: seed-role read pairing — verify existing M1.2 coverage before writing; extend only if absent
  (no duplicate).
- New rows (added to the corpus test plan is out of scope; enumerated here as the task's matrix):
  system-plane fine check (mint `system.*` grant without `system.access:manage` → 403; with
  `system_administrator` assigned → 200; assignment adding `system_administrator` without it → 403);
  OIDC sync lockout-prevented (assignment kept, Event emitted, login succeeds); shadowing (create
  tenant role named `viewer` → 409 `role_name_shadows_global`; one resolver filter test per fixed
  site — `assign_viewer_role`, `sync_oidc_roles`, `assign_owner_roles`, `resolve_oidc_user` — with a
  hostile same-name row inserted directly; for `resolve_oidc_user`, a tenant role named `user` must
  NOT be assigned); role delete cascades (grants +
  assignments gone, engine authority drops on next request); assignment invalidation
  (immediate-effect after `PUT /users/{id}/roles`); built-in delete/update → 409 (B10); tripwire
  test on the candidate sets.
- Feature worlds: OIDC-sync changes compile/test under the `oidc` feature — gate commands for
  touched crates must enable it (`web-api-auth`, `web-api`); default-feature runs alone are
  insufficient for those tests.

## Quality gates, regen, policy files

- `./scripts/regen-api.sh` in-task: `openapi.json` + generated frontend client (new endpoints, reshaped
  `RoleResponse`, changed security declarations). No wire-type changes → no asyncapi regen.
- `uptrakit-openapi-client`: new `access_grants` module + roles CRUD additions (sync invariant).
- `db_access_policy.toml`: entries for every new/changed handler in the same commit
  (`verify_db_access_policy.py`).
- `ci/verify_action_security_declarations.py` green across the converted files;
  `cargo xtask audit-coverage-check` green; standard fmt/clippy/test/markdownlint gates.
- No new external dependencies.

## Documentation deliverables

- **New** `docs/api/access-management.md`: grant/role/assignment endpoints, encoding rules surfaced to
  API consumers (subject/tenant immutability, role-subject grants are global rows), lockout 409
  semantics + reason codes, built-in immutability, system-plane fine-check requirement, revocation
  latency note. M1.9's rewrite of `docs/security/auth-and-authorization.md` consolidates; this page is
  the per-task doc obligation.
- `docs/api/user-management.md`: minimal edit — role-assignment gate change (`access:manage`) and the
  `RoleResponse` reshape (full rewrite stays M1.9).
- Rustdoc: guard module (invariant, guarded-mutation list, future obligations, `AccessEngine`
  prohibition), `system.access:manage` catalog description, `users:manage` description trim.
- No ADR here — the model-replacement ADR is M1.9 and records the `system.access:manage` addition and
  the guard-shape amendment (this spec is the interim record).
- No `AGENTS.md` change (invariant wording updates are M1.9).

## Out of scope / deferred

- Batch endpoints for grants/roles (owner-waived E1 leg; revisit when a consumer exists).
- CLI grant/role commands, catalog endpoint, preset deletion, deny Events → M1.6b.
- Selectors beyond `All` (B9 gate stands) → M2; grant web UI → none in v1.
- `access_presets.rs` guard re-point — accepted window until M1.6b deletes the file.
- OIDC mapping-config escalation direction (`settings.auth:manage` → system-plane role mapping) —
  pre-existing, recorded for a future K-row.
- Cache generation counter for in-flight invalidation races — accepted 60 s bound.
- `GET /api/v1/permissions` deletion → M1.8.

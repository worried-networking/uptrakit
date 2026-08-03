# M1.4b — Route-module sweep onto action extractors, partitioned by domain

Date: 2026-08-03. Status: approved design, pending plan.

Sixth task of the authn/authz refactoring Milestone 1 (sources of truth:
`.superpowers/authn-and-authz-refactoring/`, esp. `05-action-model.md` §Built-in catalog and
mapping (the normative rename table), `07-decision-and-enforcement.md` §Native security
declarations, `11-task-breakdown.md` §M1.4b, `12-test-plan.md` §D; and the sibling specs
`2026-07-29-access-rest-extractor-scaffolding-design.md` (M1.4a) whose conversion pattern this
spec replicates, and `2026-08-03-access-mcp-surfaces-inline-design.md` (M1.5) whose file-ownership
boundary this spec honours). Owner-settled model decisions are applied, not reopened: per-site
action mapping is purely the 05 table — "the rows are its application".

**Implementation sequencing**: M1.1–M1.4a landed (M1.4a train ends at `02b50abe9`; reference
conversion `fb6f301a2` = `routes/hosts.rs`). M1.5 has an approved spec, no plan yet. M1.6a/M1.6b
are unspecced. Batches B1–B5 below can start immediately and land in any order; **B6 is blocked on
M1.5 landing** (same-file ownership, see Handoffs).

## Problem / goal

Apply the M1.4a conversion across every remaining route module in
`crates/ui/web-api/src/routes/`: swap `permission_extractor!` extractors for `action_extractor!`
ones, and replace `x-required-permission` + `security(("bearer_token" = []))` with native
requirements per `07`'s encoding. After this task (plus the M1.5/M1.6 handoffs), no REST handler
consults the JWT permission snapshot; the OpenAPI document carries the real authorization
contract.

## Decisions locked during grilling (owner, 2026-08-03)

1. **Delivery: commit series on `main`, not PRs.** Each batch is one green-tree Conventional
   Commit (or a small commit train) passing the full per-batch gate list. The milestone's
   "sub-PRs" language maps to "green-tree batches" in this repo's solo workflow.
2. **`users.rs`, `roles.rs`, `access_presets.rs` are handed to M1.6a/M1.6b entirely.** The
   `users:manage`/`access:manage` split is an M1.6a deliverable and M1.6b deletes the
   access-presets endpoints outright; converting them here would double-touch files and pre-empt
   the split. M1.4b's exit sweep asserts these three are the **only** files retaining
   `x-required-permission`/`bearer_token`; the fully-clean sweep becomes M1.6b's exit criterion.
3. **M1.5-shared files form the final batch (B6), sequenced after M1.5 lands.** Per the M1.5
   spec's ownership rule: M1.5 owns the inline handler-body conversions in `routes/surfaces.rs`,
   `routes/system_services.rs`, `routes/plugin_type_settings.rs`, `routes/plugin_configs/crud.rs`,
   `routes/services/batch.rs`; M1.4b owns the `#[utoipa::path]` declaration + extractor work on
   those same files, done in B6 after M1.5 merges — zero same-file collisions.
4. **Partition approved as B1–B6 below** (owner note: the grilling-time draft placed some
   M1.5-shared files in B1/B2; consistency with decision 3 moves all five to B6 — recorded here as
   the applied correction, not a reopened question).

## Corrections to the milestone text (verified against the live tree, 2026-08-03)

- **Counts**: 157 `x-required-permission` sites across 49 files; 165 `bearer_token` sites across
  50 files (adds `routes/me_2fa.rs`, which carries `bearer_token` but never had the extension).
  The milestone's "156 sites / ~129 files" mixed the site count with the total-file count and
  predates M1.4a. Plans re-count from the tree at write time.
- **The "seven surface wrappers" are eight today** (`routes/surfaces.rs` sites whose extension
  reads `dynamic: declared by the surface descriptor / interaction`). The dynamic-wrapper set is
  defined by that grep, not by the number seven.
- **`./scripts/regen-api.sh` runs per batch, not only in the final one.** The OpenAPI/frontend
  staleness gates are CI-enforced per commit; M1.4a's reference commit regenerated
  `openapi.json` + `frontend/src/lib/api/generated/` in-commit. Every batch does the same.
- **The `bearer_token` scheme registration stays** (`router.rs`): the three handed-off files
  reference it until M1.6a/M1.6b convert or delete them. Scheme removal rides with the milestone
  that kills the last reference (M1.6b), not with M1.4b.

## Scope rule

A route operation is in scope iff it carries `security(("bearer_token" = []))` in
`crates/ui/web-api/src/routes/`, excluding `users.rs`, `roles.rs`, `access_presets.rs`
(decision 2). This includes `me_2fa.rs`'s five extension-less operations. Out of scope: inline
handler-body `has_permission` conversions (all M1.5's — its spec re-greps the class at plan
time), `interactive_ws.rs` (no utoipa operation; M1.5 owns it), MCP, wire, frontend gating.

## Conversion recipe (mechanical, per operation)

Reference: `routes/hosts.rs` post-`fb6f301a2`.

1. **Map the extension value to an action** via the 05 table (restated per file in the batch
   tables below). No semantic re-litigation: whatever permission the site checks today, replace
   with its mapped action.
2. **Extractor swap**: replace the `middleware::permission` extractor parameter with the
   `middleware::action` one; switch the import (a file imports exactly one of the two modules —
   name collisions across modules, e.g. `CanUpdateHosts`, are resolved by the import line, which
   is also what `ci/verify_action_security_declarations.py` keys on).
3. **Declaration rewrite** inside `#[utoipa::path]`: delete the
   `extensions(("x-required-permission" = …))` entry; replace `security(("bearer_token" = []))`
   per class:

   | Class                                                                      | Declaration                                                                                                                                            |
   | -------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------ |
   | Governed, single action                                                    | `security(("oauth2" = ["<action>"]), ("developer_token" = []))`                                                                                        |
   | Governed, two actions AND (only `software_items/merge.rs` combined op)     | one requirement, both scopes: `security(("oauth2" = ["software:delete", "software:update"]), ("developer_token" = []))`; handler takes both extractors |
   | Governed, alternatives OR (inline-engine-checked handlers, B6 only)        | one single-scope `oauth2` requirement per alternative, then `("developer_token" = [])`; **no** action extractor on the handler                         |
   | Authenticated-only (`self` sentinel, `me_2fa.rs`, surfaces list/405 stubs) | `security(("oauth2" = []), ("developer_token" = []))`, no extractor                                                                                    |
   | Dynamic surface wrappers (B6)                                              | authenticated-only form **plus** `extensions(("x-action-dynamic" = json!(true)))` — the only surviving extension                                       |

4. **Handler body**: unchanged (M1.4b is declarations + extractors only). The extractor still
   yields `AuthenticatedUser` for handlers that use the caller identity.
5. **Regen + commit**: `./scripts/regen-api.sh`; `openapi.json` + generated frontend client are
   committed in the same batch commit.

### New action extractors

Appended to the single `action_extractor!` invocation in `middleware/action.rs` (the CI script
parses exactly that invocation), each batch adding the extractors its files need — the lists
below assume B1→B5 order; since batches may land in any order, a shared extractor (e.g.
`CanManageSystemSettings`, used by B1, B2, and B3 files) is added by whichever batch lands first.
Naming rule:
derive from the **action** (verb + resource), matching the M1.4a set (`CanReadHosts`, not the
legacy `CanViewHosts`). Expected additions (plan confirms exact spelling at write time):

- B1: `CanReadServices`, `CanApproveServices`, `CanRejectServices`, `CanDeleteServices`,
  `CanUpdateServices`, `CanManageEnrollmentTokens`, `CanManageSystemSettings`
- B2: `CanReadSoftware`, `CanCreateSoftware`, `CanUpdateSoftware`, `CanDeleteSoftware`,
  `CanTriggerUpdates`, `CanManageCommands`, `CanTriggerPluginConfigs`, `CanManageDiscoveryIgnores`
- B3: `CanReadSettings`, `CanManageAuthSettings`, `CanManageCertificateSettings`,
  `CanReadConfigState`, `CanManageConfigState`
- B4: `CanManageHostTags`, `CanManageScheduler`
- B5: `CanReadAudit`, `CanReadSystemAudit`, `CanReadNotifications`, `CanManageNotifications`
- B6: `CanReadSystemServices`, `CanApproveSystemServices`, `CanRejectSystemServices`,
  `CanDeleteSystemServices`, `CanUpdateSystemServices`

`permission_extractor!` and its extractors stay compiled (pub items, no dead-code warnings);
M1.8 deletes them together with `Permission`.

## Batch plan

Every batch: conversion per the recipe, its harness rows (§Tests), regen, one green-tree commit.
Batches B1–B5 are mutually independent; B6 last, after M1.5.

### B1 — services + enrollment tokens

| File                          | Mapping                                                                           |
| ----------------------------- | --------------------------------------------------------------------------------- |
| `services/crud.rs`            | `view_services` → `services:read`; `update_services` → `services:update`          |
| `services/lifecycle.rs`       | `approve/reject/remove/update_services` → `services:approve/reject/delete/update` |
| `services/merge.rs`           | `update_services` → `services:update`                                             |
| `device_auth.rs`              | `view_services` → `services:read`                                                 |
| `enrollment_tokens.rs`        | `manage_enrollment_tokens` → `settings.enrollment-tokens:manage`                  |
| `system_enrollment_tokens.rs` | `manage_global_settings` → `system.settings:manage`                               |

(`services/batch.rs` → B6.)

### B2 — software + updates + plugin configs

| File                                 | Mapping                                                                                                                |
| ------------------------------------ | ---------------------------------------------------------------------------------------------------------------------- |
| `software_items/crud.rs`             | `view/create/update/delete_software` → `software:read/create/update/delete`                                            |
| `software_items/batch.rs`            | `delete_software` → `software:delete`                                                                                  |
| `software_items/host_assignments.rs` | `update_software` → `software:update`                                                                                  |
| `software_items/merge.rs`            | `update_software` → `software:update`; the combined `update_software and delete_software` op → AND form (recipe table) |
| `software_items/updates.rs`          | `trigger_updates` → `updates:trigger`                                                                                  |
| `software_items/version_check.rs`    | `trigger_checks` → `checks:trigger` (extractor `CanTriggerChecks` already exists)                                      |
| `update_batches.rs`                  | `trigger_updates` → `updates:trigger`; `view_software` → `software:read`                                               |
| `update_history.rs`                  | `view_software` → `software:read`                                                                                      |
| `plugin_configs/batch.rs`            | `manage_commands` → `commands:manage`                                                                                  |
| `plugin_configs/discover.rs`         | `trigger_checks` → `checks:trigger`                                                                                    |
| `plugin_configs/test_action.rs`      | `test_plugin_configs` → `plugin-configs:trigger`                                                                       |
| `instance_plugins.rs`                | `manage_global_settings` → `system.settings:manage`                                                                    |
| `autodiscovery.rs`                   | `manage_ignores` → `discovery.ignores:manage`; `view_software` → `software:read`                                       |
| `discovery_allowlist.rs`             | `update_software` → `software:update`; `view_software` → `software:read`                                               |

(`plugin_configs/crud.rs`, `plugin_type_settings.rs` → B6.)

### B3 — settings

| File                                                                                                                                                                                                                              | Mapping                                                                                  |
| --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------- |
| `settings_access.rs`                                                                                                                                                                                                              | `manage_auth_settings` → `settings.auth:manage`; `view_settings` → `settings:read`       |
| `settings_agent_certs.rs`                                                                                                                                                                                                         | `manage_agent_certs` → `settings.certificates:manage`; `view_settings` → `settings:read` |
| `settings_combined.rs`                                                                                                                                                                                                            | `view_settings` → `settings:read`                                                        |
| `settings_ca.rs`, `settings_global_combined.rs`, `settings_nats.rs`, `settings_network.rs`, `settings_oauth.rs`, `settings_provider_github.rs`, `settings_reset.rs`, `settings_zeroconf.rs`, `server_cert.rs`, `system_alerts.rs` | `manage_global_settings` → `system.settings:manage`                                      |
| `oidc_providers.rs`                                                                                                                                                                                                               | `manage_auth_settings` → `settings.auth:manage`; `view_settings` → `settings:read`       |
| `instance_config_state.rs`                                                                                                                                                                                                        | `view/manage_instance_config_state` → `system.config-state:read/manage`                  |

### B4 — host tags + scheduler

| File           | Mapping                                                                                                                        |
| -------------- | ------------------------------------------------------------------------------------------------------------------------------ |
| `host_tags.rs` | `view_hosts` (reads) → `hosts:read`; `update_hosts` (all writes: tag create/edit/delete/assign/unassign) → `hosts.tags:manage` |
| `scheduler.rs` | `manage_scheduler` → `scheduler:manage`                                                                                        |

**Known intended divergence** (05 table row `hosts.tags:manage`): tag writes leave the
`hosts:update` authority. Seed parity holds — `host_manager` carries `hosts.tags:manage`
explicitly (`m20260728_000002_seed_access_grants.rs`), and no other seed role held `update_hosts`
before — but a principal with only a `hosts:update` grant loses tag management by design (tag
assignment is access-control authority under tag-scoped grants, `06`). Pinned by a harness test,
mirroring M1.4a's `pairing_rule_operator_only_reads_hosts` precedent.

### B5 — auth-adjacent + audit + notifications

| File               | Mapping                                                                                      |
| ------------------ | -------------------------------------------------------------------------------------------- |
| `auth.rs`          | `self` sentinel ops (`me`, `logout`) → authenticated-only form                               |
| `api_tokens.rs`    | `self` → authenticated-only form                                                             |
| `me_2fa.rs`        | all five bearer-only ops → authenticated-only form                                           |
| `audit_logs.rs`    | `view_audit_logs` → `audit:read`; `view_system_audit_logs` → `system.audit:read`             |
| `notifications.rs` | `view_notifications` → `notifications:read`; `manage_notifications` → `notifications:manage` |

### B6 — M1.5-shared files + CI script extension + exit sweep (after M1.5 lands)

Declarations only; the handler bodies will already enforce through the engine (M1.5).

| File                      | Declaration                                                                                                                                                                                                            |
| ------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `surfaces.rs`             | 2 `authenticated-only` list ops + 1 `none: always 405` stub → authenticated-only form; the dynamic wrappers (8 at spec time; set = the `dynamic:` extension grep) → authenticated-only form + `x-action-dynamic: true` |
| `system_services.rs`      | singles → `system.services:read/approve/reject/delete/update` extractors; the batch op → OR alternatives (`system.services:approve` \| `system.services:reject` \| `system.services:delete`)                           |
| `services/batch.rs`       | OR alternatives (`services:approve` \| `services:reject` \| `services:delete`)                                                                                                                                         |
| `plugin_configs/crud.rs`  | `manage_commands` ops → `commands:manage` extractor; the read ops (inline OR-of-three body) → OR alternatives matching the landed M1.5 enforcement (`software:read` \| `settings:read` \| `system.settings:manage`)    |
| `plugin_type_settings.rs` | `manage_global_settings` ops → `system.settings:manage`; any op whose handler M1.5 converts to an authorize-OR → matching OR alternatives                                                                              |

For every OR endpoint, the B6 plan derives the alternative list from the **landed M1.5 code**,
not from this table — the table is the expected outcome, the code is the authority.

**CI script extension** (`ci/verify_action_security_declarations.py`), same commit as the first
OR declaration: today `_oauth2_scopes` reads only the first `("oauth2" = […])` group and R1
rejects extractor-less non-empty scopes, so OR endpoints cannot be declared. New rule R5:
collect **all** `oauth2` requirement groups; when an operation declares more than one —
(a) the handler must use **no** action extractor (alternatives exist precisely because
enforcement is inline), (b) each alternative carries exactly one scope, (c) every declared scope
(here and in R1) must exist in the catalog map. R2/R3/R4 unchanged. Verified per the gate-script
discipline: an observed end-to-end RED (scratch perturbation: add an extractor to an OR handler;
mis-spell a scope) plus the existing non-vacuity guards.

**Exit sweep** (same batch):

```sh
rg -l 'x-required-permission|"bearer_token"' crates/ui/web-api/src/routes/
# must print exactly: access_presets.rs, roles.rs, users.rs
```

plus a grep that no `routes/` handler references `middleware::permission` outside those three
files, and the doc updates (§Documentation deliverables).

## Tests

Extend `crates/ui/web-api/src/integration_tests/access_rest_enforcement.rs` (M1.4a's harness
module; `TestApp` + `register_and_get_token` + `mint_api_token` fixtures, `d<row>_…` naming),
one representative endpoint per route family per batch:

- **D1** authorized request succeeds — ×2 credentials (JWT + `upk_` API token) for the batch's
  representative family.
- **D2** no credential → 401 (never 403) on every newly converted family (cheap loop over one
  GET per family).
- **D3** valid credential, zero grants → 403 with generic body — ×2 credentials.
- **D4** grant for a _different_ action → 403 (scope absent); representative per batch.
- **D5** (B5) authenticated-only endpoints succeed for a zero-grant principal (`me`, `logout`,
  api-token CRUD, 2FA setup) — pins the `self`-sentinel semantics.
- **B4 divergence pin**: principal granted `hosts:update` only → 403 on tag write, 200 on host
  update; principal granted `hosts.tags:manage` only → 200 on tag write, 403 on host update.
- **B6**: OR endpoint — each single alternative grant individually allows (one 200 per
  alternative), zero-grant 403; dynamic wrappers keep their M1.5-added enforcement tests (no
  duplication here — B6 only asserts the declarations, via the CI script and D14).
- **D14** (script perturbation) — extended for R5 as above; runs as the gate-script RED check,
  not a Rust test.

Existing per-family route tests keep passing unchanged except where they minted permissions via
the legacy snapshot — the M1.2 seed/remap means grants already exist for seeded roles; any test
fixture that grants legacy `Permission`s to custom users is updated to insert grants (pattern
established by M1.4a's hosts conversion).

## Quality gates (per batch)

`cargo fmt --all` / `cargo fmt --check`; `cargo clippy --all-targets --all-features` (frontend
built) and the minimal-feature clippy; `cargo test -p uptrakit-web-api --all-features`;
`./scripts/regen-api.sh` + stage `openapi.json` and `frontend/src/lib/api/generated/`;
`python3 ci/verify_action_security_declarations.py`; `python3 ci/verify_db_access_policy.py`;
`bash ci/verify_handler_state_contract.sh`; `cargo xtask audit-coverage-check` (route files are
touched — the `.routes(routes!())` re-keying trap); markdownlint on any doc touched. Full
pre-push before each batch lands.

## Documentation deliverables

- `crates/ui/web-api/AGENTS.md` — flip the transition note: action extractors are the default;
  `permission_extractor!` remains only for the three M1.6-handed files (B6).
- `docs/security/auth-and-authorization.md` — extend the M1.4a-started coverage section to "all
  route families except the M1.6a/b handoffs" (B6).
- `crates/ui/web-api/openapi.json` + `frontend/src/lib/api/generated/` — regenerated every batch.
- No ADR: the architectural decision is already recorded in the refactoring corpus; M1.9 owns the
  milestone ADR. No new dependencies (registry check n/a). No wire changes (asyncapi n/a).
- `.superpowers/authn-and-authz-refactoring/11-task-breakdown.md` is a frozen planning corpus —
  not updated; this spec records the deltas (§Corrections).

## Out of scope / deferred

- `users.rs`, `roles.rs`, `access_presets.rs` conversions → M1.6a/M1.6b (decision 2).
- Inline handler-body `has_permission` conversions, `interactive_ws.rs`, MCP, surface admission →
  M1.5.
- `bearer_token` scheme deregistration → M1.6b (last-reference rule).
- JWT `permissions` claim removal, `me` action list → M1.7. `Permission` deletion → M1.8.
- Visibility/selector enforcement (D7–D11, D16) → M2.

## Risks

- **B6 is serialized behind M1.5.** If M1.5 stalls, B1–B5 still land; the sweep's exit assertion
  simply waits. Accepted by owner (decision 3).
- **Fixture churn in B2/B3** (largest batches): route tests that stage users with legacy
  permissions need grant-insertion updates; bounded by the M1.4a precedent and the shared
  fixtures.
- **OR-declaration form depends on landed M1.5 semantics** — mitigated by deriving B6's
  alternative lists from the merged code at plan time.

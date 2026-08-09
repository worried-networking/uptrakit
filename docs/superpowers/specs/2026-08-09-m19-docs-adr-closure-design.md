# M1.9 — Docs + ADR closure for the authorization-model replacement

Date: 2026-08-09
Status: approved (owner, 2026-08-09)
Depends on: M1.8 (landed — `refactor(auth)!: delete the legacy Permission model and its tables`, `9bb1da57d`)

## Problem

M1.1–M1.8 replaced the enum-based RBAC model (`Permission`, `permission_extractor!`,
`x-required-permission`, preset endpoints) with the action-string grant model and the `AccessEngine`.
The documentation still describes the deleted world: `docs/security/auth-and-authorization.md` carries the
full legacy permissions-model block, `docs/end-user/user-management.md` and `docs/api/user-management.md`
describe the old fixed-role/permission model, `AGENTS.md` MUST-FOLLOW rules name deleted symbols, and no
ADR records the model replacement. A survey found ~40 further live docs with factually wrong claims,
bare-identifier references to deleted permission names, stale glossary terms, stale milestone framing,
or cosmetic "Permission" wording.

Scope decision (owner, 2026-08-09): **full sweep** — every factually wrong live doc plus cosmetic
renames, historical files untouched. ADR decision: **one rich, self-contained ADR** (the proposal
directory `.superpowers/authn-and-authz-refactoring/` is gitignored and cannot be linked).

This milestone is docs-only: no code, schema, or dependency changes.

## Deliverable 1 — `docs/security/auth-and-authorization.md` (surgical rewrite)

**Preserve unchanged** (the standards snapshot cites binding rules from these sections; keep headings and
anchors intact). Verified deliberate: line 107's "the response carries no permission/action data" is an
accurate statement about that error-path response and stays as-is:

- Lines 1–164: auth-methods table, JWT Access Token Claims Contract, JWT Signing Key Storage, Session
  Integrity Validation, OIDC Email Verification Enforcement, Database Error Propagation, OIDC Link Token
  URL Handling.
- Lines 514–622: System Service Credential Guard, WebSocket Enrollment Secret Lookup, OAuth 2.1 for MCP,
  Content Security Policy.
- Front-matter stays; only its `description` (line 4, "role and permission model") is re-worded to the
  action/grant model.

**Replace** the block at lines 165–513 with a new block headed exactly `## Authorization Model`
(anchor `#authorization-model` — Bucket E retargets depend on this literal heading; the
runtime-valued section below is headed exactly `### Runtime-valued actions`, anchor
`#runtime-valued-actions`). Sections to delete
outright: "Permissions Model - Detailed" (165–177), the ten permission reference tables (178–267),
"How it works" legacy walkthrough (315–337), the 33-row extractor→`Permission::` table (393–436),
"Adding a new permission" (437–457). The M1.4a/M1.4b "Transition: action extractors" section (458–513)
is the seed of the real current-model content — promote it, drop all milestone framing.

New block outline (each section links to the canonical home rather than copying; no hardcoded action or
role counts anywhere — the catalog is the source of truth):

1. **Action vocabulary** — `resource:verb` grammar; closed verb set (`crates/shared/types/src/access/verb.rs`);
   catalog macro as single source (`crates/shared/types/src/access/catalog.rs`); dynamic namespaces
   `plugin.<type>` / `surface.<id>`; open-string OpenAPI schema (documented grammar, not a closed enum);
   no `Other` catch-all — an unparseable action string is a parse error and a parse error is a deny.
2. **Grant model** — `access_grants` storage (engine-owned, deliberately not `TenantScoped`); pattern
   grammar (`ActionPattern`/`ResourcePattern`/`VerbPattern`; `system.`-prefixed resources excluded from
   `*`); selectors typed but restricted to `All` until M2; roles-as-data (`roles.tenant_id` NULL = global
   built-in, partial-unique name index pair); seed grants are frozen literals with the
   `seed_patterns_stay_valid_against_live_catalog` drift guard — catalog renames ship a forward data
   migration, never a seed edit.
3. **AccessEngine** — single decision point (`crates/ui/controller-core/src/access/mod.rs`); normative
   check order: dynamic-action registry → grant match → token scope → target/selector; bounded cache
   (60 s TTL backstop, first-party read-time staleness check), `AccessInvalidated` invalidation over the
   controller-event/NATS path; fail-closed — engine unavailable is HTTP 500, never a silent permit;
   scope term vacuously true for scope-less credentials (pre-M3 behavior equivalence).
4. **Enforcement surfaces** — `action_extractor!`
   (`crates/ui/web-api/src/middleware/action.rs`; 401 no principal / 403 deny / 500 unavailable;
   generic denial body); `authorize_any` OR-gates; the documented inline-engine exception sites
   (plugin type settings, plugin configs, service/system-service batch, plugin visibility predicate);
   interactive-update WS gate (`updates:trigger` before `on_upgrade()`); MCP `ToolAuth` per-tool actions
   plus the `mcp:use` connection gate; native OpenAPI `security(...)` declarations with the
   catalog-generated scope dictionary, `x-action-dynamic: true` on the surface wrappers
   (`ci/verify_action_security_declarations.py` gates the match).
5. **Runtime-valued actions** (the old "Runtime-valued permission extension (surfaces)" exception class,
   carried forward re-typed) — surface `required_action` is a string on the wire
   (`#[serde(alias = "required_permission")]` retained), parsed once to typed `Action` at registration
   admission; unparseable value rejects the whole registration; enforced by the engine before dispatch
   for plugin- and service-backed surfaces alike. New heading/anchor; inbound links retargeted (see
   Deliverable 7, anchor bucket).
6. **Lockout prevention** — re-targeted to `access:manage` / `system.access:manage`; SQL pre-filter over
   candidate patterns + bounded in-memory confirm (`check_lockout`, `begin_guarded` in
   `crates/shared/db/src/access_grants.rs`); sentinel-row locking via the default tenant; caller
   obligation (non-`Permitted` ⇒ guarded mutation must not be written); prohibition on calling the
   engine from inside the guard; skipped for authority-adding mutations; 409 reason codes.
7. **Catalog introspection** — one paragraph + pointer to `docs/api/access-management.md` (canonical
   endpoint reference; link, don't copy).
8. **Deny events** — `deny_event_worthy()` as the single shared definition (system-plane resources,
   `commands:manage`, `access:manage`, `mcp:use` emit audit Events; everything else is a debug trace +
   `uptrakit_access_denies_total` counter only).

## Deliverable 2 — `docs/end-user/user-management.md`

- Front-matter (lines 2–4) and intro (9–11): drop "32 permissions … 8 built-in roles … each role grants
  a set of permissions"; describe roles as named bundles of grant patterns, seeded built-ins plus
  per-tenant custom roles (roles-as-data). No counts.
- Built-in roles table (line 21): "Key permissions" column → "Key actions", cells become representative
  grant patterns (e.g. `*:read`, `services:*`).
- Line 82 heading "Viewing roles and permissions" → "Viewing roles and grants" (or equivalent
  action-model wording).
- Add a short **Custom roles and grants** section: role management is API/CLI in v1 (no web UI);
  `uptrakit-cli users set-roles <user-id> --names <comma-separated>`; grants are managed via the access
  API (`docs/api/access-management.md`); the catalog endpoint (`GET /api/v1/access/catalog`) lists
  actions and role bundles.
- Line 112 "full permission model" → "authorization model", link retargeted to the rewritten security
  doc's new anchor.
- Existing CLI sections (51–90) are already accurate — keep.

## Deliverable 3 — `docs/api/user-management.md`

- Line 8 "full permission model" framing → action/grant wording.
- "Permission endpoints" section (106–110): replace with a one-line tombstone — `GET /api/v1/permissions`
  was deleted with the legacy model; actions are introspected via `GET /api/v1/access/catalog`
  (link to `docs/api/access-management.md`).
- Line 54: drop "M1.6a permission split" milestone phrasing; state the `users:manage` / `access:manage`
  split as a plain fact (role mutations additionally require `system.access:manage` when adding
  system-plane-reaching roles — already documented in access-management.md; link).
- Key-files table (137–139): re-describe in terms of `action_extractor!` types.
- Keep the `PUT /api/v1/users/{id}/active` heading text stable — `docs/api/access-management.md` links
  its anchor.

## Deliverable 4 — `AGENTS.md` + `crates/ui/web-api/AGENTS.md`

- Root rule (lines 240–245): retitle bold lead-in to **"Use typed action extractors for route
  authorization."**; body keeps the no-inline-checks sentence and the `action_extractor!`/`AccessEngine`/
  native-`security(...)` description; **drop** the sentence "The legacy `permission_extractor!` module
  was deleted in M1.7; `Permission` itself and its tables are removed in M1.8." (history — moves to the
  ADR). Note: external docs cite bold lead-ins verbatim; sweep any doc citing the old lead-in text.
- Root rule (lines 246–250): "**Surface permissions are enforced…**" → "**Surface actions are
  enforced…**"; body already accurate.
- `crates/ui/web-api/AGENTS.md` (lines 72–74): same de-milestoning — describe the current extractor
  world without M1.7/M1.8 tense.
- Re-verify both size gates (`bash ci/verify_agents_md_budget.sh`; root currently 419/500 lines).

## Deliverable 5 — ADR for the model replacement

- Created with `adrs new "<title>"` — never hand-numbered. Next free number is expected to be 0039
  (0038 is highest; 0014 is a permanent gap) but the tool allocates.
- Title (working): "Replace enum-based RBAC with action-string grants and a central access engine".
- Nygard format matching ADR-0038: `# NNNN — Title`, `Date:`, `## Status` (Accepted), `## Context`,
  `## Decision`, `## Consequences`. No YAML front-matter.
- Self-contained (proposal dir is gitignored). Content:
  - **Context**: the legacy model — closed `Permission` enum mirrored into `permissions`/
    `role_permissions` tables, JWT-embedded permission claims, `permission_extractor!`,
    `x-required-permission` vendor extension, preset endpoints; its failure modes (stale-until-refresh
    authority, enum/table drift, no resource scoping path, vocabulary closed to plugins/surfaces).
  - **Decision**: `resource:verb` action strings with a catalog macro as single source; grants as data
    (`access_grants`, pattern + selector); roles as data (tenant-scoped rows, seeded built-ins);
    single decision point (`AccessEngine`, fail-closed, bounded cache + invalidation events); native
    OpenAPI security declarations replacing `x-required-permission`; preset endpoints retired in favor
    of standard role assignment; runtime-valued actions admitted by parse-at-registration.
  - **Condensed rejected alternatives** (with rejection reason and, where meaningful, the condition
    that would reopen): keep the closed enum and extend it; keep JWT-embedded permissions and shorten
    token lifetime; per-route inline checks without a central engine; per-request DB resolution with no
    cache; keeping the preset endpoints as a parallel assignment surface.
  - **Consequences**: immediate-effect authority changes (bounded by the 60 s cache backstop);
    open vocabulary (unknown action = deny, not schema error); doc/tooling shifts
    (`ci/verify_action_security_declarations.py`, catalog-driven UI/consent); selectors land in M2;
    scopes become wire-visible in M3. Name explicitly which parts M2/M3 will amend additively
    (selector variants beyond `All`; scope enforcement on OAuth-presented tokens) so the future edit
    extends this ADR rather than superseding it.
- Doctor-hard-fail constraints (adrs.toml `warnings_as_errors = true`): no `...`, no
  describe/todo/placeholder tokens, no thin sections; use `…` if ellipsis needed.
- After creation: `bash scripts/regen-adr-toc.sh` (never hand-edit `docs/adr/README.md`);
  `adrs doctor` green.

## Deliverable 6 — `docs/api/access-management.md` self-consistency

The hub document the other rewrites link into currently promises its own M1.9 rewrite and carries
milestone framing:

- Lines 3, 11: M1.6a framing → plain-fact wording.
- Line 13 ("still describes the pre-split model; it will be rewritten in M1.9…") and line 264
  ("scheduled for a rewrite in M1.9 to cover this split") — the dangling promises resolve in this
  milestone: fold the promised content (the `users:manage`/`access:manage` split coverage) or re-point
  to the rewritten docs, and delete the forward references.
- Line 84: reference to the removed `permissions` field — delete.

## Deliverable 7 — full-sweep incidental edits

**Sweep predicate.** The original survey keyed on `Permission`-token patterns and missed bare-identifier
prose references. Before executing, enumerate the deleted permission variant names (CamelCase, e.g.
`ManageEnrollmentTokens`, `ViewServices`, `TriggerUpdates`, `ManageSoftware`, `ManageGlobalSettings`,
`AccessMcp`) **and** their serialized snake_case forms (e.g. `view_agents`, `manage_agents`,
`update_software`, `test_plugin_configs`) from the pre-M1.8 enum definition — exact source:
`git show 9bb1da57d^:crates/shared/types/src/permissions.rs` (the deletion commit's parent; the diff
alone does not cleanly yield the list) — and sweep live docs on that finite list. The file inventory
below is the known-as-of-2026-08-09 result; the predicate rerun is authoritative.

Bucket A — factually wrong claims (fix the statement, minimal surrounding churn):

- Bare-identifier prose references found by the predicate (each fixed to the action actually enforced,
  verified against the handler's `action_extractor!` declaration): `docs/api/enrollment-tokens.md:7`
  (`ManageEnrollmentTokens`), `docs/api/sse-events.md:20` (`ViewServices`),
  `docs/api/interactive-updates.md:31,33` (`TriggerUpdates`), `docs/architecture/scheduler.md:370,381`
  (`ManageSoftware`), `docs/architecture/system-services.md:253,254,265` (`view_agents`/`manage_agents`),
  `docs/architecture/unified-software-tracking.md:186,188` (whole `## Permissions` section),
  `docs/admin/instance-plugins.md:3` (`ManageGlobalSettings`), `docs/development/plugin-system.md:170`
  (`update_software`), `docs/end-user/cli-usage.md:207`, `docs/end-user/mcp-clients.md:21` (`AccessMcp`),
  `docs/end-user/plugin-configs.md:468` (`test_plugin_configs`), `docs/end-user/surfaces.md:144–155`,
  `docs/architecture/surfaces.md:108–110`.
- `docs/api/services-operations.md:117` — lists the dropped `permissions` table as live; remove from the
  list (add `access_grants` if the list's purpose calls for it).
- `docs/development/quality-gates.md:80` — cites the deleted `middleware::permission` module →
  `middleware::action`.
- `docs/development/testing.md:321,346–358` — documents the deleted `seed_permissions_for_owner()`
  fixture helper; delete the entry and describe the current grant-seeding fixture actually present in
  `crates/ui/web-api/src/test_harness/fixtures.rs` (verify the live helper name before writing).
- `docs/end-user/surfaces.md:200` — dead link `../security/extensions.md` (file does not exist) →
  `../security/surfaces.md`.
- Stale milestone framing stated as pending: `docs/development/quality-gates.md:77` ("For the M1.4b
  closing sweep"), `docs/development/surfaces.md:129` ("M1.5 `required_action` boundary"),
  `docs/security/surfaces.md:63` ("**Known regression (until M1.7):**" — M1.7 landed; delete or rewrite
  as resolved).

- `docs/api/auth-flows.md:56` — tokens do not carry resolved permissions; authority is resolved per
  request by the `AccessEngine`.
- `docs/api/http-web-api.md:21` — deleted `middleware/permission.rs` reference → `middleware/action.rs`;
  `:302` — remove `permissions`/`role_permissions` from the live-table list; add `access_grants`.
- `docs/architecture/multi-tenancy.md:39` — same dropped-table list fix; note `access_grants` is
  engine-owned, not `TenantScoped`.
- `docs/development/coding-standards.md:201,908,1392` — `Permission` enum references → `Action`
  (`crates/shared/types/src/access/`); `:1392` points at a deleted file, retarget. `:1035` — the
  `Parse{TypeName}Error` naming example cites deleted `ParsePermissionError`; replace with a live type
  (e.g. `ParseActionError`). `:1183` retired-form note stays (deliberate history — excluded from the
  Verification 4 `x-required-permission` grep).
- Stale action values verified against the handler's `action_extractor!` declaration before writing
  (same caveat as discovery-allowlist above): `docs/development/config-testing.md:175`
  (`test_plugin_configs` → the action actually enforced, `plugin-configs:trigger` per
  `CanTriggerPluginConfigs`); `docs/development/autodiscovery-internals.md:322`
  (`Permission::TestPluginConfigs` prose → same action); `docs/security/oauth-mcp.md:23`
  (`Permission::TriggerUpdates` prose → `updates:trigger`).
- `docs/hackme/13-jwt-session-token-attacks.md:59,102` — re-base the threat on the current model: no
  embedded permissions; realistic staleness window is the engine's 60 s cache backstop after a lost
  invalidation event.
- `docs/security/notifications-security.md:232,240`, `docs/development/notifications.md:616,623,642,652`,
  `docs/api/notifications.md:15,26,36` — `Permission` columns/variants (`ViewNotifications`/
  `ManageNotifications`) → `notifications:read` / `notifications:manage` catalog actions.
- `docs/api/discovery-allowlist.md:43,78,117,141,185,232`, `docs/api/autodiscovery.md:20,57,98,152,194`
  — legacy serialized permission names (`view_software`, `update_software`, `trigger_checks`,
  `manage_ignores`) → the actual catalog actions now enforced by the corresponding extractors (verify
  each against the handler's `action_extractor!` declaration before writing).

Bucket B — CONTEXT.md controlled vocabulary:

- `CONTEXT.md:189,211–213,314–315` — replace the `Permission` (typed enum) definition with `Action`
  (`resource:verb` string, catalog-validated) and `Grant` (pattern + selector conferring authority);
  update the `McpScope`/permission ambiguity notes to the action-string world. Keep entry style
  consistent with the rest of the glossary.

Bucket C — cosmetic column-header renames ("Permission" → "Action", cell values already action strings
or updated alongside): `docs/end-user/batch-actions.md:25`, `docs/end-user/audit-logs.md:171`,
`docs/end-user/dashboard-icons.md:115`, `docs/end-user/notifications.md:326`, `docs/api/host-tags.md:8`,
`docs/api/audit-logs.md:192`, `docs/api/settings-runtime.md:241,541`, `docs/api/http-web-api.md:262,619`,
`docs/api/batch-actions.md:60` (also the `x-required-permission` mention),
`docs/architecture/update-history-entity.md:172`, `docs/architecture/software-item-entity.md:147`,
`docs/architecture/host-entity.md:257`, `docs/architecture/ssh-agent.md:876`,
`docs/development/dashboard-icons.md:146`, `docs/development/ui/layout.md:130`,
`docs/security/security-architecture.md:100`, `docs/security/interactive-updates.md:31,37`,
`docs/security/surfaces.md:16,60`. (`config-testing.md:175`, `autodiscovery-internals.md:322`, and
`oauth-mcp.md:23` moved to Bucket A — their values are stale, not just their headers.)

Bucket D — index files:

- `docs/README.md:52` — "User, role, and permission management endpoints" → user/role management +
  role-assignment wording; `:64` — "permissions model" → "authorization model".
- `docs/security/README.md:18` — "role/permission model" → "action/grant authorization model".
- `docs/api/README.md` contents table — add rows for `user-management.md` and `access-management.md`.

Bucket E — inbound anchors into the rewritten security doc (retarget to the new headings):

- `#permissions-model---detailed` → `#authorization-model` (pinned in Deliverable 1):
  `docs/security/notifications-security.md`, `docs/hackme/16-rce-plugin-config-manipulation.md`.
- `#runtime-valued-permission-extension-surfaces` → `#runtime-valued-actions` (pinned in
  Deliverable 1): `docs/api/surfaces.md`, `docs/security/surfaces.md`.
- `docs/end-user/user-management.md:112`-style links into the old block likewise.

Explicitly untouched (historical/frozen): `docs/adr/0006,0008,0009,0010,0028,0031` (and all other
existing ADRs), `crates/*/CHANGELOG.md`, `docs/superpowers/specs/*`, `frontend/CODEREVIEW.md` /
`crates/**/CODEREVIEW.md` snapshots, `docs/development/database-migrations.md` frozen historical
migration examples (`role_permissions` inside code blocks documenting the SQLite-recreate pattern),
`docs/development/surfaces.md:135` serde-alias note (still true: the wire alias
`required_permission` is retained).

## Execution note — line-number drift

All `file:line` citations are anchored to the 2026-08-09 tree (`aedf47d2d`). Line numbers shift as
edits land, and several files appear in more than one bucket. When implementing: locate each edit by
the quoted text or heading, not the line number; within a single file, apply edits bottom-to-top so
earlier citations stay valid.

## Non-goals

- No code, migration, OpenAPI, or frontend changes; no regen of generated artifacts other than
  `docs/adr/README.md` via `scripts/regen-adr-toc.sh`.
- No selector (M2) or scope (M3) documentation beyond forward references — those land with their
  milestones.
- No web-UI documentation for grant/role management (none exists in v1 by design).
- No edits to gitignored working material (`.superpowers/*`).

## Verification (done-when)

1. `markdownlint --config .markdownlint.json '**/*.md'` green; markdown formatted with
   `npx prettier --write` on touched files.
2. `bash ci/verify_agents_md_budget.sh` green (root ≤ 500 lines / 60 KB; scoped ≤ 250).
3. `adrs doctor` green; `bash ci/verify_adr_numbers.sh` green; `bash scripts/regen-adr-toc.sh --check`
   green.
4. Reference greps clean over live docs (exclusions: `docs/adr/`, `docs/superpowers/`, `CHANGELOG.md`,
   `CODEREVIEW.md`, `docs/development/database-migrations.md` frozen examples, the deliberate
   anti-pattern mentions of `has_permission` in AGENTS.md/coding-standards, the serde-alias note, the
   `coding-standards.md:1183` retired-form history note):
   - `access-presets`, `apply-preset`, `apply_preset`, `/api/v1/access-presets`,
     `users apply-preset` — zero hits.
   - `permission_extractor`, `x-required-permission` — zero hits (batch-actions.md mention removed).
   - `Permission` as a type name (`Permission::`, `Vec<Permission>`, "`Permission` enum",
     `ParsePermissionError`) — zero hits.
   - Every deleted permission variant name (CamelCase) and serialized snake_case form from the
     Deliverable 7 sweep-predicate list — zero hits (this catches the bare-identifier prose class the
     type-name patterns miss). The CamelCase pattern must be boundary-guarded with a negative
     lookbehind for the live extractor prefix — `rg -nP '(?<!Can)\b(ApproveServices|TriggerChecks|…)\b'`
     — because live `action_extractor!` type names (`CanApproveServices`, `CanTriggerChecks`,
     `CanUpdateHosts`, …) embed the deleted variant names as substrings, and the rewritten docs are
     mandated to mention those types. Do not weaken the gate on a collision; fix the pattern.
   - `M1.[0-9]` milestone framing (`rg -nE '\bM1\.[0-9]'`) over live docs — zero hits outside
     `docs/superpowers/` and `docs/adr/`.
5. Inbound anchor links resolve: every `auth-and-authorization.md#…` and `user-management.md#…`
   reference in live docs points at an existing heading. Additionally, every relative link in every
   touched file points at an existing file (markdownlint's MD051 is off — this check is the only
   backstop). Mechanical form, not eyeballing: for each touched file, extract relative link targets,
   strip anchors, `test -e` each from the file's directory, e.g.

   ```sh
   for f in $(git diff --name-only <base>..HEAD -- '*.md'); do
     dir=$(dirname "$f")
     grep -oE '\]\((\./|\.\./)[^)#]+' "$f" | sed 's/^](//' | while read -r t; do
       [ -e "$dir/$t" ] || echo "DEAD: $f -> $t"
     done
   done
   ```

6. Standards-snapshot-cited rules from the preserved sections (JWT `exp`/`iss`/`aud` validation, OIDC
   `email_verified` enforcement) still present verbatim in the rewritten file.

## Documentation deliverables

This spec is entirely documentation; the enumerated files above are the deliverables. New ADR
(auto-numbered, expected 0039) + regenerated `docs/adr/README.md` included.

## Snapshot conformance

- "New ADRs created only via `adrs new`; never hand-edit `docs/adr/README.md`" — followed (Deliverable 5).
- AGENTS.md maintenance rules (no counts, no inventory tables, bold lead-ins preserved-or-cited) —
  followed; the one lead-in that changes ("typed permission extractors" → "typed action extractors")
  gets its citations swept in the same change.
- markdownlint / prettier / size-gate tooling constraints — in Verification.
- Controlled vocabulary (`CONTEXT.md`) updated rather than contradicted — Bucket B.
- Common-mistakes ledger: no relevant rows (code-plan mistakes); the deletion-analog — rewriting
  sections must not orphan inbound links — is covered by Bucket E and Verification 5.

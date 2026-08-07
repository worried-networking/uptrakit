# M1.7 — Token claims, `me`, frontend

Status: approved design (grilling round 2026-08-07). Milestone source:
`.superpowers/authn-and-authz-refactoring/10-milestones.md` (M1 staging step 3) and
`11-task-breakdown.md` § M1.7. Test matrix row: `12-test-plan.md` D13 (minus the visibility-summary
clause, which is M2). Depends on M1.4a/b–M1.6b, all landed as of `e5f012904`.

## Problem / goal

The JWT still carries a `permissions` claim, `AuthenticatedUser` still exposes
`permissions`/`has_permission()`, and every user-facing response (`me`, login family, users
list/get) still speaks the legacy `Permission` vocabulary — while every enforcement surface
already reads the `AccessEngine`. M1.7 removes authorization data from tokens and swaps the
principal-facing contract to the action vocabulary:

- `me` returns the **expanded effective action list** (wildcards expanded against the catalog,
  dynamic actions per live registries) plus `authority: "ok" | "unavailable"`.
- The SPA gains a degraded-authority banner and swaps the generated `Permission` union for a
  branded action-string type with typed constants; all UI gating re-points.
- After M1.7, **no token carries authorization data beyond `scope`** (pre-M3 session JWTs carry
  no scope at all), and **no OpenAPI schema references `Permission`** — M1.8 becomes a pure
  internal deletion (enum, shim, tables) with no regen round.

## Owner decisions (grilling round, 2026-08-07)

1. **Clear ALL wire-schema references to `Permission` in M1.7** — `UserResponse` reshape,
   `UserWithRolesResponse.permissions` removal, and deletion of `GET /api/v1/permissions`.
   Amends the task breakdown / in-code doc comment that placed the `list_permissions` deletion
   in M1.8; M1.8's deletion list shrinks accordingly (it keeps: `Permission` enum +
   `Other(String)`, the shim, the `permissions`/`role_permissions` table drop).
2. **`UserWithRolesResponse.permissions` is dropped, not replaced.** Roles stay in the response;
   effective authority is queryable via the M1.6a grants API and the catalog. No per-row engine
   resolution in `list_users` (batch-query rule).
3. **Login-family responses populate `actions` + `authority` at response time** via a shared
   helper (engine context → expansion). Engine failure at login/MFA/OIDC degrades to
   `authority: "unavailable"` + empty list with the auth flow still succeeding — this aligns the
   OIDC mint path, which today 500s on permission-load failure (documented behavior change).
4. **`UserResponse.permissions` renames to `actions`** (with `authority` beside it). CLI
   `auth status` output renames accordingly.
5. **`userinfo` rejected** (sidenote, same round): OIDC-only concept — requires the `openid`
   scope literal, colliding with the zero-exception action-string scope grammar; the payload is
   live authorization state, not identity claims; M1.7 predates the M3 AS entirely. Recorded
   under Alternatives.

## Design

### 1. Engine: `allowed_actions()`

New public method on `AccessEngine` (`crates/ui/controller-core/src/access/mod.rs`) — the only
place that can see `AccessContext`'s private `authority`/`scope` fields:

```rust
/// Expanded effective action list for principal-facing introspection
/// (`me`, login-family responses).
///
/// Membership is derived through [`AccessEngine::authorize`] itself —
/// never through a re-implementation of pattern matching — so the list
/// can never drift from real enforcement outcomes.
pub fn allowed_actions(&self, ctx: &AccessContext) -> Vec<Action>
```

Semantics:

- Candidate set = every built-in catalog action (iterate `CATALOG` exactly the way
  `routes/access_catalog.rs:60-73` does) ∪ `self.dynamic_actions()` (live registry entries;
  empty when no registry — fail-closed, matching `dynamic_actions()`'s existing contract).
- Keep a candidate iff `self.authorize(ctx, &action, None)` yields `Decision::Allow`; any other
  variant (deny, unknown future variant) drops it — same fail-closed treatment the MCP gate uses
  (`319b6b9df`). This gives wildcard expansion and scope intersection for free: `authorize`
  already applies both, including the vacuously-true scope term for scope-less pre-M3
  credentials (test-plan C15).
- Sort by `Action::as_str()` for a deterministic wire order; candidates are unique by
  construction (catalog enumerates each `(resource, verb)` once; dynamic namespace is disjoint
  from built-ins).

Cost: ~50 in-memory pattern checks per call — negligible; no extra DB work beyond the `context()`
the caller already holds.

### 2. Token claims and `AuthenticatedUser`

All in one sweep; the tree stays green because nothing outside `middleware/permission.rs`
consumes the removed items (verified: zero imports of that module anywhere).

- **`AccessTokenClaims`** (`crates/ui/web-api-auth/src/auth/jwt.rs:11-35`): delete the
  `permissions: Vec<Permission>` field. `create_access_token` drops its `permissions:
  &[Permission]` parameter; update every mint site (`routes/auth.rs:450/705/2993`,
  `mfa.rs:121`, `me_2fa.rs:789`, `oidc_auth.rs:2247`, and the `&[]`-passing test mints in
  `oauth/clients_api.rs`, `oauth/consent.rs`, `oauth/consents_api.rs`).
- **Outstanding tokens stay valid**: serde ignores unknown fields, so a pre-M1.7 JWT carrying
  the claim still authenticates. One test pins this deployment-cutover contract (decode a token
  minted with an extra `permissions` JSON key → authenticates; this tests our tolerance
  contract, not serde itself).
- **`AuthenticatedUser`** (`crates/ui/controller-core/src/auth/mod.rs:75-104`): delete the
  `permissions` field and `has_permission()`; `new()` loses the positional argument — sweep the
  ~40 test construction sites (dominated by `routes/services/tests.rs`). `audit_actor()` etc.
  stay. `require_auth.rs:300` and the API-token path (`auth/api_token.rs:71-80`) stop loading /
  passing permissions.
- **`get_user_permissions` deleted — both copies** (`require_auth.rs:308-356`,
  `api_token.rs:85+`). After this milestone every caller is gone (mint sites, `me`,
  `users.rs:72`, MFA/OIDC paths). Leaving either copy fails `warnings = "deny"` (dead code).
- **`middleware/permission.rs` deleted whole** (module, `permission_extractor!` macro, its test
  module, the `mod` declaration). Rationale: the macro body is the sole `has_permission()`
  caller; an uninvoked `macro_rules!` trips `unused_macros` under deny-warnings, so the module
  cannot outlive the method. Amends the M1.8 task text ("`permission_extractor!` contents"
  becomes a no-op) and requires the AGENTS.md sentence fix below.

### 3. Response contract: `UserResponse`, login family, users routes

**New type** in `uptrakit-web-api-types` (`src/auth.rs`):

```rust
/// Whether the access engine resolved this principal's authority.
///
/// Deliberately a closed two-variant enum (no `#[non_exhaustive]`, no
/// `Other`): the set is definitionally complete — the engine either
/// resolved grants or it did not — matching the closed-verdict-set
/// precedent (`LockoutVerdict`) rather than the wire-safe open-enum rule,
/// which targets vocabularies that can grow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum AuthorityStatus {
    Ok,
    Unavailable,
}
```

**`UserResponse`** (`src/auth.rs:79-88`): `permissions: Vec<Permission>` →
`actions: Vec<String>` + `authority: AuthorityStatus`. The wire field is deliberately
`Vec<String>`, not `Vec<Action>` (amended 2026-08-07, plan-review contrarian finding):
`Action`'s deserializer rejects any resource/verb absent from the compiled catalog, so a typed
field would make a newer controller's response unparseable to an older
CLI/openapi-client — a login-breaking skew the moment the catalog grows (M2/M3 both grow it).
Servers construct the strings from `allowed_actions()`'s `Vec<Action>`; clients treat entries
as opaque strings; the OpenAPI schema is the same open string array either way. Update the
serde round-trip tests at `src/lib.rs:370-448`.

**`me` handler** (`routes/auth.rs:2733-2789`): add `Extension(access): Extension<AccessAuthority>`
(inserted by `require_auth` on every authenticated request; keeps the single
`State<Arc<AppState>>` — no sub-state mixing). Replace the `get_user_permissions` block and its
DEVIATION comment:

- `AccessAuthority::ready()` → `Some(ctx)`: `actions = engine.allowed_actions(ctx)`,
  `authority: Ok`.
- `None` (unavailable): `actions = vec![]`, `authority: Unavailable`, still HTTP 200 — the
  resolved carve-out (`09-resolved-questions.md` § decision 4). The SPA's logout-on-non-2xx
  behavior stays untouched.
- Also update the mirroring prose in `middleware/action.rs:26-38` ("until M1.7" doc comment) —
  the guard now lives in `me`'s `authority` field, not in a fallback.

**Login-family shared helper** (new, `crates/ui/web-api/src/routes/` shared module or
`auth.rs`-local):

```rust
/// Effective-action view for embedding a `UserResponse` in an auth
/// response. Engine failure degrades to `Unavailable` + empty list; the
/// auth flow itself proceeds (same carve-out as `me`).
async fn effective_actions(
    engine: &AccessEngine,
    tenant_id: Uuid,
    user_id: Uuid,
) -> (Vec<String>, AuthorityStatus)
```

Calls `engine.context(tenant_id, user_id, None)` → `allowed_actions`; on `Err`, logs `warn` and
returns `(vec![], AuthorityStatus::Unavailable)`. Consumers: login, register, MFA
verify, `build_session_tokens` (`me_2fa.rs`), OIDC mint — every site that embeds `UserResponse`.
The OIDC path's current 500-on-load-failure becomes this degraded 200 (owner decision 3).

**Users routes** (`routes/users.rs`):

- `UserWithRolesResponse.permissions` (`web-api-types/src/users.rs:16`) deleted; handlers stop
  resolving permissions (their `get_user_permissions` calls go away with the function).
- `list_permissions` handler + `PermissionInfo` type + route registration deleted; the catalog
  endpoint (M1.6b) is the replacement. Remove the route's `db_access_policy.toml` row and its
  scope-map entry (goldens below). Deleted-route test mirrors the E9 precedent (404, not
  stubbed).

### 4. openapi-client and CLI

- **openapi-client**: delete `src/permissions.rs`, its `lib.rs` module line, `paths.rs`
  `permissions` const block, and the mock route (`mock.rs:339` area). `UserResponse` /
  `UserWithRolesResponse` re-export from `uptrakit-web-api-types`, so the reshape propagates
  without client edits. Keeps the ADR-0026 endpoint↔client pairing consistent.
- **CLI** (`crates/ui/cli/src/commands/auth.rs`): `AuthStatusOutput.permissions` →
  `actions: Vec<String>` + `authority: String`; text render becomes `Actions: …` plus, when
  degraded, a `Authority: unavailable — grants could not be resolved; shown list is empty`
  line. Update the display tests (`:951-1036`). `users.rs` show/list output drops the
  permissions column (`:212-217, :397`). Breaking output shape ⇒ the commit is `feat(cli)!`
  with a `BREAKING CHANGE:` footer naming both surfaces.

### 5. Frontend

**Regen**: `./scripts/regen-api.sh` — `Permission` disappears from `types.gen.ts` (the schema no
longer references it anywhere), `UserResponse` gains `actions: Array<string>` +
`authority: 'ok' | 'unavailable'`, `listPermissions` SDK function disappears. Keep
`openapi-ts.config.ts` `enums: 'javascript'` (other generated runtime enums still rely on it);
rewrite its inline comment, which currently justifies the setting by `Permission`'s existence.

**Branded action type + constants** (`src/lib/api/local-types.ts`, replacing the
`Permission`-based `User`/`hasAnyPermission`/`hasPermissionValue`):

```ts
/** Action string in the catalog grammar (`resource:verb`, incl. dynamic
 * `plugin.*` / `surface.*` and system-plane `system.*` forms). Open set —
 * dynamic actions exist only at runtime, so this is a branded string
 * shape, not a union. */
export type Action = `${string}:${string}`;

/** Built-in actions the UI gates on. Values are validated against the
 * server catalog by `actions.test.ts`. */
export const Actions = {
	HOSTS_READ: 'hosts:read',
	SOFTWARE_READ: 'software:read'
	// … one entry per action the gating sweep references (derived from the
	// shim mapping of every current `Permission.X` call site; no speculative
	// entries)
} as const satisfies Record<string, Action>;

export interface User {
	// id/email/… unchanged
	actions: readonly string[];
	authority: 'ok' | 'unavailable';
}

export function hasAction(user: User | null | undefined, action: Action): boolean {
	return user?.actions.includes(action) ?? false;
}

export function hasAnyAction(user: User | null | undefined, ...actions: Action[]): boolean {
	return actions.some((a) => hasAction(user, a));
}

/** Mirror of today's `hasPermissionValue`: a null/undefined requirement
 * (surface with no `required_action`) gates nothing. */
export function hasActionValue(user: User | null | undefined, action?: string | null): boolean {
	if (action === undefined || action === null) return true;
	return user?.actions.includes(action) ?? false;
}
```

**Gating sweep**: every `getUser()?.permissions.includes(Permission.X)` →
`hasAction(getUser(), Actions.Y)`, mapping `X → Y` via the shim table
(`crates/ui/controller-core/src/access/shim.rs` — the authoritative old→new mapping; e.g.
`MANAGE_GLOBAL_SETTINGS → system.settings:manage`, `VIEW_SOFTWARE → software:read`). Sweep
inventory (recon 2026-08-07): `+layout.svelte`, `+page.svelte`, `surfaces/[id]`, `hosts/[id]`,
`history`, `software` (+`[id]`, `IgnoreRulesTab`), `system-services`, `settings` (+`GlobalSettingsTab`,
`PluginConfigsTab`, `SchedulerTab`, `McpAccessTab`), `lib/surfaces/read-model.ts`
(`filterSurfacesByPermission` → renamed `filterSurfacesByAction`), `lib/surfaces/contract.ts`,
`lib/surfaces/interactions.ts`. The plan re-runs the inventory grep at write time
(`grep -rn "Permission\.\|hasAnyPermission\|hasPermissionValue" frontend/src --include='*.svelte' --include='*.ts' | grep -v generated`)
and diffs against this list.

**E2E mock fixtures are their own site class**: ~25 spec files carry `permissions: […]` arrays
inside mocked `me`/login payloads (`frontend/tests/e2e/*.ts`). Each becomes
`actions: […], authority: 'ok'` with action-string values. Inventory command:
`grep -rln "permissions" frontend/tests/e2e/`.

**Degraded-authority banner** (`+layout.svelte`, `shellBannerRegionEl` div at `:527-551`): third
`Callout` beside the offline and session-expired banners —
`tone="warning"`, `data-ui="app-shell-banner"`, shown when `getUser()?.authority === 'unavailable'`,
title/message: authorization temporarily unavailable, actions hidden until it recovers. State
updates whenever a `UserResponse` lands (login or `me`); no polling. `auth.svelte.ts`
`initialize()` catch (unconditional logout on non-2xx) stays untouched.

**Constants drift guard** (`src/lib/api/actions.test.ts`, runs under the existing `npm run test`
vitest setup): every `Actions` value must appear in the committed OpenAPI scope dictionary — an
independently generated artifact (Rust catalog → `regen-api.sh`), so hand-written constants
cannot drift silently:

```ts
import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { Actions } from './local-types';

describe('Actions constants', () => {
	it('every constant is an action the server catalog declares', () => {
		const spec = JSON.parse(
			readFileSync(new URL('../../../../crates/ui/web-api/openapi.json', import.meta.url), 'utf8')
		);
		const scopes: Record<string, string> =
			spec.components.securitySchemes.oauth2.flows.authorizationCode.scopes;
		for (const action of Object.values(Actions)) {
			expect(Object.keys(scopes), `unknown action constant ${action}`).toContain(action);
		}
	});
});
```

(Verified against the live artifact: 39 scope entries including all `system.*` actions.)

### Alternatives considered

- **`userinfo` instead of `me`** (owner sidenote): rejected — OIDC Core concept requiring the
  `openid` scope literal (collides with the zero-exception action-string scope grammar), the M3
  AS is OAuth2-only (no ID tokens/OP role), the payload is per-request authorization state rather
  than identity claims, and M1.7 predates the AS. Revisit only if an OIDC-OP capability is ever
  adopted; even then `userinfo` would be identity-claims-only beside `me`.
- **Keep the `permissions` field name carrying action strings**: rejected — old name over new
  vocabulary invites confusion and a second breaking rename; the regen is breaking either way.
- **Slim login responses (SPA calls `me` post-auth)**: rejected — extra round trip and SPA
  auth-flow rework for no capability; response-time population reuses the same helper.
- **Replace `UserWithRolesResponse.permissions` with per-user action lists**: rejected — one
  engine resolution per row in `list_users` (N-per-list against the batch rule) duplicating what
  the grants API already answers.
- **Defer `list_permissions`/`UserWithRolesResponse` to M1.8 (strict plan reading)**: rejected —
  leaves `Permission` in the OpenAPI schema after M1.7, forcing a second breaking regen +
  CLI/frontend churn inside M1.8's "one pure deletion commit".
- **Closed-union frontend `Action` type**: rejected — the action set is open (dynamic
  namespaces); the OpenAPI schema is deliberately an open string, and a closed union would
  reject every dynamic action (mirrors the Rust no-`Other` decision).

## Testing

Rust (all endpoint tests through the `TestApp` harness; success + failure paths):

- **D13 core** (`integration_tests/`, new or extended `me` module):
  - Wildcard expansion: principal granted `software:*` → `me().actions` contains exactly the
    catalog's `software:` verbs, no others; `authority == "ok"`.
  - Dynamic actions: register a surface action in the stub registry → its `surface.<id>:use`
    appears in `me`; deregister → disappears (idiom:
    `access_catalog.rs::dynamic_actions_appear_and_disappear_with_registry_state`).
  - Zero-grant principal: `me` 200, empty `actions`, `authority: "ok"` (extends the D5
    assertion at `access_rest_enforcement.rs:678`).
  - Staging note: grant fixtures must discriminate the engine (direct grant-row staging per
    ledger; never claim-stuffed principals — the claim no longer exists).
- **Unavailable leg**: handler-level test injecting `Extension(AccessAuthority::Unavailable)`
  (sibling idiom: `routes/system_services.rs:968`) → 200, empty `actions`,
  `authority: "unavailable"`. Asserts the status AND both body fields (something no existing
  test observes).
- **Login family**: login response's embedded user carries populated `actions` +
  `authority: "ok"` (`RefreshResponse` embeds no user — corrected 2026-08-07, refresh is
  claims-only); helper failure leg staged with an engine over a schema-less
  `sqlite::memory:` connection (idiom:
  `access/mod.rs::context_propagates_db_errors_never_empty_authority`) → `(empty,
  Unavailable)` while the flow still returns 200.
- **Claims**: mint → decode has no `permissions` key (serialize the claims and assert key
  absence — MCP precedent `get_current_user_mcp.rs:494`); legacy-token tolerance test (extra
  `permissions` JSON member still authenticates); update `jwt.rs:131-265` claim tests and the
  `require_auth.rs` mint-then-authenticate tests; `settings.rs:74`-style fixture mints lose the
  permissions argument.
- **Engine**: `allowed_actions` unit tests beside the existing C-row tests — wildcard grant
  expansion, scope-ceiling intersection (C14 analogue: `*:*` grant + one-action scope → exactly
  that action), no-registry ⇒ no dynamic entries, deny-by-default for unmatched actions.
- **Deleted route**: `GET /api/v1/permissions` → 404 (E9 idiom).
- **Harness cleanup**: remove the `fixtures.rs:604-614` re-login dance and its now-false
  comments (`:621-626`) — grant changes are visible on the next request without re-mint.

Frontend:

- `actions.test.ts` drift guard (above) — red case: add a bogus constant locally, watch it fail.
- `npm run check` type-clean is the sweep's structural gate (every `Permission` reference is a
  compile error after regen); e2e suites run under the updated fixtures.

Goldens tripped by this change (pre-declared, each resolved in the same commit as its cause):
`openapi_json_is_up_to_date` (cfg `oidc`+`nats`+`reset-data` — run via `./scripts/regen-api.sh`,
then assert the staleness test actually ran with a non-zero count), the generated-frontend
staleness check, and `scope-map.golden.json` (route deletion removes a row — regenerate with
`UPDATE_SCOPE_MAP=1` under the features its test declares, non-zero count asserted). AsyncAPI is
untouched (no wire-type change).

## Quality gates

Per `docs/development/quality-gates.md`, notably: `cargo fmt` / `clippy` / `test` on both
canonical feature worlds; `./scripts/regen-api.sh` with `openapi.json` +
`frontend/src/lib/api/generated/` committed in the same commit as the contract change;
`python3 ci/verify_db_access_policy.py` (deleted route row); `python3
ci/verify_action_security_declarations.py`; `bash ci/verify_agents_md_budget.sh` (AGENTS.md
edit); frontend `npm run lint` / `format:check` / `check` / `test` / `build`; markdownlint on
edited docs. No new dependencies (no version pins to record).

## Documentation deliverables

Surgical M1.7 edits (M1.9 owns the full rewrites):

- `AGENTS.md`: fix the now-false sentence "the macro and module survive for later milestones"
  (legacy `permission_extractor!` rule text); budget gate re-run.
- `docs/security/auth-and-authorization.md`: correct every claim falsified by this milestone —
  the JWT `permissions` claim, `me`'s silent empty-list fallback, `GET /api/v1/permissions`.
  Inventory by phrase grep over the whole file (`permissions claim`, `/api/v1/permissions`,
  `has_permission`), not single anchors; every hit edited or explicitly deferred to M1.9 in the
  plan.
- `docs/api/user-management.md` + `docs/end-user/user-management.md`: remove/correct references
  to the deleted `GET /api/v1/permissions` and the `permissions` response fields (full
  model-rewrite stays M1.9). Same grep-driven inventory.
- Code-attached prose: `middleware/action.rs` "until M1.7" comment; `me`'s DEVIATION comment
  (deleted with the fallback); `shim.rs` stays untouched — verified 2026-08-07: it already has
  zero callers outside its own file (a `pub` lib item, so warning-clean); M1.7 uses its mapping
  table as the frontend sweep's reference only, and M1.8 deletes it with `Permission`.
- Generated artifacts: `crates/ui/web-api/openapi.json`, `frontend/src/lib/api/generated/`,
  `crates/ui/web-api/scope-map.golden.json`.
- No ADR here: the model-replacement ADR is M1.9's deliverable per the task breakdown.

## Out of scope / deferred

- Per-action visibility summaries in `me` (M2 frontend deliverable; D13's visibility clause).
- `Permission` enum + `Other(String)`, `actions_for_permission` shim, code-defined preset lists,
  and the `permissions`/`role_permissions` table-drop migration (M1.8).
- Doc full rewrites + replacement ADR + AGENTS.md MUST-FOLLOW rework (M1.9).
- OAuth scopes on principal tokens (M3); `userinfo` (rejected, see Alternatives).
- Any grant/role web UI (post-v1 per `06-grant-model.md`).

## Ledger conformance notes (for the plan author)

- **#18 (consumer inventory)**: the reshape's consumers are enumerated here (SPA, e2e fixtures,
  CLI display + tests, openapi-client re-exports, web-api-types serde tests); re-run the
  workspace-wide field greps (`\.permissions`, `UserResponse {`, `UserWithRolesResponse {`) at
  plan-write time and diff.
- **#20 (deletion sweep site classes)**: `permissions` appears as struct fields, JSON fixture
  keys, e2e mock payloads, doc prose, and the `db_access_policy.toml` / scope-map rows — sweep
  by site class with no extension filter.
- **#51 (synthetic principals)**: every allow-leg test stages real grant rows; the claim-stuffed
  path ceases to exist — tests that relied on it (`settings.rs:74` etc.) are re-staged, never
  ungated.
- **#52/#57 (goldens)**: pre-declared above with per-artifact resolution and non-zero-count
  assertions on filtered test runs.
- **#43/#56 (commit boundaries)**: `allowed_actions` is a `pub` lib API on `AccessEngine` —
  `pub` items satisfy dead-code, so it may land one task ahead of its first caller with its
  unit tests (plan Task 1); the ledger rule's producer-before-consumer constraint binds
  non-`pub` items only, and the private helper `effective_actions` accordingly lands with the
  login-family conversion in the same task. (Amended 2026-08-07 during plan review — the
  original wording required same-task landing for both.)
- **#44 (commit mechanics)**: breaking commits (`feat(web-api)!`, `feat(cli)!`) carry bodies
  naming the broken contracts + `BREAKING CHANGE:` footers.

# OAuth Clients/Consents API — OpenAPI Exposure Design

Date: 2026-08-05. Status: implemented (2026-08-05 plan).

## Problem

Six fully-annotated handlers are registered with plain `.route(...)` in `crates/ui/web-api/src/router.rs` instead of
`.routes(routes!(...))`, so they are silently absent from the generated OpenAPI document:

| Handler | Method + path | Auth gate |
| --- | --- | --- |
| `routes/oauth/clients_api.rs::list_clients` | `GET /api/oauth/clients` | `CanManageSettingsAuth` |
| `routes/oauth/clients_api.rs::manual_register_client` | `POST /api/oauth/clients` | `CanManageSettingsAuth` |
| `routes/oauth/clients_api.rs::revoke_client` | `DELETE /api/oauth/clients/{client_id}` | `CanManageSettingsAuth` |
| `routes/oauth/clients_api.rs::trust_client` | `POST /api/oauth/clients/{client_id}/trust` | `CanManageSettingsAuth` |
| `routes/oauth/consents_api.rs::list_consents` | `GET /api/oauth/consents` | `require_auth` (user-scoped) |
| `routes/oauth/consents_api.rs::revoke_consent` | `DELETE /api/oauth/consents/{id}` | `require_auth` (user-scoped, ownership-checked) |

Registration sites: `router.rs` `auth_routes` chain (clients block and consents block; both inside the
`require_auth`-layered section — `routes/oauth/mod.rs` documents the deliberate placement).

Consequences (all verified 2026-08-05):

- `crates/ui/web-api/openapi.json` has zero `/api/oauth/clients` or `/api/oauth/consents` entries;
  `DcrRegistrationRequest`/`DcrRegistrationResponse` schemas are likewise absent.
- `uptrakit-openapi-client` has no methods for any of the six — violates the root AGENTS.md openapi-client sync rule
  (its exclusions are WebSocket, OIDC browser callback, OCSP; none apply).
- `frontend/src/lib/api/oauth.ts` hand-writes these calls (a sanctioned escape hatch per `frontend/AGENTS.md`, but
  with real type drift — see below).
- `crates/ui/web-api/scope-map.golden.json` omits all six, so the four action-gated clients operations are invisible
  in the M1.4b scope-map inventory.

**No security gap.** Enforcement is runtime via the `action_extractor!` type `CanManageSettingsAuth` (clients) and the
`require_auth` middleware plus in-handler ownership checks (consents). `ci/verify_action_security_declarations.py`
source-scans `routes/` and already validates `clients_api.rs` (4 converted operations); it never reads `openapi.json`,
so this change does not alter its behavior. The hole is documentation, inventory, and client generation only.

### Latent defects this design also fixes

- `list_consents` and `list_clients` build responses with inline `serde_json::json!` — no typed response structs exist,
  so a naive spec merge would produce schema-less operations and useless codegen.
- `consents_api.rs` declares **no** `security(...)` at all; merged as-is, both consent operations would classify as
  `public` in the scope-map golden, misrepresenting the actual `require_auth` gate.
- All six operations emit timestamps through `time::OffsetDateTime`'s **default serde** (inside `json!`), producing
  component arrays (`[2026,217,…]`) instead of RFC 3339 strings — unusable by `new Date()`/`formatDate` and contrary
  to the `uptrakit-web-api-types` convention (`#[serde(with = "time::serde::rfc3339")]` everywhere else).
- Frontend breakage (not mere drift): `listOAuthClients()` is typed `Promise<OAuthClient[]>` but the server returns a
  `PaginatedResponse` envelope, never unwrapped — `McpAccessTab.svelte` assigns the envelope to an array-typed
  variable, so `clients.length` is `undefined`, the empty-state check is skipped, and the table iterates a non-array:
  the MCP Access clients list does not render today. The TS `OAuthConsent` declares `client_name`/`last_used_at` which
  the server never emits, so the authorized-apps page renders phantom fields (blank name, "Never"); `OAuthConsent.scopes` is typed `string[]` but
  the server emits a space-delimited `String` (the authorized-apps page calls `.join(', ')` on it);
  `manualRegisterClient()` is typed `Promise<OAuthClient>` but the handler returns `DcrRegistrationResponse`.
- `RegisterClientDialog.svelte` is broken today: its POST body omits `grant_types`/`response_types` — required,
  non-defaulted fields of `DcrRegistrationRequest` — and sends `default_scope` where the server field is `scope`, so
  axum's `Json` extraction rejects the request before the handler runs. The migration to generated types surfaces and
  fixes this.
- Both revoke flows (client revoke in `McpAccessTab.svelte`, consent revoke in `authorized-apps/+page.svelte`) fire
  immediately on click with no `ConfirmDialog`, violating the destructive-action design rule (the danger button
  variant is already present; only the confirmation gate is missing).

## Decisions

Owner-approved 2026-08-05:

1. **Registration: convert to `.routes(routes!(...))`** — the canonical mechanism per `crates/ui/web-api/AGENTS.md`
   ("a handler … never passed to `routes!()` is silently absent"). Rejected alternative: a merged
   `#[derive(OpenApi)]` Doc struct (the `ZeroconfApiDoc`/`OAuthSettingsApiDoc`/`OidcApiDoc` pattern) — lower churn but
   creates a second registration surface per handler, is undocumented in the web-api AGENTS.md, and leaves audit-catalog
   alias debt alive. Sibling blocks in the same `auth_routes` chain (discovery-allowlist, OIDC providers, MFA) already
   use `.routes(routes!(...))`, proving the `require_auth` layering survives conversion.
2. **Typed responses with consents enrichment** — new response structs in `uptrakit-web-api-types`; `list_consents`
   joins `oauth_client` to add `client_name`, fixing the authorized-apps display gap the hand-written types papered
   over. Rejected alternative: mirror today's JSON exactly (codifies the gap). Review amendment (2026-08-05): the
   owner-approved option included client-level `last_used_at` "only if the column actually exists" — the column
   exists but is **never written** (verified: all writes are `Set(None)`), so per that caveat's intent the consents
   enrichment is `client_name` only; see §1.

## Design

### 1. Typed response structs — `uptrakit-web-api-types`

Location: `crates/shared/web-api-types/src/oauth/responses.rs` (alongside `DcrRegistrationRequest`/`DcrRegistrationResponse`;
same `#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]` gating).

- `OAuthClientResponse` — `id: String`, `client_name: String`, `client_uri: Option<String>`,
  `redirect_uris: Vec<String>`, `created_via: String`, `created_at: OffsetDateTime`,
  `revoked_at: Option<OffsetDateTime>`, `trusted_at: Option<OffsetDateTime>`. (`redirect_uris` is stored as a JSON
  string in `oauth_clients.redirect_uris`; the handler already parses it — the typed struct makes the element type
  explicit. Parse failure keeps the current behavior: empty list.) **`last_used_at` is dropped from the response**:
  `oauth_clients.last_used_at` is never written anywhere in the codebase (every `ActiveModel` site sets `Set(None)`;
  no token-issuance update exists), so today's field is a permanent null and the MCP Access "Last used" column
  renders the constant "Never". Publishing a never-written field in the new typed contract would codify the fiction;
  re-adding it is purely additive once a write site exists (deferred). Both UI "Last used" columns are dropped (§6).
- `OAuthConsentResponse` — current fields `id: Uuid`, `client_id: String`, `scopes: String`,
  `granted_at: OffsetDateTime`, plus **new** `client_name: String` (non-optional) sourced from the joined
  `oauth_client` row. Non-optional is safe: `fk_oauth_consents_client` is `ON DELETE RESTRICT` and client revocation
  is a soft delete (`revoked_at`), so a consent's client row always exists. The `find_also_related` `None` branch is
  FK-unreachable — handle it as a logged 500 (defensive, no `unwrap`), not an `Option` in the contract.

Timestamp encoding — **intentional response-format fix, not a byte-for-byte mirror**: every timestamp field in both
structs pairs `#[serde(with = "time::serde::rfc3339")]` (`::option` for `Option` fields) **with**
`#[cfg_attr(feature = "openapi", schema(value_type = String, format = DateTime))]` (`Option<String>` for optionals)
— copy the `api_tokens.rs` precedent verbatim. The schema attribute is mandatory, not decorative: the workspace
`utoipa` has no `time` feature, so `OffsetDateTime` does not implement `ToSchema` and the struct will not compile
without it. Today's `json!` emission goes through `time::OffsetDateTime`'s default serde, which produces a component
array (`[2026,217,13,…]`, empirically verified) — unparseable by `new Date()` and mislabeled by any generated client.
RFC 3339 strings fix that; the OpenAPI `string($date-time)` schema then tells the truth. Single-deployment reality:
the only consumers are the two broken frontend pages this spec migrates anyway; `uptrakit-openapi-client` and the CLI
have no existing methods parsing the array form (verified).

Response shapes:

- `list_clients` → `PaginatedResponse<OAuthClientResponse>` (generic `ToSchema` instantiation precedent:
  `PaginatedResponse_HostResponse` et al. in `openapi.json`).
- `list_consents` → bare `Vec<OAuthConsentResponse>` — stays unpaginated, matching today's bare-array JSON
  (user-scoped short list; adding pagination is out of scope).

Secret-safety: neither new struct carries secrets (no `client_secret_hash`/`registration_access_token_hash` — the
current handler comment "internal hashes are never exposed" carries over). No `SecretString` fields needed in the new
structs. Acknowledged: the pre-existing `DcrRegistrationResponse.registration_access_token: Option<String>` (a bearer
credential typed `String`, predating this spec) now enters the published schema, generated TS, and the new
openapi-client return type — the new client modules must not `Debug`-log responses; converting that field to
`SecretString` is deferred (touches the existing type and its emission path).

The operator-issued `registration_access_token` returned by `POST /api/oauth/clients` is **intentionally discarded**
by the UI today (and even more definitively after the refetch amendment in §6) — show-once treatment, API-token
style, is deferred.

### 2. Handler changes — `crates/ui/web-api/src/routes/oauth/`

- `clients_api.rs::list_clients`: build `OAuthClientResponse` rows instead of `json!`; add
  `body = PaginatedResponse<OAuthClientResponse>` to the `200` response in `#[utoipa::path]`. Add a deterministic
  order: `.order_by_desc(CreatedAt)` with `.order_by_asc(Id)` tiebreak — today's query has **no `ORDER BY`**, so
  page order is unspecified, cross-page pagination is unstable, and the §6 refetch-after-register amendment would
  not reliably surface the new client without it.
- `consents_api.rs::list_consents`: replace the per-row `json!` with a single joined query
  (`oauth_consent::Entity::find().find_also_related(oauth_client::Entity)` filtered by `user_id` +
  `oauth_consent::Column::RevokedAt.is_null()` — qualify the column: after the join **both** entities carry
  `revoked_at`, and the filter targets the consent's) — one query, no N+1; add `body = Vec<OAuthConsentResponse>` to
  the `200` response and `.order_by_desc(GrantedAt)` (same unordered-query defect).
- Both `consents_api.rs` operations: add `security(("oauth2" = []), ("developer_token" = []))` — the established
  authenticated-only annotation (precedent: `routes/auth.rs` `/api/v1/auth/me` family). This classifies both as
  `authenticated-only` in the scope map. `verify_action_security_declarations.py` remains satisfied: the file imports
  nothing from `middleware::action`, so only its R3 check applies. Note: `Extension<AuthenticatedUser>` here is
  authentication (identity for the ownership filter), not authorization — these operations carry no permission
  requirement by design (module doc: end-user endpoints), so the typed-permission-extractor rule does not apply; this
  is not a legacy-`x-required-permission` holdout. Deliberate semantic being documented: scope-less `oauth2` means
  **any** MCP access token acting for the user can list and revoke that user's consents (including its own grant) —
  `/auth/me`-class self-service, correct for these endpoints; the scope-map diff review confirms it consciously.
- No `operation_id` overrides: utoipa derives operationIds from fn names (`list_clients`, `manual_register_client`,
  `revoke_client`, `trust_client`, `list_consents`, `revoke_consent`); client methods use the same names.

### 3. Router + audit catalog — `crates/ui/web-api`

- Convert the clients block and consents block in `router.rs` `auth_routes` to `.routes(routes!(...))`, preserving
  position inside the `require_auth`-layered chain. `routes!` merges same-path methods
  (`GET`+`POST /api/oauth/clients`) exactly as the sibling MFA/OIDC blocks do.
- **Audit catalog**: the `.route()` walker style created four `router::` alias skip entries in
  `crates/shared/audit-log/audit-catalog.toml` (`router::manual_register_client`, `router::revoke_client`,
  `router::revoke_consent`, `router::trust_client`). Conversion makes those alias sites vanish;
  `cargo xtask audit-coverage-check` reports stale skips. **Delete the four alias entries in the same change** and run
  `cargo xtask audit-coverage-check` in-task. The real handler-qualified entries
  (`routes::oauth::clients_api::*`, `routes::oauth::consents_api::revoke_consent`) stay untouched; in-handler audit
  emission is unchanged.
- Widen the `ApiDoc` OAuth tag description (currently device-grant/metadata-specific) to cover client and consent
  management.
- Extend the pinned path list in `openapi_spec_eligible_endpoints_present`
  (`src/integration_tests/openapi_spec.rs`) with the six paths — the regression guard that catches any future slide
  back to raw `.route()`.

### 4. Regeneration — expected golden moves

1. `./scripts/regen-api.sh` — regenerates `crates/ui/web-api/openapi.json` (byte-equality golden
   `openapi_json_is_up_to_date`) and `frontend/src/lib/api/generated/`. Prerequisite: `frontend/node_modules` present
   (`npm ci` first on a fresh tree — the script false-greens without it).
2. `UPDATE_SCOPE_MAP=1 cargo test -p uptrakit-web-api --all-features scope_map` — **separate step**; `regen-api.sh`
   does not refresh the scope-map golden.

Expected diffs, pre-declared:

- `openapi.json`: +6 operations, +schemas (`OAuthClientResponse`, `OAuthConsentResponse`,
  `PaginatedResponse_OAuthClientResponse`, `DcrRegistrationRequest`, `DcrRegistrationResponse`).
- `frontend/src/lib/api/generated/`: new methods + TS types for the six operations.
- `scope-map.golden.json`: +6 rows — 4× `oauth2:settings.auth:manage`, 2× `authenticated-only`. Review the diff; any
  other classification is a defect.

### 5. `uptrakit-openapi-client`

- `paths.rs`: new constants in the existing `oauth` module for the five path templates (`/api/oauth/clients`,
  `/api/oauth/clients/{client_id}`, `/api/oauth/clients/{client_id}/trust`, `/api/oauth/consents`,
  `/api/oauth/consents/{id}`). Off-`/api/v1` paths are already supported — `base_url` is a trimmed origin and the
  prefix lives in each constant (precedents: `oauth::METADATA`, `health::HEALTHZ`).
- Two new resource modules mirroring the routes-side split: `src/oauth_clients.rs` (4 methods) and
  `src/oauth_consents.rs` (2 methods), each a bare `impl UptrakitClient` block following the `settings.rs` pattern.
  Method names = operationIds (fn names) so `cargo xtask openapi-client-check` name-matching passes without
  `RENAME_MAP` or `SPEC_ONLY` entries.
- Request/response types come from `uptrakit-web-api-types` re-exports (`DcrRegistrationRequest`,
  `DcrRegistrationResponse`, `OAuthClientResponse`, `OAuthConsentResponse`, `PaginatedResponse`, `PaginationParams`).
- Gate: `cargo xtask openapi-client-check` (every non-`SPEC_ONLY` operationId needs a client method; every spec path
  needs a `paths.rs` template).

### 6. Frontend migration

- Migrate to the generated SDK: `McpAccessTab.svelte` (list — **unwrap `.items` from the envelope**, register via
  dialog, revoke, trust), `RegisterClientDialog.svelte`, `settings/account/authorized-apps/+page.svelte` (list + revoke
  consents; now renders the real, always-present `client_name`). **Both pages drop their "Last used" columns** —
  `McpAccessTab.svelte` and `authorized-apps/+page.svelte` alike render the constant "Never" off a never-written /
  phantom field; the rule is applied uniformly (§1).
- `McpAccessTab.svelte` register callback: replace the optimistic prepend (`clients = [client, ...clients]`) with a
  **refetch of the list** after successful registration — `DcrRegistrationResponse` has `client_id` (not `id`) and no
  `created_at`/`revoked_at`/`trusted_at`, so an optimistic row has an undefined id (trust/revoke target nothing) and
  misrenders status.
- Migration specifics the generated types force:
  - `RegisterClientDialog.svelte` builds a full `DcrRegistrationRequest`: add `grant_types: ['authorization_code']`,
    `response_types: ['code']` (both in the server's allowlists; the struct's own test fixture uses exactly these) and
    rename `default_scope` → `scope`. `onSuccess` receives a `DcrRegistrationResponse`, not the phantom `OAuthClient`.
  - `authorized-apps/+page.svelte` scopes display: `scopes` is a space-delimited string — render via
    `scopes.split(' ').join(', ')` (the current `.join(', ')` on the mistyped array becomes a TS error).
- Both revoke flows (client revoke in `McpAccessTab.svelte`, consent revoke in `authorized-apps/+page.svelte`) gain a
  `ConfirmDialog` gate (danger button variant already present), closing the destructive-action design-rule violation
  ("Destructive actions require danger treatment plus ConfirmDialog confirmation", `docs/development/ui/README.md`);
  fixed here because the migration already rewrites these handlers. Follow the `hosts/+page.svelte` precedent.
- Shrink `frontend/src/lib/api/oauth.ts`: delete `listOAuthClients`, `revokeOAuthClient`, `trustOAuthClient`,
  `manualRegisterClient`, `listMyConsents`, `revokeMyConsent` and the hand-maintained `OAuthClient`,
  `ManualRegisterClientRequest`, `OAuthConsent` interfaces. Keep: browser consent-flow helpers (`getConsentDetails`,
  `approveConsent`, `denyConsent` + `MetadataDiff`, `ConsentDetails`) and the `/api/v1` settings passthroughs.
- The browser consent-flow endpoints (`/oauth/consent/{request_id}`, `/approve`, `/deny`) **stay off-spec** — they are
  a browser-interactive flow, the same exclusion class as the OIDC browser callback.

### 7. Documentation deliverables

- `docs/api/http-web-api.md`: add entries for the six endpoints (currently documents only device-grant/token/metadata
  OAuth paths). The `POST /api/oauth/clients` entry explicitly disambiguates the operator manual-registration endpoint
  from RFC 7591 dynamic registration at `POST /oauth/register` (which stays off-spec, below).
- Root `AGENTS.md`, openapi-client sync rule: extend the exclusion list with "OAuth protocol and browser consent-flow
  endpoints (RFC-discovered)" (one clause; respects the 500-line budget). This names the intentional residual
  precisely — see §9.
- `frontend/AGENTS.md` escape-hatch note: narrow the OAuth exception wording to the consent flow.
- No ADR: no new architectural decision — this applies the existing registration mechanism to handlers that missed it.
- This spec's Status line flips to "implemented" by the implementing plan.

### 8. Tests

Via the shared `TestApp` harness (`crates/ui/web-api/src/test_harness/`), success + failure paths:

- `list_consents` enrichment: consent → `client_name` populated with the client's actual name (positive-content
  assertion on the changed value, not just shape); plus the real cascade invariant, buildable through the handlers:
  revoking a client (which cascade-revokes its consents in `OAuthClientService::revoke`) removes those consents from
  `list_consents`. No missing-client test (`fk_oauth_consents_client` is `ON DELETE RESTRICT`, FKs on in tests —
  unconstructible) and no revoked-client-still-listed test (the cascade makes that state equally unconstructible
  through production paths); the non-optional `client_name` rests on the schema property, not a test.
- `list_clients`: typed envelope round-trip (`items` element fields present, pagination fields correct).
- Timestamp format: at least one assertion pins the **emitted string shape** (e.g. `granted_at` parses via
  `OffsetDateTime::parse(&s, &Rfc3339)`) so the RFC 3339 fix is load-bearing, not incidental — today's emission is a
  component array and this is the assertion that would catch a regression to default serde.
- Existing goldens: `openapi_json_is_up_to_date`, `scope_map_golden_is_up_to_date`,
  `openapi_spec_eligible_endpoints_present` (extended — asserts the six **exact path strings** as a guard against a
  future slide back to raw `.route()`, which would drop the ops from the spec again; it cannot catch
  annotation-vs-route divergence since it reads the annotation-derived document — parity of the six annotation paths
  with today's `.route()` literals is instead verified once, by diff, in the conversion change itself; verified
  byte-identical at spec time), `asyncapi` untouched (no wire change).
- Manual smoke on the dev instance (the affected UI is broken today, so green gates alone are weak evidence):
  register → list renders → trust → revoke in MCP Access; authorized-apps lists a consent with its real client name
  and revokes it behind the new `ConfirmDialog`.
- Gates: `cargo xtask audit-coverage-check`, `cargo xtask openapi-client-check`,
  `ci/verify_action_security_declarations.py`, `ci/verify_db_access_policy.py` (the six stay classified `full-state`
  in `db_access_policy.toml` — no change; migrating them off `State<Arc<AppState>>` is separate debt),
  frontend `npm run lint` / `check` / `build`.

### 9. Residual: OAuth protocol endpoints stay off-spec (intentional)

Beyond the six, ten more annotated handlers are mounted with plain `.route()` via `build_oauth_router`
(`routes/oauth/mod.rs`, merged into the router **after** `split_for_parts()`): `register.rs` (RFC 7591 dynamic
registration `POST /oauth/register` + the RFC 7592 management triad `GET/PUT/DELETE /oauth/register/{client_id}`),
`token.rs`, `authorize.rs`, and `consent.rs` (browser consent flow). These stay off-spec **deliberately**:
`POST /oauth/register`, `token`, and `authorize` are RFC-governed protocol endpoints discovered by machine clients
via RFC 8414 metadata (`/.well-known/oauth-authorization-server`); the consent flow is browser-interactive; and the
RFC 7592 management triad authenticates **solely** by the per-client `registration_access_token` (verified:
`register.rs` compares only against `registration_access_token_hash`) — an openapi-client method for it would be
unauthenticatable with operator session/oauth2/developer-token credentials. None are product API surface. The
widened AGENTS.md exclusion clause (§7) names exactly this class, so the openapi-client sync rule is closed honestly
rather than left violated by construction. Their `#[utoipa::path]` annotations remain as in-code documentation.

Frontend note: `RegisterClientDialog.svelte` hardcodes `mcp:read`/`mcp:write` scope options; the M1.5 rollout owns
scope-catalog churn (`mcp:use` seeded, `mcp:read`/`mcp:write` dropped by owner decision), so this migration leaves
the scope-option list byte-untouched.

## Constraints and invariants honored

- No raw SQL; the consents join uses SeaORM `find_also_related` (one query — batch-query invariant).
- No new dependencies; no DB migration (join uses the existing `oauth_consent → oauth_client` relation).
- OpenAPI params stay `params(<IntoParamsStruct>)` (`PaginationParams` already complies; ADR-0025).
- `oauth_clients`/`oauth_consents` are instance-scoped entities (no `tenant_id`) — `TenantDb` helpers do not apply.
- Wire protocol untouched — no `WireValidate`/asyncapi impact.
- Enforcement unchanged: no security gap exists before or after; this is documentation + inventory + codegen.

## Out of scope

- `routes/events.rs` `stream_events` (`/api/v1/events/stream`) stays unregistered — it carries no `#[utoipa::path]` at
  all and SSE does not model usefully in OpenAPI; the CLI/openapi-client hand-roll SSE by design.
- Pagination for `list_consents`.
- Migrating the six handlers off `full-state` (`State<Arc<AppState>>`) — existing tracked debt.
- Enriching `list_consents` beyond `client_name` (e.g. `client_uri`, logos).
- The OAuth protocol + browser consent-flow endpoints' OpenAPI exposure (§9 — intentional residual).
- A write site for `oauth_clients.last_used_at` (token issuance) — the field re-enters `OAuthClientResponse`
  (purely additive) once it exists.
- Converting `DcrRegistrationResponse.registration_access_token` to `SecretString` (pre-existing type), and show-once
  UI treatment of that token (discarded by the UI today).
- Touching the register dialog's scope-option list (`mcp:read`/`mcp:write` → M1.5-owned churn).
- The register dialog's `token_endpoint_auth_method` select: `client_secret_basic` is decorative today (the server
  sets `client_secret_hash: None` unconditionally and the token path never reads the method) — restricting the
  select or implementing secret-based auth is a separate decision; the migration carries the select over unchanged.

## Provenance

Discovered during M1.4b batch B6 (2026-08-05); pre-existing, not introduced by that batch. All code claims in this
spec were re-verified against the working tree on 2026-08-05.

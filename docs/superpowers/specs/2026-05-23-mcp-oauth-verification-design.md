# MCP OAuth Verification

**Date:** 2026-05-23
**Status:** Approved

## Problem

Four gaps prevent MCP OAuth from working at all:

1. **Boot path not wired.** `AppStateBuilder` in `controller-runtime/src/lib.rs` never calls `.oauth()`.
   OAuth is permanently `OAuthState::disabled()` in production regardless of DB settings — no
   request ever reaches an active authorization server.
2. **Seed forces `false`.** `seed_oauth_defaults` writes `mcp_enabled = false` on first boot,
   permanently overriding the auto-enable intent for new installations.
3. **Settings UI has no Save button.** Every `onchange` fires a live API call. No way to review a draft before committing.
4. **E2E test is a skeleton.** Steps 5–9 of `oauth_end_to_end_mcp_rs_round_trip` are `todo!()` stubs. Consent bypass infrastructure is missing.

## Scope

Four self-contained work items. Items 1 and 2 are prerequisites for item 4 (the E2E test cannot pass unless OAuth actually boots).

Out of scope: asymmetric JWT, JWKS, multi-controller, granular scopes — deferred to Phase 2 per ADR 0010.

---

## Work Item 1 — Wire the boot path

### What's missing

`controller-runtime/src/lib.rs` builds `AppState` without calling `.oauth(...)` on the builder.
The builder falls through to `OAuthState::disabled()` unconditionally. `validate_and_register`
and all OAuth boot logic exist but are never called.

### New boot phase (Phase 7d)

After `seed_oauth_defaults` (Phase 7c) and before `validate_configuration` (Phase 8), add:

```rust
// Phase 7d: OAuth boot — wire OAuthState when mcp_enabled resolves to true.
let oauth_state = startup::oauth::boot_oauth_state(&db_conn)
    .await
    .map_err(|e| e.change_context(AppError::Config("OAuth boot failed".into())))?;
```

`boot_oauth_state` returns `Result<OAuthState, rootcause::Report<OAuthBootError>>`.
It lives in `crates/core/controller-runtime/src/startup/oauth.rs` and does:

1. Read `oauth.mcp_enabled` as `Option<bool>` (absent = None, not false).
2. Read `oauth.canonical_host`.
3. Call `uptrakit_web_api::oauth::resolve_mcp_enabled(explicit, canonical_host.as_deref())`
   — see Work Item 2 for definition.
4. If resolved `false` → return `Ok(OAuthState::disabled())` (fast path, no further DB reads).
5. If resolved `true`:
   a. **Required signature change** — change `validate_and_register` in `oauth/boot.rs` from
   `db: &DatabaseConnection` to `db: &impl ConnectionTrait` (SeaORM supertrait already in
   scope). Update call sites (currently only `oauth_boot_validation.rs` tests).

   With that change, open one `BEGIN IMMEDIATE` transaction covering both the secret
   generation and the peer registration:
   - Read `oauth.jwt_signing_secret`; if null, generate a 32-byte cryptographically random
     secret and write it back via `upsert_global_setting_raw` within the open transaction.
   - Compute the fingerprint from the secret.
   - Call `validate_and_register(&tx, &boot_settings, now)` — passes the transaction handle
     so the peer-check insert is atomic with the secret write.
   - Commit the transaction.

   Without the combined transaction, two overlapping boots can each pass the secret read,
   generate divergent secrets, and produce a spurious `PeerWithDifferentFingerprint` error
   on rapid systemd restart loops.
   b. Read `oauth.accepted_audience_hosts`, TTL settings, rate-limit settings (read-only,
   no transaction required).
   c. Construct and return a live `OAuthState` with real `McpOAuthJwtSigner`,
   `McpOAuthJwtVerifier`, and `CanonicalUrlConfig`.

The `AppStateBuilder` chain gains:

```rust
.oauth(oauth_state)
```

### Auto-signing secret

The signing-secret read-or-generate and the `validate_and_register` peer check must execute in
the same `BEGIN IMMEDIATE` transaction (step 5a). This requires `validate_and_register` to
accept `&impl ConnectionTrait` rather than `&DatabaseConnection` so the open transaction handle
can be threaded through. The generated secret persists; future boots reuse it.

---

## Work Item 2 — Auto-enable default + seed fix

### Seed fix

Remove the explicit `false` seed for `OauthMcpEnabled` from `seed_oauth_defaults`:

```rust
// Remove this line:
seed!(SettingKey::OauthMcpEnabled, serde_json::json!(false));
```

`insert_global_setting_if_absent` is idempotent — existing installations that already have
`mcp_enabled = false` are unaffected. New installations leave the row absent and trigger the
auto-enable logic when `canonical_host` is set.

**Upgrade path:** installations that predate the seed AND already have a non-null `canonical_host`
value in DB will auto-enable OAuth on the first boot after upgrade. Since `oauth` has never
functioned in production (the boot path was not wired), no operator can have intentionally
configured a live OAuth flow — this case is practically unreachable. Log a prominent `tracing::warn!`
in `boot_oauth_state` when the auto-enable fires (absent row + canonical host set) so the operator
has an observable signal in startup logs.

### Auto-enable resolver

Define in `crates/ui/web-api/src/oauth/mod.rs` as `pub` so it is accessible to both
`settings_oauth.rs` (same crate) and `controller-runtime` (which imports `uptrakit_web_api`):

```rust
pub fn resolve_mcp_enabled(explicit: Option<bool>, canonical_host: Option<&str>) -> bool {
    match explicit {
        Some(v) => v,
        None => canonical_host.is_some(),
    }
}
```

| `oauth.mcp_enabled` row | `oauth.canonical_host` row | Resolved                  |
| ----------------------- | -------------------------- | ------------------------- |
| absent                  | absent / null              | `false`                   |
| absent                  | set                        | **`true`**                |
| `false`                 | any                        | `false`                   |
| `true`                  | absent                     | `true` (boot guard fires) |
| `true`                  | set                        | `true`                    |

The boot guard (`CanonicalHostMissing`) remains reachable only via the explicit `true` +
absent-host row — an operator misconfiguration that must crash loudly.

### Settings API accuracy

**Required code change** — the existing chain in `load_oauth_settings_from_db()`:

```rust
.and_then(|v| v.as_bool()).unwrap_or(false)  // current — collapses Option<bool> to bool
```

must change to preserve the `Option<bool>` before resolution:

```rust
let mcp_raw: Option<bool> = load_global_setting_raw(state.db(), "oauth.mcp_enabled")
    .await
    .unwrap_or(None)
    .and_then(|v| v.as_bool());
let mcp = resolve_mcp_enabled(mcp_raw, canonical_host.as_deref());
```

Without this change, `OAuthSettingsFromDb.mcp` is always `false` when the row is absent, causing
the `restart_required` comparison to return `true` permanently after every successful auto-enabled
boot (it compares `false` against `state.oauth.enabled = true`). The resulting permanent
"restart required" banner is a visible correctness break.

### Tests

- Unit test for `resolve_mcp_enabled` covering all five table rows above.
- Extend `oauth_boot_validation.rs`: assert that a DB with `canonical_host` set and no
  `mcp_enabled` row produces a live (non-disabled) `OAuthState` from `boot_oauth_state`.

---

## Work Item 3 — Settings UI Save button

### Location

`frontend/src/routes/settings/authentication/oauth-clients/+page.svelte` — the "OAuth settings" `SectionCard`.

### Design-language requirements

All form controls use design-system primitives per `docs/development/ui/primitives.md`:

| Field                                        | Primitive                                                                                                                                         |
| -------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------- |
| `mcp_enabled`, `dcr_enabled`, `cimd_enabled` | `Checkbox` (`$lib/components/Checkbox.svelte`) with `text-[var(--accent)]` class — not `accent-[var(--accent)]` (inert with `@tailwindcss/forms`) |
| `canonical_host`                             | `FormFieldRow` + `Input`                                                                                                                          |
| Save                                         | `Button` variant `"primary"`                                                                                                                      |
| Discard                                      | `Button` variant `"ghost"`                                                                                                                        |

No raw `<input>` elements inside the settings card.

### State model

```ts
// Mirrors the writable fields of UpdateOAuthSettingsRequest — excludes restart_required.
interface DraftOAuthSettings {
  mcp_enabled: boolean;
  dcr_enabled: boolean;
  cimd_enabled: boolean;
  canonical_host: string | null;
}

let oauthSettings = $state<OAuthSettingsResponse | null>(null); // last persisted
let draft = $state<DraftOAuthSettings | null>(null); // local edits

// Field-by-field comparison avoids JSON.stringify key-order fragility.
const isDirty = $derived(
  draft !== null &&
    oauthSettings !== null &&
    (draft.mcp_enabled !== oauthSettings.mcp_enabled ||
      draft.dcr_enabled !== oauthSettings.dcr_enabled ||
      draft.cimd_enabled !== oauthSettings.cimd_enabled ||
      (draft.canonical_host ?? null) !== oauthSettings.canonical_host),
);
```

`DraftOAuthSettings` mirrors the writable fields of `UpdateOAuthSettingsRequest` only.
`restart_required` is server-computed and must not appear in the draft — it is read from
`oauthSettings` directly for the Callout display.

### Lifecycle

- `onMount` → `loadOAuthSettings()` → sets `oauthSettings` and
  `draft = structuredClone(oauthSettings!)` (non-null assertion valid: assign only after
  successful load; TypeScript `strictNullChecks` requires the assertion).
- All controls bind to `draft` fields only — no live API calls on change.
- **Save** button: `disabled={!isDirty || savingSettings}` — calls `handleSettingsChange(draft)`
  once. On success: `oauthSettings = response; draft = structuredClone(response)`.
- **Discard** button: visible only when `isDirty` — resets `draft = structuredClone(oauthSettings!)`.
- `dcr_enabled` and `cimd_enabled` Checkboxes: `disabled` when `!draft.mcp_enabled`.
- Warning Callout (`canonical_host must be set before enabling MCP OAuth`) shown when `draft.mcp_enabled && !draft.canonical_host`.
- `restart_required` Callout shown from `oauthSettings.restart_required` (persisted value, not draft).
- API errors surface via existing `settingsError` → `Callout tone="danger"`.

---

## Work Item 4 — Consent bypass + E2E test completion

### New handler

File: `crates/ui/web-api/src/routes/test_utils.rs` (existing `#[cfg(feature = "test-utils")]` file).

```rust
/// Approve an OAuth consent request without going through the browser UI.
///
/// Looks up the pending authorization request, creates a consent grant for
/// the authenticated user, issues an authorization code, and returns the
/// redirect URI with `code` and `state` query parameters as JSON.
///
/// Returns 404 when `UPTRAKIT_TEST_UTILS_ENABLED != "true"`.
/// Returns 404 when the authorization request does not exist or is already consumed.
pub(crate) async fn oauth_auto_approve_consent(
    State(state): State<Arc<AppState>>,
    Path(request_id): Path<Uuid>,
    authenticated_user: AuthenticatedUser,
) -> Response {
    if !test_utils_allowed() {
        return StatusCode::NOT_FOUND.into_response();
    }
    // Delegates to the same consent-approval service used by
    // POST /oauth/consent/{request_id}/approve, skipping CSRF and
    // browser-redirect checks.
    // Returns JSON: { "redirect_uri": "https://...?code=<code>&state=<state>" }
    ...
}
```

**Production safety — two independent guards:**

1. Compile-time: `#[cfg(feature = "test-utils")]` on the entire file; absent from release binary unless the feature is explicitly compiled in.
2. Runtime: `test_utils_allowed()` checks `UPTRAKIT_TEST_UTILS_ENABLED=true`; returns 404 otherwise.

### Router mount

In `crates/ui/web-api/src/router.rs`, inside the existing `#[cfg(feature = "test-utils")]` block:

```rust
.route(
    "/oauth/test/auto-approve/{request_id}",
    axum::routing::post(crate::routes::test_utils::oauth_auto_approve_consent),
)
```

Requires a valid bearer token. The authenticated user becomes the consent grantor.

### `ApiClient` additions

In `crates/core/integration-tests/` test helpers:

```rust
impl ApiClient {
    /// PUT /api/v1/global-settings/oauth — set canonical_host to enable OAuth.
    pub async fn update_oauth_settings(
        &self,
        canonical_host: &str,
    ) -> serde_json::Value { ... }

    /// POST /oauth/test/auto-approve/{request_id}
    /// Returns the authorization `code` extracted from the redirect_uri.
    pub async fn auto_approve_consent(&self, request_id: &str) -> String { ... }
}
```

### E2E test revised flow

`crates/core/integration-tests/tests/oauth_end_to_end.rs` replaces `todo!()` stubs:

1. Start controller container.
2. Register test user; obtain API token.
3. `PUT /api/v1/global-settings/oauth` with `{ "canonical_host": "127.0.0.1:<port>" }`. Auto-enable fires (no `mcp_enabled` field needed).
4. Read `X-Reexec-Generation` from `GET /healthz` to get `current_gen`. Call
   `POST /test/force-reexec` → controller rexecs with OAuth now booting live. Call
   `api_client.wait_for_generation(current_gen + 1, Duration::from_secs(30))` — the
   existing helper retries healthz, ignoring connection-refused errors during the reexec
   gap, and returns only when the new generation is confirmed. Do not proceed to step 5
   until `wait_for_generation` returns.
5. `POST /oauth/register` (DCR) → get `client_id`.
6. `GET /oauth/authorize?...` with PKCE — assert the response is a 302 whose `Location` header
   starts with `/oauth/consent/` (if it redirects directly to `redirect_uri` with `code=`, a
   prior consent grant exists for this client — the test must use a fresh client per run or revoke
   existing grants before step 5). Extract `request_id` from the path segment.
7. `POST /oauth/test/auto-approve/<request_id>` → extract `code`.
8. `POST /oauth/token` with `code` + `code_verifier` → get `access_token`.
9. `POST /mcp` with `Authorization: Bearer <access_token>` → assert HTTP 200.

Remove `#[ignore]` once all steps are wired. The test still requires `uptrakit-test:latest` Docker
image. Run: `cargo test -p uptrakit-integration-tests -- --ignored`.

---

## Quality gates

All existing gates apply. Additionally:

- `cargo test --all-features -p uptrakit-web-api` — `resolve_mcp_enabled` unit tests + extended `oauth_boot_validation` tests pass
- `cargo test --all-features -p uptrakit-web-api` — `oauth_master_switch_off` tests still pass (disabled path unchanged)
- `cd frontend && npm run lint && npm run check && npm run test` — no regressions
- `cargo test -p uptrakit-integration-tests -- --ignored` — `oauth_end_to_end_mcp_rs_round_trip` passes (requires Docker + `uptrakit-test:latest`)

## Documentation deliverables

- `docs/admin/oauth-clients.md` — update first-run checklist: OAuth auto-enables when
  `canonical_host` is set; no explicit `mcp_enabled = true` step needed. Note that a controller
  restart (or reexec) is required after changing OAuth settings.
- `docs/development/oauth-mcp.md` — add `boot_oauth_state` + `resolve_mcp_enabled` to the
  developer boot-sequence section; note the `force-reexec` pattern for integration tests.
- No new ADR required: the boot-path wiring completes the already-approved ADR 0010 design;
  the auto-enable is a conservative default, not a structural decision.

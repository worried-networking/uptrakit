# CIMD Relying-Party Support — Design

Date: 2026-08-11. Status: approved for planning.

## Problem

Configuring OIDC login today requires manually registering Uptrakit as a client in the IdP and copying a
client_id/client_secret pair into the provider form. Pocket ID ≥ 2.13.0 (PR pocket-id/pocket-id#1526) supports OAuth
Client ID Metadata Documents (CIMD, `draft-ietf-oauth-client-id-metadata-document-02`): the client_id _is_ an HTTPS
URL; the IdP fetches a JSON metadata document from it, eliminating manual registration. Uptrakit already implements
CIMD on the authorization-server side for MCP clients (`crates/ui/web-api/src/oauth/cimd.rs`); this design adds the
**relying-party** side: Uptrakit publishes its own client metadata document and OIDC providers gain a registration
mode that uses its URL as the client_id.

## Protocol constraints that shape the design

Verified against `draft-ietf-oauth-client-id-metadata-document-02` and Pocket ID v2.13.0 (fosite fork
`pocket-id/fosite` v1.2.0):

- The document's `client_id` field must byte-for-byte equal the URL it is fetched from; the authorization-request
  `redirect_uri` must exact-string-match an entry in the document's `redirect_uris`. No normalization anywhere.
- The URL must be `https`, contain a path component, no userinfo/fragment (Pocket ID also rejects query components).
- Pocket ID accepts only `token_endpoint_auth_method: "none"` (public client) and forces PKCE; `private_key_jwt` is
  not supported. `token_endpoint_auth_method` must be **explicitly present** in the document.
- Fetch: must return exactly `200 OK`, JSON content type, ≤ 5 KiB; **redirects are refused**; `Cache-Control:
no-store` documents are **rejected** ("requires metadata caching"); default cache TTL 1 h, cap 24 h.
- Pocket ID refuses to fetch from private/loopback/link-local addresses with no escape hatch — CIMD is structurally
  unavailable for LAN-only deployments. Manual registration remains the working path there.
- IdP support is advertised as `client_id_metadata_document_supported: true` in `/.well-known/openid-configuration`;
  in Pocket ID that flag is computed as "CIMD URL allowlist non-empty", so it can be `true` while Uptrakit's URL is
  not allowlisted, and `false` while the operator is about to allowlist it. The flag is advisory at best.

## Decisions

All grilled with the owner on 2026-08-11; contrarian findings folded in.

1. **Base URL**: reuse the global setting `oauth.canonical_host` (storage key unchanged). Its UI placement moves from
   the MCP Access tab to a general "Instance" settings section. The CIMD code path reads the setting per request via
   the settings store — never `state.oauth.canonical`, which is a boot-time snapshot and a
   `https://disabled.invalid` placeholder while MCP is off (`crates/ui/web-api/src/oauth/mod.rs`,
   `OAuthState::disabled`).
2. **Prerequisite — explicit MCP opt-in**: today `resolve_mcp_enabled` (`crates/ui/web-api/src/oauth/mod.rs:92-97`)
   auto-enables the entire MCP OAuth AS when no explicit `oauth.mcp_enabled` row exists and `canonical_host` is set.
   With the UI move, filling in the canonical host for CIMD would silently boot a public authorization server.
   Invert to explicit opt-in (`None` ⇒ disabled) with a one-shot data migration that materializes the currently
   _resolved_ value into an explicit `oauth.mcp_enabled` row, so no existing deployment changes behavior.
   `resolve_mcp_enabled` has exactly three call sites: `oauth/boot.rs`, its own definition in `oauth/mod.rs`, and
   `routes/settings_oauth.rs` (`load_oauth_settings_from_db`), which feeds the `mcp` flag and `restart_required`
   computation behind `GET /api/v1/global-settings/oauth` — all three must move to the inverted rule together.
3. **Mode column**: `oidc_providers` gains `client_registration` (TEXT NOT NULL DEFAULT `'manual'`, values `manual` |
   `cimd`), backed by a typed enum following the established TEXT-enum-column idiom verbatim:
   `#[derive(DeriveActiveEnum)]` gated `#[cfg_attr(feature = "sea-orm", ...)]` with
   `#[sea_orm(rs_type = "String", db_type = "Text")]` + per-variant `string_value`, plus hand-written `FromStr` +
   typed parse error for non-DB parsing (exemplar: `crates/shared/types/src/service_status.rs`, the same shape as
   `ServiceStatus`/`UpdateStatus`). The enum is **defined in `crates/shared/types/`** (new file mirroring
   `service_status.rs`) and only re-exported by the entity via `pub use` — matching how `ServiceStatus` reaches
   `entity/service.rs`, and required because `web-api-types` and the CLI consume the type without depending on
   `uptrakit-shared-db`. No sentinel semantics. The term "CIMD" is reused deliberately (same protocol, opposite
   role); UI copy says "Automatic registration (CIMD)" and CONTEXT.md gets glossary entries for both roles.
4. **Document endpoint**: `GET /oauth/client-metadata.json`, unauthenticated, mounted beside the existing public
   `/oauth/*` routes. Served whenever `oauth.canonical_host` is set — independent of provider rows, so the IdP's
   cached client never breaks because an operator toggled providers (a 404 on refresh permanently kills the IdP-side
   materialized client). 404 only when the canonical host is unset. Response headers: `Cache-Control: public,
max-age=300` (never `no-store`; short TTL keeps canonical-host changes cheap). Body (serde must omit absent
   optionals entirely, never emit `null`):

   ```json
   {
     "client_id": "https://<canonical-host>/oauth/client-metadata.json",
     "client_name": "Uptrakit",
     "redirect_uris": ["https://<canonical-host>/api/v1/auth/oidc/callback"],
     "token_endpoint_auth_method": "none",
     "grant_types": ["authorization_code"],
     "response_types": ["code"]
   }
   ```

   No `logo_uri` (out of scope). URLs are built from the trailing-slash-stripped canonical origin (the
   `CanonicalUrlConfig::issuer()` shape), so exact-match comparisons at the IdP are stable.

   Mounting: the handler lives in the existing `crates/ui/web-api/src/routes/oauth/` module (not a new module) and
   is mounted as a raw axum route **outside** `OpenApiRouter`, mirroring `/oauth/register` and `/oauth/authorize` —
   it never enters `openapi.json`, so no `SPEC_ONLY`/openapi-client entry exists for it (that is the committed
   precedent for RFC-discovered OAuth endpoints; only `device_authorization`/`token`/`get_as_metadata` are
   utoipa-registered in `router.rs`). Handler-body exemplar: `routes/oauth/metadata.rs::get_as_metadata`
   (conditional 404 + JSON body, no auth middleware).

5. **Canonical-host shape validation**: on save, reject values that are not a bare host (optional port): no scheme,
   userinfo, path, query, or fragment. Today `CanonicalUrlConfig::new` accepts `example.com/app` and
   `user@example.com` (`crates/shared/web-api-types/src/oauth/canonical_url.rs`, `format!("https://{host}")`) — both
   would produce CIMD URLs the IdP rejects or that never exact-match. Changing the canonical host after an IdP has
   cached the document is a **breaking operation** for CIMD (new client identity, re-allowlist required) **and**
   changes the pinned login `redirect_uri` for every provider, manual included (decision 9): the settings UI
   confirmation warning names both consequences, the write purges all pending OIDC flows (decision 9), and the
   change is already audited as a settings write.
6. **Provider CRUD**: create/update requests gain `client_registration`; for `cimd` mode `client_id`/`client_secret`
   must be absent/empty (validation branches on mode; `manual` keeps today's non-empty requirements). Mode switching
   is allowed both directions; switching to `cimd` **keeps** stored credentials (unused, enables switch-back).
   Destroying the stored secret uses the committed tri-state Patch idiom, not a sibling boolean: a
   `ClientSecretPatch` (`Keep`/`Set(SecretString)`/`Clear`) field on `UpdateOidcProviderRequest`, mirroring
   `IconUrlPatch` in `crates/shared/web-api-types/src/software_items.rs` (absent JSON key ⇒ keep, `null` ⇒ clear,
   value ⇒ set); a `Clear` is recorded as `client_secret_cleared` in the audit details. Switching to `manual`
   requires credentials to be present (stored or supplied). The activation completeness gate
   (`routes/oidc_providers.rs`, `activate_provider`, 409 on empty credentials) branches on mode: CIMD rows require
   only a set canonical host. Note activation is exclusive (one active provider), so "a CIMD provider is active"
   means "_the_ active provider is CIMD".

   Column invariant: `client_id`/`client_secret` stay `NOT NULL` (SQLite nullability change would force table
   recreation); empty string is permitted **only** on `cimd`-mode rows, and no `EncryptedString` column elsewhere in
   the codebase carries empty/cleared semantics — so every reader of these two columns must branch on
   `client_registration`, never on emptiness. A `cimd` row's stored `client_id` may also be a stale **non-empty**
   manual value (mode switch keeps credentials), so API responses suppress `client_id`/`has_client_secret` for
   `cimd`-mode rows rather than echoing credentials the provider no longer uses. Plan task: audit all consumers of
   the two columns (admin views, `OidcProviderView`, `has_client_secret` response derivation, CLI display) for
   both non-empty assumptions and stale-value leakage.

7. **Advisory probe, never blocking**: a dedicated endpoint `POST /api/v1/settings/oidc-providers/cimd-probe`
   (gated `CanManageSettingsAuth`, request: `issuer_url` + `allow_private_network_issuers`; no provider row needed,
   so it works before create), following the committed test-then-save precedent
   (`POST /api/v1/plugin-configs/test`, `routes/plugin_configs/test_action.rs`) instead of embedding probe results
   in create/update responses — save handlers stay free of outbound HTTP. Returns a structured `CimdProbeResponse`
   rendered as a callout in the UI; purely advisory, saves never depend on it. Probe steps: (a) raw GET of the
   issuer's `/.well-known/openid-configuration` (a minimal serde struct — the flag is not representable in
   `CoreProviderMetadata`), reporting whether `client_id_metadata_document_supported` is true; (b) resolve the
   canonical host and warn when it lands in a private/loopback range (Pocket ID will refuse to fetch it); (c) GET
   Uptrakit's own document URL with a redirect-refusing client, reporting reachability and any redirect (a reverse
   proxy that 301s breaks CIMD with an opaque IdP-side error) — labeled "as seen from the server; the IdP's view
   may differ". The probe uses the same HTTP client construction as the login flow (`OidcHttpClient` with SSRF
   strictness derived from `allow_private_network_issuers && !multi_tenancy_enabled`), so probe and flow verdicts
   cannot diverge.
8. **No authorize-time flag check**: the discovery flag is advisory (see constraints) and unreadable through
   `CoreProviderMetadata` without a custom-metadata refactor; runtime failures surface through the existing
   `oidc_discovery_failed`/`oidc_token_exchange_failed` login error paths. Not worth doubling discovery fetches.
9. **Flow changes** (`routes/oidc_auth.rs`):
   - `build_oidc_client` branches on `client_registration`, not on field emptiness: CIMD ⇒ client_id = document URL,
     secret `None`. Verified against vendored sources: with `None`, `oauth2` v5 sends `client_id` in the token
     request body and no `Authorization` header (`endpoint.rs`), and `openidconnect` v4 switches to
     `IdTokenVerifier::new_public_client`. Consequence: an IdP minting HS256 ID tokens works in manual mode and
     breaks in CIMD mode — documented in the end-user guide. Branching on emptiness instead would send
     `Authorization: Basic base64("<url>:")`, which Pocket ID rejects for `token_endpoint_auth_method: none`.
   - **Redirect pinning for all providers**: when the canonical host is set, `redirect_uri` is built from it for
     every provider (manual included); header-derived `ExternalBaseUrl`/`base_url_from_headers` remains only the
     fallback when unset. This closes a pre-existing weakness (authorize and callback derive the base independently
     from attacker-influencable `Origin`/`Host` headers) and is required for CIMD's exact match. Explicit behavior
     change: deployments that already set `oauth.canonical_host` for MCP will see their OIDC login `redirect_uri`
     silently change origin on upgrade (previously header-derived) — called out in the ADR and the end-user guide's
     upgrade note; the manually registered redirect URI at the IdP may need updating to the canonical origin.
   - **Pending-flow snapshot**: the exact `redirect_uri` and `client_id` strings used at authorize time are stored
     on the `pending_oidc_flow` row; callback/token-exchange replays the snapshot instead of re-deriving. Any
     provider mutation (update, mode switch, deactivate, delete) purges that provider's pending flows inside the
     same `begin_immediate` transaction, recording the purge count in the audit details. A **changed**
     `oauth.canonical_host` value purges **all** pending flows (every in-flight flow's pinned `redirect_uri` is
     invalidated), likewise recorded in the settings write's audit details. The purge triggers on
     `before != after`, never on field presence — the frontend always sends `canonical_host` in the PUT body
     (`McpAccessTab.svelte`), so a presence-keyed purge would fire on every unrelated OAuth-settings save; the
     handler already loads the before-value in its existing `begin_immediate` transaction
     (`routes/settings_oauth.rs`). The purge itself goes through a dedicated `OidcFlowStore` purge-all helper, not
     an ad-hoc `delete_many` in the settings handler.
   - **Return-origin snapshot**: pinning the `redirect_uri` moves the _callback_ to the canonical origin, but the
     callback today ends in a path-relative `Redirect::to("/login?oidc_code=…")`
     (`routes/oidc_auth.rs`, `create_oidc_exchange_and_redirect`) and the refresh cookie is host-only — so a user
     who started login on a non-canonical origin (LAN IP, VPN hostname, split-horizon DNS) would be stranded on
     the canonical origin, or dead-end entirely if it is not routable from their network. Fix: also snapshot the
     **origin observed at authorize time** on the pending-flow row, validated against the canonical host plus
     `oauth.accepted_audience_hosts` (unlisted or absent ⇒ fall back to canonical); the post-callback browser
     redirect becomes absolute to that stored origin. The IdP still receives the canonical exact-match
     `redirect_uri`; the browser returns to where it started, and the session cookie is minted on that origin by
     the subsequent `POST /api/v1/auth/oidc/exchange` (the exchange code is the existing one-time cross-origin
     handoff). The validation reads **both** `oauth.canonical_host` and `oauth.accepted_audience_hosts` per
     request from the settings store — never `state.oauth` (its disabled placeholder carries an empty alias list
     whenever MCP is off, which would silently re-strand exactly the deployments this fix targets; decision 1's
     per-request rule applies to the whole authorize path, manual providers included). This reuse widens the
     purpose of `oauth.accepted_audience_hosts` (previously MCP audience validation only) — recorded in the ADR
     and the setting's docs, including two inherited constraints: entries compose as `https://{host}` (return
     origins are always https; acceptable since the refresh cookie is `Secure` already) and the list caps at
     `MAX_ACCEPTED_AUDIENCE_HOSTS = 5` (`canonical_url.rs`), a bound chosen for MCP that now also limits return
     origins.
10. **Multi-tenancy**: one document ⇒ one client identity ⇒ one redirect list, instance-global. All tenants share it.
    Structural limitation, recorded in the ADR; acceptable under the single-tenant deployment reality.

## Enforcement-surface inventory (every site that branches on the mode)

| Site                                                                                                                               | Change                                                                                                                                                                                       |
| ---------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `crates/shared/types/` (new enum file) + `crates/shared/db/src/entity/oidc_provider.rs` + migration                                | enum defined in shared-types (decision 3), entity re-exports via `pub use`, new column                                                                                                       |
| `crates/shared/web-api-types/src/oidc_providers.rs`                                                                                | mode field on create/update, `ClientSecretPatch`, mode-branched `Validate`, response gains mode + computed doc URL; new `CimdProbeRequest`/`CimdProbeResponse`                               |
| `crates/ui/web-api/src/routes/oidc_providers.rs`                                                                                   | create/update handlers, `cimd-probe` endpoint, activation gate branch, pending-flow purge                                                                                                    |
| `crates/ui/web-api-queries/src/queries/oidc_providers.rs`                                                                          | params/view structs; `OidcProviderView` must include the new column or the `oidc_provider.update` Stateful audit records a mode switch as a no-op diff                                       |
| `crates/ui/web-api/src/routes/oidc_auth.rs`                                                                                        | `build_oidc_client`, authorize, callback, `oidc_link`/`oidc_exchange`/`oidc_complete_registration` (all rebuild the client), redirect pinning, flow snapshot                                 |
| `crates/shared/db/src/entity/pending_oidc_flow.rs` + migration                                                                     | snapshot columns (`redirect_uri`, `client_id`, return origin)                                                                                                                                |
| `crates/ui/web-api-auth/src/auth/oidc_state.rs`                                                                                    | `OidcFlowStore` insert/take API carries the snapshot fields                                                                                                                                  |
| `crates/ui/web-api/src/routes/oauth/` (existing module)                                                                            | document endpoint handler file; raw-route mount beside `/oauth/register` (decision 4)                                                                                                        |
| `crates/ui/web-api/src/oauth/mod.rs` + `boot.rs` + `routes/settings_oauth.rs`                                                      | `resolve_mcp_enabled` inversion (all three call sites) + materialization migration; `restart_required`/`mcp` reporting unchanged post-migration; canonical-host PUT purges all pending flows |
| `crates/ui/cli/src/commands/settings/oidc.rs`                                                                                      | mode/clear flags — second write path must not create malformed rows                                                                                                                          |
| `crates/shared/openapi-client` (+ mock)                                                                                            | client parity (AGENTS.md invariant) for the provider CRUD changes + new `cimd-probe` endpoint; the document endpoint needs NO entry — it never enters `openapi.json` (decision 4)            |
| `crates/ui/web-api/db_access_policy.toml`, `crates/ui/web-api/scope-map.golden.json`, `crates/shared/audit-log/audit-catalog.toml` | gate registries for new handlers/tests                                                                                                                                                       |
| `frontend/src/routes/settings/OidcProvidersSettings.svelte`, `McpAccessTab.svelte`, settings page                                  | mode selector, conditional fields, doc-URL display, probe callout, canonical-host relocation                                                                                                 |

Regeneration: `./scripts/regen-api.sh` (openapi.json + generated frontend client). No wire-protocol change ⇒ no
asyncapi regeneration.

## Frontend

- Provider modal: registration-mode selector (default `manual`, preserving current behavior and the existing e2e
  geometry assertions); in CIMD mode the Client ID / Client Secret fields are hidden and replaced by a read-only
  computed document URL with a copy button, plus a "canonical host not configured" callout linking to the Instance
  settings section when unset. Layout uses `FormFieldRow`/`SectionCard` primitives. Caution: the recorded
  `FormFieldRow` sibling-margin bug breaks `sm:grid-cols-2` alignment — inserting the selector row above the
  credential grid is exactly the trigger; verify against `frontend/tests/e2e/oidc-settings.spec.ts` and add a
  CIMD-mode e2e case rather than mutating the manual-mode geometry test.
- Probe callout: a "Check CIMD support" action in the modal calls `POST /api/v1/settings/oidc-providers/cimd-probe`
  and renders the structured result (supported / not advertised / unreachable / private address / own-document
  redirect) as non-blocking guidance; usable before the provider is saved.
- Instance settings section: new general settings card hosting the canonical host field (moved out of
  `McpAccessTab.svelte`); MCP tab keeps its enable toggle and gains a pointer to the relocated field. Verify at plan
  time how the MCP tab visibility is gated so the field remains reachable with MCP off.
- Login page: unchanged (provider buttons and error-code mapping already cover the failure paths).

## Testing

Success and failure paths per project rule; new endpoint tests use the `TestApp` harness; mock IdP via
`httpmock::MockServer` + permissive resolver (established pattern in `oauth/cimd.rs` tests).

- Enum `FromStr` round-trip + parse-error; mode-branched `Validate` (cimd-with-credentials rejected,
  manual-without-credentials rejected, `ClientSecretPatch` absent/null/value tri-state interplay).
- Document endpoint: 200 with canonical host set / 404 unset; golden body assertions — `client_id` byte-equals the
  request URL the IdP would use, `token_endpoint_auth_method` present, **no** `client_secret*` keys serialized, no
  `null` members; `Cache-Control` exact value (assert not `no-store`); `Content-Type: application/json` pinned
  (the IdP validates it before parsing).
- Canonical-host validation: reject path/userinfo/query/fragment inputs; accept bare host and host:port.
- `resolve_mcp_enabled` inversion: absent row ⇒ disabled regardless of host; materialization migration writes the
  previously-resolved value (test both prior states); `GET /api/v1/global-settings/oauth` reports unchanged
  `mcp`/`restart_required` after migration + boot for both prior states (`routes/settings_oauth.rs` call site).
- Probe endpoint: httpmock discovery with flag true / flag false / absent / unreachable ⇒ correct advisory
  classification in `CimdProbeResponse`; provider create/update succeeds regardless of probe outcome (and performs
  no outbound HTTP itself).
- Token-exchange wire pinning (CIMD mode, httpmock IdP): assert the token request carries `client_id` in the body
  and **no `Authorization` header** — this pins the `None`-secret behavior the design depends on. Companion
  manual-mode assertion pins Basic auth so a future emptiness-based regression is caught from both sides.
- Redirect pinning: authorize with canonical host set uses it regardless of request `Origin`/`Host`; callback token
  exchange replays the snapshot `redirect_uri` (assert on the mock token endpoint's received form body).
- Return-origin round-trip: authorize from an origin listed in `accepted_audience_hosts` ⇒ callback's final browser
  redirect is absolute to that origin; unlisted origin ⇒ falls back to the canonical origin; canonical-origin start
  is unchanged; **MCP disabled + alias listed ⇒ return origin still honored** (pins the per-request settings read
  against a `state.oauth` regression).
- Pending-flow purge: provider update/mode-switch/deactivate purges rows in-transaction; canonical-host PUT purges
  all rows; callback after purge takes the existing `oidc_state_expired`/provider-gone error path.
- `cimd`-mode response suppression: after a manual→cimd switch, `GET` responses omit/blank `client_id` and report
  `has_client_secret: false` while the stored values persist for switch-back.
- Activation gate: CIMD provider activates without credentials when canonical host set, 409 when unset; manual
  provider unchanged.
- Frontend: component tests for conditional rendering + probe callout; e2e CIMD-mode case as above.

## Documentation deliverables

Swept via repo-wide grep for `canonical_host`, `mcp_enabled`, and OIDC-provider mentions (non-spec hits only):

- **New ADR** (via `adrs new`, never hand-numbered): OIDC client registration via CIMD — URL-as-client_id RP role,
  always-serve document, explicit `oauth.mcp_enabled` opt-in inversion, canonical redirect pinning for all
  providers with the return-origin snapshot (including the widened purpose of `oauth.accepted_audience_hosts`),
  single-document multi-tenant limitation.
- `CONTEXT.md`: glossary entries for CIMD (AS role vs RP role) and Client Identifier URL; line 215 already names the
  AS-side fetch.
- `docs/admin/oauth-clients.md` (lines 21, 44 describe the auto-enable rule being removed), `docs/development/oauth-mcp.md`,
  `docs/security/oauth-mcp.md`, `docs/end-user/mcp-clients.md`: explicit opt-in semantics + canonical-host relocation.
- `docs/security/auth-and-authorization.md`: OIDC section — CIMD mode, public-client model (PKCE + exact
  redirect match replace the client secret), redirect pinning, pending-flow snapshot.
- New `docs/end-user/oidc-cimd.md` (+ `docs/end-user/README.md` index row): setup walkthrough with Pocket ID
  (allowlist step, ordering freedom), private-address limitation, HS256 caveat, canonical-host-change consequences,
  and the upgrade note that setting/having `oauth.canonical_host` now pins the OIDC login redirect origin for all
  providers (decision 9).
- `docs/end-user/cli-usage.md`: new CLI flags.
- `docs/api/settings-runtime.md`: check at plan time whether it states the auto-enable rule; update if so.
- No `AGENTS.md` change: no new workspace-wide invariant, no crate added/removed, subsystem stubs unaffected.

## Dependencies

None new. Reuses `openidconnect` 4.0.1, `oauth2` 5, `reqwest`, `httpmock` (all already pinned in the workspace).

## Out of scope (deferred)

- `logo_uri` in the metadata document (no stable public asset; needs a dedicated backend-served logo route).
- `private_key_jwt` / confidential CIMD via `jwks` in the document (Pocket ID accepts only `none`).
- Per-tenant metadata documents (structurally excluded by the single-document design; ADR records it).
- Any AS-side CIMD changes (`oauth/cimd.rs` untouched).
- Custom `AdditionalProviderMetadata` refactor to read the discovery flag inside the typed flow.
- Presenting CIMD as the default/recommended mode: it stays an explicitly-labeled option; manual registration
  remains the default and the only path for LAN-only deployments.

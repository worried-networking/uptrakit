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
3. **Mode column**: `oidc_providers` gains `client_registration` (TEXT NOT NULL DEFAULT `'manual'`, values `manual` |
   `cimd`), backed by a typed enum with `FromStr` + typed parse error per coding standards. No sentinel semantics.
   The term "CIMD" is reused deliberately (same protocol, opposite role); UI copy says "Automatic registration
   (CIMD)" and CONTEXT.md gets glossary entries for both roles.
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

5. **Canonical-host shape validation**: on save, reject values that are not a bare host (optional port): no scheme,
   userinfo, path, query, or fragment. Today `CanonicalUrlConfig::new` accepts `example.com/app` and
   `user@example.com` (`crates/shared/web-api-types/src/oauth/canonical_url.rs`, `format!("https://{host}")`) — both
   would produce CIMD URLs the IdP rejects or that never exact-match. Changing the canonical host after an IdP has
   cached the document is a **breaking operation** (new client identity, re-allowlist required): the settings UI
   shows a confirmation warning naming that consequence, and the change is already audited as a settings write.
6. **Provider CRUD**: create/update requests gain `client_registration`; for `cimd` mode `client_id`/`client_secret`
   must be absent/empty (validation branches on mode; `manual` keeps today's non-empty requirements). Mode switching
   is allowed both directions; switching to `cimd` **keeps** stored credentials (unused, enables switch-back);
   a new explicit `clear_client_secret: true` request flag is the only way to destroy the stored secret, and the
   audit details record `client_secret_cleared`. Switching to `manual` requires credentials to be present (stored or
   supplied). The activation completeness gate (`routes/oidc_providers.rs`, `activate_provider`, 409 on empty
   credentials) branches on mode: CIMD rows require only a set canonical host. Note activation is exclusive (one
   active provider), so "a CIMD provider is active" means "_the_ active provider is CIMD".
7. **Advisory probe, never blocking**: on create/update of a CIMD-mode provider the handler runs a best-effort probe
   and returns a structured `cimd_probe` result in the response (rendered as a callout in the UI); it never fails
   the save. Probe steps: (a) raw GET of the issuer's `/.well-known/openid-configuration` (a minimal serde struct —
   the flag is not representable in `CoreProviderMetadata`), reporting whether
   `client_id_metadata_document_supported` is true; (b) resolve the canonical host and warn when it lands in a
   private/loopback range (Pocket ID will refuse to fetch it); (c) GET Uptrakit's own document URL with a
   redirect-refusing client, reporting reachability and any redirect (a reverse proxy that 301s breaks CIMD with an
   opaque IdP-side error) — labeled "as seen from the server; the IdP's view may differ". The probe uses the same
   HTTP client construction as the login flow (`OidcHttpClient` with SSRF strictness derived from
   `allow_private_network_issuers && !multi_tenancy_enabled`), so probe and flow verdicts cannot diverge.
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
     from attacker-influencable `Origin`/`Host` headers) and is required for CIMD's exact match.
   - **Pending-flow snapshot**: the exact `redirect_uri` and `client_id` strings used at authorize time are stored
     on the `pending_oidc_flow` row; callback/token-exchange replays the snapshot instead of re-deriving. Any
     provider mutation (update, mode switch, deactivate, delete) purges that provider's pending flows inside the
     same `begin_immediate` transaction, recording the purge count in the audit details.
10. **Multi-tenancy**: one document ⇒ one client identity ⇒ one redirect list, instance-global. All tenants share it.
    Structural limitation, recorded in the ADR; acceptable under the single-tenant deployment reality.

## Enforcement-surface inventory (every site that branches on the mode)

| Site                                                                                              | Change                                                                                                                                                       |
| ------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `crates/shared/db/src/entity/oidc_provider.rs` + migration                                        | new column, enum type                                                                                                                                        |
| `crates/shared/web-api-types/src/oidc_providers.rs`                                               | mode field on create/update, `clear_client_secret`, mode-branched `Validate`, response gains mode + computed doc URL + `cimd_probe`                          |
| `crates/ui/web-api/src/routes/oidc_providers.rs`                                                  | create/update handlers, probe, activation gate branch, pending-flow purge                                                                                    |
| `crates/ui/web-api-queries/src/queries/oidc_providers.rs`                                         | params/view structs; `OidcProviderView` must include the new column or the `oidc_provider.update` Stateful audit records a mode switch as a no-op diff       |
| `crates/ui/web-api/src/routes/oidc_auth.rs`                                                       | `build_oidc_client`, authorize, callback, `oidc_link`/`oidc_exchange`/`oidc_complete_registration` (all rebuild the client), redirect pinning, flow snapshot |
| `crates/shared/db/src/entity/pending_oidc_flow.rs` + migration                                    | snapshot columns                                                                                                                                             |
| `crates/ui/web-api/src/routes/oauth/` (new module)                                                | document endpoint                                                                                                                                            |
| `crates/ui/web-api/src/oauth/mod.rs` + `boot.rs`                                                  | `resolve_mcp_enabled` inversion + materialization migration                                                                                                  |
| `crates/ui/cli/src/commands/settings/oidc.rs`                                                     | mode/clear flags — second write path must not create malformed rows                                                                                          |
| `crates/shared/openapi-client` (+ mock)                                                           | client parity (AGENTS.md invariant); document endpoint joins the public-route exclusion list (`SPEC_ONLY`) like its `/oauth/*` siblings                      |
| `crates/ui/web-api/db_access_policy.toml`, `audit-catalog.toml`, `scope-map.golden.json`          | gate registries for new handlers/tests                                                                                                                       |
| `frontend/src/routes/settings/OidcProvidersSettings.svelte`, `McpAccessTab.svelte`, settings page | mode selector, conditional fields, doc-URL display, probe callout, canonical-host relocation                                                                 |

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
- Probe callout: renders the structured `cimd_probe` result (supported / not advertised / unreachable / private
  address / own-document redirect) as non-blocking guidance.
- Instance settings section: new general settings card hosting the canonical host field (moved out of
  `McpAccessTab.svelte`); MCP tab keeps its enable toggle and gains a pointer to the relocated field. Verify at plan
  time how the MCP tab visibility is gated so the field remains reachable with MCP off.
- Login page: unchanged (provider buttons and error-code mapping already cover the failure paths).

## Testing

Success and failure paths per project rule; new endpoint tests use the `TestApp` harness; mock IdP via
`httpmock::MockServer` + permissive resolver (established pattern in `oauth/cimd.rs` tests).

- Enum `FromStr` round-trip + parse-error; mode-branched `Validate` (cimd-with-credentials rejected,
  manual-without-credentials rejected, `clear_client_secret` interplay).
- Document endpoint: 200 with canonical host set / 404 unset; golden body assertions — `client_id` byte-equals the
  request URL the IdP would use, `token_endpoint_auth_method` present, **no** `client_secret*` keys serialized, no
  `null` members; `Cache-Control` exact value (assert not `no-store`).
- Canonical-host validation: reject path/userinfo/query/fragment inputs; accept bare host and host:port.
- `resolve_mcp_enabled` inversion: absent row ⇒ disabled regardless of host; materialization migration writes the
  previously-resolved value (test both prior states).
- Probe: httpmock discovery with flag true / flag false / absent / unreachable ⇒ correct advisory classification;
  probe never fails the save.
- Token-exchange wire pinning (CIMD mode, httpmock IdP): assert the token request carries `client_id` in the body
  and **no `Authorization` header** — this pins the `None`-secret behavior the design depends on. Companion
  manual-mode assertion pins Basic auth so a future emptiness-based regression is caught from both sides.
- Redirect pinning: authorize with canonical host set uses it regardless of request `Origin`/`Host`; callback token
  exchange replays the snapshot `redirect_uri` (assert on the mock token endpoint's received form body).
- Pending-flow purge: provider update/mode-switch/deactivate purges rows in-transaction; callback after purge takes
  the existing `oidc_state_expired`/provider-gone error path.
- Activation gate: CIMD provider activates without credentials when canonical host set, 409 when unset; manual
  provider unchanged.
- Frontend: component tests for conditional rendering + probe callout; e2e CIMD-mode case as above.

## Documentation deliverables

Swept via repo-wide grep for `canonical_host`, `mcp_enabled`, and OIDC-provider mentions (non-spec hits only):

- **New ADR** (via `adrs new`, never hand-numbered): OIDC client registration via CIMD — URL-as-client_id RP role,
  always-serve document, explicit `oauth.mcp_enabled` opt-in inversion, canonical redirect pinning for all
  providers, single-document multi-tenant limitation.
- `CONTEXT.md`: glossary entries for CIMD (AS role vs RP role) and Client Identifier URL; line 215 already names the
  AS-side fetch.
- `docs/admin/oauth-clients.md` (lines 21, 44 describe the auto-enable rule being removed), `docs/development/oauth-mcp.md`,
  `docs/security/oauth-mcp.md`, `docs/end-user/mcp-clients.md`: explicit opt-in semantics + canonical-host relocation.
- `docs/security/auth-and-authorization.md`: OIDC section — CIMD mode, public-client model (PKCE + exact
  redirect match replace the client secret), redirect pinning, pending-flow snapshot.
- New `docs/end-user/oidc-cimd.md` (+ `docs/end-user/README.md` index row): setup walkthrough with Pocket ID
  (allowlist step, ordering freedom), private-address limitation, HS256 caveat, canonical-host-change consequences.
- `docs/end-user/cli-usage.md`: new CLI flags.
- `docs/api/settings-runtime.md`: check at plan time whether it states the auto-enable rule; update if so.
- `docs/development/openapi-client.md` exclusion tables if the tool config requires a documented entry.
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

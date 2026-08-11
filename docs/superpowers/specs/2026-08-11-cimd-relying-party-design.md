# Canonical Host Hardening & OIDC Redirect Pinning — Design

Date: 2026-08-11. Status: approved for planning. Scope reduced 2026-08-11 (owner decision): the CIMD relying-party
mode this spec originally proposed is **rejected** — see "Rejected: CIMD relying-party mode" below. What remains is
the part of the design that stands on its own: explicit MCP opt-in for `oauth.canonical_host`, and canonical
pinning of the OIDC login `redirect_uri` with a pending-flow snapshot.

## Problem

Two related weaknesses around the canonical host and OIDC login URLs:

1. **Setting `oauth.canonical_host` silently boots the MCP OAuth authorization server.** `resolve_mcp_enabled`
   (`crates/ui/web-api/src/oauth/mod.rs:92-97`) auto-enables when no explicit `oauth.mcp_enabled` row exists and a
   canonical host is set. Any future feature (or operator) that sets the host for an unrelated reason turns on a
   public AS with device flow and consent endpoints, and a second controller replica then fails boot on the peer
   check. The canonical host cannot safely become a general "instance URL" setting while this rule exists.
2. **OIDC login `redirect_uri` is derived per request from attacker-influencable headers.** `oidc_authorize` and
   the callback each independently derive the base from `ExternalBaseUrl`/`Origin`/`Host`
   (`routes/oidc_auth.rs`, `base_url_from_headers` prefers `Origin`). Authorize and callback can disagree, and for
   a client whose IdP does exact-string redirect matching this is fragile; for any client it is a code-interception
   surface.

## Decisions

1. **Explicit MCP opt-in.** Invert `resolve_mcp_enabled`: a missing `oauth.mcp_enabled` row means disabled; the
   fn's now-dead `canonical_host` parameter is dropped (all three call sites — `oauth/boot.rs`, the definition,
   `routes/settings_oauth.rs::load_oauth_settings_from_db` which feeds `restart_required` — inherit the change
   through the shared fn). A one-shot data migration materializes the currently-_resolved_ value into an explicit
   row (row absent + host set ⇒ write `true`), so no existing deployment changes behavior.
2. **Canonical-host shape validation.** On save, `oauth.canonical_host` must be a bare host with optional port: no
   scheme, userinfo, path, query, fragment, or whitespace (today `CanonicalUrlConfig::new` accepts
   `example.com/app` and `user@example.com` — `crates/shared/web-api-types/src/oauth/canonical_url.rs`,
   `format!("https://{host}")`). Empty string remains "clear". Enforced in `UpdateOAuthSettingsRequest::validate`.
3. **UI move.** The canonical-host field moves from the MCP Access tab to a Global Settings "Instance" card
   (decision 1 makes this safe); the MCP tab keeps its enable toggle plus a pointer. Changing a non-empty canonical
   host shows a confirmation warning: the login redirect URL changes for every provider and in-progress logins are
   cancelled (decision 4's purge). The MCP tab's save payload stops always-sending `canonical_host`.
4. **Redirect pinning + pending-flow snapshot** (`routes/oidc_auth.rs`):
   - When the canonical host is set, `redirect_uri` is built from it for **every** provider; header-derived bases
     remain only the fallback when unset. Reads go through the settings store per request — never `state.oauth`,
     which is a boot-time snapshot and a `https://disabled.invalid` placeholder while MCP is off
     (`OAuthState::disabled`).
   - The exact `redirect_uri` string used at authorize time is stored on the `pending_oidc_flow` row;
     callback/token-exchange replays the snapshot instead of re-deriving (closes the authorize/callback divergence).
   - **Return-origin snapshot**: pinning moves the callback to the canonical origin, but the callback ends in a
     path-relative `Redirect::to("/login?oidc_code=…")` and the refresh cookie is host-only — a user who started
     login on a non-canonical origin (LAN IP, VPN hostname, split-horizon DNS) would be stranded, or dead-end if
     the canonical origin is not routable from their network. Fix: also snapshot the origin observed at authorize
     time, validated against the canonical host plus `oauth.accepted_audience_hosts` (unlisted/absent ⇒ canonical);
     the post-callback success redirect becomes absolute to that stored origin. The session cookie is minted on
     that origin by the subsequent `POST /api/v1/auth/oidc/exchange` (the exchange code is the existing one-time
     cross-origin handoff). This widens `oauth.accepted_audience_hosts` beyond MCP audience validation — recorded
     in the ADR together with its inherited constraints (entries compose as `https://{host}`, so return origins are
     always https; list capped at `MAX_ACCEPTED_AUDIENCE_HOSTS = 5`).
   - **Purges**: any provider mutation (update, deactivate, delete, activate) purges that provider's pending flows
     inside the same `begin_immediate` transaction, recording the count in the audit details. A **changed**
     `oauth.canonical_host` value purges all pending flows — keyed on `before != after` loaded in the same
     transaction, never on field presence (the frontend historically sends the field on every OAuth-settings PUT).
     Purges go through dedicated `OidcFlowStore` transaction-scoped helpers.
   - Explicit behavior change: deployments that already set `oauth.canonical_host` for MCP will see their OIDC
     login `redirect_uri` silently change origin on upgrade (previously header-derived) — the manually registered
     redirect URI at the IdP may need updating; alternate access origins keep working when listed in
     `oauth.accepted_audience_hosts`. Called out in the ADR and `docs/security/auth-and-authorization.md`.
5. **Migration hygiene**: the snapshot-columns migration deletes existing `pending_oidc_flows` rows first (they
   live ≤ 600 s and cannot satisfy the new NOT NULL columns); no legacy-row semantics.

## Enforcement-surface inventory

| Site                                                                           | Change                                                                                                                                                                                                                |
| ------------------------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `crates/ui/web-api/src/oauth/mod.rs` + `boot.rs` + `routes/settings_oauth.rs`  | `resolve_mcp_enabled` inversion (all three call sites); dead auto-enable warn block removed; `restart_required`/`mcp` reporting unchanged post-migration; canonical-host PUT purges all pending flows on value change |
| `crates/shared/db/src/migration/` (two new migrations)                         | materialize `oauth.mcp_enabled`; pending-flow snapshot columns (`redirect_uri`, `return_origin`) with row purge                                                                                                       |
| `crates/shared/db/src/entity/pending_oidc_flow.rs`                             | snapshot fields                                                                                                                                                                                                       |
| `crates/ui/web-api-auth/src/auth/oidc_state.rs`                                | `OidcFlowStore::insert`/`take` carry the snapshot; `purge_for_provider_in_tx` / `purge_all_in_tx`                                                                                                                     |
| `crates/shared/web-api-types/src/settings_oauth.rs`                            | canonical-host bare-host `Validate`                                                                                                                                                                                   |
| `crates/ui/web-api/src/routes/oidc_auth.rs`                                    | `compute_pinned_redirect` seam, authorize pin + snapshot insert, callback replay, absolute success redirect via threaded return origin                                                                                |
| `crates/ui/web-api/src/routes/oidc_providers.rs`                               | per-provider purge in the four mutation handlers' existing transactions + audit details                                                                                                                               |
| `frontend/src/routes/settings/GlobalSettingsTab.svelte`, `McpAccessTab.svelte` | Instance card, field removal + pointer, change warning                                                                                                                                                                |

No REST contract changes (no new endpoints, no request/response shape changes beyond validation) ⇒ no
`regen-api.sh`/openapi-client impact. No wire-protocol change ⇒ no asyncapi regeneration.

## Testing

- Migration: materializes `true` (host set + row absent), no row (host absent), explicit row untouched, JSON-null
  host not treated as set — index-targeted `Migrator::up` + seeded rows per the `m20260512_drop_file_keys`
  precedent.
- Inversion: fn table test (3 rows); the existing `boot_oauth_state_auto_enables_when_canonical_host_set` test
  flips into the inversion's pin; `GET /api/v1/global-settings/oauth` reports `mcp_enabled == false` +
  `restart_required == false` with only a host row seeded.
- Canonical-host validation: reject path/userinfo/query/fragment/whitespace/scheme inputs; accept bare host,
  host:port, `[::1]:8443`, empty (clear).
- Pinning seam: canonical unset ⇒ observed base for both; set ⇒ redirect canonical, return origin honored only when
  observed == canonical or allowlisted; **MCP disabled + alias listed ⇒ return origin still honored** (pins the
  per-request settings read against a `state.oauth` regression).
- Round trip: authorize (mock IdP discovery via `httpmock` + permissive resolver) from an allowlisted `Origin`
  stores `redirect_uri` = canonical callback and `return_origin` = alias; unlisted origin falls back to canonical;
  the success redirect from `create_oidc_exchange_and_redirect` is absolute to the passed origin.
- Purges: provider update purges only that provider's flows; canonical-host change purges all; re-sending the same
  canonical value keeps flows (red-check: a presence-keyed purge must fail this test).

## Documentation deliverables

- **New ADR** (via `adrs new`, never hand-numbered): explicit MCP opt-in + canonical redirect pinning with the
  return-origin snapshot and the `accepted_audience_hosts` widening; records the CIMD-RP rejection below as a
  considered alternative.
- `docs/admin/oauth-clients.md` (lines 21, 44 state the auto-enable rule), `docs/development/oauth-mcp.md`,
  `docs/security/oauth-mcp.md`, `docs/end-user/mcp-clients.md`: explicit opt-in semantics + canonical-host UI move.
- `docs/security/auth-and-authorization.md`: redirect pinning, pending-flow snapshot, return-origin validation,
  purge triggers, upgrade note.
- `docs/api/settings-runtime.md`: verified 2026-08-11 — zero mentions of canonical_host/mcp_enabled; no change.
- No `AGENTS.md` change: no new workspace-wide invariant, no crate added/removed.

## Dependencies

None new.

## Rejected: CIMD relying-party mode

The original scope of this spec: a `cimd` provider registration mode using URL-as-client_id per
`draft-ietf-oauth-client-id-metadata-document-02` (Pocket ID ≥ 2.13.0), with a served
`/oauth/client-metadata.json`, mode column, advisory probe endpoint, and public-client flow. Rejected by the owner
on 2026-08-11 after use-case analysis:

- The draft targets clients meeting authorization servers **they have no prior relationship with** (atproto,
  IndieAuth, MCP) — registration cost O(clients × AS's) with at least one side unknown in advance. Uptrakit as RP
  is the opposite shape: one deployment, one IdP (activation is exclusive), both administered by the same operator,
  registered once.
- With Pocket ID the flow is not even zero-touch: the admin still pastes the document URL into an allowlist —
  trading one one-time manual step for another.
- The audience most likely to self-host (LAN/private deployments) structurally cannot use CIMD: Pocket ID refuses
  private-address metadata fetches with no escape hatch.
- Cost carried: a public endpoint, schema column, mode branching at ~9 sites, a silent-failure class diagnosed on
  the IdP's side, and a draft-02 moving target implemented downstream via a fosite fork.

Revisit if the draft reaches RFC and IdPs move from allowlists to open acceptance with consent warnings, or if a
concrete demand for public-deployment zero-registration appears. The full CIMD design (document contents,
exact-match constraints, probe semantics, `ClientSecretPatch`, response suppression) is preserved in this file's
git history (commit `206a4e328` and earlier) and in the archived Plans C/D if ever revived.

## Out of scope (deferred)

- The entire CIMD relying-party mode (above).
- Error-path callback redirects stay path-relative (only the success/flow-completion redirects go absolute — no
  origin is known for pre-snapshot failures, and post-snapshot error UX on the canonical origin is acceptable).
- Renaming the `oauth.canonical_host` storage key (UI relabels only; a key migration buys nothing).

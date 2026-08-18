# 45. Explicit MCP OAuth Opt-In And Canonical OIDC Redirect Pinning

Date: 2026-08-18

## Status

Accepted

## Context

Two related weaknesses sat around the canonical host and OIDC login URLs.

First, setting `oauth.canonical_host` silently booted the MCP OAuth authorization server.
`resolve_mcp_enabled` (`crates/ui/web-api/src/oauth/mod.rs`) auto-enabled MCP OAuth whenever no explicit
`oauth.mcp_enabled` row existed and a canonical host was set (`None => canonical_host.is_some()`). Any future
feature — or an operator who just wanted a general instance-URL setting — that set the host for an unrelated reason
turned on a public authorization server with device flow and consent endpoints, and a second controller replica
then failed boot on the peer-fingerprint check (`crates/ui/web-api/src/oauth/boot.rs::validate_and_register`). The
canonical host could not safely become a general instance-URL setting while this rule existed.

Second, the OIDC login `redirect_uri` was derived per request from attacker-influencable headers. `oidc_authorize`
and the OIDC callback each independently called `base_url_from_headers` (`crates/ui/web-api/src/routes/oidc_auth.rs`),
which prefers the `Origin` header and falls back to `Host`. Authorize and callback could disagree with each other,
and for any IdP client that does exact-string redirect matching this was a fragile, attacker-steerable
code-interception surface: nothing pinned the redirect target to a value the operator actually configured at the
identity provider.

## Decision

### Explicit MCP opt-in

`resolve_mcp_enabled` is inverted: a missing `oauth.mcp_enabled` row now means MCP OAuth is disabled, full stop.
The function's `canonical_host` parameter becomes dead and is dropped from all three call sites — `oauth/boot.rs`,
the function definition in `oauth/mod.rs`, and `routes/settings_oauth.rs::load_oauth_settings_from_db`, which feeds
the `restart_required` computation reported to the settings UI. A one-shot data migration materializes each
deployment's currently _resolved_ value into an explicit row (row absent and host set ⇒ write `true`), so no
existing deployment silently changes behavior on upgrade. Boot-time validation is unaffected by this decision: when
`oauth.mcp_enabled` is explicitly `true` and `oauth.canonical_host` is empty or absent,
`validate_and_register` still bails with `OAuthBootError::CanonicalHostMissing`
(`crates/ui/web-api/src/oauth/boot.rs`) — opt-in only removes the _implicit_ enable path, not the requirement that
an explicit enable have a host to boot with.

### Canonical Host card relocation

Because MCP OAuth no longer auto-activates from the host's presence, the canonical-host field can safely leave the
MCP-specific settings tab. It moves to a Global Settings **Canonical Host** card (named for the "Canonical Host"
setting itself, distinct from the existing "Instance Configuration" tab on the same settings page); the MCP tab
keeps only its explicit enable toggle plus a pointer to the new location. Changing a non-empty canonical host shows
a confirmation warning, because it changes the login redirect origin for every provider and cancels in-progress
logins (see Redirect pinning below). The MCP tab's save payload stops always-sending `canonical_host`.

### Canonical Host shape validation

On save, `oauth.canonical_host` must be a bare host with an optional port — no scheme, userinfo, path, query,
fragment, or whitespace. `CanonicalUrlConfig::new` (`crates/shared/web-api-types/src/oauth/canonical_url.rs`)
already composes the stored value as `https://{host}` and `https://{host}/mcp` and validates the composed strings
via `CanonicalResourceUrl::parse`, but the raw stored string was previously under-validated at the write boundary
(e.g. `example.com/app` or `user@example.com` could be accepted and only fail, confusingly, downstream at
composition time). Shape validation moves forward into `UpdateOAuthSettingsRequest::validate`, so a malformed host
is rejected at the settings API instead of surfacing later as an opaque composition error. An empty string remains
the documented way to clear the setting.

### Canonical redirect pinning with a pending-flow snapshot

When the canonical host is set, `redirect_uri` is built from it for **every** OIDC provider at authorize time;
header-derived bases (`base_url_from_headers`, preferring `Origin` then falling back to `Host`) remain only the
fallback when no canonical host is configured. Reads go through the settings store per request rather than the
boot-time `state.oauth` snapshot, which holds an inert `https://disabled.invalid` stand-in value
(`OAuthState::disabled`) while MCP OAuth is off and must never leak into a per-request pinning decision.

The exact `redirect_uri` string used at authorize time is stored on the `pending_oidc_flow` row, and the callback
replays that stored value instead of re-deriving it — closing the authorize/callback divergence that made the
previous header-derived scheme fragile.

### Return-origin snapshot and the `accepted_audience_hosts` widening

Pinning the callback to the canonical origin creates a new problem: the callback still ends in a path-relative
`Redirect::to("/login?oidc_code=…")`, and the refresh cookie is host-only, so a user who started login from a
non-canonical origin (a LAN IP, a VPN hostname, split-horizon DNS) would be stranded — or dead-ended entirely if
the canonical origin is not routable from their network. The fix snapshots the origin observed at authorize time in
addition to the redirect URI, validates it against the canonical host plus `oauth.accepted_audience_hosts`
(unlisted or absent host ⇒ falls back to canonical), and makes the post-callback success redirect absolute to that
stored origin. The session cookie itself is still minted on that origin by the subsequent
`POST /api/v1/auth/oidc/exchange` — the exchange code was already the one-time cross-origin handoff mechanism, so
no new credential-bearing hop is introduced.

This decision widens `oauth.accepted_audience_hosts` beyond its original purpose of MCP resource-audience
validation into general return-origin validation, and it inherits that setting's existing composition and cap:
`CanonicalUrlConfig::new` composes every accepted alias as `https://{alias}/mcp` (return-origin validation reuses
the same host-parsing rules, so an accepted origin is always `https`), and the alias list is capped at
`MAX_ACCEPTED_AUDIENCE_HOSTS = 5` (`crates/shared/web-api-types/src/oauth/canonical_url.rs`) —
`CanonicalUrlConfigError::TooManyAliases` if exceeded.

### Purges keyed on value change, not presence

Any provider mutation (update, deactivate, delete, activate) purges that provider's pending OIDC flows inside the
same `begin_immediate` transaction as the mutation, recording the purge count in the audit details. A **changed**
`oauth.canonical_host` value purges every pending flow for the deployment — keyed on `before != after` loaded
inside the same transaction, deliberately never on field presence, because the settings frontend historically
resends the field on every OAuth-settings `PUT` regardless of whether the operator touched it; a presence-keyed
purge would cancel every in-flight login on every unrelated settings save. Purges run through dedicated
`OidcFlowStore` transaction-scoped helpers (`crates/ui/web-api-auth/src/auth/oidc_state.rs`).

## Rejected alternatives

### CIMD relying-party mode

A `cimd` provider-registration mode using URL-as-`client_id` per
`draft-ietf-oauth-client-id-metadata-document-02` (implemented by Pocket ID ≥ 2.13.0) was considered and rejected.
Under that draft, Uptrakit would serve a `/oauth/client-metadata.json` document and register with an IdP by URL
instead of a pre-shared `client_id`/`client_secret` pair, eliminating a manual registration step.

The draft is designed for clients that meet authorization servers they have no prior relationship with at scale —
atproto, IndieAuth, MCP-style ecosystems where registration cost is `O(clients × authorization servers)` with at
least one side unknown in advance. Uptrakit's relying-party shape is the opposite: one deployment talks to one IdP
(OIDC provider activation is exclusive), both sides administered by the same operator, registered exactly once.
Even with a CIMD-capable IdP like Pocket ID, the flow is not actually zero-touch — the administrator still has to
paste the metadata document's URL into an allowlist, trading one one-time manual step (registering `client_id` and
`client_secret`) for a different one-time manual step (allowlisting a URL), with no net reduction in operator
effort.

The audience most likely to self-host — LAN-only or private-network deployments — structurally cannot use CIMD at
all: Pocket ID refuses to fetch client-metadata documents from private addresses, with no escape hatch. Adopting
CIMD would also have carried real implementation cost for a shape that helps almost nobody in this deployment
model: a new public endpoint, a mode column, branching logic scattered across roughly nine call sites, a new class
of silent failure diagnosed only from the IdP's own logs, and ongoing maintenance against a draft-02 specification
that is still a moving target.

This is revisited if the draft reaches RFC status and identity providers move from URL allowlists toward open
acceptance with consent-time warnings, or if concrete demand emerges for a public, zero-registration deployment
mode that the current one-IdP-per-deployment shape does not serve.

## Consequences

Deployments that already set `oauth.canonical_host` purely to enable MCP OAuth keep working unchanged: the
migration materializes their resolved `true` into an explicit `oauth.mcp_enabled` row. Deployments that set the
host for some other reason (once the field is safe to reuse as a general instance-URL setting) no longer risk
accidentally booting a public OAuth authorization server as a side effect.

Canonical-host changes now cancel every in-progress OIDC login and change the registered redirect URL for every
configured provider simultaneously — an operator changing the host must be prepared to update the corresponding
redirect-URI registration at each IdP, and the UI's confirmation warning exists specifically to surface that before
the operator commits the change.

Deployments that already set `oauth.canonical_host` for MCP will see their OIDC login `redirect_uri` silently
change origin on upgrade — it was previously derived per-request from `Origin`/`Host` headers, and after this
change it is always the canonical origin. The manually registered redirect URI at the IdP may need updating as a
result. Alternate access origins (a LAN IP, a VPN hostname) keep completing login successfully as long as they are
listed in `oauth.accepted_audience_hosts`; an unlisted alternate origin still completes login but the
post-callback redirect lands the user on the canonical origin instead of the one they started from.

Widening `oauth.accepted_audience_hosts` from "MCP audience validation only" to "MCP audience validation and OIDC
return-origin validation" means a future change to one purpose's requirements (for example, a narrower audience
cap, or a different composition rule) needs to consider both consumers before landing — the two purposes now share
one setting and one cap.

Sequencing note: this ADR may land before the redirect-pinning implementation itself. If that implementation later
diverges from the specifics recorded here — the pending-flow snapshot shape, or the `MAX_ACCEPTED_AUDIENCE_HOSTS = 5`
cap — the task that introduces the divergence must update this ADR through the process in
[Architecture Decision Records](../development/architecture-decision-records.md); `adrs.toml` sets `no_edit = true`,
so an accepted ADR is never hand-edited in place.

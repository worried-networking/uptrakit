---
title: MCP OAuth 2.1 — Security Guide
weight: 45
description: Security guide for reviewers and auditors evaluating the MCP OAuth 2.1 implementation.
---

# MCP OAuth 2.1 — Security Guide

This guide is for security reviewers and auditors evaluating the MCP OAuth 2.1 implementation.

Related: [ADR 0010](../adr/0010-mcp-oauth-authorization-server-placement.md) ·
[Developer guide](../development/oauth-mcp.md) · [Admin guide](../admin/oauth-clients.md) ·
[Spec](../superpowers/specs/2026-05-12-mcp-oauth-authorization-design.md)

## Threat Model

### Phishing via Dynamic Client Registration (DCR)

DCR allows any network-reachable caller to register an OAuth client without authentication. An attacker
who can reach `/oauth/register` can create a client named "uptrakit Security Update Required" with a
`client_uri` that resembles the controller domain. Because the consent screen displays
Operator-controlled strings from the client record, DCR turns the consent screen into a phishing
primitive bounded only by the Operator's existing Permission grants. `Permission::TriggerUpdates` is
effectively root-on-fleet, making this a high-severity vector.

### Phishing via CIMD Silent Re-Keying

Client ID Metadata Documents (CIMD) allow an HTTPS URL to serve as a `client_id`. If an attacker
compromises the CIMD URL (DNS takeover, expired domain, GitHub Pages repo takeover) they can change
`redirect_uris` to a malicious callback. Any existing consent grant would then redirect authorization
codes to the attacker's server.

### Token Theft

A stolen access token (15-minute TTL by default) allows the attacker to call any MCP tool the token's
scopes and Permissions allow, for up to 15 minutes after theft. A stolen refresh token allows
indefinite access until the family is revoked.

### Multi-Controller Secret Drift

If two controller nodes share the same `oauth.jwt_signing_secret` and one node's secret is rotated
independently, tokens issued by the first node fail signature verification on the second. The risk is
silent misconfiguration that produces intermittent 401 errors affecting all users.

### Audience Confusion

A token minted with `aud = "https://controller.example.com/mcp"` must not be accepted by a different
Resource Server (e.g., a future Dashboard API RS). Conversely, a Dashboard JWT (which carries
`aud = ["uptrakit"]`) must not be accepted by the MCP RS.

## Mitigations

### DCR and CIMD Are Opt-In (Default OFF)

`oauth.dcr_enabled` and `oauth.cimd_enabled` both default to `false`. Operators must explicitly read
the runbook (`docs/admin/oauth-clients.md`) before enabling either surface. The AS metadata document
omits `registration_endpoint` and `client_id_metadata_document_supported` when the corresponding
toggle is off.

### Typed-Confirmation Consent for Unverified Clients

Every DCR-registered, CIMD-fetched, or manually-registered client that an Operator has not explicitly
trusted (`oauth_clients.trusted_at IS NULL`) displays a danger-toned "Unverified client" badge and
requires a typed-confirmation step before the Allow button is enabled. The user must type the
**redirect URI hostname** (the field most relevant to detecting phishing, not the attacker-controlled
`client_name`) into a text input. Server-side re-validation at `POST /oauth/consent/{id}/approve`
rejects submissions where the typed value does not match `Url::parse(redirect_uris[0]).host()`.

### CIMD Content-Hash Material-Change Re-Consent

The fetcher computes `SHA-256(metadata_bytes)` on every CIMD refresh. Changes to any field not in the
cosmetic-field allowlist (`tos_uri`, `policy_uri`, `software_version`, `software_id`) are treated as
material changes. On material change:

- Every active `oauth_consents` row for the client is marked `revalidation_required_at = now`.
- The next authorization round for that client forces the consent screen regardless of the
  skip-consent conditions.
- The consent screen shows a warning callout: "This client's published metadata has changed since you
  last authorized it." A diff of the changed fields is rendered below.

This closes the CIMD silent re-keying attack: a compromised CIMD URL cannot silently reroute an
existing consent grant.

### `SsrfSafeResolver` for CIMD Fetch

The CIMD fetcher builds its `reqwest::Client` with `SsrfSafeResolver::new()` (not
`SsrfSafeResolver::permissive()`). This prevents DNS rebinding from routing CIMD metadata fetches
into RFC 1918 / link-local networks. `SsrfSafeResolver::permissive()` must never be used for
Operator-supplied or user-supplied URLs in the CIMD path.

### Multi-Controller Boot Guard

When `oauth.mcp_enabled = true` and the controller detects conflicting `oauth.jwt_signing_secret`
values across a multi-controller deployment, it refuses to start with a clear error. The
`oauth_controller_instances` table records the `kid` fingerprint of each node's signing secret.
`oauth.allow_multi_controller_unsafe` is an intentional footgun — it skips the boot check but emits
a WARN log on every boot. Operators who flip it accept responsibility for token-validation divergence.

### Algorithm Pinning (HS256 Only)

The JWT verifier sets `algorithms: vec![Algorithm::HS256]`. Tokens with `alg=none`, `RS256`, `ES256`,
or any other algorithm are rejected before any signature or claim is examined. The `kid` header is
present on all issued tokens for migration purposes but does not broaden algorithm acceptance in v1.

### Refresh-Token Family Replay Detection

Refresh tokens use rotating-token semantics with family tracking. All steps run inside a single SQLite
`BEGIN IMMEDIATE` transaction:

1. If a token's `rotated_at` is already set (token was already used), the entire family is revoked
   (`UPDATE ... SET revoked_at = now WHERE family_id = ... AND revoked_at IS NULL`).
2. `OAUTH_REFRESH_REPLAY_DETECTED` is emitted with `family_id` and `replayed_refresh_id`.
3. The client receives `400 invalid_grant`.

An attacker who steals a refresh token and replays it after the legitimate client has already rotated
it triggers family revocation — the legitimate client's next refresh attempt also fails, alerting the
user that their session was compromised.

### Double-Locking of Audience Confusion

Two independent mechanisms prevent Dashboard JWT / MCP OAuth token cross-use:

1. The `aud` claims differ: Dashboard tokens carry `aud = ["uptrakit"]`; MCP tokens carry
   `aud = "https://<canonical_host>/mcp"`.
2. The signing secrets differ: `oauth.jwt_signing_secret` is distinct from the Dashboard JWT secret.
   A token signed with the wrong secret fails signature verification before any claim is checked.

### Rate Limiting

All AS endpoints are covered by `RateLimitStore`-backed sliding-window limits. See
`docs/admin/oauth-clients.md` for the full limit table and tuning guidance. The DCR endpoint has a
per-IP lifetime cap of 20 clients in addition to the per-hour rate limit.

## Deviation from RFC 9068

RFC 9068 ("JSON Web Token (JWT) Profile for OAuth 2.0 Access Tokens") specifies asymmetric signing
(RS256 or ES256) as the implied default for access tokens, because asymmetric keys allow Resource
Servers to verify tokens offline via a JWKS endpoint without possessing the signing key.

uptrakit v1 deviates from this implied default: it uses HMAC HS256 with a shared secret. The
deviation is justified by the deployment model described in ADR 0010: the Controller is the only
Resource Server in v1, and it already possesses the signing secret for issuance. There is no second RS
that would need offline verification.

The migration path to asymmetric signing is:

- Phase 2 adds RS256 or EdDSA signing behind a feature flag.
- Every issued token already carries a `kid` header derived from the secret fingerprint. Clients that
  follow `kid` will fetch the new public key from the JWKS endpoint and verify tokens asymmetrically
  without a wire-breaking change.
- The JWKS endpoint (`GET /.well-known/jwks.json`) is a Phase 2 deliverable. It does not exist in v1.
- A fixed overlap window (`oauth_jwt_keys` table) allows in-flight tokens signed with the old key to
  remain valid during the cutover.

Until Phase 2 lands, the HS256 key must be kept strictly server-side. It must never appear in logs,
audit events, API responses, or frontend state.

## Key Rotation

### v1 Rotation Behavior (Hard Cut)

v1 supports only a hard-cut rotation:

1. Stop the controller.
2. Set a new `oauth.jwt_signing_secret` value.
3. Restart the controller. The boot-time `kid` changes to the new secret's fingerprint.
4. All previously issued access tokens are immediately invalid (wrong signature).
5. All previously issued refresh tokens are also invalid (stored with the old `kid` binding; the
   rotation endpoint rejects them with `invalid_grant`).
6. All active MCP clients must re-authenticate. Access-token TTL is 15 minutes by default, so the
   disruption window equals one TTL cycle. Refresh tokens require users to re-authorize via the
   browser consent flow.

### Planned Phase 2 Behavior (Overlap Window)

Phase 2 introduces an `oauth_jwt_keys` table that allows an overlap window during key rotation. During
the overlap:

- New tokens are issued with the new `kid`.
- Old tokens signed with the previous `kid` are still validated against the previous key, until the
  overlap window expires.
- The overlap window defaults to the access-token TTL (15 minutes) so that no in-flight access token
  is invalidated mid-session.

The Phase 2 spec owns the detailed `oauth_jwt_keys` design.

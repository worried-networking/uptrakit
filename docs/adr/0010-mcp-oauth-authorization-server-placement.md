# 0010 — MCP OAuth 2.1 Authorization Server Placement

Date: 2026-05-12

## Status

Accepted

- Spec: `docs/superpowers/specs/2026-05-12-mcp-oauth-authorization-design.md`

## Context

The MCP Authorization 2025-11-25 specification requires OAuth 2.1 for browser-capable MCP clients
(Claude Desktop, Cursor, MCP Inspector). Adding OAuth 2.1 means deploying an Authorization Server (AS)
that handles authorization codes, PKCE S256, consent, token issuance, refresh-token rotation, Dynamic
Client Registration (DCR), and Client ID Metadata Documents (CIMD).

Where that AS lives is a non-trivial architectural choice. `uptrakit-web-api` already owns
`JwtManager`, `AuthState`, `RateLimitStore`, `SessionService`, the session-cookie middleware
(`require_auth`), and the SvelteKit consent route in the frontend. Any AS candidate depends on all of
these. A dedicated crate (`uptrakit-oauth-as`) would have to import every one of those types, creating
a seam as wide as the module it replaces.

ADR 0001 defines three criteria for extraction into a dedicated crate:

1. **Coherent concept** — the AS passes: "OAuth Authorization Server" is a well-bounded concept.
2. **Clear seam** — the AS fails today: it couples tightly to `JwtManager`, `AuthState`, and the
   frontend consent route. A seam that spans session management, JWT signing, and UI routing is not
   clean.
3. **Self-contained test surface** — the AS fails today: integration tests require the full DB schema,
   session middleware, and a running frontend dev server for consent E2E flows.

The MCP Authorization spec also mandates a priority order for client registration:
pre-registration > CIMD > DCR. This order is enforced in the AS implementation, not in client code.

The existing MCP server (`uptrakit-mcp`) was extracted as its own crate per ADR 0001 (see
`docs/superpowers/specs/2026-05-01-extract-mcp-crate-design.md`). It holds the Resource Server (RS)
layer and the Protected Resource Metadata (PRM) endpoint. It must not gain a dependency on
`uptrakit-web-api` — that would create a circular dependency.

## Decision

Embed the Authorization Server inside `uptrakit-web-api` for Phase 1. Defer extraction to Phase 2.

Specific sub-decisions:

- **No new crate.** AS routes live in `crates/ui/web-api/src/routes/oauth/`, sharing `AppState`
  and all existing auth infrastructure.
- **HS256 v1 with `kid` header.** HMAC HS256 with a boot-time `kid` derived from the signing-secret
  fingerprint. The `kid` header is present so future clients following the `kid` convention can switch
  to asymmetric signing (RS256 / EdDSA) without breaking. Asymmetric JWT is explicitly deferred to
  Phase 2.
- **CIMD over DCR by default.** Both are opt-in (default OFF). When both are enabled, the AS resolves
  HTTPS-URL `client_id` values via CIMD first; DCR is the fallback. This matches the 2025-11-25
  priority order and limits the phishing surface of unauthenticated client registration.
- **Model B rejected for v1.** Model B (external IdP issuing MCP tokens, Controller acting as RS only)
  requires JWKS fetching, per-issuer JWT validator dispatch, and scope→Permission mapping per AS row.
  v1 keeps three named seams (`PRM.authorization_servers` array, `uptrakit-mcp::auth` dispatch point,
  `oauth_authorization_servers` reserved table name) so Phase 2 can add Model B without renaming any v1
  type or breaking any deployed client.
- **Extraction seams preserved.** Every AS service (`OAuthAuthorizationService`,
  `OAuthTokenService`, `OAuthRefreshTokenService`, `OAuthConsentService`) takes its dependencies via
  constructor injection, not by reading `AppState` directly. This is the pre-condition for future
  extraction into `uptrakit-oauth-as` once a second OAuth Resource Server appears (e.g., Dashboard API)
  or once the seam is genuinely clean.

## Consequences

### Positive

- No new crate, no new workspace dependency edge, no new compile-time boundary to manage.
- Simple deployment: single binary, no separate AS process, no inter-service auth between AS and RS.
- Reuses the existing `JwtManager` (clock injection, signing, validation), `AuthState`,
  `RateLimitStore`, and `SessionService` without duplication.
- The consent screen uses the existing session cookie and the existing SvelteKit SPA pattern
  (`fetch()` with Dashboard JWT). No new auth protocol between frontend and backend.
- The `kid` header on HS256 tokens provides a migration path to asymmetric signing in Phase 2 without
  a wire-breaking change.

### Negative / Accepted Trade-offs

- **Tight coupling to `web-api`.** The AS shares `AppState`. Extracting it later will require
  introducing a sub-state struct (e.g., `OAuthAsDeps`) and pulling dependencies through it — the same
  work deferred from today.
- **Single-controller only (v1).** HS256 with a per-deployment secret means a second controller node
  cannot validate tokens issued by the first without sharing the same `oauth.jwt_signing_secret`. The
  multi-controller boot guard enforces that sharing explicitly. v1 makes no promises about
  active-active multi-controller topologies.
- **HS256 limits token portability.** A client that wants to validate an MCP access token offline (e.g.,
  for a sidecar resource server) cannot fetch a JWKS — the symmetric secret would have to be distributed
  out-of-band. Phase 2's RS256 / EdDSA migration resolves this.
- **Extraction deferred.** If a second OAuth Resource Server appears before Phase 2 is scoped, the
  extraction work lands in that phase's scope rather than proactively. The cost of deferral is bounded
  by the constructor-injection seam already baked in.

## Alternatives Considered

### Extract to `uptrakit-oauth-as`

Rejected for Phase 1 because the "clear seam" criterion fails: `JwtManager`, `AuthState`, the session
middleware, and the consent UI are all inside `web-api`. Extraction today would move the fat crate
problem rather than solve it. The extraction is explicitly allowed in Phase 2 once the seam is clean.

### Delegate to external IdP (Model B)

Rejected for Phase 1. Requires JWKS fetcher, per-issuer validator dispatch, and per-AS
scope→Permission mapping. The feature complexity is disproportionate to the single-controller
self-hosted deployment model. Phase 2 seams are preserved so this path remains open without v1 breakage.

### Use `oxide-auth` crate

Evaluated and rejected. `oxide-auth` provides an RFC 6749 framework but does not cover the 2025-11-25
MCP spec extensions (CIMD, PKCE S256 enforcement, Resource Indicators binding, `mcp:*` scope model,
`kid`-bearing HS256 tokens, refresh-token family replay detection). Wrapping `oxide-auth` to cover
those extensions would add more integration surface than writing the AS directly on `axum`, `sea-orm`,
and `jsonwebtoken` — which is already the project's stack.

### Asymmetric JWT (RS256 / EdDSA) in v1

Rejected. Asymmetric signing requires exposing a JWKS endpoint and managing key pairs. The single
controller deployment model does not need offline verification or cross-signer trust today. The `kid`
header is present in every HS256 token so clients that follow `kid` will pick up asymmetric keys in
Phase 2 without a wire-breaking change.

### Granular per-tool scopes in v1

Rejected. Two scopes (`mcp:read`, `mcp:write`) are sufficient for the current tool set. Granular
scopes (e.g., `mcp:fleet:trigger`) are additive — they refine but never replace the coarse pair.
`mcp:read` and `mcp:write` retain their v1 semantics permanently. Phase N scopes carry their own
consent UI hints and DCR default-scope deltas.

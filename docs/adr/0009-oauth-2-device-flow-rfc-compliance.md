# 0009 — OAuth 2.0 Device Authorization Grant: strict RFC compliance, minimum-viable issuance

- Status: Accepted
- Date: 2026-05-12
- Spec: `docs/superpowers/specs/2026-05-12-rfc8628-device-auth-design.md`

## Context

uptrakit's CLI uses an OAuth-flavoured device authorization flow today. The wire
shape diverges from RFC 8628 in several ways: poll responses use HTTP 200 with a
custom `status` enum plus HTTP 404 instead of the RFC's HTTP 400 + JSON `error`
field; the start endpoint returns `verification_url` instead of
`verification_uri`; there is no `verification_uri_complete`; no per-flow
`slow_down` cadence enforcement; no operator-driven `access_denied` path; no
`/.well-known` discovery document. The product is self-hosted, single-tenant,
with one known CLI consumer.

## Decision

Refactor the wire to strict RFC 8628 + RFC 8414 conformance in a single hard
break:

- Replace `/api/v1/auth/device{,/poll,/stream}` with the standard OAuth
  endpoints: `POST /api/v1/oauth/device_authorization`, `POST /api/v1/oauth/token`
  (a `grant_type` dispatcher), and `GET /.well-known/oauth-authorization-server`.
- Adopt RFC 6749 §5.1 / §5.2 response shapes: success returns `access_token`/
  `token_type: "Bearer"`; failure returns HTTP 400 with `{"error": <code>}`.
- Add per-flow `slow_down` cadence enforcement and an explicit Operator-driven
  `access_denied` path.
- Drop the SSE stream; clients poll plain at `interval` cadence.
- Keep today's minimum-viable token issuance: indefinite API tokens, no refresh
  token, no scope enforcement, single hardcoded `client_id = "uptrakit-cli"`.
  These deliberate omissions are paired with four named extension seams so future
  features land as targeted refactors rather than redesigns.

## Consequences

Positive:

- Any conformant RFC 8628 client works end-to-end without uptrakit-specific
  knowledge.
- `slow_down` is a per-flow protocol-correct signal; the IP rate limit no longer
  collides with well-behaved clients on shared NAT.
- The `access_denied` path gives Operators a phishing/mis-direction defence —
  active denial instead of "user closes tab and waits for expiry".
- The token endpoint dispatcher is the single integration point for future
  OAuth grants (refresh, password, client credentials).

Negative / accepted trade-offs:

- Hard break: backend + CLI + frontend ship together. There is no
  cross-version-compatible interim.
- Indefinite-lifetime tokens land alongside the new endpoints. A future
  refresh-token migration will require operators to rotate; this is documented
  as Seam 1.

### Four named seams (extension points)

The implementation deliberately localises each anticipated future feature to a
single named function:

1. **Token issuance — Seam 1.** `crates/ui/web-api-auth/src/auth/device_flow.rs`,
   `issue_access_token`. Today mints an indefinite API token. Future: returns
   `TokenPair { access_token, expires_in, refresh_token }` and the
   `OAuthTokenResponse` fields stop being `None`. The `refresh_token` grant arm
   slots into the `/api/v1/oauth/token` dispatcher.

2. **Scope enforcement — Seam 2.** `crates/ui/web-api-auth/src/auth/device_flow.rs`,
   `apply_scope_to_token`. Today a no-op stub. Future: parses the `scope`
   string (RFC 6749 §3.3), maps each scope to a `Permission` subset, and
   attaches the narrowed permission set to the minted token. The `scope`
   parameter is already persisted on `pending_device_flows.scope` and echoed in
   audit; no other call sites change.

3. **Client registry — Seam 3.** `crates/ui/web-api-auth/src/auth/device_flow.rs`,
   `validate_client_id` + `CLIENT_ID` constant. Today validates an exact
   match. Future: replaces the function body with an `oauth_clients` table
   lookup; the constant is deleted. No route handler changes.

4. **Long-poll — Seam 4.** `crates/ui/web-api/src/routes/oauth/token.rs`,
   `device_code_grant` arm. Today returns the current outcome immediately.
   Future: an opt-in `wait` form parameter (capped ≤30s, below typical
   reverse-proxy idle timeouts). When present and the outcome would be
   `authorization_pending`, the handler awaits a `tokio::sync::Notify` keyed
   by `device_code` up to the cap, then re-evaluates. RFC-compliant clients
   that omit `wait` see the existing behaviour.

## Notes

- `CONTEXT.md` is unchanged: RFC 8628 vocabulary is OAuth standard, not
  uptrakit-specific. The existing reservation of the noun "device" for this
  flow continues to hold.
- The implementation plan that lands these changes is split into two PRs to
  keep review tractable: backend (this plan), then client (CLI + frontend +
  openapi-client).

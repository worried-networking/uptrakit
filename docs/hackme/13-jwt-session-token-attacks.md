# ATK-13: JWT and Session Token Attacks

| Field | Value |
| --- | --- |
| Severity | Medium |
| Attack surface | Authentication (JWT, refresh tokens, API tokens) |
| Prerequisites | Network position to intercept tokens, or database read access |
| STRIDE | Spoofing, Repudiation |

## Attack description

### Token theft and replay

1. The attacker intercepts a valid JWT access token from the network (e.g., via XSS,
   browser extension, or compromised proxy).
2. The attacker replays the token within its 15-minute validity window to make
   authenticated API requests as the victim user.
3. The token contains embedded permissions, so the attacker operates with the victim's
   full authorization scope without any database lookup.

### Refresh token theft

1. The attacker obtains a refresh token from the victim's browser (stored in an
   `HttpOnly; Secure; SameSite=Strict` cookie).
2. The attacker uses the refresh token to obtain new access tokens, extending their
   session for up to 7 days.
3. Each refresh rotates the token atomically (old token revoked, new token issued),
   so the legitimate user's next refresh will fail — alerting them to the compromise.

### API token abuse

1. The attacker obtains an API token (prefixed `upk_`). API tokens are long-lived,
   revocable bearer tokens.
2. The attacker makes unlimited API requests using the token. There is no rate limit
   on the API token authentication path.
3. API tokens are stored as SHA-256 hashes in the database. Unlike JWTs, API token
   authentication performs a fresh database lookup on every request, ensuring
   permissions are always current.

### Session fixation via OIDC link token

1. The OIDC link token is exposed in the browser URL bar during the account linking
   flow (`/login?link_required=true&link_token=<token>`).
2. The attacker captures this URL from browser history, server access logs, or
   shoulder surfing.
3. The attacker uses the link token to link their OIDC identity to the victim's
   account.

## Worst-case impact

- **Unauthorized API access.** Stolen JWT or API tokens grant full access to the API
  with the victim's permissions for the token's lifetime.
- **Persistent access.** Stolen refresh tokens provide up to 7 days of renewed access.
  Stolen API tokens provide indefinite access until revoked.
- **Account linking hijack.** A captured OIDC link token allows the attacker to link
  their external identity to the victim's account, gaining permanent access via OIDC
  login.
- **Permission staleness.** JWT access tokens carry embedded permissions that are not
  re-validated for 15 minutes. A user whose permissions are revoked retains their old
  permissions until the token expires.

## Current mitigations

- **Short-lived access tokens.** JWT access tokens expire after 15 minutes
  (`ACCESS_TOKEN_EXPIRY_SECS = 900`), limiting the replay window.
- **Token denylist.** An in-memory `TokenDenylist` supports per-JTI and per-user
  revocation. On logout, all tokens for the user are denied for the remaining access
  token lifetime. The denylist is persisted to the database and survives restarts.
- **Strict JWT validation.** `decode_access_token()` validates `exp`, `iss`
  (`"uptrakit"`), and `aud` (`["uptrakit"]`). Tokens missing these claims or with
  wrong values are rejected, preventing cross-deployment replay.
- **Refresh token rotation.** Refresh tokens are rotated atomically in a database
  transaction on every use. The old token is revoked and cannot be replayed.
- **Secure cookie attributes.** Refresh tokens are delivered in `HttpOnly; Secure;
  SameSite=Strict` cookies scoped to `/api/v1/auth`, preventing XSS-based theft
  and cross-site request forgery.
- **SHA-256 hashed refresh tokens.** Refresh tokens are stored as SHA-256 hashes,
  not plaintext. Database access alone does not yield usable tokens.
- **Single-use link tokens.** OIDC link tokens are stored as SHA-256 hashes, have
  a 10-minute TTL, and are consumed atomically on first use.
- **`Referrer-Policy: no-referrer`.** Set on OIDC redirect responses to prevent
  link token leakage via the `Referer` header.
- **HMAC-SHA256 JWT signing.** The signing key is 64 bytes (512 bits) of CSPRNG
  randomness, stored encrypted in the database.

## Residual risk

- **Per-instance denylist.** The token denylist is in-memory per controller instance.
  Without NATS, cross-instance revocation relies on natural token expiry (15 minutes).
  A token revoked on one instance remains valid on others until expiry.
- **No rate limit on API token path.** API tokens bypass the auth rate limiter. An
  attacker with a stolen API token can make unlimited requests without triggering rate
  limits.
- **Link token URL exposure.** The OIDC link token appears in the browser address bar,
  browser history, and server access logs. Despite `Referrer-Policy` and single-use
  semantics, the 10-minute window provides an exploitation opportunity.
- **Permission staleness window.** After a role change, the user's old permissions
  remain embedded in active JWT tokens for up to 15 minutes. During this window, a
  demoted user retains elevated access, and a promoted user lacks new permissions.
- **SHA-256 for API tokens.** API tokens are stored as SHA-256 hashes (fast), not
  Argon2id (slow). With 256-bit token entropy, offline brute-force is infeasible,
  but SHA-256 provides weaker resistance to preimage attacks than Argon2id if token
  entropy were ever reduced.
- **No token binding.** JWT and API tokens are not bound to a specific client IP,
  user agent, or TLS session. A stolen token is usable from any network location.

## Recommended improvements

- Implement cross-instance token denylist synchronization via NATS (already partially
  supported: `TokenRevoked` NATS messages exist but require NATS to be configured).
- Add rate limiting to the API token authentication path, similar to the existing auth
  endpoint limits.
- Consider reducing the access token lifetime from 15 minutes to 5 minutes to narrow
  the staleness and replay windows.
- Add optional token binding to client characteristics (e.g., IP address or
  User-Agent fingerprint) for sensitive administrative operations.
- Implement a "forced re-authentication" mechanism that invalidates all active tokens
  for a user when their role is changed, closing the permission staleness window.
- Add access log monitoring for API tokens that detects usage from unexpected IP
  addresses or at unusual rates.

## References

- [Auth and Authorization](../security/auth-and-authorization.md)
- [Secrets and Encryption](../security/secrets-and-encryption.md)
- `crates/ui/web-api/src/auth/jwt.rs` — `JwtManager`, `AccessTokenClaims`
- `crates/ui/web-api/src/auth/session.rs` — `SessionService`, refresh token rotation
- `crates/ui/web-api/src/auth/api_token.rs` — `ApiTokenService`
- `crates/ui/web-api/src/auth/token_denylist.rs` — `TokenDenylist`
- `crates/ui/web-api/src/auth/refresh_cookie.rs` — cookie attributes
- `crates/ui/web-api/src/middleware/require_auth.rs` — auth middleware

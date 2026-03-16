# ATK-01: Enrollment Token Brute Force and Timing

| Field | Value |
| --- | --- |
| Severity | Medium |
| Attack surface | Authentication (WebSocket enrollment) |
| Prerequisites | Network access to the controller WebSocket endpoint |
| STRIDE | Spoofing |

## Attack description

1. The attacker identifies the controller's WebSocket endpoint (`/api/v1/ws/service`).
2. The attacker connects anonymously and sends `enroll` messages with guessed
   enrollment tokens.
3. For each attempt, the controller loads all active tokens from the database and
   iterates through them, verifying the provided secret against each stored Argon2id
   hash via `verify_password()`.
4. The attacker attempts to brute-force the token value, or uses timing side channels
   to infer whether the token matched any stored hash.

## Worst-case impact

- A correctly guessed token grants automatic service approval, bypassing the manual
  approval workflow entirely.
- The attacker's rogue agent is enrolled with whatever capabilities the matched token
  permits, potentially including `software_discovery` and `update_hooks`.
- The rogue agent receives mTLS certificates and can participate in the update
  pipeline, receiving version check assignments and potentially triggering or
  intercepting updates.

## Current mitigations

- **High token entropy.** Enrollment tokens are 32 bytes (256 bits) from OS CSPRNG,
  base64url-encoded to 43 characters. Brute-force is computationally infeasible.
- **Argon2id hashing.** Tokens are hashed with Argon2id using OWASP-recommended
  parameters (19 MiB memory, 2 iterations). Each verification attempt is
  deliberately expensive (~100ms+ per token).
- **Constant-time verification.** The `argon2` crate performs constant-time equality
  of the derived key internally, preventing byte-by-byte timing leaks.
- **Token lifecycle controls.** Tokens support `max_uses` limits, `expires_at` TTL,
  and soft-delete revocation. Expired or exhausted tokens are excluded from the
  verification loop.
- **Capability scoping.** Tokens can restrict which service types they approve by
  requiring capability intersection. A token scoped to `update_tracking` will not approve
  an agent-type service.
- **WebSocket connection rate limiting.** The controller enforces 30 connections per
  60 seconds per IP and 10 authentication failures per 300 seconds per IP.

## Residual risk

- **No per-endpoint rate limit on enrollment messages.** The WebSocket message rate
  limiter allows 50 messages per second per connection. While each Argon2id
  verification is expensive, an attacker with many source IPs could sustain moderate
  throughput.
- **Linear scan over active tokens.** The controller iterates all active tokens on
  every enrollment attempt, calling `verify_password()` for each. With N active
  tokens, each attempt costs O(N x Argon2id). This amplifies the cost of legitimate
  enrollment when many tokens exist, but also means the total server-side CPU cost
  scales with token count, creating a potential denial-of-service vector if an
  attacker floods enrollment attempts.
- **Timing oracle on token count.** The total response time is proportional to the
  number of active tokens (early-exit on match, full iteration on miss). An attacker
  could infer the approximate number of active tokens by measuring response latency.
- **Enrollment secret uses SHA-256.** The post-enrollment bearer secret (used during
  the brief window before mTLS certificate issuance) is stored as SHA-256, not
  Argon2id. This is lower risk because the secret is 256-bit random and short-lived,
  but it is a weaker storage mechanism than the enrollment token itself.

## Recommended improvements

- Add a dedicated rate limit on the WebSocket enrollment message type (e.g., 3
  enrollment attempts per minute per IP) separate from the general message rate limit.
- Consider capping the maximum number of active enrollment tokens per tenant to bound
  the linear scan cost.
- Add monitoring and alerting for repeated enrollment failures from the same IP.
- Consider moving the enrollment secret to Argon2id hashing for consistency, even
  though the short-lived window and 256-bit entropy make SHA-256 acceptable.

## References

- [Enrollment Tokens API](../api/enrollment-tokens.md)
- [Wire Protocol — Agent Lifecycle](../api/wire-protocol.md#agent-lifecycle)
- [Auth and Authorization](../security/auth-and-authorization.md)
- [Secrets and Encryption](../security/secrets-and-encryption.md)
- `crates/ui/web-api/src/routes/agents.rs` — `do_enroll()`
- `crates/ui/web-api-auth/src/auth/password.rs` — `hash_password()`, `verify_password()`
- `crates/ui/web-api-auth/src/auth/token.rs` — `generate_secure_token()`, `hash_token()`

# Code Review: `uptrakit-web-api-auth`

- Review date: 2026-03-17
- Scope: current-state review

## Summary

The authentication crate is in substantially better shape than older review history suggests.
Token revocation persistence, OIDC helper flows, and settings-store behavior are all stronger now.
Rate limiting is atomically correct and free of raw-SQL injection risks. One new correctness issue
was found in this review cycle: the OIDC state store returns the same sentinel value for both
expired tokens and tokens that never existed.

## Strengths

- Auth behavior is cleanly separated from transport and query logic.
- Rate limiting uses atomic upsert semantics; sliding-window counters are correct and backend-
  agnostic with fully parameterized SQL.
- JWT validation enforces `iss="uptrakit"` and `aud=["uptrakit"]` claims; legacy tokens without
  these claims are explicitly rejected.
- Argon2id password verification uses constant-time comparison from the `argon2` library.
- OIDC CSRF state tokens use single-use, delete-on-consume semantics.
- Current tests cover password, session, token, OIDC, and device-flow behavior broadly.

## Active Findings

### [MEDIUM] OIDC state store does not distinguish expired tokens from never-existed tokens

- Dimension: security, user experience
- Scope: `crates/ui/web-api-auth/src/auth/oidc_state.rs:OidcFlowStore::take`
- Why it matters: `take()` returns `None` for both an expired CSRF state token and a token that was
  never issued. Callers cannot provide a specific "session expired, please log in again" message.
  More importantly, any code that checks only for `None` before proceeding cannot distinguish a
  benign expiry from a token that was never issued, which could mask relay attempts in future
  handler changes.
- Failure scenario: a user's OIDC login times out (> 10 minutes); the callback handler returns a
  generic "not found" error rather than a user-friendly "session expired" message. For a security
  reviewer, the lack of a distinct expired-vs-unknown response makes it harder to audit the CSRF
  protection path.
- Fix: add a separate `OidcFlowStore::check_expired` method, or return a typed enum
  (`NotFound | Expired | Consumed`) from `take()` so callers can distinguish all three cases.

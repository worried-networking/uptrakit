# ATK-10: OIDC Provider Compromise

| Field | Value |
| --- | --- |
| Severity | High |
| Attack surface | Authentication / OIDC |
| Prerequisites | Compromise of a configured OIDC identity provider |
| STRIDE | Spoofing, Elevation of Privilege |

## Attack description

1. The attacker compromises an OIDC identity provider (IdP) configured in Uptrakit,
   or controls the IdP's token signing keys.
2. The attacker forges ID tokens with arbitrary claims:
   - **`email`**: set to an existing user's email to attempt account takeover.
   - **`sub`**: set to an existing OIDC subject to hijack a linked account.
   - **Role claims** (e.g., `realm.roles`): set to values that map to `owner` or
     `admin` roles via the provider's role mapping configuration.
   - **`email_verified`**: set to `true` to bypass the email verification check.
3. The attacker initiates an OIDC login flow. The controller's `resolve_oidc_user()`
   processes the forged token and:
   - Logs in as the matched user (if `oidc_subject` matches an existing link).
   - Creates a new user with attacker-controlled roles (if `auto_create_users` is
     enabled and no existing account matches).
   - Escalates an existing user's roles via `sync_oidc_roles()` which fully replaces
     the user's role set on every login.

## Worst-case impact

- **Account takeover.** If the attacker's forged `oidc_subject` matches an existing
  OIDC link, they gain full access to that user's account.
- **Privilege escalation to owner.** Via role claim manipulation, the attacker maps
  their account to the `owner` role, gaining all nine permissions including
  `manage_global_settings`.
- **Mass account creation.** With `auto_create_users` enabled, the attacker creates
  an arbitrary number of accounts with elevated roles.
- **Persistent access.** Once an OIDC link is established, the attacker can log in
  repeatedly via the compromised provider. Revoking the user account is the only
  remediation (disabling the OIDC provider blocks all users linked to it, including
  legitimate ones).

## Current mitigations

- **No auto-linking by email.** `resolve_oidc_user()` does not automatically link a
  new OIDC subject to an existing user by email match alone. If the email matches
  an existing user with a different OIDC link or a password account, explicit account
  linking is required. This prevents email-based account takeover.
- **`email_verified` enforcement.** If the IdP returns `email_verified: false`, the
  login is rejected with `OidcUserResolution::EmailNotVerified`. This prevents
  accounts with unverified email addresses from being created or linked.
- **PKCE and CSRF protection.** The OIDC authorization flow uses PKCE code challenges
  and CSRF state tokens (stored encrypted, single-use, 10-minute TTL), preventing
  authorization code injection and cross-site request forgery.
- **Nonce validation.** The OIDC nonce is validated in the ID token to prevent token
  replay.
- **OIDC client secret encryption.** Provider client secrets are stored encrypted
  (`EncryptedString`) in the database, protected by the master key.
- **Session integrity validation.** OIDC sessions enforce `oidc_provider_id IS NOT
  NULL` via a database CHECK constraint. Tokens with `auth_method = "oidc"` but
  missing `oidc_provider_id` are rejected.
- **Rate limiting on OIDC endpoints.** Exchange, link, and complete-registration
  endpoints are rate-limited (5-10 requests per 60 seconds per IP).

## Residual risk

- **`email_verified = None` passes.** When the IdP omits the `email_verified` claim
  entirely (rather than setting it to `false`), the check is bypassed. Many
  legitimate providers omit this claim for confirmed accounts, but a compromised
  provider could exploit this to skip verification.
- **Role mapping fully replaces roles on login.** `sync_oidc_roles()` deletes all
  existing `user_roles` and inserts the mapped roles on every OIDC login. A
  compromised provider can escalate or de-escalate any user's roles silently.
- **Auto-create enables mass provisioning.** When `auto_create_users` is enabled, the
  compromised provider can create unlimited accounts. There is no per-provider account
  creation rate limit.
- **No IdP health monitoring.** Uptrakit does not monitor the IdP's JWKS endpoint for
  unexpected key rotations or anomalies that might indicate compromise.
- **Authorize endpoint is not rate-limited.** The OIDC authorize redirect
  (`/api/v1/auth/oidc/{slug}/authorize`) is not in the rate limit table. An attacker
  could flood this endpoint to trigger excessive IdP discovery requests.
- **Provider deactivation affects all linked users.** Disabling a compromised OIDC
  provider blocks all users linked to it, including legitimate ones. There is no
  mechanism to selectively block compromised accounts while maintaining access for
  others.

## Recommended improvements

- Add rate limiting to the OIDC authorize endpoint to prevent abuse of IdP discovery
  and authorization redirects.
- Implement per-provider account creation limits (e.g., max new accounts per hour) to
  bound the damage from a compromised IdP with `auto_create_users` enabled.
- Add an admin notification when new OIDC accounts are created, especially when they
  receive elevated roles via role mapping.
- Provide a "freeze" mode for OIDC providers that blocks new account creation and role
  changes while allowing existing linked users to continue logging in.
- Consider treating `email_verified = null/absent` as `false` by default, with a
  per-provider toggle for IdPs that are known to omit the claim.
- Add monitoring for unexpected JWKS key rotations on configured OIDC providers.
- Document the security implications of `auto_create_users` and role mapping in
  operator guides, emphasizing that these features should only be enabled for
  fully trusted IdPs.

## References

- [Auth and Authorization](../security/auth-and-authorization.md)
- [Auth Flows](../api/auth-flows.md)
- [Secrets and Encryption](../security/secrets-and-encryption.md)
- `crates/ui/web-api/src/auth/authentication.rs` — `resolve_oidc_user()`,
  `sync_oidc_roles()`
- `crates/ui/web-api/src/auth/oidc_state.rs` — `OidcFlowStore`, PKCE, CSRF
- `crates/ui/web-api/src/routes/oidc_auth.rs` — OIDC route handlers

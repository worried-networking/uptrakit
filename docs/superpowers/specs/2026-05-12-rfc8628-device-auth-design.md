# RFC 8628 Device Authorization Grant Compliance

## Goal

Refactor uptrakit's CLI device-authentication surface so it is strictly
compliant with RFC 8628 (OAuth 2.0 Device Authorization Grant) and exposes
the RFC 8414 (OAuth 2.0 Authorization Server Metadata) discovery document
for the device grant.

The refactor preserves today's product behaviour (CLI logs in by opening a
browser, Operator approves a short code, CLI receives a long-lived API
token) while replacing every wire detail that diverges from the RFC. The
spec also lands four labelled extension seams so later OAuth features land
as additive refactors rather than redesigns.

## Scope

### In scope

- Replace the device-auth wire surface with strict RFC 8628 endpoints,
  request shapes, response shapes, and error codes.
- Adopt the standard OAuth 2.0 token endpoint (`POST /api/v1/oauth/token`)
  as a `grant_type` dispatcher. Only the device-code grant is handled
  today; every other grant type returns `unsupported_grant_type` per
  RFC 6749 §5.2.
- Expose `/.well-known/oauth-authorization-server` (RFC 8414) covering
  only the device grant.
- Add a per-flow `slow_down` cadence check on the token endpoint.
- Add an explicit Operator-driven `access_denied` path: a new
  `/api/v1/auth/device/deny` UI op plus a Deny button on the approval
  page.
- Add `verification_uri_complete` to the device-authorization response
  and wire it through the CLI.
- Keep the existing single-bearer-token issuance behaviour (indefinite
  lifetime, no refresh, no scope enforcement) and label every place where
  future enrichment will plug in.
- Hard break: backend, CLI, and frontend ship in the same release. No
  versioned routes, no dual-shape responses, no deprecation window.

### Out of scope (explicitly deferred)

- Short-lived bearer tokens, refresh tokens, token rotation.
- Real scope enforcement (mapping scopes to `Permission` subsets).
- A real OAuth client registry (per-client allowlist, client secrets,
  per-client policies). Today's server validates a single hardcoded
  `client_id` constant.
- Long-poll on the token endpoint. The CLI polls at `interval` cadence.
- Any non-device OAuth grant (authorization code, client credentials,
  password, refresh token).
- PKCE. Device flow does not use it (RFC 8628 §6); discovery doc
  reflects an empty `code_challenge_methods_supported`.
- Re-keying the user-code charset. Current consonants-only `XXXX-XXXX`
  charset already aligns with RFC §6.1 BCP; spec verifies and documents.

### Explicitly not addressed

- No new domain term enters `CONTEXT.md`. RFC 8628 vocabulary
  (`device_code`, `user_code`, `verification_uri`, `client_id`) is OAuth
  standard, not uptrakit-specific. `CONTEXT.md` already reserves the
  noun "device" for this flow; that reservation continues to hold.
- The CLI's local token-on-disk format and the `crates/ui/cli` config
  loader are untouched. Only the wire calls and the error parsing
  change.
- The `pending_device_flows` cleanup task (background sweeper for
  expired rows) is untouched.

## Design principles

- **Strict RFC on the wire.** Any conformant RFC 8628 client must work
  end-to-end without uptrakit-specific knowledge. That rules out hybrid
  response shapes, custom enum status fields, and HTTP 404 for
  protocol-level negative outcomes.
- **Minimum viable token issuance.** RFC 8628 references RFC 6749 §5.1
  for the token response, but it does not require refresh tokens,
  `expires_in`, or scope enforcement. The spec ships exactly what the
  RFC requires today and labels every seam for later enrichment.
- **Labelled seams beat hidden flexibility.** Where a future migration is
  anticipated (token issuance, scope enforcement, client registry,
  long-poll), the code contains one named function or trait whose entire
  purpose is to be the swap-point. The spec lists the location.
- **Hard break, single PR.** The product is self-hosted, single-tenant,
  with no third-party consumers of the current API. A versioned shim
  layer would carry permanent maintenance cost for a one-time migration.
  Backend, CLI, frontend, and OpenAPI client ship together.
- **Idiomatic Axum / SeaORM / tokio.** No middleware tricks, no custom
  body-extraction layers, no fights with the framework. `Form<T>` for
  form-urlencoded bodies; `Json<T>` only on the surviving UI-internal
  endpoints. SeaORM migrations and `Entity`/`ActiveModel` for the
  schema changes. No new in-process state — `slow_down` cadence lives
  on the flow row.

## Wire surface

### Endpoint map

| Method | Path                                      | Purpose                                                                                                              | Auth              | Body                                |
| ------ | ----------------------------------------- | -------------------------------------------------------------------------------------------------------------------- | ----------------- | ----------------------------------- |
| `POST` | `/api/v1/oauth/device_authorization`      | RFC 8628 §3.1 device-authorization request                                                                           | none              | `application/x-www-form-urlencoded` |
| `POST` | `/api/v1/oauth/token`                     | RFC 6749 §3.2 / RFC 8628 §3.4 token endpoint (`grant_type` dispatcher)                                               | none              | `application/x-www-form-urlencoded` |
| `GET`  | `/.well-known/oauth-authorization-server` | RFC 8414 §3 authorization server metadata                                                                            | none              | n/a                                 |
| `POST` | `/api/v1/auth/device/approve`             | UI-internal: Operator approves a user_code (existing route, signature changes only as noted)                         | `CanViewServices` | `Json<ApproveRequest>`              |
| `POST` | `/api/v1/auth/device/deny`                | UI-internal: Operator denies a user_code (new route)                                                                 | `CanViewServices` | `Json<DenyRequest>`                 |
| `GET`  | `/api/v1/auth/device/lookup`              | UI-internal: read `client_name` + `expires_at` for a `user_code` so the approval page can render context (new route) | `CanViewServices` | query: `user_code`                  |

### Routes deleted

- `POST /api/v1/auth/device` — replaced by `/api/v1/oauth/device_authorization`.
- `POST /api/v1/auth/device/poll` — replaced by `/api/v1/oauth/token`.
- `GET /api/v1/auth/device/stream` — SSE endpoint removed (no
  replacement in this spec; long-poll is the documented future
  migration on `/api/v1/oauth/token`).

### `/api/v1/oauth/device_authorization`

Request body (form-urlencoded):

| Field         | Required | Source                         | Notes                                                                                                                                    |
| ------------- | -------- | ------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------- |
| `client_id`   | yes      | RFC 8628 §3.1                  | Must equal the hardcoded `uptrakit-cli` constant. Mismatch returns 400 `invalid_client`.                                                 |
| `scope`       | no       | RFC 8628 §3.1 (OAuth 2.0 §3.3) | Stored on flow row, echoed on token response. Not enforced.                                                                              |
| `client_name` | no       | uptrakit extension             | Free-form audit label (e.g. `cli-laptop-2026-05-12`). Persisted on flow row, surfaces in audit `details`, shown in frontend approval UI. |

Response (HTTP 200, `application/json`, RFC 8628 §3.2):

```json
{
  "device_code": "<opaque>",
  "user_code": "ABCD-EFGH",
  "verification_uri": "https://controller.example/device",
  "verification_uri_complete": "https://controller.example/device?user_code=ABCD-EFGH",
  "expires_in": 600,
  "interval": 5
}
```

Notes:

- `verification_uri` and `verification_uri_complete` are derived from the
  same external base URL the current implementation already resolves
  (`ExternalBaseUrl` extractor, then `Origin`, then `Host`).
- `expires_in` and `interval` are constants today (600s and 5s
  respectively). Future configurability is out of scope.
- The response shape is exactly RFC 8628 §3.2 — no extra fields, no
  renames. Off-the-shelf RFC 8628 clients deserialize directly.

### `/api/v1/oauth/token`

Request body (form-urlencoded):

| Field         | Required               | Notes                                                                                                                 |
| ------------- | ---------------------- | --------------------------------------------------------------------------------------------------------------------- |
| `grant_type`  | yes                    | Only `urn:ietf:params:oauth:grant-type:device_code` is handled. Any other value returns 400 `unsupported_grant_type`. |
| `device_code` | yes (for device grant) | The opaque code returned from `device_authorization`.                                                                 |
| `client_id`   | yes (for device grant) | Must equal the hardcoded `uptrakit-cli` constant.                                                                     |

Handler parses `grant_type` as a `String` first, then dispatches on it.
This avoids serde-derived deserialization rejections that would mask the
RFC error response (RFC 6749 §5.2 requires `unsupported_grant_type` as
the JSON error code, not a generic 400).

Success response (HTTP 200, `application/json`, RFC 6749 §5.1):

```json
{
  "access_token": "<api-token-value>",
  "token_type": "Bearer"
}
```

`expires_in`, `refresh_token`, and `scope` are omitted (not null —
per RFC 6749 §5.1 "REQUIRED if omitted" wording, the correct OAuth idiom
is omission). Future addition is additive.

Error responses (HTTP 400, `application/json`, RFC 6749 §5.2 +
RFC 8628 §3.5):

```json
{ "error": "authorization_pending" }
{ "error": "slow_down", "interval": 10 }
{ "error": "access_denied" }
{ "error": "expired_token" }
{ "error": "invalid_request", "error_description": "missing device_code" }
{ "error": "invalid_client" }
{ "error": "invalid_grant" }
{ "error": "unsupported_grant_type" }
```

Semantics:

- `authorization_pending`: flow exists, status is `pending`, last-poll
  cadence is healthy.
- `slow_down`: flow exists, but the gap between `now` and
  `last_polled_at` is less than the flow's current `interval`. The
  server response includes a bumped `interval` (`current_interval + 5`,
  per RFC 8628 §3.5 client-side bump rule). The bumped value is
  persisted on `pending_device_flows.interval` so subsequent polls
  honour it.

  **RFC 8628 §3.5 only mandates the client-side bump.** Sending the
  bumped `interval` field in the `slow_down` error body is an uptrakit
  extension that lets a stricter RFC client honour the server's exact
  recommendation without computing it itself. The extension is
  backward-compatible: a stock RFC 8628 client that ignores extra
  fields applies its own `interval + 5` bump and reaches the same
  cadence within one extra poll. CLI deserialisation tolerates a
  server that omits the field (falls back to local `interval + 5`).

- `access_denied`: status is `denied` (Operator clicked Deny).
- `expired_token`: status is `expired`, OR the device_code is not in the
  table at all (unknown device_codes collapse into `expired_token` to
  avoid leaking an existence oracle).
- `invalid_request`: missing/empty required form field, or validation
  failure surfaced by the `Validate` trait.
- `invalid_client`: `client_id` mismatch against the constant.
- `invalid_grant`: device_code parameter present but malformed (not a
  valid opaque form). Distinct from `expired_token` (which covers
  "well-formed but unknown/expired").
- `unsupported_grant_type`: any `grant_type` value other than
  `urn:ietf:params:oauth:grant-type:device_code`.

### `/.well-known/oauth-authorization-server`

HTTP 200, `application/json` (RFC 8414 §3.2):

```json
{
  "issuer": "<external-base-url>",
  "device_authorization_endpoint": "<base>/api/v1/oauth/device_authorization",
  "token_endpoint": "<base>/api/v1/oauth/token",
  "grant_types_supported": ["urn:ietf:params:oauth:grant-type:device_code"],
  "response_types_supported": [],
  "token_endpoint_auth_methods_supported": ["none"],
  "code_challenge_methods_supported": []
}
```

- `response_types_supported: []` because the authorization-code grant is
  not implemented.
- `token_endpoint_auth_methods_supported: ["none"]` because the device
  flow is a public-client grant (RFC 8628 §3.4 says the token endpoint
  uses no client authentication for the device grant).
- `issuer` and endpoint URLs derive from the same `ExternalBaseUrl`
  chain the device-authorization handler already uses.
- The endpoint requires no auth and is safe to expose publicly.

### Error-code enum (wire-facing)

New type `OAuthErrorCode` in `crates/shared/web-api-types/src/oauth.rs`
(home of the new RFC types). Mirrors the project's wire-safe `Other(String)`
pattern (`crates/shared/wire/src/lib.rs` `EnrollmentStatus` / `ErrorCode`):

Generated via the `wire_safe_enum!` macro from
`uptrakit_shared_macros` — the project-mandated way to define wire-safe
enums (`docs/development/coding-standards.md` §"Wire-Safe `Other(String)`
Catch-All — Required implementation"). The macro auto-emits
`#[non_exhaustive]`, the `Other(String)` variant, `as_str`, `Display`,
`From<String>` (with `tracing::debug!` on unknown values), `Serialize`
via `serialize_str(self.as_str())`, infallible `Deserialize`, a strict
`FromStr`, and the `ParseOAuthErrorCodeError` type.

```rust
use uptrakit_shared_macros::wire_safe_enum;

wire_safe_enum! {
    /// OAuth 2.0 error codes per RFC 6749 §5.2 and RFC 8628 §3.5.
    #[derive(Clone, Debug, PartialEq, Eq, Hash)]
    #[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
    pub enum OAuthErrorCode {
        AuthorizationPending => "authorization_pending",
        SlowDown             => "slow_down",
        AccessDenied         => "access_denied",
        ExpiredToken         => "expired_token",
        InvalidRequest       => "invalid_request",
        InvalidClient        => "invalid_client",
        InvalidGrant         => "invalid_grant",
        UnsupportedGrantType => "unsupported_grant_type",
    }
    parse_error = ParseOAuthErrorCodeError("invalid OAuth 2.0 error code");
}
```

Constraints:

- Hand-written `Serialize`/`Deserialize` or `#[serde(rename_all)]` are
  forbidden — the macro is the only acceptable implementation
  (project standard).
- A test-local `KNOWN_VARIANTS` array in `#[cfg(test)] mod tests` drives
  exhaustive round-trip tests; no public `KNOWN_VARIANTS` constant on
  the type.
- `EnumIter` is not generated and never added (incompatible with the
  appended `Other(String)` tuple variant).

CLI consumes this enum on every 400 response from `/api/v1/oauth/token`.
A future server update introducing a new `error` value (e.g.
`temporarily_unavailable`) deserializes as
`Other("temporarily_unavailable")` on older CLIs without panicking and
emits a `tracing::debug!` log automatically — the macro guarantees
this.

### Form-urlencoded request types

Both `DeviceAuthorizationRequest` and `OAuthTokenRequest` are flat
structs deserialized via Axum's `Form<T>` extractor. Two serde rules
apply:

- **No struct-level `#[serde(rename_all = "...")]`**. RFC 8628 / 6749
  field names (`client_id`, `device_code`, `grant_type`, `scope`,
  `client_name`) are already snake*case, so the default mapping is
  correct. A `rename_all` attribute would also silently transform field
  \_values* when a downstream user accidentally moves an enum field into
  the struct.
- **`grant_type` is a plain `String`**, never a typed enum. The
  device-code grant value is the literal URI
  `urn:ietf:params:oauth:grant-type:device_code`; deserializing into
  an enum would either require an opaque `#[serde(rename = "...")]` on
  every variant or risk mangling. The handler matches the `String`
  against known grant types and returns RFC `unsupported_grant_type`
  for anything else.

### `Validate` trait

Every new request type implements `uptrakit_web_api_types::Validate`:

- `DeviceAuthorizationRequest` — `client_id` non-empty and equals
  constant; `scope` (if present) non-empty after trimming.
- `OAuthTokenRequest` — `grant_type` non-empty; for the device-code
  branch, `device_code` and `client_id` non-empty.
- `DeviceAuthApproveRequest` / `DeviceAuthDenyRequest` — `user_code`
  matches the `XXXX-XXXX` consonants pattern (the regex already lives in
  the device-flow store).

A `validate()` failure on the OAuth endpoints surfaces as HTTP 400
`invalid_request` with `error_description` set to the validator's
message. The internal UI endpoints retain their existing
`ApiError::BadRequest` shape (no need to RFC-flavour internal routes).

## Data model

### Migration `m20260512_000001_device_flow_rfc8628`

Single migration in `crates/shared/db/src/migration/`:

- `ALTER TABLE pending_device_flows ADD COLUMN last_polled_at TIMESTAMP NULL`.
  Default `NULL`. Updated atomically on each poll inside a
  `BEGIN IMMEDIATE` transaction (CLAUDE.md "SQLite Transaction Rules":
  read-then-write requires `BEGIN IMMEDIATE` to avoid
  `SQLITE_BUSY_SNAPSHOT`).
- `ALTER TABLE pending_device_flows ADD COLUMN interval INTEGER NOT NULL DEFAULT 5`.
  Stores the current effective polling interval (seconds) for the flow.
  Initialised to `5` on insert. Bumped by `5` each time a `slow_down`
  is returned. Read on every poll to drive the cadence check.
- `ALTER TABLE pending_device_flows ADD COLUMN scope TEXT NULL`.
  Stores the literal `scope` parameter from the device-authorization
  request. Echoed on the token response (Seam 2 future enrichment).
- `ALTER TABLE pending_device_flows ADD COLUMN denied_by UUID NULL`.
  Records the Operator's user-id when `deny` is invoked. Kept separate
  from the existing `user_id` column (which is used by `consume` to
  mint the token for the approver) so the two decisions never share a
  field with conflicting semantics.
- `pending_device_flows.status` widens to include `denied`. The column
  remains a `TEXT` column; allowed values are
  `pending|authorized|denied|expired`. Existing rows are unaffected
  (no value migration required).

No backfill required: `last_polled_at`, `scope`, and `denied_by` are
nullable; `interval` has a default; `denied` is a new value, not a
rename. The hard break means no rows exist in the wild needing
translation.

> **Operational note.** `DeviceAuthStatus` derives
> `sea_orm::DeriveActiveEnum`, whose generated DB→Rust conversion
> panics on unknown string values rather than returning an error.
> The "hard break, single PR" deployment posture is essential because
> a controller built before this PR that reads a row with
> `status = 'denied'` written by a newer peer would panic at runtime.
> Mixed-version operation across this PR boundary is unsupported.

### Entity update

`crates/shared/db/src/entity/pending_device_flow.rs`:

- Add `last_polled_at: Option<DateTimeUtc>` field.
- Add `interval: i32` field (non-nullable, default `5`).
- Add `scope: Option<String>` field.
- Add `denied_by: Option<Uuid>` field. Rustdoc comment clarifies that
  `user_id` is the approver and `denied_by` is the denier; the two
  fields are mutually exclusive in practice (a row in `denied` status
  has `user_id = NULL` and `denied_by = Some(...)`; a row in
  `authorized` status has `user_id = Some(...)` and
  `denied_by = NULL`).
- Add `denied` variant to `DeviceAuthStatus` in
  `crates/shared/types/src/device_auth_status.rs`. Per CLAUDE.md, this
  enum is already `#[non_exhaustive]`; adding a variant is additive at
  the type level. The implementation must also:
  - Update the `as_str`, `Display`, and `FromStr` match arms.
  - Update the three test arrays in `mod tests` (currently
    `[Pending, Authorized, Expired]`) to include `Denied`. These
    arrays drive `serde_round_trip`, `from_str_round_trip`, and
    `display_round_trip` — without the update, `Denied` would not be
    covered by the existing tests.
  - Add a new test asserting `DeviceAuthStatus::Denied.as_str() == "denied"`.

### Audit reuse

Existing constants in `crates/shared/audit-log/src/action_type.rs` cover
the new flow without additions:

- `AUTH_DEVICE_START` — emitted by `/oauth/device_authorization`.
- `AUTH_DEVICE_POLL` — emitted by `/oauth/token` (device-code grant
  branch). Outcome distinguishes `Success`, `Denied` (for
  `access_denied`), `Failed` (for `expired_token`, `invalid_grant`),
  and uses a new `reason_code: "slow_down"` detail for cadence
  violations.
- `AUTH_DEVICE_APPROVE` — emitted by `/auth/device/approve`. Unchanged.
- `AUTH_DEVICE_DENY` — emitted by the new `/auth/device/deny` route.
  Already exists in `auth_audit_classification.rs:73`.

`details` JSON gains a `slow_down: true` marker on poll-cadence
violations and a `scope` field when the requesting client included a
scope (so audit history records the requested permission level once
enforcement lands).

## Implementation outline (per crate)

### `crates/shared/types`

- Add `Denied` variant to `DeviceAuthStatus`. Update `FromStr`,
  `Display`, and the `as_str` method. Existing match-site discipline
  (wildcard arm, `tracing::warn!`) covers external consumers.
- Add unit test asserting `KNOWN_VARIANTS` includes `denied`.

### `crates/shared/web-api-types`

- New module `oauth.rs`:
  - `OAuthErrorCode` enum (wire-safe `Other(String)` pattern — see
    [Error-code enum (wire-facing)](#error-code-enum-wire-facing) for
    the constraints on `Serialize`, `Deserialize`, and `KNOWN_VARIANTS`).
  - `DeviceAuthorizationRequest` (form-urlencoded body — no
    struct-level `rename_all`).
  - `DeviceAuthorizationResponse` (RFC 8628 §3.2 shape including
    `verification_uri_complete`).
  - `OAuthTokenRequest` (form-urlencoded; `grant_type: String` (plain,
    no rename_all on the struct), optional `device_code`, optional
    `client_id`).
  - `OAuthTokenResponse` (RFC 6749 §5.1 success shape). `access_token`
    and `token_type` always present; `expires_in`, `refresh_token`,
    `scope` are `Option<...>` and `#[serde(skip_serializing_if = "Option::is_none")]`
    so they are omitted on the wire (RFC §5.1 idiom — omit, do not
    serialise as `null`).
  - `OAuthErrorResponse` (RFC 6749 §5.2 error shape: `error: OAuthErrorCode`,
    optional `error_description: String`, optional `interval: i32` (matches the DB column type) —
    uptrakit extension for `slow_down`).
  - `OAuthAuthorizationServerMetadata` (RFC 8414 §3 minimal subset).
  - `DeviceAuthDenyRequest` / `DeviceAuthDenyResponse` (mirrors
    `Approve` variants).
  - `DeviceAuthLookupQuery` (Axum `Query<...>` extractor type with one
    field, `user_code: String`).
  - `DeviceAuthLookupResponse` (`client_name: Option<String>`,
    `expires_at: DateTime<Utc>`).
- Implement `Validate` for every request type above. Failures on the
  OAuth endpoints map to `invalid_request`; failures on
  approve/deny/lookup keep the existing `ApiError::BadRequest`.
- Delete `DeviceAuthStartRequest`, `DeviceAuthStartResponse`,
  `DeviceAuthPollRequest`, `DeviceAuthPollResponse` from
  `device_auth.rs`. Keep `DeviceAuthApproveRequest` /
  `DeviceAuthApproveResponse` (internal UI op). Add
  `DeviceAuthDenyRequest` / `DeviceAuthDenyResponse` and the lookup
  types listed above.

### `crates/ui/web-api-auth`

`crates/ui/web-api-auth/src/auth/device_flow.rs`:

- Add `last_polled_at`, `interval`, `scope`, and `denied_by` to the
  `PendingDeviceFlow` model and the insert/select code paths.
- New method `poll(&self, device_code: &str, now: DateTime<Utc>) -> Result<PollOutcome, _>`
  returning a typed (internal) `PollOutcome` enum:
  - `Authorized { token: SecretString }`
  - `Pending`
  - `SlowDown { bumped_interval: i32 }`
  - `Denied`
  - `Expired`
  - `Unknown` (route layer maps to `expired_token`)
  - `MalformedDeviceCode` (route layer maps to `invalid_grant`)

  `now` is an injected parameter, not `Utc::now()` read inside the
  method. This keeps the cadence test path deterministic without
  depending on `#[tokio::test(start_paused = true)]`, which CLAUDE.md
  explicitly forbids combining with SeaORM SQLite tests. Route handlers
  pass `Utc::now()` directly.

- `poll` is implemented as a single `BEGIN IMMEDIATE` transaction
  (CLAUDE.md "SQLite Transaction Rules" — read-then-write requires
  Immediate to avoid `SQLITE_BUSY_SNAPSHOT`):
  1. Read the flow row (`SELECT ... WHERE device_code_hash = $1`).
  2. If status is `pending` and `last_polled_at` is `Some(prev)` and
     `now - prev < flow.interval` seconds, set
     `flow.interval = flow.interval + POLL_INTERVAL_BUMP`, write the
     bumped value + the new `last_polled_at = now`, commit, return
     `SlowDown { bumped_interval: flow.interval }`.
  3. If status is `pending`, update `last_polled_at = now`, commit,
     return `Pending`.
  4. If status is `authorized`, run an atomic delete keyed by
     `(device_code_hash, status = 'authorized')` (preserving the
     existing HA-safe double-consume pattern), commit, mint a token via
     `issue_access_token`, return `Authorized`.
  5. If status is `denied`, commit, return `Denied`. The row stays
     until the background sweeper clears it.
  6. If status is `expired`, commit, return `Expired`. If the row is
     absent, return `Unknown` without writing anything.

  This `poll` method supersedes the existing `consume`. The existing
  `consume` method's two-step `find()` + conditional `delete_many()`
  is folded into step 4 of `poll`, now under `BEGIN IMMEDIATE`.

  > **Existing-code correction.** The current
  > `device_flow.rs::consume` does **not** use `BEGIN IMMEDIATE` (see
  > `crates/ui/web-api-auth/src/auth/device_flow.rs:173-204`). Its
  > read-then-conditional-write pattern is technically subject to
  > `SQLITE_BUSY_SNAPSHOT` even though the conditional `delete_many`
  > is itself atomic. Folding the consume path into `poll` under
  > `BEGIN IMMEDIATE` closes that gap as a side effect of this
  > refactor. After the refactor, `consume` is removed; the only
  > caller is `poll`.

- New method `deny(&self, user_code: &str, denied_by: UserId) -> Result<(), _>`
  parallels the existing `approve`. It looks up the flow by
  normalized `user_code` (matching `approve`'s path — `user_code` is
  stored as plain normalized text, not hashed). The atomic update is
  written as `UPDATE pending_device_flows SET status = 'denied',
denied_by = $1 WHERE user_code = $2 AND status = 'pending'`. The
  zero-rows-affected branch maps to the existing approval-failure
  error type via `approval_classification`. Concurrent approve/deny
  races resolve at the atomic CAS: whichever transaction commits
  first wins; the other receives the same `not-pending` error
  `approve` already raises. `user_id` and `denied_by` are never both
  set on the same row. `deny` does not take a `now` parameter (no
  timestamp column is set); this matches the existing `approve`
  signature.

- Add `pub const POLL_INTERVAL_BUMP: i32 = 5` and
  `pub const CLIENT_ID: &str = "uptrakit-cli"`. The `client_id` constant
  lives next to the validation function (`validate_client_id`). This
  function is **Seam 3** in the Future Migrations section: a future
  client-registry table swap replaces this function only.

- New function `issue_access_token(&self, user_id: UserId) -> Result<SecretString, _>`
  is **Seam 1** in the Future Migrations section: today it calls the
  same API-token mint code the current `consume` method uses. A future
  short-lived bearer + refresh-token feature replaces this function
  only.

- Delete `get_device_code_hash_by_user_code` from `device_flow.rs`. Its
  only consumer is the SSE-broadcaster `notify_status_changed` call in
  the approve handler, both of which are deleted in this PR (see
  `crates/ui/web-api` section below).

### `crates/ui/web-api`

New module `crates/ui/web-api/src/routes/oauth/`:

- `mod.rs` — re-exports the three handlers and registers them.
- `device_authorization.rs` — handler for `POST /api/v1/oauth/device_authorization`.
- `token.rs` — handler for `POST /api/v1/oauth/token`. Parses
  `grant_type` first (plain `String`, no enum), dispatches to
  `device_code_grant(...)`. A small `OAuthGrantHandler` trait
  abstracts the per-grant logic so future grant arms (refresh,
  password, client*credentials) are additive — but this trait is \_not*
  one of the four named Future Migrations seams; it is an ordinary
  extension point left in place by the dispatcher shape.
- `metadata.rs` — handler for `GET /.well-known/oauth-authorization-server`.

`crates/ui/web-api/src/routes/device_auth.rs`:

- Delete `device_auth_start`, `device_auth_poll`, `device_auth_stream`.
- Keep `device_auth_approve`.
- Add `device_auth_deny`, structurally a clone of `device_auth_approve`
  invoking `device_flow_store.deny(...)` and emitting `AUTH_DEVICE_DENY`.

`crates/ui/web-api/src/router.rs`:

- Mount `/.well-known/oauth-authorization-server` outside the
  `/api/v1` prefix (the `well-known` namespace is RFC 8615 reserved
  and must not be nested).
- Mount the new `/api/v1/oauth/*` routes.
- Replace the deleted `/api/v1/auth/device`, `/api/v1/auth/device/poll`,
  `/api/v1/auth/device/stream` mount points.

`crates/ui/web-api/src/middleware/rate_limit.rs`:

- Remove the `/api/v1/auth/device` and `/api/v1/auth/device/poll`
  entries.
- Add `/api/v1/oauth/device_authorization` with the previous
  `/api/v1/auth/device` budget (10 req/60s).
- Add `/api/v1/oauth/token` at a relaxed budget of **60 req/60s** —
  well above the per-flow cadence (~12 polls/min at default `interval`),
  comfortable for several concurrent flows on shared NAT, still trips
  hostile actors. The spec calls out the explicit number so future
  readers can revisit if the default `interval` changes.
- Add `/api/v1/auth/device/deny` mirroring `/approve` (5 req/60s).
- Add `/api/v1/auth/device/lookup` at a generous read budget (60 req/60s)
  since the page may re-fetch on focus/visibility events.
- Update the test module's path lists. The existing tests
  `rate_limited_paths_list` (line 231), `non_rate_limited_paths`
  (line 263), and `device_poll_has_higher_limit` (line 280) all
  reference the deleted paths and must be rewritten to assert the new
  set. `device_poll_has_higher_limit` is renamed to
  `oauth_token_has_higher_limit` and points at `/api/v1/oauth/token`.

`crates/ui/web-api/src/device_flow_broadcaster.rs`:

- Delete the whole file. The SSE channel only served the dropped
  `/stream` endpoint.
- Remove the `device_flow_broadcaster` field from `BroadcastState`
  (`crates/ui/web-api/src/app_state.rs:110`) and from
  `BroadcastStateBuilder` (`app_state.rs:344, 401, 801`) plus any
  default-impl wiring. `BroadcastState` is `#[non_exhaustive]`; the
  workspace's test-harness construction sites
  (`crates/ui/web-api/src/test_harness/mod.rs`,
  `crates/ui/web-api/src/lib.rs:226-227`) must drop the field.
- `device_auth_approve` no longer calls
  `get_device_code_hash_by_user_code` or
  `device_flow_broadcaster.notify_status_changed`. Both call sites
  (`crates/ui/web-api/src/routes/device_auth.rs:118` for start, and
  `:328-334` for approve) are removed in step 6 of the Migration
  plan.

### `crates/ui/cli`

`crates/ui/cli/src/commands/auth.rs`:

- Replace JSON request bodies with form-urlencoded
  (`reqwest::RequestBuilder::form(...)`).
- Switch all 400 response handling to deserialize `OAuthErrorResponse`.
  Match on `OAuthErrorCode`:
  - `AuthorizationPending` — sleep `interval`, loop.
  - `SlowDown` — set local interval to the server-supplied bumped
    `interval` (or `interval + 5` if server omitted it for forward
    compat with `Other(String)`), sleep, loop.
  - `AccessDenied` — bail with "Authorization denied by Operator."
  - `ExpiredToken` — bail with "Authorization request expired, please run again."
  - `InvalidGrant` / `InvalidClient` / `InvalidRequest` /
    `UnsupportedGrantType` — bail with a "CLI/server version mismatch:
    {error_code}" message (these indicate a build skew, not a user
    action).
  - `Other(s)` — bail with "Unexpected OAuth error: {s}".
- Delete `stream_device_auth` and the SSE branch entirely. The
  polling branch becomes the only path.
- Open the browser at `verification_uri_complete` when present, falling
  back to `verification_uri`.
- Continue printing `user_code` and the plain `verification_uri` to
  stderr regardless, so a manual fallback always exists.
- The URL-scheme validation (HTTPS / `--insecure` HTTP) survives
  unchanged.

### `frontend/src/routes/device/+page.svelte`

- Rename the query parameter from `?code=` to `?user_code=`. The frontend
  reads `$page.url.searchParams.get('user_code')`.
- Add a Deny button alongside Approve. Deny issues a
  `POST /api/v1/auth/device/deny` request with `{ user_code }`.
- On page load, fetch the lookup endpoint (`GET
/api/v1/auth/device/lookup?user_code=...`) and render "Approve
  sign-in from `{client_name}`?" when `client_name` is present in
  the response. This context is essential before an Operator
  approves; without it the page shows only the code itself.
- Wire the new endpoints via the same `fetch` helpers the existing
  page uses for `approve`. No new TypeScript client crate; the
  frontend does not consume `uptrakit-openapi-client`.

### `crates/shared/openapi-client`

The `uptrakit-openapi-client` crate is **hand-written**, not
code-generated (see `docs/development/openapi-client.md` §"Design
decisions" — "Hand-written instead of code-generated"). Update the
crate by editing the source files directly:

- `src/lib.rs`: add a new internal helper
  `post_form_unauth<T: DeserializeOwned, F: Serialize>(&self, path: &str, form: &F) -> Result<T>`
  alongside the existing `post_json_unauth` (line 352). It mirrors
  the JSON helper but uses `reqwest::RequestBuilder::form(form)` and
  sets `Content-Type: application/x-www-form-urlencoded`. The
  response-deserialisation branch must handle both success (200,
  JSON token response) and RFC error (400, JSON
  `OAuthErrorResponse`) — return `Err(ClientError::OAuthError(OAuthErrorResponse))`
  on 400, where `ClientError::OAuthError` is a new typed variant on
  the existing `ClientError` enum. This keeps the typed-client error
  path symmetric with the existing JSON helpers.
- `src/auth.rs`: delete the `device_auth_start` (line 53) and
  `device_auth_poll` (line 65) methods. Add new methods
  `oauth_device_authorization`, `oauth_token`,
  `oauth_authorization_server_metadata`, `device_auth_deny`,
  `device_auth_lookup`. `oauth_device_authorization` and
  `oauth_token` use the new `post_form_unauth` helper;
  `device_auth_deny` and `device_auth_lookup` use the existing JSON
  helpers (`post_json_no_content` and `get_unauth` respectively).
  Keep `device_auth_approve`. Update the request/response types
  imported from `uptrakit-web-api-types` to the new types defined in
  the web-api-types section.
- `src/device_auth_stream.rs`: delete the entire file (`stream_device_auth`
  is removed with the SSE endpoint).
- Update `src/lib.rs` module declarations to drop `device_auth_stream`.
- Update the existing `device_auth_*_request_serialization` tests in
  `src/auth.rs` to cover the new request shapes.

The `#[utoipa::path]` macros on the Axum handlers feed the OpenAPI
JSON document served at the API-docs endpoint (consumed by external
tooling and the API documentation page). No TypeScript client is
generated; the frontend calls the controller's HTTP API directly via
SvelteKit `fetch`. Frontend functions live in
`frontend/src/lib/api/` (or its current equivalent — verify at
implementation time) and use the new endpoint paths verbatim:
`POST /api/v1/auth/device/deny`, `GET /api/v1/auth/device/lookup`,
etc.

### `crates/shared/db/src/migration`

Add `m20260512_000001_device_flow_rfc8628.rs` per the Data Model
section above. Register it in `migration/mod.rs` in chronological
order.

## Migration plan

Single PR, single squash-merge. No feature flag — the change is
breaking and complete. Order of staging within the PR (each step is
self-contained so review is tractable):

1. **Migration & entity additions** — add `last_polled_at`,
   `interval`, `scope`, `denied_by` columns; widen `status` to
   include `denied`. Update `DeviceAuthStatus` test arrays in
   `crates/shared/types/src/device_auth_status.rs` to include
   `Denied`. Existing routes still pass tests because old code doesn't
   touch the new columns.
2. **New `OAuth*` types in `web-api-types`** — types and `Validate`
   impls. No routes wired yet. Unit tests for `OAuthErrorCode`
   `Other(String)` round-tripping pass.
3. **`device_flow.rs` additions** — `poll`, `deny`,
   `issue_access_token` seam, `validate_client_id` seam,
   `POLL_INTERVAL_BUMP`. Store-level tests pass.
4. **New routes** — `/oauth/device_authorization`, `/oauth/token`,
   metadata, `/auth/device/deny`, `/auth/device/lookup`. Route-level
   tests pass.
5. **Rate-limit map update** — swap the entries.
6. **Delete old routes & SSE plumbing** — `device_auth_start`,
   `device_auth_poll`, `device_auth_stream`, `device_flow_broadcaster`,
   deprecated types. Compilation forces every consumer to update.
7. **Update hand-written openapi-client** — add new methods to
   `src/auth.rs`, delete `src/device_auth_stream.rs`, drop the module
   reference from `src/lib.rs`, refresh the
   `*_request_serialization` tests. The crate is hand-written; no
   generator runs.
8. **CLI rewrite** — `commands/auth.rs` swaps to the new client.
9. **Frontend rewrite** — `?user_code=`, Deny button, client-name
   display via lookup.
10. **ADR `0009`** committed alongside the implementation.

No background data migration. No two-phase deploy. Anyone running an
older CLI build against a controller built from this PR sees a clear
error from step 8's CLI; rebuilding the CLI fixes it.

## Testing plan

Every test name below is the minimum coverage bar; the implementation
may add more.

### `crates/ui/web-api-auth/src/auth/device_flow.rs` (`mod tests`)

- `slow_down_when_polled_too_fast` — second poll with `now2 - now1 <
flow.interval` returns `PollOutcome::SlowDown`. Both `now` values
  are passed in as arguments to `poll(...)`; the test does not depend
  on wall-clock time or `tokio::time::advance`.
- `slow_down_returns_bumped_interval` — repeated `SlowDown` outcomes
  show the bumped interval persisted on the row.
- `deny_marks_flow_denied_and_sets_denied_by` — `deny` transitions
  `pending` → `denied` and records the deciding Operator on
  `denied_by` (with `user_id` left null).
- `poll_after_deny_returns_access_denied` — polling a `denied` flow
  returns `PollOutcome::Denied`.
- `last_polled_at_updates_on_each_poll` — successive polls (with
  injected monotonic `now`) write strictly-monotonic `last_polled_at`.
- `unknown_device_code_returns_expired_token` — `poll` on a hash with
  no matching row returns `PollOutcome::Unknown`.
- `malformed_device_code_returns_invalid_grant` — pure-non-base32
  garbage input is rejected before DB lookup.
- `concurrent_poll_does_not_double_consume` — two parallel polls of
  an authorized flow result in exactly one token mint. **New test**
  (the existing `consume`-level tests cover sequential double-consume
  but not concurrency); reuses the `BEGIN IMMEDIATE` transaction the
  new `poll` opens.
- `concurrent_approve_and_deny_resolves_atomically` — parallel calls
  to `approve` and `deny` on the same `pending` row result in exactly
  one terminal status; the loser returns the existing approval-failure
  error type.

Store-level tests that depend on cadence inject `now: DateTime<Utc>`
into `poll`. `deny` matches the existing `approve` signature (no
injected `now`); the concurrent-approve-and-deny test relies on the
atomic CAS, not on observing timestamps. None rely on
`#[tokio::test(start_paused = true)]` (CLAUDE.md explicitly
disallows that combination with SeaORM SQLite tests). Route-level
tests that need to drive `tokio::time::sleep` (none today; future
long-poll seam) will use the start_paused pattern.

### `crates/ui/web-api/src/routes/oauth/token.rs` (`mod tests`)

- `unsupported_grant_type_response` — `grant_type=client_credentials`
  returns HTTP 400 + `error: "unsupported_grant_type"`.
- `invalid_grant_when_device_code_unknown` — well-formed but unknown
  `device_code` returns HTTP 400 + `error: "expired_token"` (oracle
  hardening).
- `invalid_client_when_client_id_mismatches` — correct `device_code`
  but wrong `client_id` returns HTTP 400 + `error: "invalid_client"`.
- `invalid_request_when_missing_fields` — missing `device_code` returns
  400 + `invalid_request` with an `error_description`.
- `success_returns_bearer_token` — happy path returns 200 +
  `access_token`/`token_type: "Bearer"`. The response JSON has no
  `expires_in`, `refresh_token`, or `scope` keys (omitted, not null).
- `slow_down_400_with_interval` — second poll within `interval` returns
  400 + `slow_down` + `interval: <bumped>`.
- `authorization_pending_400` — pending status returns 400 +
  `authorization_pending`.
- `access_denied_400` — denied status returns 400 + `access_denied`.
- `expired_token_400` — expired status returns 400 + `expired_token`.
- `audit_records_slow_down_outcome` — `AUTH_DEVICE_POLL` with
  `details.slow_down = true` is emitted.
- `token_response_omits_optional_fields_on_wire` — JSON serialisation
  of `OAuthTokenResponse` with `expires_in = None` produces no key in
  the output (verifies the `skip_serializing_if` attribute).

### `crates/ui/web-api/src/routes/oauth/device_authorization.rs` (`mod tests`)

- `success_response_shape_matches_rfc` — JSON response has exactly
  `device_code`, `user_code`, `verification_uri`,
  `verification_uri_complete`, `expires_in`, `interval`.
- `client_id_mismatch_returns_invalid_client` — `client_id=wrong`
  returns 400 + `invalid_client`.
- `client_name_extension_field_persists_to_audit` — `client_name`
  reaches the `AUTH_DEVICE_START` audit `details`.
- `verification_uri_complete_contains_user_code` — string-suffix check.
- `external_base_url_resolution_unchanged` — both URIs honour
  `ExternalBaseUrl` → `Origin` → `Host` order.

### `crates/ui/web-api/src/routes/oauth/metadata.rs` (`mod tests`)

- `discovery_doc_lists_device_grant_endpoints` — full JSON shape match
  including `grant_types_supported`,
  `token_endpoint_auth_methods_supported: ["none"]`.
- `discovery_doc_no_auth_required` — request without bearer token
  returns 200.

### `crates/ui/web-api/src/routes/device_auth.rs` (`mod tests`)

Existing tests for `approve` survive unchanged. New tests:

- `deny_requires_permission` — unauthenticated and
  `lacks_view_services` callers get 401/403.
- `deny_emits_audit_event` — `AUTH_DEVICE_DENY` with
  `outcome: Success`.
- `deny_unknown_user_code_returns_not_found` — internal-UI 404 still
  applies (RFC-shape is only for OAuth endpoints).
- `lookup_returns_client_name_and_expiry`.
- `lookup_unknown_user_code_returns_404`.

### `crates/shared/web-api-types/src/oauth.rs` (`mod tests`)

- `oauth_error_code_unknown_deserializes_to_other` — a JSON
  `{"error": "temporarily_unavailable"}` deserializes to
  `OAuthErrorCode::Other("temporarily_unavailable".into())` without
  panicking. Mirrors `EnrollmentStatus`'s round-trip test.
- `oauth_error_code_known_variants_serialize_canonically` — every
  variant in `KNOWN_VARIANTS` round-trips.
- `validate_rejects_empty_client_id` — `Validate` impl returns Err.

### `frontend/tests` (Playwright)

- `/device?user_code=ABCD-EFGH` happy approval path (existing,
  parameter renamed).
- `/device?user_code=ABCD-EFGH` Deny path: Deny button visible, click
  issues `POST /api/v1/auth/device/deny`, page shows "Denied" state.
- `/device` with malformed `user_code` query param shows the same
  validation message the existing page shows.

### CLI integration tests

(`crates/ui/cli` already has `assert_cmd`-based tests against a mock
server. Where they exist, extend them; otherwise document the gap and
defer to manual verification.)

- `auth_command_handles_slow_down` — mock server returns `slow_down`
  twice then success; CLI completes.
- `auth_command_handles_access_denied` — CLI exits non-zero with the
  user-facing "denied by Operator" message.
- `auth_command_opens_verification_uri_complete_when_present`.

## Project conformance

Each item below is either satisfied by this spec or explicitly waived.

| Rule (source)                                                                             | Status    | Notes                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| ----------------------------------------------------------------------------------------- | --------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `#[non_exhaustive]` on public enums (CLAUDE.md "Code Standards")                          | Satisfied | `OAuthErrorCode` gets `#[non_exhaustive]` automatically from the `wire_safe_enum!` macro; existing `DeviceAuthStatus` is already annotated. `PollOutcome` is intentionally crate-private — the rule scopes to extensible public enums only.                                                                                                                                                                                                           |
| Wire-safe `Other(String)` catch-all (CLAUDE.md / `crates/shared/wire/src/lib.rs`)         | Satisfied | `OAuthErrorCode` is defined via the project-mandated `wire_safe_enum!` macro from `uptrakit_shared_macros` (per `docs/development/coding-standards.md` §"Wire-Safe Other(String) Catch-All — Required implementation"). The macro generates `#[non_exhaustive]`, `Other(String)`, `as_str`, `Display`, `From<String>` with `tracing::debug!`, infallible `Serialize`/`Deserialize`, and strict `FromStr`. No hand-written boilerplate, no `EnumIter`. |
| Typed enums for internal write-path discriminators                                        | Satisfied | New `PollOutcome` is internal, not wire — no `Other(String)`, no `#[non_exhaustive]` requirement.                                                                                                                                                                                                                                                                                                                                                     |
| `parking_lot::Mutex` for sync locks in async code                                         | n/a       | No new sync locks introduced.                                                                                                                                                                                                                                                                                                                                                                                                                         |
| `BEGIN IMMEDIATE` for read-then-write transactions (CLAUDE.md "SQLite Transaction Rules") | Satisfied | `device_flow::poll` opens a transaction with `SqliteTransactionMode::Immediate` and folds the consume path into step 4 of `poll`. The existing `consume` (which used a non-transactional read + conditional `delete_many`) is removed; folding it into `poll` closes the pre-existing `SQLITE_BUSY_SNAPSHOT` exposure as a side effect of this refactor.                                                                                              |
| `Validate` trait on all HTTP request types                                                | Satisfied | Every new request type implements `Validate`: `DeviceAuthorizationRequest`, `OAuthTokenRequest`, `DeviceAuthApproveRequest` (existing), `DeviceAuthDenyRequest` (new), and `DeviceAuthLookupQuery` (new). The lookup query's `validate()` enforces the `XXXX-XXXX` consonants pattern; the handler calls `req.validate()` before reading the DB. Failure surfaces as `invalid_request` on OAuth endpoints, existing `BadRequest` on UI endpoints.     |
| HTTP client SSRF guards                                                                   | n/a       | No new outgoing HTTP from this surface.                                                                                                                                                                                                                                                                                                                                                                                                               |
| `EncryptedString::plaintext_for_test` testing feature                                     | n/a       | No encrypted columns added.                                                                                                                                                                                                                                                                                                                                                                                                                           |
| `#[must_use]` on security-sensitive return values                                         | Satisfied | `issue_access_token` and `validate_client_id` both annotated.                                                                                                                                                                                                                                                                                                                                                                                         |
| `rootcause::Report` for errors, `report!()`/`bail!()`                                     | Satisfied | All new error paths use these. No new `.unwrap()` in production.                                                                                                                                                                                                                                                                                                                                                                                      |
| Tenant isolation via `TenantDb::find_via_tenant_join`                                     | n/a       | `pending_device_flows` is not tenant-scoped; the existing single-tenant assumption (`default_tenant_id`) carries forward.                                                                                                                                                                                                                                                                                                                             |
| Single-tenant audit emission                                                              | Satisfied | All new audit calls use `state.default_tenant_id` (matching the existing `device_auth.rs` pattern).                                                                                                                                                                                                                                                                                                                                                   |
| Quality gates (CLAUDE.md)                                                                 | Pinned    | All eight commands listed below are part of the implementation Definition of Done.                                                                                                                                                                                                                                                                                                                                                                    |
| Conventional Commits                                                                      | Pinned    | PR commits follow `feat(web-api): …` / `feat(cli): …` style.                                                                                                                                                                                                                                                                                                                                                                                          |

Quality-gate commands the implementation must pass:

```sh
cargo fmt --all
cargo check --no-default-features --features db-sqlite
cargo check --all-features
cargo clippy --all-targets --no-default-features --features db-sqlite
cargo clippy --all-targets --all-features
cargo test --all-features
cargo deny check
markdownlint --config .markdownlint.json '**/*.md'
cd frontend && npm run lint && npm run format:check && npm run check && npm run test && npm run build
```

## Future migrations (named seams)

Each future feature lands as a focused refactor at exactly one named
seam. The spec records both the seam and the file where it lives.

### Seam 1 — Token issuance (short-lived bearer + refresh)

**Location:** `crates/ui/web-api-auth/src/auth/device_flow.rs`,
function `issue_access_token(user_id: UserId) -> Result<SecretString, _>`.

Today this function mints an indefinite API token. A future migration
returns a `TokenPair { access_token, expires_in, refresh_token }`,
adjusts the `OAuthTokenResponse` shape (additive — fields are
currently omitted, not absent from the struct), and adds a
`refresh_token` grant arm to the `OAuthGrantHandler` dispatcher. No
other call site changes.

### Seam 2 — Scope enforcement

**Location:** `crates/ui/web-api-auth/src/auth/device_flow.rs`,
new function `apply_scope_to_token(token: &mut MintedToken, scope: Option<&str>)`.

Today this function is a no-op stub. A future migration parses the
`scope` string (space-separated per RFC 6749 §3.3), maps each scope
token to a `Permission` subset, and attaches the narrowed permission
set to the minted token. The token-permission storage layer is the
existing API-token model; only this stub function changes.

### Seam 3 — Client registry

**Location:** `crates/ui/web-api-auth/src/auth/device_flow.rs`,
function `validate_client_id(client_id: &str) -> Result<(), OAuthErrorCode>`
and the constant `CLIENT_ID: &str = "uptrakit-cli"`.

Today the function compares `client_id` to the constant and returns
`invalid_client` on mismatch. A future migration introduces an
`oauth_clients` table (admin-managed allowlist), the function gains a
DB lookup, and the constant is deleted. No route handler changes.

### Seam 4 — Long-poll on the token endpoint

**Location:** `crates/ui/web-api/src/routes/oauth/token.rs`,
device-code grant arm.

Today the handler returns immediately with the current poll outcome.
A future migration introduces an opt-in `wait` form parameter (capped
≤30s, below typical reverse-proxy idle timeouts). When `wait` is
present and the current outcome would be `authorization_pending`, the
handler awaits a `tokio::sync::Notify` keyed by `device_code` (the
broadcaster pattern already prototyped by `device_flow_broadcaster`
before deletion) up to the cap, then re-evaluates. RFC-compliant
clients that omit `wait` see the existing behaviour unchanged.

## Documentation deliverables

| Document                                                                      | Status                                                            | Justification                                                                                                                                                                                                                                                                                                               |
| ----------------------------------------------------------------------------- | ----------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `docs/superpowers/specs/2026-05-12-rfc8628-device-auth-design.md` (this file) | Non-optional                                                      | Owning design document.                                                                                                                                                                                                                                                                                                     |
| `docs/adr/0009-oauth-2-device-flow-rfc-compliance.md`                         | Non-optional                                                      | Hard-to-reverse decision (RFC wire shape + named seams), surprising-without-context (constant `client_id`, no refresh tokens, no scope enforcement, but full RFC 8414 discovery), real trade-off (strict RFC vs hybrid; minimum-viable token vs full auth-server build-out). ADR captures the rationale and the four seams. |
| OpenAPI documentation (`utoipa` annotations on the new routes)                | Non-optional (auto-generated)                                     | Regenerated `openapi-client` crate and any static API docs derived from it pick up the new shapes automatically. The implementation must include `#[utoipa::path]` macros on every new route.                                                                                                                               |
| `docs/development/cli-output.md`                                              | Update if affected                                                | Audit whether existing CLI auth user-facing strings are referenced. The new error-code → message map (Q19 above) is the canonical source.                                                                                                                                                                                   |
| `docs/development/openapi-client.md`                                          | Update if regeneration command needs new entry points or features | Spec implementation step 7 references this doc.                                                                                                                                                                                                                                                                             |
| `README.md` and other top-level docs                                          | Conditional                                                       | A grep across the repo for `verification_url`, `/api/v1/auth/device`, `?code=` returned zero matches in `README*.md` and `docs/development/*.md`. The implementation must re-grep at completion time and update any new matches that landed in the interim.                                                                 |
| `CONTEXT.md`                                                                  | Explicitly not updated                                            | RFC 8628 vocabulary (`device_code`, `user_code`, `verification_uri`, `client_id`, `scope`) is OAuth standard, not uptrakit-specific. The existing `CONTEXT.md` reservation of "device" for the OAuth CLI flow (line 17) continues to hold and accommodates the new RFC vocabulary as additive non-glossary terminology.     |
| Public type docstrings                                                        | Non-optional                                                      | Every new public type in `crates/shared/web-api-types/src/oauth.rs` and every new public function in `device_flow.rs` carries a Rustdoc comment citing the relevant RFC section (e.g. `/// Per RFC 8628 §3.2.`).                                                                                                            |

## Open questions

None. All design decisions were resolved during the grilling phase
(`grill-with-docs` skill session 2026-05-12). The implementation plan
that follows this spec will surface implementation-detail questions
that do not require further design input.

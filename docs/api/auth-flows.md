# Authentication Flows

## Device Authorization (CLI) - Overview

1. `POST /api/v1/auth/device` with optional `client_name` → returns `device_code`, `user_code`, `verification_url`, `expires_in` (600s), `interval`
   (5s).
1. CLI opens `verification_url` and displays `user_code` for the user.
1. User logs in via browser (password or OIDC) and approves the request (`POST /api/v1/auth/device/approve`).
1. CLI polls `POST /api/v1/auth/device/poll` every `interval` seconds until approval; response contains access token and refresh token.
1. On approval, the flow is removed from the DB to prevent reuse.

## Device Authorization (CLI) - Detailed Flow

The CLI uses an RFC 8628-style device authorization flow instead of password-based login. This allows the CLI to
authenticate even when password auth is disabled (OIDC-only environments).

### Flow

1. CLI calls `POST /api/v1/auth/device` with an optional `client_name`. Returns `device_code`, `user_code`,
   `verification_url`, `expires_in` (600s), and `interval` (5s).
1. CLI opens `verification_url` in the user's browser and displays the `user_code`.
1. User logs in via the browser (password or OIDC) and approves the device code at `/device?code=XXXX-XXXX`.
1. CLI polls `POST /api/v1/auth/device/poll` with the `device_code` every `interval` seconds.
1. On approval, the poll response contains an API token. The CLI stores it locally.

### Endpoints

| Endpoint | Auth | Purpose |
| --- | --- | --- |
| `POST /api/v1/auth/device` | Public | Start device flow, get device code + user code |
| `POST /api/v1/auth/device/poll` | Public | Poll for authorization status |
| `POST /api/v1/auth/device/approve` | Bearer (JWT or API token) | Approve a device code (browser-side) |

### Security

- **Device code**: 32-byte crypto random (base64url), unguessable.
- **User code**: 8 uppercase consonants from a 20-char alphabet (avoids vowels to prevent offensive words), ~34.5 bits
  entropy, formatted `XXXX-XXXX`.
- **Rate limiting**: all public auth endpoints (including device/poll) are rate-limited via the unified
  `api_rate_limits` DB table; see the "API rate limiting" section below.
- **URL scheme validation**: the CLI validates that the `verification_url` uses `https://` (or `http://` when
  `--insecure` is active) before opening it in the user's browser. This prevents a compromised server from triggering
  dangerous URL schemes (e.g., `file://`, `javascript:`).
- **One-time use**: consuming an authorized flow removes it atomically; a second poll gets 404.
- **10-minute expiry**: flows auto-expire; cleanup runs every 5 minutes alongside OIDC state cleanup.
- **Database-backed store**: all pending device flow state is persisted to the `pending_device_flows` table (shared with
  OIDC flow, account link, token exchange, and OIDC registration stores). Survives controller restarts and supports HA
  multi-instance deployments. Only the resulting API token is persisted to the `api_tokens` table.

## Access and Refresh Tokens

- Access tokens are short-lived and stored in memory only. They carry resolved permissions (`Vec<Permission>`) that the controller embeds when issuing
  the JWT.
- Refresh tokens are hashed (`sha256`) in the database, stored in `HttpOnly; Secure; SameSite=Strict` cookies, and rotated on every use, revoking the
  predecessor.
- Logout adds entries to the in-memory `TokenDenylist` to deny future requests for the remaining lifetime (15 min).

## API Tokens

- Long-lived, revocable tokens stored in `api_tokens` table.
- Treat them like passwords; never log them and avoid reusing them across services.

## OIDC Feature Availability

OIDC authentication is available only when the `oidc` Cargo feature is enabled on
`uptrakit-web-api` (default). Without it, OIDC routes (`/api/v1/auth/oidc/*`) are not
registered, OIDC OpenAPI schemas are omitted, and disabling password authentication returns
an error stating OIDC support is not available. Password-based and device authorization
flows remain fully functional regardless of the feature flag.

## Service Enrollment

- All service types (agents, SSH agents, MQTT) share a single enrollment token managed via
  `/api/v1/services/enrollment-token` (DB key `service_enrollment.token_hash`).
- Services send `Enroll` with their `capabilities: BTreeSet<Capability>` over the WebSocket.
  The controller persists capabilities in the `services.capabilities` column and derives the
  `ServiceProfile` (behavioral defaults) from them.
- All services follow the same CSR/mTLS issuance flow.

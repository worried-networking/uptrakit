# Authentication Flows

## Device Authorization (CLI)

1. `POST /api/v1/auth/device` with optional `client_name` → returns `device_code`, `user_code`, `verification_url`, `expires_in` (600s), `interval`
   (5s).
2. CLI opens `verification_url` and displays `user_code` for the user.
3. User logs in via browser (password or OIDC) and approves the request (`POST /api/v1/auth/device/approve`).
4. CLI polls `POST /api/v1/auth/device/poll` every `interval` seconds until approval; response contains access token and refresh token.
5. On approval, the flow is removed from the DB to prevent reuse.

## Access and Refresh Tokens

- Access tokens are short-lived and stored in memory only. They carry resolved permissions (`Vec<Permission>`) that the controller embeds when issuing
  the JWT.
- Refresh tokens are hashed (`sha256`) in the database, stored in `HttpOnly; Secure; SameSite=Strict` cookies, and rotated on every use, revoking the
  predecessor.
- Logout adds entries to the in-memory `TokenDenylist` to deny future requests for the remaining lifetime (15 min).

## API Tokens

- Long-lived, revocable tokens stored in `api_tokens` table.
- Treat them like passwords; never log them and avoid reusing them across services.

## MQTT Enrollment

- MQTT services enroll via `/api/v1/services/enrollment-token?type=mqtt`.
- Enrollment tokens live in settings (`mqtt_enrollment.token_hash`) and can expire or be limited by use count.
- MQTT services follow the same CSR/mTLS issuance flow as agents.

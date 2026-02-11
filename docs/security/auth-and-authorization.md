# Authentication and Authorization

| Method | Scope | Details |
| --- | --- | --- |
| Password (Argon2id) | User login | Local accounts with hashed passwords. |
| OIDC | User login | External identity providers with auto-create or account linking. |
| Device authorization | CLI login | RFC 8628-style flow: device code, browser approval, API token issuance. |
| JWT access tokens | API requests | Short-lived tokens that carry resolved permissions (never stored). |
| Refresh tokens | API requests | SHA-256 hashed, 7-day expiry, rotated on each use. |
| API tokens | Programmatic access | Long-lived, revocable bearer tokens stored in the database. |
| mTLS client certs | Agent/MQTT connections | Issued after CSR approval and validated per connection. |
| Forwarded cert headers | Reverse proxy | Trusted proxies forward cert info/PEM; issuer CN verified. |
| Enrollment tokens | Agent onboarding | One-time tokens with optional expiry/use limit. |
| MQTT enrollment tokens | MQTT service enrollment | Stored separately (`mqtt_enrollment.token_hash`). |

Authorization uses typed `Permission` enums instead of raw roles. JWTs resolve the user's permissions (e.g., `view_settings`, `manage_agents`), and route handlers call `user.has_permission(...)`. The frontend receives permissions as strings and relies on a TypeScript `Permission` enum.

Roles:

- `owner`: all permissions including global settings.
- `admin`: all except `manage_global_settings`.
- `user`: `view_agents` only.

Adding a new permission requires updating the Rust enum, adding a database migration, guarding the relevant route, and extending the frontend Permission enum.

# Code Review: `uptrakit-openapi-client`

**Date:** 2026-02-17
**Reviewer:** AI-assisted
**Scope:** Full crate review (`crates/shared/openapi-client/`)
**Overall quality:** HIGH

## Executive Summary

The crate provides a typed HTTP client around the Uptrakit web API with strong
error handling and solid type safety. No critical issues were found. Resiliency
improvements (pagination iterator and optional retry strategy) have been implemented.

## Code Quality Findings

### O-5 (Info): Raw response fallback is intentional and acceptable

**Location:** `crates/shared/openapi-client/src/lib.rs`

`raw_request` falls back from JSON parsing to a string JSON value for non-JSON
responses. This is a safe display fallback.

## Extensibility Findings

### Clean dependency chain

**Dependencies:** `uptrakit-web-api-types`, `uptrakit-shared-types`, `uptrakit-shared-macros`,
`reqwest`, `rootcause`, `thiserror`, `serde`, `serde_json`, `uuid`.

No server, database, or wire protocol dependencies. This is exactly right for an API client
library.

### Full API coverage

18 endpoint modules covering all major API domains:

| Module | Coverage |
| --- | --- |
| `auth` | Register, login, logout, refresh, device auth, user info |
| `services` | List, get, approve, reject, merge, enrollment tokens |
| `software_items` | CRUD, assign hosts, trigger update, check versions |
| `hosts` | List, get, update, deactivate |
| `provider_configs` | CRUD operations |
| `settings` | Combined, registration, auth, agent certs, network, CA rotation, cert renewal |
| `settings_mqtt` | MQTT client CRUD, limit get/update |
| `scheduler` | List, get, update, trigger |
| `update_history` | List, get |
| `api_tokens` | Create, list, revoke |
| `oidc_auth` | Auth methods, authorize, callback, link, exchange, complete registration |
| `oidc_providers` | CRUD, activate/deactivate |
| `pki` | Get CA cert, get CRL |
| `health` | Health check |
| `system_alerts` | Get alerts |

### Good re-export strategy

**Location:** `src/lib.rs:22-38`

```rust
pub use uptrakit_web_api_types as types;
pub use uptrakit_shared_types::DeviceAuthStatus;
pub use uptrakit_shared_types::ServiceType;
pub use uuid::Uuid;
pub use reqwest::Error as ReqwestError;
```

External developers get a single dependency (`uptrakit-openapi-client`) with access to all
request/response types via `client::types::*`. No need to add `uuid` or `reqwest` as direct
dependencies.

### Missing: batch operations

All operations are single-item. No bulk update, bulk delete, or batch check endpoints.

## Strengths

- Consistent typed API surface and endpoint coverage.
- Entity ID parameters use `&Uuid`.
- Proper typed errors with `thiserror` + `rootcause` context propagation.
- No `unsafe`, no `panic!`, and no production `unwrap()` usage.
- Clean module split and re-exports for downstream ergonomics.
- **Discriminated error enum** (`ClientError`) with specific variants for rate limiting, not
  found, auth errors, and API errors -- enables precise error handling by consumers.
- **`raw_request` escape hatch** for unmapped or custom endpoints.
- **Device flow support** for CLI/headless authentication.
- **Insecure mode** for development (TLS verification bypass).
- **`list_all_*` pagination helpers**: `list_all_hosts`, `list_all_services`, `list_all_software_items`,
  `list_all_provider_configs`, `list_all_update_history` automatically iterate all pages using
  `MAX_PER_PAGE = 1000` per request.
- **Optional retry strategy** (`RetryConfig` + `with_retry`): exponential backoff for 5xx, respects
  `Retry-After` for 429, no retry for 4xx or auth failures. Opt-in, disabled by default.

## Mock Testing Feature

A `mock` feature flag was added to support integration testing of crates that use
`uptrakit-openapi-client`:

```toml
[dev-dependencies]
uptrakit-openapi-client = { path = "...", features = ["mock"] }
```

The `mock` module (`src/mock.rs`) provides:

- `MockApiServer`: starts an `httpmock::MockServer` on a random port, with endpoint-aware
  convenience methods (`on_list_hosts`, `on_get_host`, `on_approve_service`, etc.) so test
  code never needs to know API URL paths.
- `MockEndpoint<'a>`: a response builder with pre-defined methods (`ok`, `no_content`,
  `unauthorized`, `not_found`, `rate_limited`, `internal_error`) using `reqwest::StatusCode`
  for type safety, plus `respond` and `respond_raw` for custom responses.
- All response builder methods use `StatusCode` enum values (not raw `u16` literals).

The CLI crate (`uptrakit-cli`) uses this feature in 14 integration tests covering command
execution, error mapping, and output formatting.

## AGENTS.md Compliance Check

- No `unsafe`: pass
- No `#[allow(...)]`: pass
- Typed error boundary + context propagation: pass
- Entity IDs as `&Uuid`: pass
- API coverage parity with web-api (excluding documented exceptions): pass

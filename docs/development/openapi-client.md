# OpenAPI Client

The `uptrakit-openapi-client` crate (`crates/shared/openapi-client/`) provides a typed HTTP client for the Uptrakit web API. It centralises URL construction, authentication, error handling, and JSON serialization behind compile-time type-safe methods.

## Motivation

The CLI previously used a hand-rolled `ApiClient` that made raw `reqwest` calls with string-based URL construction, manual query parameter assembly, and `serde_json::Value`-based request/response handling. This was fragile: URL typos, missing query parameters, or type mismatches were caught only at runtime.

The typed client provides:

- Compile-time type safety for all API endpoints
- Centralised URL construction and authentication
- Type-safe query parameter serialization via `reqwest::RequestBuilder::query()`
- Consistent error handling with typed `ClientError` variants
- Clean separation between API communication and presentation logic

## Design decisions

**Hand-written instead of code-generated:** `uptrakit-web-api-types` already provides all request/response types. A code generator would duplicate them and generate code that doesn't follow the project's strict coding standards (rootcause errors, no unwrap, no `#[allow]`).

**Re-exports all types:** The crate re-exports `uptrakit-web-api-types` as `types` and `DeviceAuthStatus` from `uptrakit-shared-types`. Downstream crates (e.g. the CLI) depend only on `uptrakit-openapi-client` and import types via `uptrakit_openapi_client::types::*`.

## Crate structure

```text
crates/shared/openapi-client/
├── Cargo.toml
└── src/
    ├── lib.rs              # UptrakitClient struct, builder, re-exports, internal helpers
    ├── error.rs            # ClientError enum, Result type
    ├── auth.rs             # Device auth + user info endpoints
    ├── api_tokens.rs       # API token CRUD
    ├── hosts.rs            # Host list/get
    ├── software_items.rs   # Software item list/get/check/update
    ├── update_history.rs   # Update history list/get
    └── scheduler.rs        # Scheduler task list/get/trigger
```

## Client usage

### Creating a client

```rust
use uptrakit_openapi_client::UptrakitClient;

// Unauthenticated (for device auth flow)
let client = UptrakitClient::new("https://example.com", None, false)?;

// Authenticated
let client = UptrakitClient::with_token("https://example.com", "tok-abc", false)?;
```

The `insecure` parameter disables TLS certificate verification (development only).

### Typed endpoint methods

Each endpoint module implements methods on `UptrakitClient`:

```rust
use uptrakit_openapi_client::types::pagination::PaginationParams;

let params = PaginationParams { page: Some(1), per_page: Some(20) };
let resp = client.list_hosts(&params).await?;
// resp is PaginatedResponse<HostResponse> — fully typed
```

### Raw request escape hatch

For the CLI `api` command that allows arbitrary API calls:

```rust
let resp = client.raw_request("GET", "/api/v1/some/path", None).await?;
// resp.status: u16, resp.body: serde_json::Value
```

## Error handling

The `ClientError` enum covers all failure modes:

| Variant | Meaning |
| --- | --- |
| `Http(reqwest::Error)` | Network/transport error |
| `Json(serde_json::Error)` | JSON serialization/deserialization error |
| `Api { status, message }` | Server returned an error response (4xx/5xx) |
| `RateLimited` | Server returned HTTP 429 |
| `NotFound(String)` | Server returned HTTP 404 |
| `NotAuthenticated` | No bearer token available |
| `InvalidMethod(String)` | Invalid HTTP method string (raw request only) |

All methods return `Result<T>` which is `std::result::Result<T, rootcause::Report<ClientError>>`. The CLI maps these to `CliError` variants via `impl_report_conversion!`.

## Query parameter handling

Type-safe query parameter serialization uses `reqwest::RequestBuilder::query()`. Since `PaginationParams`, `UpdateHistoryQuery`, etc. already implement `Serialize`, they are passed directly. `Option::None` fields are automatically skipped by `serde_urlencoded`.

## Available endpoint methods

### Auth (`auth.rs`)

- `device_auth_start(&self, req) -> Result<DeviceAuthStartResponse>` -- unauthenticated
- `device_auth_poll(&self, req) -> Result<DeviceAuthPollResponse>` -- unauthenticated, returns `ClientError::RateLimited` on 429
- `me(&self) -> Result<UserResponse>`

### API tokens (`api_tokens.rs`)

- `create_api_token(&self, req) -> Result<CreateApiTokenResponse>`
- `list_api_tokens(&self) -> Result<ApiTokenListResponse>`
- `revoke_api_token(&self, id) -> Result<()>`

### Hosts (`hosts.rs`)

- `list_hosts(&self, params) -> Result<PaginatedResponse<HostResponse>>`
- `get_host(&self, id) -> Result<HostResponse>`

### Software items (`software_items.rs`)

- `list_software_items(&self, params) -> Result<PaginatedResponse<SoftwareItemResponse>>`
- `get_software_item(&self, id) -> Result<SoftwareItemDetailResponse>`
- `check_versions(&self, item_id) -> Result<TriggerVersionCheckResponse>`
- `check_versions_host(&self, item_id, host_id) -> Result<TriggerVersionCheckResponse>`
- `trigger_update(&self, item_id, host_id, req) -> Result<TriggerUpdateResponse>`

### Update history (`update_history.rs`)

- `list_update_history(&self, query) -> Result<PaginatedResponse<UpdateHistoryResponse>>`
- `get_update_history(&self, id) -> Result<UpdateHistoryResponse>`

### Scheduler (`scheduler.rs`)

- `list_scheduled_tasks(&self) -> Result<Vec<ScheduledTaskResponse>>`
- `get_scheduled_task(&self, id) -> Result<ScheduledTaskResponse>`
- `trigger_scheduled_task(&self, id) -> Result<TriggerScheduledTaskResponse>`

## Adding a new endpoint

1. Identify the request/response types in `uptrakit-web-api-types` (or add them if new).
2. Add a method to `UptrakitClient` in the appropriate module file.
3. Use the internal helpers (`get`, `get_with_query`, `post_json`, `post_empty`, `delete`).
4. Add a unit test for request serialization if the endpoint takes a request body.
5. Update the CLI command to use the new typed method.

## Testing

Unit tests cover:

- URL construction and base URL trailing slash trimming
- Token storage and authentication error handling
- Error message extraction from JSON and plain-text responses
- Request body serialization for all request types
- Query parameter serialization (with and without optional fields)
- `RawResponse` serialization

Run tests:

```bash
cargo test -p uptrakit-openapi-client
```

## Related documentation

- [HTTP Web API](../api/http-web-api.md) -- API endpoint reference
- [CLI Output](cli-output.md) -- CLI output formatting conventions
- [CLI Usage](../end-user/cli-usage.md) -- end-user CLI guide
- [Coding Standards](coding-standards.md) -- error handling and quality requirements

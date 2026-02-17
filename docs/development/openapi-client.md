# OpenAPI Client

The `uptrakit-openapi-client` crate (`crates/shared/openapi-client/`) provides a
typed HTTP client for the Uptrakit web API. It centralises URL construction,
authentication, error handling, and JSON serialization behind compile-time
type-safe methods.

## Motivation

The CLI previously used a hand-rolled `ApiClient` that made raw `reqwest` calls
with string-based URL construction, manual query parameter assembly, and
`serde_json::Value`-based request/response handling. This was fragile: URL typos,
missing query parameters, or type mismatches were caught only at runtime.

The typed client provides:

- Compile-time type safety for all API endpoints
- Centralised URL construction and authentication
- Type-safe query parameter serialization via `reqwest::RequestBuilder::query()`
- Consistent error handling with typed `ClientError` variants
- Clean separation between API communication and presentation logic

## Design decisions

**Hand-written instead of code-generated:** `uptrakit-web-api-types` already
provides all request/response types. A code generator would duplicate them and
generate code that doesn't follow the project's strict coding standards
(rootcause errors, no unwrap, no `#[allow]`).

**Re-exports all types:** The crate re-exports `uptrakit-web-api-types` as
`types`, `DeviceAuthStatus` and `ServiceType` from `uptrakit-shared-types`, and
`uuid::Uuid`. Downstream crates (e.g. the CLI) depend only on
`uptrakit-openapi-client` and import types via
`uptrakit_openapi_client::types::*` and `uptrakit_openapi_client::Uuid`.

**Full API coverage:** The client covers all JSON REST endpoints exposed by the
web API. Excluded endpoints are WebSocket (`/api/v1/ws/service`), OIDC browser
callback (`/api/v1/auth/oidc/callback`), and OCSP binary protocol endpoints
(`/api/v1/pki/ocsp`).

## Crate structure

```text
crates/shared/openapi-client/
├── Cargo.toml
└── src/
    ├── lib.rs              # UptrakitClient struct, builder, re-exports, internal helpers
    ├── error.rs            # ClientError enum, Result type
    ├── auth.rs             # Auth (register, login, refresh, logout, device auth, auth methods)
    ├── api_tokens.rs       # API token CRUD
    ├── health.rs           # Health check endpoint
    ├── hosts.rs            # Host list/get/update/deactivate
    ├── oidc_auth.rs        # OIDC auth flows (authorize, exchange, link, complete registration)
    ├── oidc_providers.rs   # OIDC provider CRUD + activate/deactivate
    ├── pki.rs              # PKI endpoints (CA cert, CRL download)
    ├── provider_configs.rs # Provider configuration CRUD
    ├── scheduler.rs        # Scheduler task list/get/update/trigger
    ├── services.rs         # Service list/get/approve/reject/remove/merge + enrollment tokens
    ├── settings.rs         # Settings (registration, auth, certs, network, CA, server cert)
    ├── settings_mqtt.rs    # MQTT client settings CRUD + limit management
    ├── software_items.rs   # Software item CRUD + host assignment + version check/update
    ├── system_alerts.rs    # System alerts
    └── update_history.rs   # Update history list/get
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

Each endpoint module implements methods on `UptrakitClient`. All ID parameters
use `&Uuid` instead of `&str`, providing compile-time validation:

```rust
use uptrakit_openapi_client::{UptrakitClient, Uuid};
use uptrakit_openapi_client::types::pagination::PaginationParams;

let params = PaginationParams { page: Some(1), per_page: Some(20) };
let resp = client.list_hosts(&params).await?;
// resp is PaginatedResponse<HostResponse> — fully typed

let host_id: Uuid = "019...".parse().expect("valid UUID");
let host = client.get_host(&host_id).await?;
// host.id is Uuid, not String
```

### Raw request escape hatch

For the CLI `api` command that allows arbitrary API calls:

```rust
let resp = client.raw_request("GET", "/api/v1/some/path", None).await?;
// resp.status: u16, resp.body: serde_json::Value
```

## UUID type safety

All entity ID parameters across the client use `&Uuid` rather than `&str`.
This enforces valid UUIDs at compile time and eliminates runtime parsing errors
from invalid ID strings.

**Re-exported type:** `uptrakit_openapi_client::Uuid` is a re-export of
`uuid::Uuid`. Downstream crates (e.g. the CLI) do not need a direct dependency
on the `uuid` crate — they import `Uuid` from the openapi-client.

**Response types:** All entity ID fields in `uptrakit-web-api-types` response
structs (e.g. `HostResponse::id`, `ServiceResponse::id`,
`SoftwareItemResponse::id`) are `Uuid`, not `String`. The only exception is
`SystemAlert::id`, which uses hardcoded string identifiers (not database UUIDs).

**Feature gating:** The `uuid` dependency in both `openapi-client` and
`web-api-types` uses only the `serde` feature — these crates never generate
UUIDs (the `v7` feature is only needed by crates that create new entities).

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

HTTP status code checks use `reqwest::StatusCode` constants and helper methods
(`is_client_error()`, `is_server_error()`) rather than raw integer comparisons.

## Query parameter handling

Type-safe query parameter serialization uses
`reqwest::RequestBuilder::query()`. Since `PaginationParams`,
`UpdateHistoryQuery`, etc. already implement `Serialize`, they are passed
directly. `Option::None` fields are automatically skipped by
`serde_urlencoded`.

## Available endpoint methods

### Auth (`auth.rs`)

- `register(&self, req) -> Result<AuthResponse>` -- unauthenticated
- `login(&self, req) -> Result<AuthResponse>` -- unauthenticated
- `refresh(&self, req) -> Result<RefreshResponse>` -- unauthenticated
- `logout(&self, req) -> Result<()>`
- `auth_methods(&self) -> Result<AuthMethodsResponse>` -- unauthenticated
- `device_auth_start(&self, req) -> Result<DeviceAuthStartResponse>` -- unauthenticated
- `device_auth_poll(&self, req) -> Result<DeviceAuthPollResponse>` -- unauthenticated, returns `ClientError::RateLimited` on 429
- `device_auth_approve(&self, req) -> Result<DeviceAuthApproveResponse>`
- `me(&self) -> Result<UserResponse>`

### API tokens (`api_tokens.rs`)

- `create_api_token(&self, req) -> Result<CreateApiTokenResponse>`
- `list_api_tokens(&self) -> Result<ApiTokenListResponse>`
- `revoke_api_token(&self, id: &Uuid) -> Result<()>`

### Health (`health.rs`)

- `healthz(&self) -> Result<String>` -- unauthenticated, returns `"ok"`

### Hosts (`hosts.rs`)

- `list_hosts(&self, params) -> Result<PaginatedResponse<HostResponse>>`
- `get_host(&self, id: &Uuid) -> Result<HostResponse>`
- `update_host(&self, id: &Uuid, req) -> Result<HostResponse>`
- `deactivate_host(&self, id: &Uuid) -> Result<HostMessageResponse>`

### OIDC auth (`oidc_auth.rs`)

- `oidc_authorize(&self, provider_id: &Uuid) -> Result<OidcAuthorizeResponse>` -- unauthenticated
- `oidc_exchange(&self, req) -> Result<AuthResponse>` -- unauthenticated
- `oidc_link(&self, req) -> Result<AuthResponse>`
- `oidc_complete_registration(&self, req) -> Result<AuthResponse>` -- unauthenticated

### OIDC providers (`oidc_providers.rs`)

- `create_oidc_provider(&self, req) -> Result<OidcProviderResponse>`
- `list_oidc_providers(&self) -> Result<Vec<OidcProviderResponse>>`
- `get_oidc_provider(&self, id: &Uuid) -> Result<OidcProviderResponse>`
- `update_oidc_provider(&self, id: &Uuid, req) -> Result<OidcProviderResponse>`
- `delete_oidc_provider(&self, id: &Uuid) -> Result<()>`
- `activate_oidc_provider(&self, id: &Uuid) -> Result<OidcProviderResponse>`
- `deactivate_oidc_provider(&self, id: &Uuid) -> Result<OidcProviderResponse>`

### PKI (`pki.rs`)

- `ca_cert(&self) -> Result<String>` -- unauthenticated, returns PEM
- `ca_crl(&self) -> Result<String>` -- unauthenticated, returns PEM

### Provider configs (`provider_configs.rs`)

- `create_provider_config(&self, req) -> Result<ProviderConfigResponse>`
- `list_provider_configs(&self, params) -> Result<PaginatedResponse<ProviderConfigResponse>>`
- `get_provider_config(&self, id: &Uuid) -> Result<ProviderConfigResponse>`
- `update_provider_config(&self, id: &Uuid, req) -> Result<ProviderConfigResponse>`
- `delete_provider_config(&self, id: &Uuid) -> Result<()>`

### Scheduler (`scheduler.rs`)

- `list_scheduled_tasks(&self) -> Result<Vec<ScheduledTaskResponse>>`
- `get_scheduled_task(&self, id: &Uuid) -> Result<ScheduledTaskResponse>`
- `update_scheduled_task(&self, id: &Uuid, req) -> Result<ScheduledTaskResponse>`
- `trigger_scheduled_task(&self, id: &Uuid) -> Result<TriggerScheduledTaskResponse>`

### Services (`services.rs`)

- `list_services(&self, query) -> Result<PaginatedResponse<ServiceResponse>>`
- `get_service(&self, id: &Uuid) -> Result<ServiceResponse>`
- `approve_service(&self, id: &Uuid) -> Result<ServiceResponse>`
- `reject_service(&self, id: &Uuid) -> Result<ServiceResponse>`
- `remove_service(&self, id: &Uuid) -> Result<MessageResponse>`
- `merge_service(&self, target_id: &Uuid, req) -> Result<ServiceResponse>`
- `create_enrollment_token(&self, service_type) -> Result<EnrollmentTokenResponse>`
- `revoke_enrollment_token(&self, service_type) -> Result<()>`
- `enrollment_token_status(&self, service_type) -> Result<EnrollmentTokenStatusResponse>`

### Settings (`settings.rs`)

- `get_combined_settings(&self) -> Result<CombinedSettingsResponse>`
- `get_registration_settings(&self) -> Result<RegistrationSettingsResponse>`
- `update_registration_settings(&self, req) -> Result<RegistrationSettingsResponse>`
- `get_authentication_settings(&self) -> Result<AuthenticationSettingsResponse>`
- `update_authentication_settings(&self, req) -> Result<AuthenticationSettingsResponse>`
- `get_agent_certificate_settings(&self) -> Result<AgentCertificateSettingsResponse>`
- `update_agent_certificate_settings(&self, req) -> Result<AgentCertificateSettingsResponse>`
- `get_network_settings(&self) -> Result<NetworkSettingsResponse>`
- `update_network_settings(&self, req) -> Result<NetworkSettingsResponse>`
- `rotate_ca(&self) -> Result<RotateCaResponse>`
- `renew_server_certificate(&self) -> Result<RenewServerCertResponse>`

### Settings MQTT (`settings_mqtt.rs`)

- `list_mqtt_settings(&self) -> Result<Vec<MqttClientResponse>>`
- `create_mqtt_settings(&self, req) -> Result<MqttClientResponse>`
- `get_mqtt_limit(&self) -> Result<MqttLimitResponse>`
- `update_mqtt_limit(&self, req) -> Result<MqttLimitResponse>`
- `get_mqtt_settings(&self, id: &Uuid) -> Result<MqttClientResponse>`
- `update_mqtt_settings(&self, id: &Uuid, req) -> Result<MqttClientResponse>`
- `delete_mqtt_settings(&self, id: &Uuid) -> Result<()>`

### Software items (`software_items.rs`)

- `list_software_items(&self, params) -> Result<PaginatedResponse<SoftwareItemResponse>>`
- `get_software_item(&self, id: &Uuid) -> Result<SoftwareItemDetailResponse>`
- `create_software_item(&self, req) -> Result<SoftwareItemResponse>`
- `update_software_item(&self, id: &Uuid, req) -> Result<SoftwareItemResponse>`
- `delete_software_item(&self, id: &Uuid) -> Result<()>`
- `assign_hosts(&self, id: &Uuid, req) -> Result<SoftwareItemDetailResponse>`
- `unassign_host(&self, item_id: &Uuid, host_id: &Uuid) -> Result<()>`
- `check_versions(&self, item_id: &Uuid) -> Result<TriggerVersionCheckResponse>`
- `check_versions_host(&self, item_id: &Uuid, host_id: &Uuid) -> Result<TriggerVersionCheckResponse>`
- `trigger_update(&self, item_id: &Uuid, host_id: &Uuid, req) -> Result<TriggerUpdateResponse>`

### System alerts (`system_alerts.rs`)

- `get_system_alerts(&self) -> Result<SystemAlertsResponse>`

### Update history (`update_history.rs`)

- `list_update_history(&self, query) -> Result<PaginatedResponse<UpdateHistoryResponse>>`
- `get_update_history(&self, id: &Uuid) -> Result<UpdateHistoryResponse>`

## Internal helpers

The `UptrakitClient` provides these internal HTTP methods used by endpoint modules:

| Helper | Auth | Description |
| --- | --- | --- |
| `get<T>(path)` | yes | GET with JSON response |
| `get_with_query<T>(path, query)` | yes | GET with query params |
| `get_unauth<T>(path)` | no | GET without auth |
| `get_text_unauth(path)` | no | GET returning raw text (PKI, health) |
| `post_json<T>(path, body)` | yes | POST with JSON body |
| `post_empty<T>(path)` | yes | POST without body |
| `post_empty_with_query<T>(path, query)` | yes | POST with query params, no body |
| `post_json_unauth<T>(path, body)` | no | POST without auth |
| `post_json_no_content(path, body)` | yes | POST expecting 204 No Content |
| `put_json<T>(path, body)` | yes | PUT with JSON body |
| `delete(path)` | yes | DELETE expecting empty response |
| `delete_json<T>(path)` | yes | DELETE with JSON response |
| `delete_with_query(path, query)` | yes | DELETE with query params |

## Adding a new endpoint

1. Identify the request/response types in `uptrakit-web-api-types` (or add them if new).
   - All entity ID fields in response types must be `Uuid`, not `String` (except `SystemAlert::id`).
2. Add a method to `UptrakitClient` in the appropriate module file.
   - All ID parameters must use `&Uuid`, not `&str`.
3. Use the internal helpers listed above.
4. Add a unit test for request serialization if the endpoint takes a request body or query params.
5. Update the CLI command to use the new typed method.
6. Update this documentation to list the new method.

## Keeping the client in sync

The openapi-client must mirror the web API:

- **New endpoint** in `web-api` -> add a corresponding client method.
- **Changed request/response types** -> update the client method signature.
- **Removed endpoint** -> remove the client method.

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

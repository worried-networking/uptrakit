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
`types`, `DeviceAuthStatus` from `uptrakit-shared-types`, `uuid::Uuid`,
`reqwest::Error` as `ReqwestError`, and `reqwest::StatusCode`. Downstream
crates (e.g. the CLI) depend only on `uptrakit-openapi-client` and import
types via `uptrakit_openapi_client::types::*`,
`uptrakit_openapi_client::Uuid`, `uptrakit_openapi_client::StatusCode`, and
`uptrakit_openapi_client::ReqwestError`.

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
    ├── mock.rs             # (feature = "mock") MockApiServer + MockEndpoint for testing
    ├── auth.rs             # Auth (register, login, refresh, logout, device auth, auth methods)
    ├── api_tokens.rs       # API token CRUD
    ├── health.rs           # Health check endpoint
    ├── hosts.rs            # Host list/get/update/deactivate
    ├── oidc_auth.rs        # OIDC auth flows (authorize, exchange, link, complete registration)
    ├── oidc_providers.rs   # OIDC provider CRUD + activate/deactivate
    ├── pki.rs              # PKI endpoints (CA cert, CRL download)
    ├── plugin_configs.rs   # Plugin configuration CRUD
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
// resp.status: reqwest::StatusCode, resp.body: serde_json::Value
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
| `Api { status: StatusCode, message }` | Server returned an error response (4xx/5xx); `status` is `reqwest::StatusCode` |
| `RateLimited { retry_after_seconds }` | Server returned HTTP 429; `retry_after_seconds: Option<u64>` parsed from the `Retry-After` header (seconds format) |
| `NotFound(String)` | Server returned HTTP 404 |
| `NotAuthenticated` | Server returned HTTP 401, or no bearer token available |
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

### Plugin configs (`plugin_configs.rs`)

- `create_plugin_config(&self, req) -> Result<PluginConfigResponse>`
- `list_plugin_configs(&self, params) -> Result<PaginatedResponse<PluginConfigResponse>>`
- `get_plugin_config(&self, id: &Uuid) -> Result<PluginConfigResponse>`
- `update_plugin_config(&self, id: &Uuid, req) -> Result<PluginConfigResponse>`
- `delete_plugin_config(&self, id: &Uuid) -> Result<()>`

### Scheduler (`scheduler.rs`)

- `list_scheduled_tasks(&self) -> Result<Vec<ScheduledTaskResponse>>`
- `get_scheduled_task(&self, id: &Uuid) -> Result<ScheduledTaskResponse>`
- `update_scheduled_task(&self, id: &Uuid, req) -> Result<ScheduledTaskResponse>`
- `trigger_scheduled_task(&self, id: &Uuid) -> Result<TriggerScheduledTaskResponse>`

### Services (`services.rs`)

- `list_services(&self, query) -> Result<PaginatedResponse<ServiceResponse>>`
- `get_service(&self, id: &Uuid) -> Result<ServiceResponse>`
- `update_service(&self, id: &Uuid, req) -> Result<ServiceResponse>` -- update configurable settings (e.g. ping interval)
- `approve_service(&self, id: &Uuid) -> Result<ServiceResponse>`
- `reject_service(&self, id: &Uuid) -> Result<ServiceResponse>`
- `remove_service(&self, id: &Uuid) -> Result<MessageResponse>`
- `merge_service(&self, target_id: &Uuid, req) -> Result<ServiceResponse>`
- `create_enrollment_token(&self) -> Result<EnrollmentTokenResponse>`
- `revoke_enrollment_token(&self) -> Result<()>`
- `enrollment_token_status(&self) -> Result<EnrollmentTokenStatusResponse>`

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

## Path constants (`paths` module)

All URL strings used by the client and the mock are defined exactly once in
`src/paths.rs` as `pub(crate)` sub-modules — one per resource group. Each
sub-module holds:

- `pub(crate) const NAME: &str` — static paths with no runtime parameters.
- `pub(crate) fn name(id: &Uuid) -> String` — paths that embed one or more
  IDs resolved at call time.

When an API path changes, update `paths.rs`. The compiler will catch every
stale reference in both the client methods and the mock helpers.

Sub-modules: `auth`, `api_tokens`, `health`, `hosts`, `oidc_auth`,
`oidc_providers`, `pki`, `plugin_configs`, `scheduler`, `services`,
`settings`, `settings_mqtt`, `software_items`, `system_alerts`,
`update_history`.

## Mock testing feature

The `mock` feature provides a first-class HTTP mocking layer for integration tests in crates
that depend on `uptrakit-openapi-client`. It uses [`httpmock`](https://crates.io/crates/httpmock)
under the hood.

### Enabling the feature

Add to `[dev-dependencies]` in your crate's `Cargo.toml`:

```toml
[dev-dependencies]
uptrakit-openapi-client = { path = "...", features = ["mock"] }
```

### Using `MockApiServer`

`MockApiServer` starts a real HTTP server on a random port. Endpoints are
accessed through typed **section accessors** that mirror the client module
structure. Test code never needs to know API URL paths.

```rust
use uptrakit_openapi_client::mock::MockApiServer;
use uptrakit_openapi_client::types::pagination::PaginatedResponse;
use uptrakit_openapi_client::types::hosts::HostResponse;

#[tokio::test]
async fn list_hosts_returns_empty() {
    let server = MockApiServer::start();

    // Register a mock through the typed section accessor
    let _m = server.hosts().on_list().ok(&PaginatedResponse::<HostResponse>::default());

    // Get a pre-authenticated client pointing at the mock server
    let client = server.client();
    let result = client.list_hosts(&Default::default()).await.unwrap();
    assert_eq!(result.items.len(), 0);
    // _m goes out of scope here; httpmock checks for unexpected requests on drop
}
```

### `MockEndpoint` response methods

All response methods use `reqwest::StatusCode` for type safety:

| Method | Status | Notes |
| --- | --- | --- |
| `ok(body)` | 200 OK | Serializes `body` as JSON |
| `no_content()` | 204 No Content | No response body |
| `unauthorized()` | 401 Unauthorized | JSON `{"error":"Unauthorized"}` |
| `not_found(msg)` | 404 Not Found | JSON `{"error":"<msg>"}` |
| `rate_limited(secs)` | 429 Too Many Requests | Optional `Retry-After` header |
| `internal_error(msg)` | 500 Internal Server Error | JSON `{"error":"<msg>"}` |
| `respond(status, body)` | custom | Serializes `body` as JSON |
| `respond_raw(status, json)` | custom | Raw JSON string |

All methods return `httpmock::Mock<'_>` which can be used for call-count assertions:

```rust
let m = server.hosts().on_list().ok(&response);
// ... exercise code under test ...
m.assert();       // exactly 1 call
m.assert_hits(2); // exactly N calls
```

### Section accessors and endpoint helpers

`MockApiServer` exposes a typed section accessor per resource group. Each section
provides `on_*` helpers for every endpoint, so tests never hard-code paths.

#### `server.auth()` → `MockAuth`

| Method | HTTP | Path |
| --- | --- | --- |
| `on_register()` | POST | `/api/v1/auth/register` |
| `on_login()` | POST | `/api/v1/auth/login` |
| `on_refresh()` | POST | `/api/v1/auth/refresh` |
| `on_logout()` | POST | `/api/v1/auth/logout` |
| `on_me()` | GET | `/api/v1/auth/me` |
| `on_auth_methods()` | GET | `/api/v1/auth/methods` |
| `on_device_auth_start()` | POST | `/api/v1/auth/device` |
| `on_device_auth_poll()` | POST | `/api/v1/auth/device/poll` |
| `on_device_auth_approve()` | POST | `/api/v1/auth/device/approve` |

#### `server.api_tokens()` → `MockApiTokens`

| Method | HTTP | Path |
| --- | --- | --- |
| `on_list()` | GET | `/api/v1/auth/api-tokens` |
| `on_create()` | POST | `/api/v1/auth/api-tokens` |
| `on_revoke(id)` | DELETE | `/api/v1/auth/api-tokens/{id}` |

#### `server.health()` → `MockHealth`

| Method | HTTP | Path |
| --- | --- | --- |
| `on_healthz()` | GET | `/healthz` |

#### `server.hosts()` → `MockHosts`

| Method | HTTP | Path |
| --- | --- | --- |
| `on_list()` | GET | `/api/v1/hosts` |
| `on_get(id)` | GET | `/api/v1/hosts/{id}` |
| `on_update(id)` | PUT | `/api/v1/hosts/{id}` |
| `on_deactivate(id)` | DELETE | `/api/v1/hosts/{id}` |

#### `server.oidc_auth()` → `MockOidcAuth`

| Method | HTTP | Path |
| --- | --- | --- |
| `on_authorize(provider_id)` | GET | `/api/v1/auth/oidc/{provider_id}/authorize` |
| `on_exchange()` | POST | `/api/v1/auth/oidc/exchange` |
| `on_link()` | POST | `/api/v1/auth/oidc/link` |
| `on_complete_registration()` | POST | `/api/v1/auth/oidc/complete-registration` |

#### `server.oidc_providers()` → `MockOidcProviders`

| Method | HTTP | Path |
| --- | --- | --- |
| `on_list()` | GET | `/api/v1/settings/oidc-providers` |
| `on_create()` | POST | `/api/v1/settings/oidc-providers` |
| `on_get(id)` | GET | `/api/v1/settings/oidc-providers/{id}` |
| `on_update(id)` | PUT | `/api/v1/settings/oidc-providers/{id}` |
| `on_delete(id)` | DELETE | `/api/v1/settings/oidc-providers/{id}` |
| `on_activate(id)` | POST | `/api/v1/settings/oidc-providers/{id}/activate` |
| `on_deactivate(id)` | POST | `/api/v1/settings/oidc-providers/{id}/deactivate` |

#### `server.pki()` → `MockPki`

| Method | HTTP | Path |
| --- | --- | --- |
| `on_ca_cert()` | GET | `/api/v1/pki/ca.crt` |
| `on_ca_crl()` | GET | `/api/v1/pki/ca.crl` |

#### `server.plugin_configs()` → `MockPluginConfigs`

| Method | HTTP | Path |
| --- | --- | --- |
| `on_list()` | GET | `/api/v1/plugin-configs` |
| `on_create()` | POST | `/api/v1/plugin-configs` |
| `on_get(id)` | GET | `/api/v1/plugin-configs/{id}` |
| `on_update(id)` | PUT | `/api/v1/plugin-configs/{id}` |
| `on_delete(id)` | DELETE | `/api/v1/plugin-configs/{id}` |

#### `server.scheduler()` → `MockScheduler`

| Method | HTTP | Path |
| --- | --- | --- |
| `on_list()` | GET | `/api/v1/scheduler/tasks` |
| `on_get(id)` | GET | `/api/v1/scheduler/tasks/{id}` |
| `on_update(id)` | PUT | `/api/v1/scheduler/tasks/{id}` |
| `on_trigger(id)` | POST | `/api/v1/scheduler/tasks/{id}/trigger` |

#### `server.services()` → `MockServices`

| Method | HTTP | Path |
| --- | --- | --- |
| `on_list()` | GET | `/api/v1/services` |
| `on_get(id)` | GET | `/api/v1/services/{id}` |
| `on_update(id)` | PUT | `/api/v1/services/{id}` |
| `on_approve(id)` | POST | `/api/v1/services/{id}/approve` |
| `on_reject(id)` | POST | `/api/v1/services/{id}/reject` |
| `on_remove(id)` | DELETE | `/api/v1/services/{id}` |
| `on_merge(target_id)` | POST | `/api/v1/services/{id}/merge` |
| `on_create_enrollment_token()` | POST | `/api/v1/services/enrollment-token` |
| `on_revoke_enrollment_token()` | DELETE | `/api/v1/services/enrollment-token` |
| `on_enrollment_token_status()` | GET | `/api/v1/services/enrollment-token/status` |

#### `server.settings()` → `MockSettings`

| Method | HTTP | Path |
| --- | --- | --- |
| `on_get_combined()` | GET | `/api/v1/settings` |
| `on_get_registration()` | GET | `/api/v1/settings/registration` |
| `on_update_registration()` | PUT | `/api/v1/settings/registration` |
| `on_get_authentication()` | GET | `/api/v1/settings/authentication` |
| `on_update_authentication()` | PUT | `/api/v1/settings/authentication` |
| `on_get_agent_certificates()` | GET | `/api/v1/settings/agent-certificates` |
| `on_update_agent_certificates()` | PUT | `/api/v1/settings/agent-certificates` |
| `on_get_network()` | GET | `/api/v1/settings/network` |
| `on_update_network()` | PUT | `/api/v1/settings/network` |
| `on_rotate_ca()` | POST | `/api/v1/settings/rotate-ca` |
| `on_renew_server_certificate()` | POST | `/api/v1/settings/renew-server-certificate` |

#### `server.settings_mqtt()` → `MockSettingsMqtt`

| Method | HTTP | Path |
| --- | --- | --- |
| `on_list()` | GET | `/api/v1/settings/mqtt` |
| `on_create()` | POST | `/api/v1/settings/mqtt` |
| `on_get_limit()` | GET | `/api/v1/settings/mqtt/limit` |
| `on_update_limit()` | PUT | `/api/v1/settings/mqtt/limit` |
| `on_get(id)` | GET | `/api/v1/settings/mqtt/{id}` |
| `on_update(id)` | PUT | `/api/v1/settings/mqtt/{id}` |
| `on_delete(id)` | DELETE | `/api/v1/settings/mqtt/{id}` |

#### `server.software_items()` → `MockSoftwareItems`

| Method | HTTP | Path |
| --- | --- | --- |
| `on_list()` | GET | `/api/v1/software-items` |
| `on_create()` | POST | `/api/v1/software-items` |
| `on_get(id)` | GET | `/api/v1/software-items/{id}` |
| `on_update(id)` | PUT | `/api/v1/software-items/{id}` |
| `on_delete(id)` | DELETE | `/api/v1/software-items/{id}` |
| `on_assign_hosts(id)` | POST | `/api/v1/software-items/{id}/hosts` |
| `on_unassign_host(item_id, host_id)` | DELETE | `/api/v1/software-items/{id}/hosts/{host_id}` |
| `on_check_versions(id)` | POST | `/api/v1/software-items/{id}/check-versions` |
| `on_check_versions_host(item_id, host_id)` | POST | `/api/v1/software-items/{id}/hosts/{host_id}/check-versions` |
| `on_trigger_update(item_id, host_id)` | POST | `/api/v1/software-items/{id}/hosts/{host_id}/update` |

#### `server.system_alerts()` → `MockSystemAlerts`

| Method | HTTP | Path |
| --- | --- | --- |
| `on_get()` | GET | `/api/v1/system/alerts` |

#### `server.update_history()` → `MockUpdateHistory`

| Method | HTTP | Path |
| --- | --- | --- |
| `on_list()` | GET | `/api/v1/update-history` |
| `on_get(id)` | GET | `/api/v1/update-history/{id}` |

#### Generic escape hatch

```rust
// Match any endpoint by method and path (case-insensitive method)
server.on("DELETE", "/api/v1/some/custom/path")
```

### Client helpers

```rust
// Authenticated client (bearer token = "test-token", TLS verification disabled)
let client = server.client();

// Unauthenticated client
let client = server.client_unauth();

// Raw httpmock server for advanced scenarios
let raw = server.server();
```

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
# Unit tests only
cargo test -p uptrakit-openapi-client

# With mock feature (no additional tests in openapi-client itself, but enables mock for consumers)
cargo test -p uptrakit-openapi-client --all-features
```

## Related documentation

- [HTTP Web API](../api/http-web-api.md) -- API endpoint reference
- [CLI Output](cli-output.md) -- CLI output formatting conventions
- [CLI Usage](../end-user/cli-usage.md) -- end-user CLI guide
- [Coding Standards](coding-standards.md) -- error handling and quality requirements

# Coding Standards

## Error Handling

For comprehensive error handling patterns, conventions, and the full decision guide, see
[Error Handling](error-handling.md). Key points:

- Wrap errors in `rootcause::Report` and define a `Result<T>` alias per boundary.
- Use `thiserror::Error` with `#[derive(Debug, Error)]` to describe failures.
- Implement `ReportConversion` (via `impl_report_conversion!`) for all downstream errors and prefer `.context_to()?` to preserve the chain.
- Use `report!()` for creating new reports and `bail!()` for early returns.
- Avoid `Result<T, String>`; prefer typed enums.
- Logging should never expose secrets (tokens, passwords, keys).

## Panic Policy

- `unwrap()`, `expect()`, and `panic!()` are forbidden in production code (tests are the only exception).
- Locking primitives (`Mutex::lock()`, `RwLock::read()`, `RwLock::write()`) may use `.unwrap()` because the release build aborts on panic, rendering
  poisoning impossible.
- When serialization/parsing can fail, use `match` to handle errors gracefully or propagate them with context.
- **`Default` impls must not call `.parse().unwrap()` or `.expect()`.** Construct values directly using infallible constructors instead. If no
  infallible constructor exists, use an `Option` field and populate it lazily, or use a `const`-compatible builder.

```rust
// ✓ Correct — infallible; no parse path
use std::net::{Ipv6Addr, SocketAddr, SocketAddrV6};

https_addr: SocketAddr::V6(SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, 8443, 0, 0)),

// ✗ Wrong — panics at startup if the string is ever edited incorrectly
https_addr: "[::]:8443".parse().unwrap(),
```

## Public Enum Extensibility

All public enums that may gain new variants must carry the `#[non_exhaustive]` attribute. This allows the project to evolve without semver-breaking
changes for downstream consumers. External crates matching on these enums must include a wildcard `_ =>` arm.

Enums currently annotated with `#[non_exhaustive]`:

- `PluginType` (`shared-types`)
- `ServiceMessage`, `ControllerMessage` (`wire`)
- `PluginCapability` (`plugin-infrastructure-core`)

When adding a new public enum, apply `#[non_exhaustive]` by default unless the enum is explicitly guaranteed to be closed (e.g., a two-variant
boolean-like enum).

## Feature Flags

All feature flags in this workspace are **additive** — enabling a feature adds functionality;
it never removes or restricts code compiled without the feature.

### Additive-only rule

**Never** use `#[cfg(not(feature = "X"))]` attribute-style conditionals. This syntax makes
feature `X` subtract from the binary, which violates the additive model and can cause
incorrect builds when features are combined.

Instead, use the `cfg!()` macro in expression position:

```rust
// ✓ Correct — expression form; compiles the same code path in all builds
if !cfg!(feature = "embed-frontend") {
    let dir = resolve_static_dir(args.static_dir.clone())?;
    // ...
}

// ✗ Wrong — attribute conditionally excludes a function from the compilation unit
#[cfg(not(feature = "embed-frontend"))]
fn resolve_static_dir(...) -> Result<Option<PathBuf>> { ... }
```

The expression form `cfg!(feature = "X")` evaluates to a `bool` at compile time (the dead
branch is eliminated by the optimizer), but every code path still compiles under every
feature combination — which is what "additive" means.

**Exception:** `#[cfg(feature = "X")]` (without `not`) is allowed for blocks that are
*purely additive* — they add code only when the feature is enabled and are never present
in the base build. Only `#[cfg(not(feature = "X"))]` is prohibited.

### Additive patterns in tests

Test modules that vary expected values by feature should build a single vec and extend it:

```rust
let mut expected = vec!["/api/v1/auth/login", /* ... */];
if cfg!(feature = "oidc") {
    expected.extend_from_slice(&["/api/v1/auth/oidc/exchange", /* ... */]);
}
```

This keeps a single test that compiles and runs correctly under every feature combination.

### Additive route registration

When a route is only meaningful with a specific feature (e.g. Swagger UI), use
`#[cfg(feature = "swagger-ui")]` on the *additive* registration block only — never to
remove an existing route:

```rust
// Always present — raw JSON route is always available
router = router.route("/api/openapi.json", get(json_handler));

// Additive — UI overlay only when the feature is enabled
#[cfg(feature = "swagger-ui")]
{
    use utoipa_swagger_ui::SwaggerUi;
    router = router.merge(SwaggerUi::new("/api/docs").url("/api/openapi.json", api));
}
```

See also: [Embedded Frontend](embedded-frontend.md) for the `embed-frontend` feature pattern.

### Feature-gated external APIs

When an external crate API (e.g. `bollard::Docker::connect_with_ssh`) is only available under a
specific feature, the `#[cfg(feature = "X")] return ...; <fallback>` idiom **cannot** be used
because the fallback code after an unconditional `return` becomes unreachable when the feature is
on — triggering the `unreachable_code` lint that is denied workspace-wide.

Use one of the following approved patterns instead.

**Pattern A — gate the entire match arm**

Move the feature-specific arm behind `#[cfg(feature = "X")]` and handle the disabled case in the
default arm with a runtime check:

```rust
// ✓ Correct — the default arm handles the disabled case at runtime; no #[cfg(not)] needed
fn connect(docker_host: Option<&str>, ssh_key_path: Option<&str>) -> Result<bollard::Docker> {
    match docker_host {
        #[cfg(feature = "ssh")]
        Some(h) if h.starts_with("ssh://") => {
            bollard::Docker::connect_with_ssh(h, TIMEOUT, API_VERSION,
                                              ssh_key_path.map(str::to_string))
                .context_to::<DockerError>()
        }

        Some(h) => {
            // When the `ssh` feature is disabled, give a clear error for SSH URLs.
            if h.starts_with("ssh://") {
                let _ = ssh_key_path; // suppress unused-variable warning
                bail!(DockerError::Configuration(
                    "SSH Docker connections require the 'ssh' Cargo feature".to_string()
                ));
            }
            bollard::Docker::connect_with_http(h, TIMEOUT, API_VERSION)
                .context_to::<DockerError>()
        }
    }
}
```

**Pattern B — stub + upgrade helper**

Initialize with an always-available stub, then override in a `#[cfg(feature = "X")]` block by
calling a helper that accepts and discards the stub:

```rust
// ✓ Correct — stub is passed as an argument (counts as "read"), suppressing unused_assignments
fn new(config: &Config) -> Result<Self> {
    let docker_client: Arc<dyn DockerClient> = Arc::new(NoopDockerClient);
    #[cfg(feature = "daemon")]
    let docker_client = Self::upgrade_to_daemon_client(docker_client, config)?;
    Self::init(config, docker_client)
}

#[cfg(feature = "daemon")]
fn upgrade_to_daemon_client(
    _stub: Arc<dyn DockerClient>,
    config: &Config,
) -> Result<Arc<dyn DockerClient>> {
    Ok(Arc::new(RealDockerClient::new(config)?))
}
```

**Pattern C — always-present tracking field**

When a struct field only exists under a feature but you need a cfg-free accessor method, add an
always-present `bool` that mirrors its presence:

```rust
struct NotificationService {
    /// Always present so that `has_nats()` needs no `#[cfg]`.
    nats_configured: bool,
    #[cfg(feature = "nats")]
    nats: Option<NatsTransport>,
}

impl NotificationService {
    #[cfg(feature = "nats")]
    pub fn with_nats(mut self, nats: NatsTransport) -> Self {
        self.nats = Some(nats);
        self.nats_configured = true; // keep the mirror in sync
        self
    }

    // No #[cfg] needed — reads a field that always exists
    pub fn has_nats(&self) -> bool {
        self.nats_configured
    }
}
```

**Pattern D — conditional early-return inside a guard**

When the feature-gated code path is guarded by a runtime condition (`if let`, `if`, etc.), the
`return` is conditional rather than unconditional, so the fallback code remains reachable in all
builds:

```rust
// ✓ Correct — `return` is inside `if let Some(...)`, not at the top level
async fn maybe_publish_nats(&self, msg: ControllerMessage) {
    #[cfg(feature = "nats")]
    if let Some(ref nats) = self.nats {
        nats.publish(msg).await;
        return; // reachable only when `nats` is Some; fallback below is always compiled
    }
    let _ = msg; // suppress unused-variable warning when nats feature is disabled
}
```

**Anti-patterns to avoid**

```rust
// ✗ Wrong — #[cfg(not)] violates the additive rule
#[cfg(not(feature = "ssh"))]
bail!(DockerError::Configuration("..."));

// ✗ Wrong — unconditional return makes the fallback unreachable when feature is ON
#[cfg(feature = "ssh")]
return bollard::Docker::connect_with_ssh(...).context_to::<DockerError>();
let _ = (h, ssh_key_path); // unreachable_code lint fires here under --all-features

// ✗ Wrong — init-then-override triggers unused_assignments lint
let mut client: Arc<dyn DockerClient> = Arc::new(NoopDockerClient);
#[cfg(feature = "daemon")]
{ client = Arc::new(RealDockerClient::new(config)?); } // initial value "never read"
```

See also: [Security — Secure Development](../security/secure-development.md).

## Design Principles

- Keep every boundary clear: the controller orchestrates scheduling, upstream checks, API/UI; the MQTT service handles MQTT/Home Assistant
  integration; agents manage installed versions and update execution; plugins focus on version detection/updating logic.
- Treat custom scripts and command execution paths as handling untrusted input; validate before execution to avoid shell injection.
- Agent connections are outbound-only, unprivileged, and rely on explicit sudo allowlists for privileged update commands.
- Logs should contain operational summaries only; do not store full command output internally.
- New protected routes must check the typed `Permission` enum rather than comparing raw role names.
- Document every behavioral change either in code or via an external docs page (e.g., update `docs/api` or `docs/development`).

Refer to [docs/security/secure-development.md](../security/secure-development.md) when the change touches PKI, secrets, reverse proxies, or filesystem
security.

## Service Reconnect Backoff

All reconnect loops in service binaries must use `uptrakit_service_sdk::Backoff` — not a fixed
sleep. Fixed delays hammer a recovering broker or controller and produce bursty log storms.

```rust
use uptrakit_service_sdk::Backoff;
use std::time::Duration;

// Construct once per connection attempt sequence
let mut backoff = Backoff::new(Duration::from_secs(2), Duration::from_secs(60));

loop {
    match connect().await {
        Ok(conn) => {
            backoff.reset();   // Reset on successful connection
            handle(conn).await;
        }
        Err(e) => {
            let delay = backoff.next_delay();
            tracing::warn!(error = %e, delay = ?delay, "connection failed; retrying");
            tokio::select! {
                _ = shutdown_token.cancelled() => break,
                _ = tokio::time::sleep(delay) => {}
            }
        }
    }
}
```

Standard parameters: **base 2 s, cap 60 s** with ~25 % jitter (the `Backoff` default). The
`tokio::select!` on `shutdown_token` ensures the loop exits promptly on SIGTERM even when the
delay is long.

**Never** replace this with `tokio::time::sleep(Duration::from_secs(5))`. A fixed delay:

- Does not back off under sustained outages, hammering the broker.
- Cannot be interrupted by a shutdown signal without an additional `select!`.
- Has no jitter, causing thundering-herd reconnects when many agents restart simultaneously.

See also: [Service Lifecycle](service-lifecycle.md) for the full reconnect and enrollment flow.

## String-to-Type Conversions

All string-to-type conversions must use the standard `FromStr` trait. Do not add ad-hoc `parse(&str)` methods that serve the same purpose.

### Required pattern

```rust
use std::str::FromStr;
use thiserror::Error;

#[derive(Debug, Error)]
#[error("invalid my type value")]
pub struct ParseMyTypeError;

impl FromStr for MyType {
    type Err = ParseMyTypeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "variant_a" => Ok(Self::VariantA),
            _ => Err(ParseMyTypeError),
        }
    }
}
```

### Conventions

- **Error type naming:** `Parse{TypeName}Error` (e.g., `ParsePermissionError`, `ParseMqttTransportError`).
- **Error derivation:** Use `thiserror::Error` in crates that depend on `thiserror`. In crates without `thiserror` (e.g., `uptrakit-internal-wire`),
  implement `Display` and `Error` manually.
- **Call sites:** Prefer `s.parse::<MyType>()` over explicit `MyType::from_str(s)`.
- **Fallible conversions with defaults:** Use `s.parse::<MyType>().unwrap_or_default()` when a default is acceptable.
- **`from_url_scheme()` and similar:** Domain-specific parsers that convert from a different representation (e.g., URL schemes like `mqtt`/`mqtts` to
  `MqttTransport`) are not `FromStr` candidates and should remain as named methods.

### Anti-pattern

- **Ad-hoc `parse(&str)` methods** returning `Option<Self>` or `Result<Self, String>` -- always implement `FromStr` instead.

The full error handling reference (20 patterns, anti-patterns, decision table, approved exceptions, and rules summary)
is in [Error Handling](error-handling.md).

## Request Type Validation

All HTTP request types in `uptrakit-web-api-types` that accept user input must implement the `Validate` trait (defined in `validation.rs`). Route
handlers call `req.validate()` at entry and return HTTP 400 on failure.

### Validate trait

```rust
use uptrakit_web_api_types::validation::{Validate, ValidationError};

pub trait Validate {
    fn validate(&self) -> Result<(), ValidationError>;
}
```

`ValidationError` carries a `field: &'static str` and `message: String`, providing structured field-level error reporting to API consumers.

### Implementation pattern

```rust
impl Validate for CreateSoftwareItemRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.name.trim().is_empty() {
            return Err(ValidationError {
                field: "name",
                message: "must not be empty".to_string(),
            });
        }
        Ok(())
    }
}
```

### Route handler wiring

```rust
if let Err(e) = req.validate() {
    return error_response(StatusCode::BAD_REQUEST, &e.to_string());
}
```

### Currently validated request types

| Type | Key validations | | --- | --- | | `RegisterRequest` | email format (contains `@`, max 254 chars), `first_name` non-empty, password 8-1024
chars | | `LoginRequest` | email format, password non-empty | | `CreateOidcProviderRequest` | name non-empty, slug format (lowercase+digits+hyphens,
1-64), issuer_url scheme, client_id non-empty | | `UpdateScheduledTaskRequest` | cron_expression non-empty, 5 whitespace-separated fields | |
`UpdateNetworkSettingsRequest` | trusted_proxies items non-empty, real_ip_header non-empty, pki_addr URL format | | `CreateSoftwareItemRequest` | name
non-empty, exactly one of plugin_config_id/plugin_config | | `CreatePluginConfigRequest` | name non-empty |

See also: the `update_hooks.rs` module provides a similar validation pattern (`HookValidationError`) for hook configuration types.

## Route Authorization Pattern

All protected web-API route handlers enforce authorization via typed Axum extractors. Never call
`user.has_permission(...)` inline in a handler body.

### Required pattern

```rust
use crate::middleware::permission::CanViewHosts;

pub async fn list_hosts(
    tenant_db: TenantDb,
    CanViewHosts(_user): CanViewHosts,   // permission enforced here
) -> Response {
    // handler body — 401/403 already handled by the extractor
}
```

Use the bound variable name `_user` when the `AuthenticatedUser` value is not used in the body, and `user`
when it is (e.g. `user.user_id` for an `actor_id` field).

### Required utoipa extension

Every protected endpoint must declare its required permission as an OpenAPI extension:

```rust
#[utoipa::path(
    get,
    path = "/api/v1/hosts",
    extensions(("x-required-permission" = json!("view_hosts"))),
    security(("bearer_token" = [])),
    ...
)]
```

The `json!` value must match the `as_str()` serialization of the corresponding `Permission` variant.

### Adding a new extractor

Add one line to the `permission_extractor!` macro call in
`crates/ui/web-api/src/middleware/permission.rs`:

```rust
permission_extractor! {
    ...
    CanNewThing => Permission::NewThing,
}
```

The macro generates a `#[derive(Debug)]` struct `CanNewThing(pub AuthenticatedUser)` with a `FromRequestParts`
impl and a `::new(user)` test constructor.

### Anti-pattern

```rust
// WRONG — do not call has_permission in handlers
pub async fn list_hosts(
    Extension(user): Extension<AuthenticatedUser>,
) -> Response {
    if !user.has_permission(Permission::ViewHosts) {
        return error_response(StatusCode::FORBIDDEN, "Insufficient permissions");
    }
    ...
}
```

See also: [Authentication and Authorization](../security/auth-and-authorization.md).

## Typed Path Extractors

Route handlers that accept UUID path parameters must use `Path<Uuid>` (or `Path<(Uuid, Uuid)>` for
multi-param routes) instead of `Path<String>` with manual `Uuid::parse_str`. Axum returns a typed
422 response automatically on malformed input.

### Required pattern

```rust
use uuid::Uuid;

pub async fn get_host(
    tenant_db: TenantDb,
    CanViewHosts(_user): CanViewHosts,
    Path(host_id): Path<Uuid>,        // Axum validates the UUID
) -> Response {
    // use host_id directly — no parse_str needed
}
```

For multi-param routes:

```rust
pub async fn unassign_host(
    tenant_db: TenantDb,
    CanManageSoftware(_user): CanManageSoftware,
    Path((item_id, host_id)): Path<(Uuid, Uuid)>,
) -> Response { ... }
```

### utoipa annotations

Path parameters must declare `Uuid` type (not `String`) so the OpenAPI schema emits `format: uuid`:

```rust
#[utoipa::path(
    get,
    path = "/api/v1/hosts/{id}",
    params(("id" = Uuid, Path, description = "Host UUID")),
    ...
)]
```

### Anti-pattern

```rust
// WRONG — do not manually parse UUIDs from path parameters
Path(id): Path<String>,
let host_id = match uuid::Uuid::parse_str(&id) {
    Ok(id) => id,
    Err(_) => return error_response(StatusCode::BAD_REQUEST, "Invalid UUID"),
};
```

**Exception:** `Path<String>` is correct for non-UUID path parameters (e.g., base64-encoded OCSP
requests in `ocsp.rs`).

## Tenant-Safe Database Queries

All database queries in route handlers and query helpers **must** enforce tenant isolation. Failure to do so can leak
data across tenants (a high-severity security issue).

### Rules

**Rule 1 — Always use `TenantDb` helpers for `TenantScoped` entities.**

Use `TenantDb.find::<E>()`, `.find_by_id::<E>(id)`, `.update_many::<E>()`, or `.delete_many::<E>()` for any entity
that implements `TenantScoped`. These methods automatically inject `WHERE tenant_id = ?`.

**Rule 2 — Use `find_via_tenant_join` for join-table entities without `tenant_id`.**

Some entities (e.g. `service_host`) are join tables that have no `tenant_id` column of their own. Enforce tenant
isolation by joining through a `TenantScoped` entity with `TenantDb.find_via_tenant_join::<Target, Scoped>(relation)`.

```rust
// service_host has no tenant_id — scope it via service (TenantScoped)
tenant_db
    .find_via_tenant_join::<service_host::Entity, service::Entity>(
        service_host::Relation::Service.def(),
    )
    .filter(service::Column::DeactivatedAt.is_null())
    .all(tenant_db.db())
    .await?
```

**Rule 3 — Never call `Entity::find().all(tenant_db.db())` on a `TenantScoped` entity.**

`tenant_db.db()` is the raw `DatabaseConnection`; it carries no tenant filter. Calling
`Entity::find().all(tenant_db.db())` on a `TenantScoped` entity loads **all** rows from all tenants.

**Rule 4 — Prefer batch queries over per-item query loops (N+1 prevention).**

Always use `.is_in(ids)` to load multiple records in one round-trip, then join in memory via `HashMap`.

```rust
// Correct: 1 query for N hosts
let hosts: HashMap<Uuid, host::Model> = tenant_db
    .find::<host::Entity>()
    .filter(host::Column::Id.is_in(host_ids))
    .all(tenant_db.db()).await?
    .into_iter().map(|h| (h.id, h)).collect();

// Then look up in the loop — O(1) per access
let Some(h) = hosts.get(&link.host_id) else { continue; };
```

### Anti-pattern table

| Wrong | Right | Reason |
| --- | --- | --- |
| `Host::find().all(tenant_db.db())` | `tenant_db.find::<host::Entity>().all(tenant_db.db())` | No tenant filter applied |
| `ServiceHost::find().all(tenant_db.db())` | `tenant_db.find_via_tenant_join::<service_host::Entity, service::Entity>(rel)` | Cross-tenant leak |
| Per-item `Host::find_by_id(id).one(db)` inside a loop | Batch `find().filter(id.is_in(ids))` then in-memory lookup | N+1 queries |
| `Entity::update_many().col_expr(...)` loop | `Entity::update_many().filter(id.is_in(ids)).col_expr(...).exec(db)` | N+1 updates |

See also: [Architecture — Multi-Tenancy](../architecture/multi-tenancy.md) and
[Security — Secure Development](../security/secure-development.md).

## HTTP Status Codes

Always use `reqwest::StatusCode` (re-exported as `uptrakit_openapi_client::StatusCode` for CLI code) instead of raw `u16` for HTTP status codes.

- **Comparisons**: `status == StatusCode::NOT_FOUND`, not `status == 404`
- **Range checks**: `status.is_client_error()`, `status.is_server_error()`, `status.is_success()` -- not `status >= 400`
- **Reason phrases**: `status.canonical_reason()` -- not hand-written match tables
- **Error types**: `status: StatusCode` -- not `status: u16`
- **Serialization**: When a `StatusCode` must appear as a number in JSON, use `#[serde(serialize_with = "serialize_status_code")]`
- **The only approved `.as_u16()` call** is inside serde serialization helpers for JSON wire compatibility

## Database Enum Columns (`DeriveActiveEnum`)

All entity columns that store a fixed set of string values must use a typed Rust enum with SeaORM's `DeriveActiveEnum` instead of `String`. This
provides compile-time type safety and eliminates string parsing at query boundaries.

### Pattern

Define the enum in `uptrakit-shared-types` with feature-gated sea-orm derives, following the `DeviceAuthStatus` template:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "sea-orm", derive(strum::EnumIter, sea_orm::DeriveActiveEnum))]
#[cfg_attr(feature = "sea-orm", sea_orm(rs_type = "String", db_type = "Text"))]
pub enum MyStatus {
    #[cfg_attr(feature = "sea-orm", sea_orm(string_value = "active"))]
    Active,
    #[cfg_attr(feature = "sea-orm", sea_orm(string_value = "inactive"))]
    Inactive,
}
```

Also implement `FromStr`, `Display`, `as_str()`, and `Default` (see existing types for the full pattern). The entity model then uses the enum type
directly:

```rust
pub struct Model {
    pub status: MyStatus,  // NOT String
}
```

### Existing typed enum columns

| Entity | Column | Enum | | --- | --- | --- | | `mqtt_client` | `transport` | `MqttTransport` | | `mqtt_client` | `connection_status` |
`MqttClientConnectionStatus` | | `session` | `token_type` | `SessionTokenType` | | `update_output_line` | `stream` | `OutputStreamType` | |
`pending_device_flow` | `status` | `DeviceAuthStatus` | | `service` | `status` | `ServiceStatus` | |
`update_history` | `status` | `UpdateStatus` |

Note: the `service` entity stores capabilities as a JSON text column (`services.capabilities`) rather
than a typed enum column. The capability set is parsed into `BTreeSet<Capability>` at read time. See
[Service Lifecycle -- Capability-based enrollment](service-lifecycle.md#capability-based-enrollment).

### Re-exports

`uptrakit-shared-db` re-exports all entity-relevant enums from `uptrakit-shared-types` for downstream convenience. Crates that depend on
`uptrakit-shared-db` (like `uptrakit-web-api`) should import from `uptrakit_shared_db` rather than adding a direct dependency on
`uptrakit-shared-types`.

## SeaORM Integration for Custom Types

When creating new wrapper types (newtypes) for use in SeaORM entity models, implement the following four traits behind the `sea-orm` feature flag
in `uptrakit-shared-types`. Both `SecretString` and `MaskedEmail` follow this pattern.

### Required trait implementations

| Trait | Purpose |
| --- | --- |
| `From<T> for sea_orm::Value` | Converts the wrapper into a `Value` for query binding |
| `sea_orm::TryGetable` | Extracts the wrapper from a `QueryResult` row |
| `sea_orm::sea_query::ValueType` | Declares the column type and provides `try_from(Value)` |
| `sea_orm::sea_query::Nullable` | Provides the null `Value` representation for `Option<T>` columns |

### Implementation template

```rust
#[cfg(feature = "sea-orm")]
mod sea_orm_impl {
    use super::MyWrapper;
    use sea_orm::entity::prelude::*;
    use sea_orm::sea_query::ValueType;
    use sea_orm::{TryGetError, TryGetable};

    impl From<MyWrapper> for Value {
        fn from(w: MyWrapper) -> Self {
            Value::String(Some(w.into_inner()))
        }
    }

    impl TryGetable for MyWrapper {
        fn try_get_by<I: sea_orm::ColIdx>(
            res: &QueryResult,
            index: I,
        ) -> std::result::Result<Self, TryGetError> {
            let val: String = res.try_get_by(index)?;
            Ok(MyWrapper::new(val))
        }
    }

    impl ValueType for MyWrapper {
        fn try_from(v: Value) -> std::result::Result<Self, sea_orm::sea_query::ValueTypeErr> {
            match v {
                Value::String(Some(s)) => Ok(MyWrapper::new(s)),
                _ => Err(sea_orm::sea_query::ValueTypeErr),
            }
        }

        fn type_name() -> String {
            "MyWrapper".to_string()
        }

        fn array_type() -> sea_orm::sea_query::ArrayType {
            sea_orm::sea_query::ArrayType::String
        }

        fn column_type() -> sea_orm::ColumnType {
            sea_orm::ColumnType::String(sea_orm::sea_query::StringLen::None)
        }
    }

    impl sea_orm::sea_query::Nullable for MyWrapper {
        fn null() -> Value {
            Value::String(None)
        }
    }
}
```

### Important notes

- **Import `ValueType` from `sea_orm::sea_query`**, not from the prelude. In sea-orm 2.0, `ValueType` is not re-exported from the prelude.
- Place all four trait implementations inside a `#[cfg(feature = "sea-orm")] mod sea_orm_impl { ... }` block in the type's source file.
- Add corresponding `#[cfg(feature = "sea-orm")]` test functions to verify the roundtrip (see `SecretString` and `MaskedEmail` tests for examples).
- For types that perform fallible conversion (like `EncryptedString` which decrypts on read), convert typed errors via `.map_err()` to the required
  SeaORM error type (see Pattern 13 above).

### Existing custom types with SeaORM integration

| Type | Crate | Entity usage |
| --- | --- | --- |
| `SecretString` | `uptrakit-shared-types` | `user.password_hash` |
| `MaskedEmail` | `uptrakit-shared-types` | `user.email` |
| `EncryptedString` | `uptrakit-shared-db` | `mqtt_client.password`, `oidc_provider.client_secret`, `ca_certificate.key_pem`, `ssh_host.private_key` |

See also: [Secrets Handling and Encryption](../security/secrets-and-encryption.md) for security properties of `SecretString` and `MaskedEmail`.

## Database Query Patterns

Paginated list endpoints must never issue per-record queries. Violating this rule produces O(N) database round-trips
that make the API unusable at scale.

### Batch loading rule

After fetching a page of N records, collect all unique foreign-key IDs from the page and load related entities in a
single `is_in(ids)` query. Build a `HashMap<Uuid, …>` for O(1) lookup during response construction. Never call
`find_by_id` inside a loop.

```rust
// ✓ Correct — two queries regardless of page size
let host_ids: Vec<Uuid> = records.iter().map(|r| r.host_id).collect();
let hosts: HashMap<Uuid, String> = Host::find()
    .filter(host::Column::Id.is_in(host_ids))
    .all(db).await?
    .into_iter().map(|h| (h.id, h.friendly_name)).collect();

// ✗ Wrong — N+1 queries
for record in &records {
    let name = resolve_host_name(db, record.host_id).await?;
}
```

### Tenant-scoped subquery

Use `Expr::in_subquery(…)` or a JOIN for tenant-scoping tables that reference `host_id` (e.g. `update_history`).
Loading all host IDs into application memory and passing them to `is_in(Vec<Uuid>)` is only acceptable when the
tenant is guaranteed to have fewer than ~100 rows; for unbounded collections use a subquery:

```rust
let host_subquery = Query::select()
    .column(host::Column::Id)
    .from(host::Entity)
    .and_where(Expr::col(host::Column::TenantId).eq(tenant_id))
    .to_owned();
Entity::find().filter(Column::HostId.in_subquery(host_subquery))
```

### Scope pre-loaded sets tightly

When pre-loading a lookup set to avoid per-item queries, scope it to the narrowest key available. For example,
pre-load autodiscovery ignore rules per `(tenant_id, plugin_config_id)`, not per `tenant_id` alone — the
per-config set is bounded to what a user has explicitly configured for that one plugin, while the per-tenant set
can be unbounded.

### Avoid `unwrap_or(0)` on count queries

A silently-zero count on DB failure hides errors. Propagate DB errors with `?` or log and return an explicit error
response. Do not use `count.unwrap_or(0)` as a silent default.

## Exhaustive Enum Dispatch

Wildcard arms in dispatch functions are forbidden. A function that maps enum variants to domain values (timeout,
routing key, HTTP status code) must enumerate every known variant explicitly — a new variant must not silently
inherit an arbitrary default.

Extend the `#[non_exhaustive]` rule from the "Public Enum Extensibility" section:

- **Closed enum**: remove the wildcard entirely. The compiler enforces exhaustiveness at compile time.
- **`#[non_exhaustive]` enum** (e.g., `Capability`, `UpdateFinalStatus`): a wildcard is required in external
  crates, but it must never be silent. Replace `_ => some_default` with a `tracing::warn!` + a documented safe
  fallback, and replace `_ => unreachable!()` with `tracing::warn!` + early return. Never use `unreachable!()` on
  values that come from wire or database state.

```rust
// ✓ Correct — unknown profile logged; safe fallback chosen explicitly
match profile {
    ServiceProfile::Agent | ServiceProfile::Unknown => Some(AGENT_SHUTDOWN_TIMEOUT_SECS),
    ServiceProfile::MqttBridge => None,
    _ => {
        tracing::warn!(?profile, "unknown ServiceProfile for shutdown timeout; using agent default");
        Some(AGENT_SHUTDOWN_TIMEOUT_SECS)
    }
}

// ✗ Wrong — silent incorrect behaviour for future variants
_ => Some(120),

// ✗ Wrong — panics when a new wire variant arrives
_ => unreachable!("unknown ServiceProfile variant"),
```

## Parameter Struct Pattern

Functions must not require `#[allow(clippy::too_many_arguments)]`. No Clippy suppression is approved in this
codebase (AGENTS.md invariant 13). When a function's non-`self` parameter count exceeds Clippy's threshold (7),
introduce a named grouped struct to batch related scalar or reference parameters:

```rust
struct ProcessDiscoveryArgs<'a> {
    package_identifier: &'a str,
    name: &'a str,
    installed_version: &'a str,
    plugin_type_str: &'a str,
}

async fn process_one_discovery(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    host_id: Uuid,
    plugin_config_id: Uuid,
    args: ProcessDiscoveryArgs<'_>,   // replaces 4 separate parameters
    ignore_set: &HashSet<String>,
    now: OffsetDateTime,
) -> Result<()> { ... }
```

Name the struct after its semantic role (`ProcessDiscoveryArgs`, `CreateServiceArgs`), not a generic label like
`Params`. The struct should be private to the module unless it is part of a public API.

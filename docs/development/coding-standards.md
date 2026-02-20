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

## Public Enum Extensibility

All public enums that may gain new variants must carry the `#[non_exhaustive]` attribute. This allows the project to evolve without semver-breaking
changes for downstream consumers. External crates matching on these enums must include a wildcard `_ =>` arm.

Enums currently annotated with `#[non_exhaustive]`:

- `ProviderType` (`shared-types`)
- `ServiceMessage`, `ControllerMessage` (`wire`)
- `ProviderCapability` (`provider-core`)

When adding a new public enum, apply `#[non_exhaustive]` by default unless the enum is explicitly guaranteed to be closed (e.g., a two-variant
boolean-like enum).

## Design Principles

- Keep every boundary clear: the controller orchestrates scheduling, upstream checks, API/UI; the MQTT service handles MQTT/Home Assistant
  integration; agents manage installed versions and update execution; providers focus on version detection/updating logic.
- Treat custom scripts and command execution paths as handling untrusted input; validate before execution to avoid shell injection.
- Agent connections are outbound-only, unprivileged, and rely on explicit sudo allowlists for privileged update commands.
- Logs should contain operational summaries only; do not store full command output internally.
- New protected routes must check the typed `Permission` enum rather than comparing raw role names.
- Document every behavioral change either in code or via an external docs page (e.g., update `docs/api` or `docs/development`).

Refer to [docs/security/secure-development.md](../security/secure-development.md) when the change touches PKI, secrets, reverse proxies, or filesystem
security.

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

The full error handling reference (19 patterns, anti-patterns, decision table, approved exceptions, and rules summary)
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
non-empty, exactly one of provider_config_id/provider_config | | `CreateProviderConfigRequest` | name non-empty |

See also: the `update_hooks.rs` module provides a similar validation pattern (`HookValidationError`) for hook configuration types.

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
`pending_device_flow` | `status` | `DeviceAuthStatus` | | `service` | `service_type` | `ServiceType` | | `service` | `status` | `ServiceStatus` | |
`update_history` | `status` | `UpdateStatus` |

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

# Coding Standards

## Error Handling - Overview

- Wrap errors in `rootcause::Report` and define a `Result<T>` alias per boundary (e.g.
  `pub type Result<T> = std::result::Result<T, Report<MyError>>`).
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

## Error Handling - Detailed

Use [`rootcause`](https://github.com/rootcause-rs/rootcause) for error propagation and [`thiserror`](https://github.com/dtolnay/thiserror) for error
enum definition. Every module boundary must define its own error type following the patterns below.

### Import convention

Prefer importing the `rootcause::prelude` module. It provides `Report`, `markers`, `report!`, `bail!`, `ResultExt` (for `.context()`, `.context_to()`,
and `.context_transform()`), `IteratorExt` (for `collect_reports()`), `handlers` (report handler configuration), and `IntoRootcause` (for converting
plain `Result<T, E>` into `Result<T, Report<E>>`). When implementing `ReportConversion`, use the `impl_report_conversion!` macro from
`uptrakit-shared-macros` (it handles the `ReportConversion` import internally):

```rust
use rootcause::prelude::*;
use thiserror::Error;
use uptrakit_shared_macros::impl_report_conversion;
```

### Pattern 1: Define an error enum with a `Result<T>` alias

Each boundary (crate, module, or logical subsystem) defines its own error enum and a `Result` alias using `Report`:

```rust
use rootcause::prelude::*;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum MyError {
    #[error("database error: {0}")]
    Database(#[from] sea_orm::DbErr),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("internal error: {0}")]
    Internal(String),
}

pub type Result<T> = std::result::Result<T, Report<MyError>>;
```

Real example: [`crates/ui/web-api/src/auth/error.rs`](crates/ui/web-api/src/auth/error.rs) (`AuthError`),
[`crates/core/controller/src/db/error.rs`](crates/core/controller/src/db/error.rs) (`DbError`).

### Pattern 2: Implement `ReportConversion` for cross-boundary error conversion

When your module calls code that returns a different error type, implement `ReportConversion` so that `.context_to()` can convert automatically. Use
the `impl_report_conversion!` macro from `uptrakit-shared-macros`:

```rust
use uptrakit_shared_macros::impl_report_conversion;

// Simple variant mapping (source error maps directly via #[from]):
impl_report_conversion!(sea_orm::DbErr => MyError::Database);

// Multiple conversions in one block:
impl_report_conversion! {
    sea_orm::DbErr => MyError::Database,
    std::io::Error => MyError::Io,
}

// Closure-based for errors that don't map directly (e.g. Box wrapping):
impl_report_conversion!(tungstenite::Error => MyError, |e| MyError::WebSocket(Box::new(e)));
```

Each macro invocation expands to a full `impl<T> ReportConversion<Source, markers::Mutable, T> for Target` block with the appropriate
`context_transform` call.

### Pattern 3: Use `context_to()` in function bodies

Call `.context_to()?` on any `Result` whose error type has a `ReportConversion` impl for your boundary:

```rust
let user = users::Entity::find_by_id(id)
    .one(db)
    .await
    .context_to()?           // converts sea_orm::DbErr → MyError::Database
    .ok_or_else(|| report!(MyError::NotFound(format!("user {id}"))))?;
```

### Pattern 4: Use `report!()` to create reports inside combinators

`report!()` creates a `Report` value without returning. Use it inside `.ok_or_else()`, `.map_err()`, or when building a `Report` to store in a
variable:

```rust
let user = results
    .ok_or_else(|| report!(MyError::NotFound("item not found".to_string())))?;
```

### Pattern 5: Adding parent context with `.context()`

Used when wrapping a low-level error with a higher-level description. Creates a parent node in the error tree:

```rust
db::connect(&db_config.url)
    .await
    .context(AppError::Database)?;
```

### Pattern 6: `context_transform()` with closures

For non-`#[from]` error conversions where you need to compute the target variant:

```rust
hostname::get()
    .context_transform(|e| PkiError::Hostname(e.to_string()))?;
```

Unlike `.context()` which creates a parent node, `.context_transform()` replaces the context type in place (single-node structure).

### Pattern 7: `map_err` with `report!()` for one-off conversions

When there's no `ReportConversion` impl and adding one isn't justified:

```rust
serde_json::from_str(json_str)
    .map_err(|e| report!(CliError::Other(format!("Invalid JSON: {e}"))))?;
```

### Pattern 8: Error inspection with `.current_context()`

Pattern-match on typed `Report` errors for semantic handling (e.g., retry logic):

```rust
if let Err(e) = operation().await {
    match e.current_context() {
        MyError::Transient => retry(),
        MyError::Fatal(msg) => return Err(e),
    }
}
```

### Pattern 9: `bail!()` for early error returns

`bail!(ErrorEnum::Variant(...))` is sugar for `return Err(report!(...))`. Use `bail!()` for guard-clause early returns. Use `report!()` only inside
`.ok_or_else()`, `.map_err()`, or when building a `Report` without returning:

```rust
fn validate(input: &str) -> Result<()> {
    if input.is_empty() {
        bail!(MyError::Validation("input must not be empty".into()));
    }
    Ok(())
}
```

### Pattern 10: Decision guide — which context method to use

| Scenario | Method | Effect | | --- | --- | --- | | Foreign error has `ReportConversion` impl | `.context_to()` | Delegates to impl | | Wrap
low-level error with high-level meaning | `.context(Higher::Variant)` | New parent node | | Change error type in-place (1:1 mapping) |
`.context_transform(\|e\| ...)` | Replace context, preserve children | | One-off, no conversion impl | `.map_err(\|e\| report!(...))` | Manual wrap |
| Guard clause / early return | `bail!(...)` | Return immediately |

### Pattern 11: Custom error helper methods

Define helper methods on error enums for semantic classification that callers match on for retry/reconnect logic:

```rust
impl MyError {
    pub fn is_transient(&self) -> bool {
        matches!(self, Self::ConnectionReset | Self::Timeout)
    }
}
```

Real example: `crates/core/agent/src/error.rs` defines `is_receive_closed()` and `is_cert_expired()` for reconnect decision logic.

### Pattern 12: External crates without `std::error::Error`

When a source error type does not implement `std::error::Error` (e.g. `aws_lc_rs::Unspecified`, certain `rcgen` errors), string-based variants are
acceptable. Use `.map_err(|e| report!(Err::Variant(e.to_string())))`:

```rust
UnboundKey::new(&AES_256_GCM, key_bytes)
    .map_err(|e| report!(CryptoError::KeyCreation(e.to_string())))?;
```

### Pattern 13: Fixed-signature trait boundaries (SeaORM)

SeaORM trait impls (`ValueType`, `TryGetable`) have mandated return types (`ValueTypeErr`, `TryGetError`). At these boundaries, convert typed errors
via `.map_err()` to the required SeaORM error type. Internally, the helper functions still use `Report<CryptoError>`:

```rust
impl sea_orm::sea_query::ValueType for EncryptedString {
    fn try_from(v: sea_orm::Value) -> std::result::Result<Self, ValueTypeErr> {
        // decrypt_value() returns Result<String, Report<CryptoError>>
        let plaintext = decrypt_value(&s).map_err(|_| ValueTypeErr)?;
        Ok(EncryptedString::from_db(plaintext, s))
    }
}
```

Note: `EncryptedString::new()` is fallible — it encrypts eagerly at construction time and returns `Result<Self, Report<CryptoError>>`. Callers must
propagate the error via `.context_to()?`.

### Pattern 14: Clap `value_parser` functions

Clap's `#[arg(value_parser = ...)]` attribute requires `Result<T, String>`. Functions used as clap value parsers (e.g. `parse_pki_addr`,
`parse_proxy`, `parsed_url`) are the only place where `Result<T, String>` is acceptable:

```rust
fn parse_my_value(s: &str) -> Result<MyType, String> {
    // clap API mandates this signature
    s.parse::<MyType>().map_err(|e| e.to_string())
}
```

### Pattern 15: HTTP input validation helpers

Thin validation functions that produce user-facing HTTP 400 error messages may return `Result<T, String>` when the string goes directly into an HTTP
error response (e.g. `validate_provider_config_request`, `validate_homebrew_package_identifier`). These are display-only error strings, not propagated
errors.

### Pattern 16: Display fallbacks

`unwrap_or_else` / `unwrap_or_default` used for non-critical display formatting (e.g. pretty-printing JSON with a `to_string()` fallback) are not
error propagation and do not need typed errors:

```rust
let pretty = serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string());
```

### Anti-patterns

These are error handling patterns that MUST NOT be used:

- **`Result<T, String>`** — always define a typed error enum with `Report<E>`. Approved exceptions: Clap value parsers (Pattern 14), HTTP input
  validation helpers (Pattern 15).
- **`Result<T, (StatusCode, &str)>`** — use typed errors; map to HTTP status at the handler level.
- **Reusing unrelated error variants** (e.g. `PkiError::Hostname` for a database error) — add a new variant.
- **`format!("error: {e}")` losing the error chain** — use `#[from]`, `context_transform()`, or `context_to()` to preserve the original error.
- **Bare error enums without `Report`** — every boundary error type should use `pub type Result<T> = std::result::Result<T, Report<MyError>>`.
- **`return Err(report!(...))`** — use `bail!(...)` instead for early returns.

### Pattern 17: Batch error collection with `IteratorExt`

The `IteratorExt` trait (from `rootcause::prelude`) provides `collect_reports()` for collecting results from iterators where individual items may
fail. Instead of short-circuiting on the first error, `collect_reports()` accumulates all successes and all failures, allowing batch validation
scenarios to report every problem at once:

```rust
let (successes, errors): (Vec<_>, Vec<_>) = items
    .into_iter()
    .map(|item| validate(item))
    .collect_reports();
```

### Pattern 18: Orphan rule limitation for `IntoResponse`

Due to Rust's orphan rule, downstream crates cannot implement `impl IntoResponse for Report<E>` because both `IntoResponse` (from `axum`) and `Report`
(from `rootcause`) are foreign types. At HTTP handler call sites, handle errors inline instead:

```rust
pub async fn my_handler(State(state): State<AppState>) -> impl IntoResponse {
    match do_work(&state).await {
        Ok(result) => Json(result).into_response(),
        Err(report) => {
            tracing::error!(error = %report, "operation failed");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}
```

### Pattern 19: Error assertion in tests with `current_context()`

Use `current_context()` with `matches!()` to assert specific error variants in tests without matching on internal string messages:

```rust
#[test]
fn rejects_empty_input() {
    let err = parse("").unwrap_err();
    assert!(
        matches!(err.current_context(), MyError::Validation(_)),
        "expected Validation error, got: {err}"
    );
}
```

This is more resilient than string matching because it survives error message changes.

### Mutex and RwLock locks

The release profile uses `panic = "abort"`, so lock poisoning **cannot occur in production**. `.unwrap()` is allowed on `Mutex::lock()`,
`RwLock::read()`, and `RwLock::write()`:

```rust
let guard = store.lock().unwrap();
```

Do NOT use `.map_err()` to convert `PoisonError` into an application error — this adds unnecessary complexity since poisoning is impossible with
`panic = "abort"`.

### Rules summary

1. **Every boundary has its own error enum.** Do not reuse error types across crate boundaries.
1. **Derive `Debug` and `Error`** (via thiserror) on all error enums.
1. **Use structured context** -- prefer typed variants (`NotFound(String)`) over generic string errors.
1. **No secrets in error messages.** Never include tokens, passwords, keys, or credentials.
1. **Use `SecretString` for all secret fields in API types.** Any field in `uptrakit-web-api-types` that carries a password, token, client secret, or
   other credential must use `SecretString` (from `uptrakit-shared-types`, re-exported by `uptrakit-web-api-types`). This prevents accidental exposure
   through `Debug` output and log messages. Consumers access the inner value via `.expose_secret()` and construct via `SecretString::new(...)`. See
   [Secrets Handling](../security/secrets-and-encryption.md) for details.
1. **Use `Report<MyError>` as the error type**, not bare `MyError`. The `Result<T>` alias enforces this.
1. **Implement `ReportConversion`** (via `impl_report_conversion!` macro) for every foreign error type your boundary may encounter.

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

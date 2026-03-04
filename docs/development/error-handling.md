# Error Handling

Use [`rootcause`](https://github.com/rootcause-rs/rootcause) for error propagation and [`thiserror`](https://github.com/dtolnay/thiserror) for error
enum definition. Every module boundary must define its own error type following the patterns below.

For security implications of error handling (secret redaction, logging), see
[Secure Development](../security/secure-development.md).

## Import Convention

Prefer importing the `rootcause::prelude` module. It provides `Report`, `markers`, `report!`, `bail!`, `ResultExt` (for `.context()`, `.context_to()`,
and `.context_transform()`), `IteratorExt` (for `collect_reports()`), `handlers` (report handler configuration), and `IntoRootcause` (for converting
plain `Result<T, E>` into `Result<T, Report<E>>`). When implementing `ReportConversion`, use the `impl_report_conversion!` macro from
`uptrakit-shared-macros` (it handles the `ReportConversion` import internally):

```rust
use rootcause::prelude::*;
use thiserror::Error;
use uptrakit_shared_macros::impl_report_conversion;
```

`ReportConversion` is **not** re-exported from the prelude. Always use the `impl_report_conversion!` macro instead of
importing `ReportConversion` manually.

Use `report!()` to create `Report` values; do **not** call `Report::new()` directly.

**`#[from]` vs `impl_report_conversion!`:** `#[from]` on an error variant provides `impl From<SourceError> for
MyError`, which the standard `?` operator uses. When the function return type is `Result<T, Report<MyError>>`,
bare `?` can no longer call that `From` impl — use `.context_to()?` instead. Prefer `impl_report_conversion!`
over `#[from]` and **omit** `#[from]` on variants whose only callers use `.context_to()?`. Having both
`#[from]` and `impl_report_conversion!` on the same variant is dead code: the `From` impl is never called.

## Error Chain Structure

`rootcause::Report` builds a tree of context nodes. Two methods modify the chain differently:

```text
.context(HigherError)            .context_transform(|e| NewError)
========================         ================================

  ┌─────────────────┐              ┌─────────────────┐
  │  HigherError    │  ← parent    │   NewError       │  ← replaced
  └────────┬────────┘              └─────────────────┘
           │                       (original context removed,
  ┌────────┴────────┐               children preserved)
  │  OriginalError  │  ← child
  └─────────────────┘

.context()  → adds a NEW parent node; original stays as child.
.context_transform()  → REPLACES the context type in-place.
```

Use `.context()` when you want to add semantic meaning on top of the original error (e.g., "database error" wrapping a
`DbErr`). Use `.context_transform()` when you want a 1:1 type change without adding a nesting level.

## Error Flow Across Boundaries

Errors propagate from plugin crates through the controller into HTTP responses:

```text
Plugin crate                    Controller / Web-API              HTTP handler
────────────                    ──────────────────                ────────────
PluginError                     AgentRouteError                   axum::Response
 ├─ Configuration(String)        ├─ Database(DbErr)                ├─ 400 Bad Request
 ├─ Network(String)              ├─ BadRequest(String)             ├─ 404 Not Found
 └─ PluginInternal(String)       ├─ NotFound(String)               ├─ 500 Internal Error
                                 └─ Plugin(String)                 └─ (JSON body)
       │                                │                                │
       │  impl_report_conversion!       │  match on Report               │
       │  + .context_to()?              │  + error_response()            │
       └────────────────────────────────┘────────────────────────────────┘
```

Each boundary owns its error enum and `Result<T>` alias. Cross-boundary conversion is handled by
`impl_report_conversion!` and `.context_to()`. At the HTTP boundary, the orphan rule prevents
`impl IntoResponse for Report<E>`, so errors are handled inline (see Pattern 18).

## Complete Real-World Example

A minimal but complete boundary showing all pieces together:

```rust
use rootcause::prelude::*;
use thiserror::Error;
use uptrakit_shared_macros::impl_report_conversion;

// 1. Define the error enum
#[derive(Debug, Error)]
pub enum WidgetError {
    #[error("database error: {0}")]
    Database(sea_orm::DbErr),

    #[error("widget not found: {0}")]
    NotFound(uuid::Uuid),

    #[error("validation failed: {0}")]
    Validation(String),
}

// 2. Define the Result alias
pub type Result<T> = std::result::Result<T, Report<WidgetError>>;

// 3. Register cross-boundary conversions
impl_report_conversion!(sea_orm::DbErr => WidgetError::Database);

// 4. Use in functions
pub async fn get_widget(db: &DatabaseConnection, id: uuid::Uuid) -> Result<Widget> {
    let widget = Widget::find_by_id(id)
        .one(db)
        .await
        .context_to()?                          // DbErr → WidgetError::Database
        .ok_or_else(|| report!(WidgetError::NotFound(id)))?;
    Ok(widget)
}

pub async fn create_widget(db: &DatabaseConnection, name: &str) -> Result<Widget> {
    if name.is_empty() {
        bail!(WidgetError::Validation("name must not be empty".into()));
    }
    // ... insert logic using .context_to()? ...
    # Ok(todo!())
}
```

## Error Message Style Guide

Follow these conventions for error messages in `#[error("...")]` attributes:

- **Lowercase first letter** — `"database error: {0}"`, not `"Database error: {0}"`.
- **No trailing punctuation** — `"widget not found"`, not `"widget not found."`.
- **Structured context over free-form strings** — prefer typed variants (`NotFound(Uuid)`) over
  `Internal(String)` where possible.
- **Include the failing value** — `"widget not found: {id}"` is more useful than `"widget not found"`.
- **Never include secrets** — no tokens, passwords, API keys, or credentials in error messages. Use `SecretString` for
  sensitive fields. See [Secrets Handling](../security/secrets-and-encryption.md).

## Patterns Reference

### Pattern 1: Define an error enum with a `Result<T>` alias

Each boundary (crate, module, or logical subsystem) defines its own error enum and a `Result` alias using `Report`:

```rust
use rootcause::prelude::*;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum MyError {
    #[error("database error: {0}")]
    Database(sea_orm::DbErr),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("internal error: {0}")]
    Internal(String),
}

pub type Result<T> = std::result::Result<T, Report<MyError>>;
```

Real example: [`crates/ui/web-api-auth/src/auth/error.rs`](../../crates/ui/web-api-auth/src/auth/error.rs) (`AuthError`),
[`crates/core/controller/src/db/error.rs`](../../crates/core/controller/src/db/error.rs) (`DbError`).

### Pattern 2: Implement `ReportConversion` for cross-boundary error conversion

When your module calls code that returns a different error type, implement `ReportConversion` so that `.context_to()`
can convert automatically. Use the `impl_report_conversion!` macro from `uptrakit-shared-macros`:

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

// Closure-based with conditional mapping (inspects source variant to choose target variant):
impl_report_conversion!(EnrollmentError => LoopError, |e| {
    if e.is_cert_expired() {
        LoopError::CertExpired
    } else if e.is_receive_closed() {
        LoopError::ReceiveClosed
    } else {
        LoopError::Other(e.to_string())
    }
});
```

Each macro invocation expands to a full `impl<T> ReportConversion<Source, markers::Mutable, T> for Target` block with
the appropriate `context_transform` call.

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

`report!()` creates a `Report` value without returning. Use it inside `.ok_or_else()`, `.map_err()`, or when building
a `Report` to store in a variable:

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

Unlike `.context()` which creates a parent node, `.context_transform()` replaces the context type in place
(single-node structure).

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

`bail!(ErrorEnum::Variant(...))` is sugar for `return Err(report!(...))`. Use `bail!()` for guard-clause early
returns. Use `report!()` only inside `.ok_or_else()`, `.map_err()`, or when building a `Report` without returning:

```rust
fn validate(input: &str) -> Result<()> {
    if input.is_empty() {
        bail!(MyError::Validation("input must not be empty".into()));
    }
    Ok(())
}
```

### Pattern 10: Decision guide -- which context method to use

| Scenario | Method | Effect |
| --- | --- | --- |
| Foreign error has `ReportConversion` impl | `.context_to()` | Delegates to impl |
| Wrap low-level error with high-level meaning | `.context(Higher::Variant)` | New parent node |
| Change error type in-place (1:1 mapping) | `.context_transform(\|e\| ...)` | Replace context, preserve children |
| One-off conversion, no impl needed | `.map_err(\|e\| report!(...))` | Manual wrap |
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

Real example: `crates/core/agent/src/error.rs` defines `is_receive_closed()` and `is_cert_expired()` for reconnect
decision logic.

### Pattern 12: External crates without `std::error::Error`

When a source error type does not implement `std::error::Error` (e.g. `aws_lc_rs::Unspecified`, certain `rcgen`
errors), string-based variants are acceptable. Use `.map_err(|e| report!(Err::Variant(e.to_string())))`:

```rust
UnboundKey::new(&AES_256_GCM, key_bytes)
    .map_err(|e| report!(CryptoError::KeyCreation(e.to_string())))?;
```

### Pattern 13: Fixed-signature trait boundaries (SeaORM)

SeaORM trait impls (`ValueType`, `TryGetable`) have mandated return types (`ValueTypeErr`, `TryGetError`). At these
boundaries, convert typed errors via `.map_err()` to the required SeaORM error type. Internally, the helper functions
still use `Report<CryptoError>`:

```rust
impl sea_orm::sea_query::ValueType for EncryptedString {
    fn try_from(v: sea_orm::Value) -> std::result::Result<Self, ValueTypeErr> {
        // decrypt_value() returns Result<String, Report<CryptoError>>
        let plaintext = decrypt_value(&s).map_err(|_| ValueTypeErr)?;
        Ok(EncryptedString::from_db(plaintext, s))
    }
}
```

Note: `EncryptedString::new()` is fallible -- it encrypts eagerly at construction time and returns
`Result<Self, Report<CryptoError>>`. Callers must propagate the error via `.context_to()?`.

### Pattern 14: Clap `value_parser` functions

Clap's `#[arg(value_parser = ...)]` attribute requires `Result<T, String>`. Functions used as clap value parsers
(e.g. `parse_pki_addr`, `parse_proxy`, `parsed_url`) are the only place where `Result<T, String>` is acceptable:

```rust
fn parse_my_value(s: &str) -> Result<MyType, String> {
    // clap API mandates this signature
    s.parse::<MyType>().map_err(|e| e.to_string())
}
```

### Pattern 15: HTTP input validation helpers

Thin validation functions that produce user-facing HTTP 400 error messages may return `Result<T, String>` when the
string goes directly into an HTTP error response (e.g. `validate_plugin_config_request`,
`validate_homebrew_package_identifier`). These are display-only error strings, not propagated errors.

### Pattern 16: Display fallbacks

`unwrap_or_else` / `unwrap_or_default` used for non-critical display formatting (e.g. pretty-printing JSON with a
`to_string()` fallback) are not error propagation and do not need typed errors:

```rust
let pretty = serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string());
```

### Pattern 17: Batch error collection with `IteratorExt`

The `IteratorExt` trait (from `rootcause::prelude`) provides `collect_reports()` for collecting results from iterators
where individual items may fail. Instead of short-circuiting on the first error, `collect_reports()` accumulates all
successes and all failures, allowing batch validation scenarios to report every problem at once:

```rust
let (successes, errors): (Vec<_>, Vec<_>) = items
    .into_iter()
    .map(|item| validate(item))
    .collect_reports();
```

### Pattern 18: Orphan rule limitation for `IntoResponse`

Due to Rust's orphan rule, downstream crates cannot implement `impl IntoResponse for Report<E>` because both
`IntoResponse` (from `axum`) and `Report` (from `rootcause`) are foreign types. At HTTP handler call sites, handle
errors inline instead:

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

Use `current_context()` with `matches!()` to assert specific error variants in tests without matching on internal
string messages:

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

### Pattern 20: DB errors in authentication and authorization handlers

When a database query determines an authentication or authorization outcome, propagate DB
failures as a 500 Internal Server Error. **Never silently substitute a default on DB failure.**

Two dangerous anti-patterns:

```rust
// ✗ Wrong — DB outage silently grants empty permissions → 403 Forbidden instead of 500
let permissions = get_user_permissions(db, user_id)
    .await
    .unwrap_or_default();

// ✗ Wrong — DB outage returns 0 users, allowing unintended first-admin OIDC registration
let user_count = User::find().count(db).await.unwrap_or(false);
```

The correct pattern propagates the error:

```rust
// ✓ Correct — DB outage becomes 500; access is never silently granted or denied
let permissions = get_user_permissions(db, user_id)
    .await
    .map_err(|e| {
        tracing::error!(err = %e, user_id = %user_id, "failed to load user permissions");
        AuthFailure::InternalError
    })?;
```

This rule applies to all queries whose result controls access decisions:

- Loading user permissions (affects 403/200 outcome)
- Counting users for first-admin detection (affects whether new admin registration is allowed)
- Querying revocation lists or session validity

Outside of auth paths, `unwrap_or_default()` on non-critical display queries (e.g. a count
displayed in a dashboard) is covered by Pattern 16.

## Mutex and RwLock Locks

The release profile uses `panic = "abort"`, so lock poisoning **cannot occur in production**. `.unwrap()` is allowed
on `Mutex::lock()`, `RwLock::read()`, and `RwLock::write()`:

```rust
let guard = store.lock().unwrap();
```

Do NOT use `.map_err()` to convert `PoisonError` into an application error -- this adds unnecessary complexity since
poisoning is impossible with `panic = "abort"`.

## Anti-Patterns

These are error handling patterns that MUST NOT be used:

- **`Result<T, String>`** -- always define a typed error enum with `Report<E>`. Approved exceptions: Clap value
  parsers (Pattern 14), HTTP input validation helpers (Pattern 15).
- **`Result<T, (StatusCode, &str)>`** -- use typed errors; map to HTTP status at the handler level.
- **Reusing unrelated error variants** (e.g. `PkiError::Hostname` for a database error) -- add a new variant.
- **`format!("error: {e}")` losing the error chain** -- use `#[from]`, `context_transform()`, or `context_to()` to
  preserve the original error.
- **Bare error enums without `Report`** -- every boundary error type should use
  `pub type Result<T> = std::result::Result<T, Report<MyError>>`.
- **`return Err(report!(...))`** -- use `bail!(...)` instead for early returns.
- **`Report::new()`** -- use the `report!()` macro instead for consistent error creation.

## Approved Exceptions

| Exception | Reason |
| --- | --- |
| `Mutex::lock().unwrap()` / `RwLock::{read,write}().unwrap()` | `panic = "abort"` in release makes poisoning impossible |
| String-based variants for types without `std::error::Error` | e.g. `aws_lc_rs::Unspecified`, certain `rcgen` errors (Pattern 12) |
| `Result<T, String>` in clap value parsers | Clap API mandates this signature (Pattern 14) |
| `Result<T, String>` in HTTP validation helpers | Display-only strings for HTTP 400 responses (Pattern 15) |
| Display fallbacks (`unwrap_or_else` / `unwrap_or_default`) | Non-critical formatting, not error propagation (Pattern 16) |

## Rules Summary

1. **Every boundary has its own error enum.** Do not reuse error types across crate boundaries.
1. **Derive `Debug` and `Error`** (via thiserror) on all error enums.
1. **Use structured context** -- prefer typed variants (`NotFound(String)`) over generic string errors.
1. **No secrets in error messages.** Never include tokens, passwords, keys, or credentials. See
   [Secrets Handling](../security/secrets-and-encryption.md).
1. **Use `SecretString` for all secret fields in API types.** Any field in `uptrakit-web-api-types` that carries a
   password, token, client secret, or other credential must use `SecretString` (from `uptrakit-shared-types`,
   re-exported by `uptrakit-web-api-types`). This prevents accidental exposure through `Debug` output and log messages.
   Consumers access the inner value via `.expose_secret()` and construct via `SecretString::new(...)`. See
   [Secrets Handling](../security/secrets-and-encryption.md) for details.
1. **Use `Report<MyError>` as the error type**, not bare `MyError`. The `Result<T>` alias enforces this.
1. **Implement `ReportConversion`** (via `impl_report_conversion!` macro) for every foreign error type your boundary
   may encounter.

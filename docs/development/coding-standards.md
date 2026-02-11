# Coding Standards

## Error Handling - Overview

- Wrap errors in `rootcause::Report` and define a `Result<T>` alias per boundary (e.g. `pub type Result<T> = Report<MyError>`).
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

## Error Handling - Detailed

Use [`rootcause`](https://github.com/rootcause-rs/rootcause) for error propagation and
[`thiserror`](https://github.com/dtolnay/thiserror) for error enum definition. Every module boundary must define its own
error type following the patterns below.

### Import convention

Prefer importing the `rootcause::prelude` module. It provides `Report`, `markers`, `report!`, `bail!`, `ResultExt` (for
`.context()` and `.context_to()`), and `IteratorExt`. When implementing `ReportConversion`, use the
`impl_report_conversion!` macro from `uptrakit-shared-macros` (it handles the `ReportConversion` import internally):

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

### Pattern 4: Use `report!()` to create reports directly

```rust
return Err(report!(MyError::NotFound("item not found".to_string())));
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

Unlike `.context()` which creates a parent node, `.context_transform()` replaces the context type in place (single-node
structure).

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

### Anti-patterns

These are error handling patterns that MUST NOT be used:

- **`Result<T, String>`** — always define a typed error enum with `Report<E>`.
- **`Result<T, (StatusCode, &str)>`** — use typed errors; map to HTTP status at the handler level.
- **Reusing unrelated error variants** (e.g. `PkiError::Hostname` for a database error) — add a new variant.
- **`format!("error: {e}")` losing the error chain** — use `#[from]`, `context_transform()`, or `context_to()` to
  preserve the original error.
- **Bare error enums without `Report`** — every boundary error type should use
  `pub type Result<T> = std::result::Result<T, Report<MyError>>`.

### Mutex and RwLock locks

The release profile uses `panic = "abort"`, so lock poisoning **cannot occur in production**. `.unwrap()` is allowed on
`Mutex::lock()`, `RwLock::read()`, and `RwLock::write()`:

```rust
let guard = store.lock().unwrap();
```

Do NOT use `.map_err()` to convert `PoisonError` into an application error — this adds unnecessary complexity since
poisoning is impossible with `panic = "abort"`.

### Rules summary

1. **Every boundary has its own error enum.** Do not reuse error types across crate boundaries.
1. **Derive `Debug` and `Error`** (via thiserror) on all error enums.
1. **Use structured context** -- prefer typed variants (`NotFound(String)`) over generic string errors.
1. **No secrets in error messages.** Never include tokens, passwords, keys, or credentials.
1. **Use `Report<MyError>` as the error type**, not bare `MyError`. The `Result<T>` alias enforces this.
1. **Implement `ReportConversion`** (via `impl_report_conversion!` macro) for every foreign error type your boundary may
   encounter.

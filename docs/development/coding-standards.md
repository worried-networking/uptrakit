# Coding Standards

For maintainability-focused Rust conventions beyond the hard rules in this file, see [Rust Idioms](rust-idioms.md).

## Error Handling

For comprehensive error handling patterns, conventions, and the full decision guide, see [Error Handling](error-handling.md). Key points:

- Wrap errors in `rootcause::Report` and define a `Result<T>` alias per boundary.
- Use `thiserror::Error` with `#[derive(Debug, Error)]` to describe failures.
- Implement `ReportConversion` (via `impl_report_conversion!`) for all downstream errors and prefer `.context_to()?` to preserve the chain.
- Use `report!(MyError::Variant(…))` for creating new error reports. Never call `rootcause::Report::new(…)` directly — the macro additionally captures
  source location.
- Use `bail!(MyError::Variant(…))` for early returns.
- Avoid `Result<T, String>`; prefer typed enums. **Exception:** in `web-api` route handlers and their private validation helpers, `Result<T, String>`
  is acceptable when the string is a user-facing error message that the caller maps to an HTTP error response (e.g., via
  `error_response(StatusCode::BAD_REQUEST, msg)`). This avoids `clippy::result_large_err` from returning `Response` directly and keeps validation
  helpers decoupled from HTTP types.
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

## Lint Suppression

Lint suppression is a last resort. Fix the code first; suppress only when the lint is a false positive or the fix would genuinely worsen readability
or correctness.

Use `#[expect(lint_name, reason = "...")]`, never `#[allow(lint_name)]`. The `reason` field is mandatory (`allow_attributes_without_reason = "deny"`).
When the lint stops firing at a site, the `#[expect]` becomes a compile error via `unfulfilled_lint_expectations` (promoted to error by
`warnings = "deny"`), so stale suppressions are caught automatically.

```rust
// ✓ Correct
#[expect(clippy::too_many_arguments, reason = "mirrors the eight DB columns of Update")]
fn create_update_record(…) { … }

// ✗ Wrong — no reason, and will silently persist if the lint is fixed
#[allow(clippy::too_many_arguments)]
fn create_update_record(…) { … }
```

When two lints fire on the same expression, list both in one attribute:

```rust
#[expect(clippy::unwrap_used, clippy::unwrap_in_result, reason = "infallible: regex compiled from a literal")]
let re = Regex::new(PATTERN).unwrap();
```

**Feature-gated items** (struct fields, modules, `let` bindings that are unused when a feature is disabled) use `#[cfg_attr]` so the suppression only
applies in the affected build variant:

```rust
pub(crate) struct Foo {
    #[cfg_attr(
        not(feature = "embedded"),
        expect(dead_code, reason = "field only accessed when the embedded feature is enabled")
    )]
    handle: Arc<EmbeddedHandle>,
}
```

**Feature-conditional expression sites** (where a `let` binding is only mutated under a `#[cfg(feature = "...")]` block) are also handled by
`#[cfg_attr]`. The suppression only exists in the build variant where the lint fires:

```rust
#[cfg_attr(
    not(feature = "interactive"),
    expect(unused_mut, reason = "mut only needed when interactive feature enables .take() calls below")
)]
let mut handle = start(…);
```

`clippy::allow_attributes` and `clippy::allow_attributes_without_reason` must **never** be suppressed. They exist specifically to catch bare
`#[allow]` attributes — suppressing the watchdog defeats the whole mechanism. If you believe you need a bare `#[allow]`, the correct fix is always
`#[cfg_attr(..., expect(...))]` or `#[expect(..., reason = "...")]`.

### Test-mode exemptions

`clippy.toml` enables `allow-unwrap-in-tests`, `allow-expect-in-tests`, `allow-panic-in-tests`, `allow-dbg-in-tests`, and
`allow-indexing-slicing-in-tests`. These exemptions cover **only functions annotated with `#[test]`** (or `#[tokio::test]`, etc.). Helper functions
inside `mod tests {}` blocks and integration-test helpers driven by macros like `db_test!` are **not** covered — they need an explicit
`#![expect(...)]` at the module top with a specific reason.

### Prefer refactor over suppression

Many suppressions can be eliminated by changing the code rather than annotating it. A few recurring substitutions:

- `args[0]` after a length check → `args.first()` with `let-else`
- `Vec<T>` plus `vec[..n]` slices when the length is fixed → `[T; N]` plus `split_at(n)`
- `let _ = result_expr` to discard a `Result` you knowingly ignore → `let _ignored = result_expr` (named bindings prefixed with `_` do not trigger
  `let_underscore_must_use` and convey intent)
- `Some(row).unwrap()` after a `.get()` guard → consume the guard's binding directly

When a suppression is the right call, write a **specific** reason: name the invariant ("`pos` from `str::find` is on a char boundary") or the guard
("idx came from `lines.iter().rposition(...)`"). Avoid generic placeholders like "in bounds" or "checked above" — they rot the moment surrounding code
shifts.

### Raw-SQL ban (`disallowed-methods` / `disallowed-macros`)

`clippy.toml` bans the raw-SQL entry points `sea_orm::Statement::from_string`,
`sea_orm::Statement::from_sql_and_values`, `sea_orm::ConnectionTrait::execute_unprepared`,
`sea_query::Expr::cust`, `sea_query::Expr::cust_with_values`, `sea_query::Expr::cust_with_expr`,
`sea_query::Expr::cust_with_exprs`, and the `sea_orm::raw_sql!` macro
(AGENTS.md rule "No raw SQL."). Consumers (`execute_raw`, `query_one_raw`, …) are deliberately
unbanned — banning the `Statement` sources chokes them all. Known lint-invisible gaps, also
deliberate: constructing `Statement` as a struct literal (all its fields are `pub`), the direct
sqlx query API, and driver-inherent `execute_unprepared` on the concrete `Sqlx*PoolConnection`
types — none occur in the workspace; the evasion grep in the quality-gate sweep is the backstop.
`Func::cust` is likewise deliberately unbanned: it names a custom SQL function as an iden while
all arguments stay builder-typed expressions — the idiomatic escape for functions sea-query
lacks, not a raw-fragment constructor.
Liveness probes use a builder `SELECT 1`
(`Query::select().expr(Expr::val(1))` sent via `query_one`), never
`execute_unprepared("SELECT 1")` — and not `DatabaseConnection::ping()`, which executes no SQL
on SQLite (a worker-thread liveness round-trip only) and is a strictly weaker check.

A site may opt out only with
`#[expect(clippy::disallowed_methods, reason = "<category>: <concrete limitation>")]`
(`clippy::disallowed_macros` for `raw_sql!`), where `<category>` is exactly one of:

1. **builder limitation** — SQL genuinely inexpressible in sea_query, wherever it occurs:
   SQLite `ALTER TABLE`/`PRAGMA` shapes (including `PRAGMA foreign_keys` toggles in test setup),
   `CREATE DATABASE`, window functions, functional indexes, `typeof()`, the SQLite
   table-recreation pattern. Detail: [database-migrations.md](database-migrations.md#no-raw-sql--use-sea_query-builders-for-dml).
2. **connectivity probe** — only where no builder query can be issued at all (the builder
   `SELECT 1` works over any `ConnectionTrait`, so this category is currently empty — prefer
   the builder form; the category exists for genuinely builder-less handles).
3. **test-only schema sabotage** — corrupting schema/data to exercise DB-failure paths, only
   where inexpressible via builders (e.g. `CREATE TRIGGER` fault injection). Plain `DROP TABLE`
   never qualifies — web-api tests use the `test_harness::fixtures::drop_table` builder helper.
4. **frozen merged migration** — raw SQL inside an `up()`/`down()` body merged to `main`,
   regardless of expressibility: merged migrations are treated as applied, and rewriting one
   risks live-vs-fresh-install divergence. Migration files' `#[cfg(test)]` halves are ordinary
   test code — rewrite-by-default. The frozen set is bounded by the union of category 1 and
   category 4 annotations inside migration `up()`/`down()` bodies (the shape rule routes
   `ALTER`/`PRAGMA`/window/functional-index SQL to category 1 even when frozen); the taxonomy
   gate (`ci/verify_raw_sql_expect_taxonomy.sh`) pins the category-4 count so it can only
   shrink without owner sign-off.

The reason must name the real limitation — never an unverified claim about a dependency's API;
reviewers verify the stated rationale independently. Granularity: statement-level by default;
fn-level only on free or plain-impl single-rationale migration helper fns that open no
transaction — never on an `#[async_trait]` trait method like `up()`/`down()` itself, where the
body moves into a generated `Box::pin` and `#[expect]` fulfillment is unverified
(`disallowed_methods`
also carries the `begin*` bans — a fn-level expect would mask a stray `.begin()`, and it
likewise swallows any future `disallowed-methods` entry: whoever adds a new ban entry must
re-audit the fn-level-expected migration fns); never file-level. There is no test exemption:
`disallowed-methods` has no `allow-*-in-tests` flag, so test code follows the same taxonomy.

Each ban entry has a canary in `crates/shared/db-tx`'s `#[cfg(test)]` canary module: if an
upgrade renames a banned path, the entry degrades to a config warning, but the canary's
`#[expect]` goes unfulfilled and `unfulfilled_lint_expectations = "deny"` fails the build.

## Shared Contract Crates

- Public fallible APIs in shared or reusable contract crates should document a `# Errors` section.
- When practical, touched shared or reusable crates should run `cargo clippy -p <crate> --all-targets -- -D clippy::missing_errors_doc` to keep that
  contract enforced.

## Error Masking Anti-Patterns

Never use `.unwrap_or(N)` or `.unwrap_or_default()` as a silent fallback for database errors. When the database is unavailable a fallback value
produces incorrect program behavior:

- **Security paths:** `count(db).await.unwrap_or(1) > 0` in a registration check treats a DB error as "user exists", silently blocking legitimate
  registrations or skipping the registration-token-required path.
- **Data-integrity guards:** `count_linked_hosts(db).await.unwrap_or(0)` treats a DB error as "no linked hosts", potentially allowing a soft-delete
  that would orphan active records.

### Required pattern — route handlers

Use a `match` and return `StatusCode::INTERNAL_SERVER_ERROR` on `Err`, logging the error at the `error` level:

```rust
// Handlers use focused sub-states (State<DbState>) rather than the full State<Arc<AppState>>.
// Access the connection via db.db() when using State<DbState>.
let has_user = match User::find()
    .filter(user::Column::Email.eq(&email))
    .count(db.db())
    .await
{
    Ok(n) => n > 0,
    Err(e) => {
        tracing::error!(err = %e, "DB error checking for duplicate user");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }
};
```

See [AppState Architecture](app-state.md) for the full sub-state pattern, service extractors, and `db_access_policy.toml` classification rules.

### Required pattern — query functions

Return `Result<T, DbErr>` (or a crate-local `Result`) and propagate errors with `?` at the call site. Never collapse errors into a default value:

```rust
// ✓ Correct — DB error is surfaced to the caller
async fn count_linked_hosts(
    db: &DatabaseConnection,
    item_id: Uuid,
) -> Result<u64, sea_orm::DbErr> {
    SoftwareItemHost::find()
        .filter(software_item_host::Column::SoftwareItemId.eq(item_id))
        .count(db)
        .await   // ? at call site propagates the error
}

// ✗ Wrong — DB outage silently returns 0, making a guard useless
async fn count_linked_hosts(db: &DatabaseConnection, item_id: Uuid) -> u64 {
    SoftwareItemHost::find()
        .filter(...)
        .count(db)
        .await
        .unwrap_or(0)
}
```

The narrower `unwrap_or(0)` rule for count queries in paginated list endpoints is documented in [Database Query Patterns](#database-query-patterns).
This section covers the broader class of `.unwrap_or(N)` misuse in security and data-integrity code paths.

See also: [Error Handling](error-handling.md).

## Public Enum Extensibility

All public enums that may gain new variants must carry the `#[non_exhaustive]` attribute. This allows the project to evolve without semver-breaking
changes for downstream consumers. External crates matching on these enums must include a wildcard `_ =>` arm.

Enums currently annotated with `#[non_exhaustive]`:

**`uptrakit-shared-types`:**

- `PluginTypeId` (newtype; replaces the former `PluginType` enum)
- `MqttTransport`
- `MqttClientConnectionStatus`
- `OutputStreamType`
- `DeviceAuthStatus`
- `ServiceStatus`
- `BatchStatus`
- `UpdateStatus`
- `RoleBundle`

**`uptrakit-wire`:**

- `CloseReason`
- `ServiceMessage`, `ControllerMessage`
- `EnrollmentStatus` (with `Other(String)` catch-all)
- `ErrorCode` (with `Other(String)` catch-all)
- `UpdateFinalStatus` (with `Other(String)` catch-all — loses `Copy`)
- `DisconnectReason` (with `Other(String)` catch-all — loses `Copy`)

**`uptrakit-web-api-types`:**

- `AlertSeverity`
- `TriggerUpdateStatus`
- `RegistrationMode`
- `NotificationEventType`, `NotificationDeliveryStatus`

**`uptrakit-plugin-infrastructure-core`:**

- `PluginCapability`
- `HostCompatibility`
- `PluginError`

When adding a new public enum, apply `#[non_exhaustive]` by default unless the enum is explicitly guaranteed to be closed (e.g., a two-variant
boolean-like enum).

## Exhaustive Enum Test Coverage

Enum tests (serde round-trips, `Display`/`FromStr` checks, `as_str()` invariants) must automatically cover every variant. A manually maintained array
like `const ALL_VARIANTS: [T; 4]` silently skips any new variant added later.

### Required pattern — `strum::EnumIter`

Use `#[cfg_attr(test, derive(strum::EnumIter))]` on the enum and call `T::iter()` inside the test:

```rust
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(all(test, not(feature = "sea-orm")), derive(strum::EnumIter))]
#[serde(rename_all = "snake_case")]
pub enum MyStatus { Pending, Active, Completed }

#[cfg(test)]
mod tests {
    use super::*;
    use strum::IntoEnumIterator;

    #[test]
    fn serde_round_trip() {
        for variant in MyStatus::iter() {
            let json = serde_json::to_string(&variant).unwrap();
            let back: MyStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(back, variant);
        }
    }
}
```

### cfg guards for sea-orm enums

Enums with `#[cfg_attr(feature = "sea-orm", derive(strum::EnumIter, ...))]` already derive `EnumIter` when the `sea-orm` feature is active. Adding
`#[cfg_attr(test, derive(strum::EnumIter))]` on top causes a duplicate implementation when both `cfg(test)` and `feature = "sea-orm"` are active
simultaneously. Use the combined guard:

```rust
#[cfg_attr(all(test, not(feature = "sea-orm")), derive(strum::EnumIter))]
```

This ensures:

- Tests run without `sea-orm` → the `cfg(test)` guard derives `EnumIter`.
- Tests run with `sea-orm` → the sea-orm feature already derives `EnumIter`; the guard is a no-op.

### cfg propagation caveat

`#[cfg(test)]` is **not** propagated to dependency crates. If crate B depends on crate A, the `#[cfg_attr(test, derive(strum::EnumIter))]` on an enum
in crate A will not make `EnumIter` available in crate B's test code. For enums in external crates, use inline arrays in tests (keeping them complete
by always listing all known variants explicitly).

### Anti-pattern — hardcoded const array

```rust
// ✗ Wrong — silently skips new variants; no compile-time enforcement
const ALL_STATUSES: [MyStatus; 3] = [MyStatus::Pending, MyStatus::Active, MyStatus::Completed];
for status in &ALL_STATUSES { ... }

// ✓ Correct
for status in MyStatus::iter() { ... }
```

### `strum::EnumIter` incompatibility with `Other(String)` variants

`strum::EnumIter` cannot be derived on enums that contain an `Other(String)` catch-all variant (see
[Wire-Safe `Other(String)` Catch-All](#wire-safe-otherstring-catch-all-for-enums) below). Instead, enumerate known variants explicitly in a `const`
array inside the test:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const KNOWN_VARIANTS: &[MyStatus] = &[
        MyStatus::Pending,
        MyStatus::Approved,
    ];

    #[test]
    fn serde_round_trip() {
        for variant in KNOWN_VARIANTS {
            // ...
        }
    }

    #[test]
    fn unknown_becomes_other() {
        let s = "\"future_variant\"";
        let v: MyStatus = serde_json::from_str(s).unwrap();
        assert!(matches!(v, MyStatus::Other(_)));
    }
}
```

## Wire-Safe `Other(String)` Catch-All for Enums

Any enum serialised over the wire (WebSocket, NATS, REST body) that may gain new variants in future releases **must** include an `Other(String)`
catch-all variant. This ensures rolling upgrades are safe: an older peer receiving an unknown variant from a newer peer deserialises it as
`Other("future_variant")` and handles it gracefully instead of returning a deserialization error.

### When to use

Apply this pattern to every `#[non_exhaustive]` enum that:

- is transmitted over a network protocol (`ServiceMessage`, `ControllerMessage`, `EnrollmentStatus`, `ErrorCode`, etc.),
- is returned as a JSON string in a REST API response (`NotificationEventType`, `NotificationDeliveryStatus`, etc.),
- or is persisted in a column and read back by potentially older software versions.

### Required implementation — use `wire_safe_enum!`

Use the `wire_safe_enum!` macro from `uptrakit-shared-macros` to generate all required boilerplate. The macro emits: `#[non_exhaustive]` +
`Other(String)`, `as_str()`, `Display`, `From<String>` (infallible with `tracing::debug!` on unknown), `Serialize`, `Deserialize`, a named parse-error
type, and strict `FromStr`.

```rust
use uptrakit_shared_macros::wire_safe_enum;

wire_safe_enum! {
    /// The status of a thing.
    #[derive(Clone, Debug, PartialEq, Eq, Hash)]
    pub enum MyStatus {
        Pending  => "pending",
        Approved => "approved",
    }
    parse_error = ParseMyStatusError("invalid my status");
}
```

This generates an enum equivalent to:

```rust
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum MyStatus {
    Pending,
    Approved,
    /// Unknown variant received from a newer peer or future schema version.
    Other(String),
}
// + as_str(), Display, From<String>, Serialize, Deserialize,
// + ParseMyStatusError, FromStr
```

For enums whose `From<String>` or `FromStr` impls require custom logic not expressible as a simple string table (e.g. infallible `FromStr` that maps
unknowns to a sentinel rather than `Err`), write the impls by hand following the pattern in `crates/shared/wire/src/lib.rs`.

### Consequences

- The enum **loses `Copy`** (because `String` is not `Copy`). Any call-site that relied on copy semantics must be updated to `.clone()`.
- `strum::EnumIter` cannot be derived. See the [test coverage section above](#strumenumiter-incompatibility-with-otherstring-variants).

## `#[non_exhaustive]` on Public Structs

`#[non_exhaustive]` applies to structs as well as enums. Add it to any public struct defined in a shared crate (`wire`, `shared-types`,
`web-api-types`, etc.) that may gain new fields in the future. This prevents external crates from using struct-literal syntax and breaks at compile
time if they try to match exhaustively.

### Required constructor

Because `#[non_exhaustive]` prevents external callers from constructing the struct with a literal, every `#[non_exhaustive]` struct **must** expose a
constructor or implement `Default`:

```rust
// In the shared crate (defining crate — struct literal is allowed here):
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PingPayload {
    pub service_ts: Timestamp,
}

impl PingPayload {
    pub fn new(service_ts: Timestamp) -> Self {
        Self { service_ts }
    }
}

// Empty structs use Default:
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RequestCrlRenewalPayload {}
```

### External crate callers

External crates constructing the struct must use the provided constructor:

```rust
// ✓ Correct — constructor provided by the shared crate
PingPayload::new(service_ts)

// ✗ Wrong — breaks when a new field is added
PingPayload { service_ts }
```

Pattern matching in external crates must use `..` to ignore unknown fields:

```rust
// ✓ Correct — forward-compatible
ServiceMessage::Ping(PingPayload { service_ts, .. }) => { ... }

// ✗ Wrong — breaks on new fields
ServiceMessage::Ping(PingPayload { service_ts }) => { ... }
```

## Typed Enum Parameters for Internal Write APIs

Internal query functions that write to the database should use typed enums instead of bare `&str` parameters for discriminator values such as actor
type, batch type, and similar classification fields. Bare strings produce no compile-time guarantees and make it trivial to introduce silent typos.

Define the typed enum in the relevant `queries` module and implement `as_str()` + `Display`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActorType {
    User,
    Mqtt,
    Scheduler,
}

impl ActorType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User      => "user",
            Self::Mqtt      => "mqtt",
            Self::Scheduler => "scheduler",
        }
    }
}

impl std::fmt::Display for ActorType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
```

When writing to the database, convert with `.as_str().to_string()` (not `format!("{self}")`):

```rust
actor_type: Set(params.actor_type.as_str().to_string()),
```

These enums are **internal** (not wire-protocol types) and therefore:

- do **not** need `#[non_exhaustive]` (they are exhaustively matched in the same crate),
- do **not** need `Other(String)` (they are never deserialised from untrusted input),
- **do** implement `Copy` (no heap allocation).

### Legacy on-disk spellings

`ActorType::Mqtt` returns `"uptrakit-mqtt"` (not `"mqtt"`) from `as_str()` for backwards compatibility with rows written by the MQTT Service before
the typed enum landed.

New code paths that classify Service-originated writes use `ActorType::from_service_app_name(...)`, which collapses every non-MQTT Service binary
(including `"uptrakit-agent-ssh"` and the registration fallback `"unknown"`) to `ActorType::Service` (`"service"`). The granular Service identity is
recoverable via the row's `actor_id` (the Service UUID) joined to `service.service_app_name`.

## Credential-Holding Types and Debug

Any internal struct that contains a credential (password, token, secret key, etc.) **must** store it as `SecretString` (not `String`). This enforces
the masking guarantee at the type level:

- `SecretString`'s `Debug` impl emits `"***"` automatically — no hand-written `Debug` needed.
- The value is zeroed from memory on drop (`ZeroizeOnDrop`).
- `.expose_secret()` is the only way to access the inner value, making every access site explicit and auditable.

```rust
// ✓ Correct — Debug is auto-derived; password never appears in logs
#[derive(Debug)]
struct SmtpSettings {
    host: String,
    password: Option<SecretString>,
}

// ✗ Wrong — Debug prints the password; requires a hand-written Debug impl that can drift
#[derive(Debug)]
struct SmtpSettings {
    host: String,
    password: Option<String>,
}
```

When passing the secret to an external API, call `.expose_secret()`:

```rust
if let Some(pw) = &config.password {
    mailer.set_password(pw.expose_secret());
}
```

`SecretString` is re-exported from `uptrakit_shared_types`. It requires no feature flags.

See also: [Secrets Handling and Encryption at Rest](../security/secrets-and-encryption.md#credential-holding-structs-and-debug).

## Feature Flags

All feature flags in this workspace are **additive** — enabling a feature adds functionality; it never removes or restricts code compiled without the
feature.

### Additive-only rule

**Never** use `#[cfg(not(feature = "X"))]` attribute-style conditionals. This syntax makes feature `X` subtract from the binary, which violates the
additive model and can cause incorrect builds when features are combined.

This rule is CI-enforced by `ci/verify_no_new_cfg_not_feature.sh` (all negated-feature `cfg` spellings, including
`all(…, not(…))` compositions and inner attributes); pre-existing sites are grandfathered in a shrink-only allowlist —
see the gate script's header for the exception process.

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

The expression form `cfg!(feature = "X")` evaluates to a `bool` at compile time (the dead branch is eliminated by the optimizer), but every code path
still compiles under every feature combination — which is what "additive" means.

**Exception:** `#[cfg(feature = "X")]` (without `not`) is allowed for blocks that are _purely additive_ — they add code only when the feature is
enabled and are never present in the base build. Only `#[cfg(not(feature = "X"))]` is prohibited.

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

When a route is only meaningful with a specific feature (e.g. Swagger UI), use `#[cfg(feature = "swagger-ui")]` on the _additive_ registration block
only — never to remove an existing route:

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

When an external crate API (e.g. `bollard::Docker::connect_with_ssh`) is only available under a specific feature, the
`#[cfg(feature = "X")] return ...; <fallback>` idiom **cannot** be used because the fallback code after an unconditional `return` becomes unreachable
when the feature is on — triggering the `unreachable_code` lint that is denied workspace-wide.

Use one of the following approved patterns instead.

#### Pattern A — gate the entire match arm

Move the feature-specific arm behind `#[cfg(feature = "X")]` and handle the disabled case in the default arm with a runtime check:

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

#### Pattern B — stub + upgrade helper

Initialize with an always-available stub, then override in a `#[cfg(feature = "X")]` block by calling a helper that accepts and discards the stub:

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

#### Pattern C — always-present tracking field

When a struct field only exists under a feature but you need a cfg-free accessor method, add an always-present `bool` that mirrors its presence:

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

#### Pattern D — conditional early-return inside a guard

When the feature-gated code path is guarded by a runtime condition (`if let`, `if`, etc.), the `return` is conditional rather than unconditional, so
the fallback code remains reachable in all builds:

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

#### Anti-patterns to avoid

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

### Lint suppressions for feature-gated items

When a function, type, field, or constant is _only reachable_ via a `#[cfg(feature = "X")]` additive block, the compiler may emit `dead_code` (or a
related lint) when that feature is disabled. Because `#[cfg(not(feature = "X"))]` is prohibited and the item is genuinely needed under the feature,
suppressing the lint with `#[expect(dead_code, reason = "...")]` is the approved solution.

**Requirement:** every such suppression must carry a detailed inline comment that:

1. Names the Cargo feature that gates the sole caller or user of the item.
2. Explains why the item cannot be removed or restructured to avoid the suppression.

```rust
// ✓ Correct — suppression with mandatory explanation
/// Upgrades a no-op stub client to a real bollard client.
// Only called from the `daemon` feature block in `DockerPlugin::new`. Without the `daemon`
// feature the function is unreferenced, but it must remain compiled for the feature to work.
#[expect(dead_code, reason = "only referenced from the `daemon` feature block; unreachable without it")]
#[cfg(feature = "daemon")]
fn upgrade_to_daemon_client(
    _stub: Arc<dyn DockerClient>,
    config: &Config,
) -> Result<Arc<dyn DockerClient>> {
    Ok(Arc::new(RealDockerClient::new(config)?))
}

// ✗ Wrong — suppression without any comment
#[allow(dead_code)]
fn upgrade_to_daemon_client(...) { ... }
```

Note: when the item is already behind `#[cfg(feature = "X")]`, the dead-code lint only fires under a build that enables `X` but not the specific
caller — which is rare. If the item and its sole caller are both inside the same `#[cfg(feature = "X")]` block, no suppression is needed (the compiler
sees them together). Use `#[expect(dead_code, reason = "...")]` only after confirming the lint is genuine.

No other `#[allow()]` suppressions are permitted without explicit approval.

### Contribution Monotonicity

Enabling a Cargo feature may only **add** plugin descriptor contributions (surfaces, agent surfaces, migrations, role
slots, capabilities); it must never remove or alter contributions that exist without it
([ADR-0032](../adr/0032-plugin-contribution-monotonicity.md)). Features unify across workspace members, so a plugin
crate can never know which binary enabled its features — a "controller-only" contribution is expressed by populating
only the controller-consumed descriptor field, never by a feature predicate.

Legitimate: positive `#[cfg(feature = "x")]` on modules/items whose code cannot compile without the feature, using the
additive `std::iter::empty().chain(...)` shape or a paired empty stub (inline comment naming the reason required).
Banned: any `cfg!`/`#[cfg]` branch that returns _less_ registration data when a feature is ON — in any spelling.
Enforced behaviorally by the registry catalog guards
(`crates/plugins/infrastructure/registry/tests/contribution_monotonicity_guard.rs`); ADR-0032 additionally specifies a
cross-build diff gate (Layer B, separate rollout).

## Atomic Ordering Requirements

Security-critical `AtomicBool` flags (such as `PLAINTEXT_MODE` in `uptrakit-crypto`) must use `Ordering::Release` for stores and `Ordering::Acquire`
for loads. `Ordering::Relaxed` is incorrect for flags that gate security behavior — on weakly-ordered architectures (ARM), a thread could see a stale
value and either skip encryption or encrypt when plaintext mode was intended.

```rust
// ✓ Correct — Release/Acquire guarantees cross-thread visibility
static PLAINTEXT_MODE: AtomicBool = AtomicBool::new(false);

fn enable_plaintext_mode() {
    PLAINTEXT_MODE.store(true, Ordering::Release);
}

fn is_plaintext_mode() -> bool {
    PLAINTEXT_MODE.load(Ordering::Acquire)
}

// ✗ Wrong — Relaxed allows stale reads on ARM
PLAINTEXT_MODE.store(true, Ordering::Relaxed);
PLAINTEXT_MODE.load(Ordering::Relaxed);
```

**Rule:** Any `AtomicBool` or `AtomicU*` that controls a security-sensitive code path must use at minimum `Release`/`Acquire` ordering. `Relaxed` is
only acceptable for pure counters or statistics where stale reads have no correctness impact.

## Synchronous Locks in Async Code

When a synchronous lock is required anywhere in async code, use `parking_lot::Mutex` or `parking_lot::RwLock`. **Never use `std::sync::Mutex`,
`std::sync::RwLock`, `tokio::sync::Mutex`, or `tokio::sync::RwLock`.**

- **Sub-microsecond critical sections** with no `.await` across the lock make a sync lock correct (no risk of holding across a yield point).
- `parking_lot` primitives are faster under contention and return the guard directly — no `Result`/`.unwrap()` needed, which aligns with the workspace
  panic policy.
- `tokio::sync::Mutex`/`RwLock` are unnecessary overhead for critical sections that do not span `.await` points, and their guards are not `Send`,
  preventing use in `tokio::spawn` closures unless the guard is dropped before the first `.await`.
- **Always drop `parking_lot` guards before any `.await` point.** Clone or copy the protected value out of the guard, drop the guard, then `.await`.

```rust
use parking_lot::{Mutex, RwLock};

// ✓ Correct — parking_lot::Mutex, no .unwrap() needed
static FALLBACK: LazyLock<Mutex<HashMap<String, Entry>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

let mut guard = FALLBACK.lock(); // direct guard, no Result

// ✓ Correct — parking_lot::RwLock, drop guard before .await
let value = {
    let guard = self.inner.read(); // synchronous, no .await
    guard.get(&key).cloned()        // copy/clone out of the guard
}; // guard dropped here, before any .await
do_something_async(value).await;

// ✗ Wrong — std::sync::Mutex requires .unwrap() and is slower under contention
let mut guard = FALLBACK.lock().unwrap();

// ✗ Wrong — tokio::sync::RwLock; use parking_lot::RwLock instead
let guard = self.inner.read().await;
```

**Amortize expensive operations under the lock.** If the critical section includes cleanup (e.g., `HashMap::retain()`), do not run it on every call.
Use an `AtomicU64` counter to run cleanup every N calls, keeping per-request lock hold time O(1):

```rust
static CALL_COUNT: AtomicU64 = AtomicU64::new(0);
const CLEANUP_INTERVAL: u64 = 100;

let call_count = CALL_COUNT.fetch_add(1, Ordering::Relaxed);
if call_count.is_multiple_of(CLEANUP_INTERVAL) {
    guard.retain(|_, entry| entry.last_seen >= cutoff);
}
```

## TLS Session Resumption with mTLS Cert Rotation

**Scope.** Applies only to rustls `ClientConfig` builders that use a swappable `ResolvesClientCert` — i.e. the agent's mTLS connector built via
`uptrakit_service_sdk::tls::build_client_config_with_resolver` / `build_system_trust_client_config_with_resolver`. Static-cert builders
(`build_pinned_ca_client_config`, `build_mtls_client_config`, `build_system_roots_client_config`, `build_tofu_client_config`) cannot rotate identity
at runtime and intentionally configure no resumption.

**Invariant.** Every resolver-based mTLS `ClientConfig` MUST register a
[`CertScopedClientSessionStore`](../../crates/shared/service-sdk/src/session_store.rs) as its resumption store, and the same `Arc` instance MUST be
the one attached to the matching `AgentClientCertResolver` via `AgentClientCertResolver::new(initial, session_store)`. `AgentClientCertResolver::swap`
publishes the new `CertifiedKey` and then atomically flushes the store. When a CA-rebuild or any other event reconstructs the `ClientConfig`, thread
the _same_ `Arc<CertScopedClientSessionStore>` through to it — building a fresh store silently re-introduces the resumption-after-revocation bug
because the resolver's `swap()` would reset an orphan cache while rustls keeps replaying tickets in the live config. `clippy.toml` bans
`rustls::client::Resumption::in_memory_sessions` (and the crate-root re-export) via `disallowed-methods` to enforce this at compile time.

**Why.** TLS 1.3 PSK resumption (RFC 8446 §2.2) skips `Certificate`/`CertificateVerify`, so on every resumed handshake the server reads back the
_original_ session's client cert via `peer_certificates()`. Without per-rotation invalidation, a rotated agent keeps re-presenting the old
(now-revoked) cert through cached tickets; the server rejects every reconnect as `CertificateRevoked` and the process loops until the in-memory ticket
cache is wiped by a restart. Observed in production on 2026-05-14.

## Parallel Broadcast Pattern

When broadcasting messages to multiple consumers via `mpsc::Sender`, use parallel sends with a per-send timeout. Sequential sends allow a single slow
consumer (full channel buffer) to block all other recipients.

```rust
use futures_util::future::join_all;
use tokio::time::timeout;

const BROADCAST_SEND_TIMEOUT: Duration = Duration::from_secs(5);

async fn send_parallel(senders: &[mpsc::Sender<Message>], msg: Message) {
    let futures: Vec<_> = senders
        .iter()
        .map(|sender| {
            let msg = msg.clone();
            let sender = sender.clone();
            async move {
                if timeout(BROADCAST_SEND_TIMEOUT, sender.send(msg))
                    .await
                    .is_err()
                {
                    tracing::warn!("broadcast send timed out for a consumer");
                }
            }
        })
        .collect();
    join_all(futures).await;
}
```

**Key points:**

- Use `futures_util::future::join_all()` for parallel sends (already in workspace dependencies).
- Add a per-send `tokio::time::timeout` to prevent indefinite blocking.
- Log a warning when a send times out to identify slow consumers.
- Clone both the message and the sender to avoid lifetime issues in the async closures.

See also: `crates/ui/web-api/src/service_connections.rs` for the reference implementation.

## Design Principles

- Keep every boundary clear: the controller orchestrates scheduling, upstream checks, API/UI; the MQTT service handles MQTT/Home Assistant
  integration; agents manage installed versions and update execution; plugins focus on version detection/updating logic.
- Treat custom scripts and command execution paths as handling untrusted input; validate before execution to avoid shell injection.
- Agent connections are outbound-only, unprivileged, and rely on explicit sudo allowlists for privileged update commands.
- Logs should contain operational summaries only; do not store full command output internally.
- New protected routes must use a typed action extractor backed by the `Action` catalog rather than comparing raw role names.
- Document every behavioral change either in code or via an external docs page (e.g., update `docs/api` or `docs/development`).

Refer to [docs/security/secure-development.md](../security/secure-development.md) when the change touches PKI, secrets, reverse proxies, or filesystem
security.

## Service Reconnect Backoff

All reconnect loops in service binaries must use `backon::ExponentialBuilder` via the
`uptrakit_service_sdk::reconnect_backoff_builder()` helper — not a fixed sleep. Fixed delays hammer a recovering broker or controller and produce bursty
log storms.

### Standard Builder

Use `reconnect_backoff_builder()` from `uptrakit-service-sdk` to get a preconfigured builder (2 s base, 60 s cap, jitter, infinite). Call
`.build()` to get an iterator and advance it with `.next().unwrap_or(Duration::from_secs(60))`. To "reset" the backoff (e.g. after a successful
connection), discard the iterator and call `.build()` again on the stored builder:

```rust
use uptrakit_service_sdk::reconnect_backoff_builder;
use backon::BackoffBuilder;
use std::time::Duration;

let builder = reconnect_backoff_builder();
let mut backoff = builder.build();

loop {
    match connect().await {
        Ok(conn) => {
            // reset: connection succeeded — rebuild iterator from base.
            backoff = builder.build();
            handle(conn).await;
        }
        Err(e) => {
            let delay = backoff.next().unwrap_or(Duration::from_secs(60));
            tracing::warn!(error = %e, ?delay, "connection failed; retrying");
            tokio::select! {
                _ = shutdown_token.cancelled() => break,
                _ = tokio::time::sleep(delay) => {}
            }
        }
    }
}
```

### Safety Property

`without_max_times()` is encapsulated inside `reconnect_backoff_builder()` — never construct an `ExponentialBuilder` inline for reconnect loops without
it. The default `backon` `max_times = Some(3)` would silently stop the loop after three attempts.

Every reconnect builder must ship a guard test confirming the iterator is actually infinite:

```rust
#[test]
fn reconnect_backoff_is_infinite() {
    use backon::BackoffBuilder;
    let builder = reconnect_backoff_builder();
    assert!(builder.build().nth(1_000).is_some(), "backoff must be infinite");
}
```

### Bounded-Retry Idiom

For operations that should give up after a fixed number of attempts (e.g. version detection, HTTP fetch), use `backon`'s `Retryable` combinator instead
of a manual loop:

```rust
use backon::{ExponentialBuilder, Retryable};

let result = fetch_version
    .retry(ExponentialBuilder::default().with_max_times(4))
    .when(|e| e.is_transient())
    .notify(|e, delay| tracing::warn!(error = %e, ?delay, "retrying"))
    .await?;
```

Note: `with_max_times(M)` means **M + 1 total attempts** (M retries after the first try).

### API Reference

Standard parameters: **base 2 s, cap 60 s** with ~25 % jitter. See `crates/shared/service-sdk/src/lib.rs` for `reconnect_backoff_builder()` and
`crates/shared/service-sdk/src/lifecycle.rs` for the canonical enrollment/reconnect loop.

**Never** replace this with `tokio::time::sleep(Duration::from_secs(5))`. A fixed delay:

- Does not back off under sustained outages, hammering the broker.
- Cannot be interrupted by a shutdown signal without an additional `select!`.
- Has no jitter, causing thundering-herd reconnects when many agents restart simultaneously.

See also: [Service Lifecycle](service-lifecycle.md) for the full reconnect and enrollment flow.

## `ServiceHandler` Transport Contract

`ServiceHandler` implementations must not import or depend on `ControllerConnection`. All handler method signatures use `&mut dyn ServiceTransport`
(from `uptrakit-wire`). A handler impl that compiles against `uptrakit-wire` types only is transport-agnostic by construction and can run in both
standalone (WebSocket) and embedded (in-process) modes.

`agreed_capabilities` for capability-dependent initialization must be read from the `on_settings` parameter, not from a connection method.

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

- **Error type naming:** `Parse{TypeName}Error` (e.g., `ParseActionError`, `ParseMqttTransportError`).
- **Error derivation:** Use `thiserror::Error` in crates that depend on `thiserror`. In crates without `thiserror` (e.g., `uptrakit-wire`),
  implement `Display` and `Error` manually.
- **Call sites:** Prefer `s.parse::<MyType>()` over explicit `MyType::from_str(s)`.
- **Fallible conversions with defaults:** Use `s.parse::<MyType>().unwrap_or_default()` when a default is acceptable.
- **`from_url_scheme()` and similar:** Domain-specific parsers that convert from a different representation (e.g., URL schemes like `mqtt`/`mqtts` to
  `MqttTransport`) are not `FromStr` candidates and should remain as named methods.

### Anti-pattern

- **Ad-hoc `parse(&str)` methods** returning `Option<Self>` or `Result<Self, String>` -- always implement `FromStr` instead.

The full error handling reference (20 patterns, anti-patterns, decision table, approved exceptions, and rules summary) is in
[Error Handling](error-handling.md).

## Request Type Validation

Route handlers never take a request body as a raw extractor. They take one of two typed extractors, both defined in
`crates/ui/web-api/src/extract.rs`:

- **`Unvalidated<T>`** (JSON) and its form-borne counterpart **`UnvalidatedForm<T>`** deserialize the body without running validation. The inner
  value is private — the only way to reach it is `require_valid()`, which runs `Validate::validate()` and returns `Result<T, ValidationError>`. This
  puts the handler in control of what happens on failure: which status code to return, whether to emit a `ValidationFailed` audit event, and whether
  to validate before or after authorization has run.
- **`Validated<T>`** deserializes and validates in the extractor itself, rejecting with a generic `400 Bad Request` before the handler body runs.

These two are not co-equal defaults. `Unvalidated<T>` is the default for mutations on an audited entity family — only it can emit the family's
`ValidationFailed` audit event, return a non-400 status, or defer validation until after an authorization check. Reach for `Validated<T>` only when
the entity family does not audit validation failures.

### Reason-code namespaces

`"validation_error"` is an **HTTP error-envelope code** (`ApiError` paths, consumed by API clients);
`"invalid_request"` is an **audit details `reason_code`** (consumed by audit review). They are different namespaces
by design — renaming either would churn recorded audit rows or the API error contract for zero information gain.
Second axis: action families with site-namespaced reason codes (e.g. `SOFTWARE_UPDATE_TRIGGERED`'s
`trigger_update.*`, produced beside `TriggerUpdateError::trigger_audit_classification`) keep that prefix on their
validation-reject rows (`trigger_update.invalid_request`), while the generic `require_valid()` mirror family uses
bare `invalid_request`. Follow the action family's convention when one exists, the mirror convention otherwise.

### Raw body extractors are banned

A raw `Json<T>`/`Form<T>` body parameter (or a raw `Request`/`Bytes` body read) in `crates/ui/web-api/src/routes/` is banned, CI-enforced by
`bash ci/verify_no_raw_body_extractors.sh`. The gate carries an allowlist (`ci/verify_no_raw_body_extractors_allowlist.txt`) covering the legacy
handlers that still call `.validate()` manually plus a small set of enumerated raw-body reads. The allowlist row _set_ — not just its count — is
mechanically frozen: every current row must already exist at a baseline outside the commit's control (a baseline-subset check with bijective rename
support, so a file-move or facade split can carry its row to a new path without being treated as an addition). CI passes the pull request's base ref,
or the push event's prior `before` SHA on `main`, as that baseline; runs off CI degrade to the merge-base of the default branch, or — if no baseline
is resolvable at all, e.g. offline or a shallow clone — warn and skip the sub-check while the other checks still run. A commit that deletes one
legacy allowlist row while adding a different, unconverted one is caught: the new row cannot match any row removed at the baseline, so it is flagged
as an addition regardless of the row count staying flat. The remaining, deliberately review-gated escape hatch is amending the gate script itself;
see [ADR-0038](../adr/0038-type-state-request-body-validation-via-unvalidated-extractor.md) for the full boundary.

### Validate trait

Every request body type implements `Validate` — the extractor bounds on `Unvalidated<T>`/`UnvalidatedForm<T>`/`Validated<T>` compile-force it, so
there is no path to a handler that skips it.

```rust
use uptrakit_web_api_types::validation::{Validate, ValidationError};

pub trait Validate {
    fn validate(&self) -> Result<(), ValidationError>;
}
```

`ValidationError` carries a `field: &'static str` and `message: String`, providing structured field-level error reporting to API consumers. When a
type genuinely has no format/length invariants to check, the implementation is still required — return `Ok(())` with a comment explaining why:

```rust
fn validate(&self) -> Result<(), ValidationError> {
    // No format/length invariants beyond field types; capability/existence checks are handler-side.
    Ok(())
}
```

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

### Validating `Option`-wrapped update fields

Update/PATCH request types wrap mutable fields in `Option<T>` so "omitted" and "explicitly set" are distinguishable; validate only when the field is
`Some`. A field that is required whenever present (i.e. not clearable) must reject `Some("")` — see `UpdateHostRequest::validate()` in
`crates/shared/web-api-types/src/hosts.rs`, which rejects `friendly_name: Some("")` but allows `None` (keep current value). A field that can
genuinely be cleared must not overload the empty string for that — it needs an unambiguous tri-state representation instead. The established idiom
is `Option<serde_json::Value>`, where absence keeps the current value, `null` clears it, and any other JSON value sets it; see
`UpdateNatsSettingsRequest`, `UpdateHostTagRequest`, and `UpdateScheduledTaskRequest` in `crates/shared/web-api-types/src/`.

### Route handler wiring

```rust
body: Unvalidated<UpdateHostRequest>,
) -> Response {
    let body = match body.require_valid() {
        Ok(body) => body,
        Err(e) => return error_response(StatusCode::BAD_REQUEST, e.to_string()),
    };
    // ... use `body`
}
```

Handlers on an audited entity family wrap the `Err` arm with the family's `ValidationFailed` audit emission before returning the error response —
see `update_host` in `crates/ui/web-api/src/routes/hosts.rs` for the pattern: build an `AuditEntry::<Event>` with
`.outcome(AuditOutcome::ValidationFailed)`, call `state.audit_emitter.emit_event(entry)`, then return `error_response(StatusCode::BAD_REQUEST,
e.to_string())`.

Coverage of which request types are validated is compile-enforced by the extractor bound and CI-enforced by `verify_no_raw_body_extractors.sh`; a
hand-maintained inventory here would only drift out of sync. The `*Request` naming convention remains a soft convention, not an enforced one.

## Route Authorization Pattern

All protected web-API route handlers enforce authorization via typed Axum extractors. Never perform an inline authorization check in a handler
body — there is no `has_permission`-style method to call; use the extractor pattern below.

### Required pattern

```rust
use crate::middleware::action::CanReadHosts;

pub async fn list_hosts(
    tenant_db: TenantDb,
    CanReadHosts(_user): CanReadHosts,   // action enforced here via the AccessEngine
) -> Response {
    // handler body -- 401/403/500 already handled by the extractor
}
```

Use the bound variable name `_user` when the `AuthenticatedUser` value is not used in the body, and `user` when it is (e.g. `user.user_id` for an
`actor_id` field).

Extractors are generated by the `action_extractor!` macro call in `crates/ui/web-api/src/middleware/action.rs`; each maps to one catalog action
constant in `uptrakit_shared_types::access::actions`. See [Authentication and
Authorization](../security/auth-and-authorization.md) for the action catalog and the deny semantics.

### Required utoipa security requirement

Every protected endpoint declares its action as a native OpenAPI `security` requirement — the `oauth2` scope list carries the action string, and
`developer_token` is the alternative scheme:

```rust
#[utoipa::path(
    get,
    path = "/api/v1/hosts",
    security(("oauth2" = ["hosts:read"]), ("developer_token" = [])),
    ...
)]
```

The scope string must match the action the handler's extractor enforces — `ci/verify_action_security_declarations.py` gates that pairing. The legacy
`extensions(("x-required-permission" = ...))` form and the `bearer_token` scheme are retired; do not reintroduce either.

### Adding a new extractor

Add one line to the `action_extractor!` macro call in `crates/ui/web-api/src/middleware/action.rs`:

```rust
action_extractor! {
    ...
    /// `newthing:read` — what it gates.
    CanReadNewThing => actions::NEWTHING_READ,
}
```

The macro generates a struct `CanReadNewThing(pub AuthenticatedUser)` with a `FromRequestParts` impl that resolves the request's `AccessContext`
through the `AccessEngine` and records the deny metric on a policy deny. The `access.denied` audit Event is additionally emitted only for actions
`deny_event_worthy()` classifies as audit-worthy (`crates/shared/types/src/access/mod.rs`) — system-plane actions plus `commands:manage`,
`access:manage`, and `mcp:use`; every other denial is the counter and a debug trace.

### Anti-pattern

```rust
// WRONG — no such method exists; this shape (ad-hoc inline authorization instead of a typed extractor) is the anti-pattern
pub async fn list_hosts(
    Extension(user): Extension<AuthenticatedUser>,
) -> Response {
    if !user.has_permission("hosts:read") {
        return error_response(StatusCode::FORBIDDEN, "Insufficient permissions");
    }
    ...
}
```

### Approved exception: custom authentication paths

Handlers that perform their own token extraction (e.g., reading a `token` query parameter or `Authorization` header because browser WebSocket
connections cannot set custom headers) cannot use Axum extractors for authentication. In these handlers, an inline `AccessEngine` call —
`build_access_authority(&state, user_id).await` followed by `authorize_any(&state.access_engine, access_ctx, &state.audit_emitter, &[action])` —
is acceptable **only** when:

1. The token validation is already done manually (JWT or API token, same logic as the standard middleware), **and**
2. No typed extractor exists that covers the custom auth path.

Any such handler must include a `// APPROVED: custom auth path` comment alongside the inline engine call. The `interactive_ws` WebSocket
endpoint (`crates/ui/web-api/src/routes/interactive_ws.rs`) is the canonical example.

See also: [Authentication and Authorization](../security/auth-and-authorization.md).

## ETag Route-Layer Pattern

Settings endpoints use route-level ETag middleware rather than per-handler extractors (see [ADR-0017](../adr/0017-etag-route-layer-middleware.md)).
New settings routes **must** be covered by `etag_middleware`.

### How to add a new settings route

1. Decide scope: `SettingsVersion` for `/api/v1/settings/*` and `/api/v1/plugin-configs/*`, `GlobalSettingsVersion` for `/api/v1/global-settings/*`.
2. In `router.rs`, add the route to the appropriate sub-router (`tenant_settings`, `global_settings`, or `plugin_configs`). Do not add it to the outer
   `auth_routes` chain.
3. Handler bodies contain no ETag code — no `If-Match` parameter, no `settings_version_cache` lookup, no ETag header construction.

The middleware validates `If-Match` on `PUT`/`PATCH` only. Routes in a sub-router whose side effects are asynchronous or that never bump the version
counter during the HTTP transaction must stay outside it — for plugin configs that means `list_plugin_types`, `test_plugin_config`, and
`discover_plugin_config`.

### POST endpoints

POST routes included in an ETag sub-router receive an ETag on success if they mutate state. POST endpoints that are destructive teardowns (e.g.
`POST /settings/reset-data`) must **not** be included in any ETag sub-router.

### `IfMatch<S>` extractor

No route uses it — `plugin_configs.rs`, its last holdout, moved to the layer pattern. Do not add it to new handlers.

## Typed Path Extractors

Route handlers that accept UUID path parameters must use `Path<Uuid>` (or `Path<(Uuid, Uuid)>` for multi-param routes) instead of `Path<String>` with
manual `Uuid::parse_str`. Axum returns a typed 422 response automatically on malformed input.

### Required pattern

```rust
use uuid::Uuid;

pub async fn get_host(
    tenant_db: TenantDb,
    CanReadHosts(_user): CanReadHosts,
    Path(host_id): Path<Uuid>,        // Axum validates the UUID
) -> Response {
    // use host_id directly — no parse_str needed
}
```

For multi-param routes:

```rust
pub async fn unassign_host(
    tenant_db: TenantDb,
    CanDeleteSoftware(_user): CanDeleteSoftware,
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

**Exception:** `Path<String>` is correct for non-UUID path parameters (e.g., base64-encoded OCSP requests in `ocsp.rs`).

## UUID Query Parameters

Use `Option<Uuid>` (not `Option<String>`) for UUID-typed query parameters. Axum's serde deserialization automatically rejects malformed UUIDs with
`422 Unprocessable Entity`. Manual `.and_then(|s| Uuid::parse_str(s).ok())` silently swallows invalid values, returning the "no filter" behaviour
instead of an error.

### Required pattern

```rust
use uuid::Uuid;

// Derive IntoParams so the OpenAPI params come from THIS struct — never a
// hand-maintained duplicate (see "OpenAPI parameter & schema authoring" below).
#[derive(Deserialize, utoipa::IntoParams)]
struct MyQuery {
    /// Filter by plugin config UUID.
    // Axum returns 422 automatically for malformed UUIDs.
    plugin_config_id: Option<Uuid>,
}

// In the handler, use params.plugin_config_id directly — no parse needed
```

### utoipa annotations

Reference the query struct with `params(MyQuery)` (mixed with any inline Path tuples) — do NOT
re-declare a query field the struct already owns. `IntoParams` still emits `format: uuid` for the
`Option<Uuid>` field, and the field's `///` doc-comment becomes the OpenAPI description:

```rust
#[utoipa::path(
    delete,
    path = "/api/v1/hosts/{id}/discovered",
    params(
        ("id" = Uuid, Path, description = "Host UUID"),
        MyQuery,
    ),
    ...
)]
```

### Anti-pattern

```rust
// WRONG — silently ignores malformed UUIDs; no 422 returned
#[derive(Deserialize)]
struct MyQuery {
    plugin_config_id: Option<String>,
}

// … then in the handler:
let id = params.plugin_config_id.as_deref().and_then(|s| Uuid::parse_str(s).ok());
```

See also: [Typed Path Extractors](#typed-path-extractors) for the equivalent rule on path parameters.

## OpenAPI parameter & schema authoring (drift-proof)

The OpenAPI spec (`crates/ui/web-api/openapi.json`) is generated from the `#[utoipa::path]` and
`#[derive(...)]` annotations, then drives the frontend client. Any schema value that is
**hand-maintained separately from its source of truth** can silently drift from it. The
`openapi_json_is_up_to_date` golden test (`integration_tests/openapi_spec.rs`) does **not** catch this
class: it only proves the committed spec equals what the annotations regenerate — if an annotation is
wrong, both are wrong and the test passes while the generated client is broken. This once dropped the
software-items name filter. Follow these rules; see
[ADR-0025](../adr/0025-drift-proof-openapi-params.md) for the full rationale.

- **Query / request params: derive, don't hand-list.** Author them as `params(<IntoParamsStruct>)` over
  the handler's `Query<Struct>` extractor — never a `params(("field" = …, Query, …))` list that
  duplicates struct fields. Field `///` doc-comments become the param descriptions. Enforced by
  `ci/verify_no_inline_query_params.sh` (fails on any inline `Query` tuple; allowlist a genuine
  non-`Query<Struct>`-backed exception). Examples: `list_software_items`
  (`routes/software_items/crud.rs`), `list_audit_logs` (`routes/audit_logs.rs`).
- **`IntoParams` derive gating.** Structs in **`uptrakit-web-api-types`** use
  `#[cfg_attr(feature = "openapi", derive(utoipa::IntoParams))]` (the crate is reusable without OpenAPI).
  Structs **local to `uptrakit-web-api` route files** use an **unconditional** `#[derive(utoipa::IntoParams)]`
  — `web-api` has no `openapi` feature and `utoipa` is a non-optional dependency, so a `cfg_attr` gate
  would never fire and fail to compile. E.g. `ListSoftwareItemsParams` (types) vs `OidcCallbackParams`
  (local, in `routes/oidc_auth.rs`).
- **Path params stay inline; mixed Path + Query go in one block.** Single/few path params are fine as
  inline tuples. When a handler has both, keep the Path tuples inline and add the `IntoParams` struct as
  a further entry — `unassign_host` (`routes/software_items/host_assignments.rs`):
  `params(("id" = Uuid, Path, …), ("host_id" = Uuid, Path, …), DeleteHostAssignmentParams)`.
- **Enum schemas: source `enum_values` from one place.** A manual `utoipa::PartialSchema` must derive its
  values from the same source as the serde wire format — `Self::all()` (via `strum::EnumIter`) for plain
  enums, or the `wire_safe_enum!` macro's `$wire` list (`crates/shared/macros/src/lib.rs`). For an enum
  with an `Other(String)` catch-all (which can't derive `EnumIter`), hardcoding is unavoidable — pair it
  with a guard test asserting the schema equals the `as_str()` set (`PluginRole` +
  `plugin_role_schema_enum_values_match_wire_strings`, `crates/shared/types/src/plugin_role.rs`).
- **Regenerate + commit both artifacts** after any change — see the "REST API contract staleness gates"
  section in [quality-gates.md](quality-gates.md) (`./scripts/regen-api.sh`).

## Tenant-Safe Database Queries

All database queries in route handlers and query helpers **must** enforce tenant isolation. Failure to do so can leak data across tenants (a
high-severity security issue).

### Rules

**Rule 1 — Always use `TenantDb` helpers for `TenantScoped` entities.**

Use `TenantDb.find::<E>()`, `.find_by_id::<E>(id)`, `.update_many::<E>()`, or `.delete_many::<E>()` for any entity that implements `TenantScoped`.
These methods automatically inject `WHERE tenant_id = ?`.

**Rule 2 — Use `find_via_tenant_join` for join-table entities without `tenant_id`.**

Some entities (e.g. `service_host`) are join tables that have no `tenant_id` column of their own. Enforce tenant isolation by joining through a
`TenantScoped` entity with `TenantDb.find_via_tenant_join::<Target, Scoped>(relation)`.

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

`tenant_db.db()` is the raw `DatabaseConnection`; it carries no tenant filter. Calling `Entity::find().all(tenant_db.db())` on a `TenantScoped` entity
loads **all** rows from all tenants.

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

| Wrong                                                 | Right                                                                          | Reason                   |
| ----------------------------------------------------- | ------------------------------------------------------------------------------ | ------------------------ |
| `Host::find().all(tenant_db.db())`                    | `tenant_db.find::<host::Entity>().all(tenant_db.db())`                         | No tenant filter applied |
| `ServiceHost::find().all(tenant_db.db())`             | `tenant_db.find_via_tenant_join::<service_host::Entity, service::Entity>(rel)` | Cross-tenant leak        |
| Per-item `Host::find_by_id(id).one(db)` inside a loop | Batch `find().filter(id.is_in(ids))` then in-memory lookup                     | N+1 queries              |
| `Entity::update_many().col_expr(...)` loop            | `Entity::update_many().filter(id.is_in(ids)).col_expr(...).exec(db)`           | N+1 updates              |

See also: [Architecture — Multi-Tenancy](../architecture/multi-tenancy.md) and [Security — Secure Development](../security/secure-development.md).

## HTTP Status Codes

Always use `reqwest::StatusCode` (re-exported as `uptrakit_openapi_client::StatusCode` for CLI code) instead of raw `u16` for HTTP status codes.

- **Comparisons**: `status == StatusCode::NOT_FOUND`, not `status == 404`
- **Range checks**: `status.is_client_error()`, `status.is_server_error()`, `status.is_success()` -- not `status >= 400`
- **Reason phrases**: `status.canonical_reason()` -- not hand-written match tables
- **Error types**: `status: StatusCode` -- not `status: u16`
- **Serialization**: When a `StatusCode` must appear as a number in JSON, use `#[serde(serialize_with = "serialize_status_code")]`
- **The only approved `.as_u16()` call** is inside serde serialization helpers for JSON wire compatibility

## Tracing Status Codes

When logging HTTP status codes in `tracing!` macros, use the `%` display format rather than `.as_u16()`:

```rust
// Correct — uses Display impl, emits "200 OK" or "404 Not Found"
tracing::debug!(status = %response.status(), "request complete");

// Wrong — emits a bare integer with no reason phrase
tracing::debug!(status = response.status().as_u16(), "request complete");
```

The `StatusCode` type's `Display` implementation produces `"<code> <reason>"` (e.g., `"429 Too Many Requests"`), which is more informative in logs
than a bare integer. The `.as_u16()` method is approved only inside serde serialization helpers where JSON wire compatibility requires a numeric
value.

## Pinned-CA-only reqwest clients

When building a reqwest client that must use a custom CA and exclude system roots, use `tls_certs_only`. Example:

```rust
let cert = reqwest::Certificate::from_pem(pem.as_bytes())?;
builder = builder.tls_certs_only(std::iter::once(cert));
```

Do **not** use `add_root_certificate` — deprecated in reqwest 0.13 because it appends to system roots rather than replacing them.

## CLI CA fingerprint helpers

`parse_fingerprint(s: &str) -> Result<String>` — normalize a `--tofu` flag value to 64-char lowercase hex. Located in
`crates/ui/cli/src/commands/auth.rs`.

`establish_ca_trust(server, fingerprint_hint, allow_rotation, config) -> Result<()>` — shared bootstrap function used by `auth login --tofu` and
`auth ca trust`. Fetches `GET /api/v1/pki/ca.crt`, verifies SHA-256 fingerprint, persists PEM. Located in `crates/ui/cli/src/commands/auth.rs`.

## Constant-Time Secret Comparison

Externally-provided secrets (webhook tokens, API keys, and similar short-lived credentials) must **never** be compared using `==` or `!=`. Rust's
default `PartialEq` on `&str` short-circuits on the first differing byte, leaking timing information that an attacker can exploit to infer the secret
one byte at a time.

**Rule:** Whenever code validates a caller-supplied secret against an expected value, use `subtle::ConstantTimeEq` after normalising both sides to a
fixed-length representation.

### Required pattern

Add `subtle = { workspace = true }` to the crate's `Cargo.toml` and use the SHA-256 + `ct_eq` idiom:

```rust
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

// Hash both values so the comparison is always over two fixed-size 32-byte arrays.
// This prevents length information from leaking through ct_eq.
let expected_hash: [u8; 32] = Sha256::digest(expected_secret.as_bytes()).into();
let provided_hash: [u8; 32] = Sha256::digest(provided_secret.as_bytes()).into();
let secrets_match: bool = expected_hash.ct_eq(&provided_hash).into();

// Guard against the "no secret configured accepts all" case.
if expected_secret.is_empty() || !secrets_match {
    return Err(/* unauthorized error */);
}
```

Hashing first ensures both inputs are exactly 32 bytes before calling `ct_eq`, making the comparison unconditionally constant-time regardless of input
length differences.

### Anti-pattern table

| Wrong                                       | Right                                        | Reason                                                                |
| ------------------------------------------- | -------------------------------------------- | --------------------------------------------------------------------- |
| `provided != expected`                      | `ct_eq` with SHA-256 pre-hashing (see above) | Short-circuit timing leak                                             |
| `provided == expected`                      | `ct_eq` with SHA-256 pre-hashing (see above) | Short-circuit timing leak                                             |
| `subtle::ConstantTimeEq` directly on `&str` | Hash first, then `ct_eq`                     | Length difference leaks through variable-time `ct_eq` implementations |

See also: [Security — Secure Development](../security/secure-development.md).

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
`pending_device_flow` | `status` | `DeviceAuthStatus` | | `service` | `status` | `ServiceStatus` | | `update_history` | `status` | `UpdateStatus` |

Note: the `service` entity stores capabilities as a JSON text column (`services.capabilities`) rather than a typed enum column. The capability set is
parsed into `BTreeSet<Capability>` at read time. See
[Service Lifecycle -- Capability-based enrollment](service-lifecycle.md#capability-based-enrollment).

### Re-exports

`uptrakit-shared-db` re-exports all entity-relevant enums from `uptrakit-shared-types` for downstream convenience. Crates that depend on
`uptrakit-shared-db` (like `uptrakit-web-api`) should import from `uptrakit_shared_db` rather than adding a direct dependency on
`uptrakit-shared-types`.

## SeaORM Integration for Custom Types

When creating new wrapper types (newtypes) for use in SeaORM entity models, implement the following four traits behind the `sea-orm` feature flag in
`uptrakit-shared-types`. Both `SecretString` and `MaskedEmail` follow this pattern.

### Required trait implementations

| Trait                           | Purpose                                                          |
| ------------------------------- | ---------------------------------------------------------------- |
| `From<T> for sea_orm::Value`    | Converts the wrapper into a `Value` for query binding            |
| `sea_orm::TryGetable`           | Extracts the wrapper from a `QueryResult` row                    |
| `sea_orm::sea_query::ValueType` | Declares the column type and provides `try_from(Value)`          |
| `sea_orm::sea_query::Nullable`  | Provides the null `Value` representation for `Option<T>` columns |

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

| Type              | Crate                   | Entity usage                                                                                            |
| ----------------- | ----------------------- | ------------------------------------------------------------------------------------------------------- |
| `SecretString`    | `uptrakit-shared-types` | `user.password_hash`                                                                                    |
| `MaskedEmail`     | `uptrakit-shared-types` | `user.email`                                                                                            |
| `EncryptedString` | `uptrakit-shared-db`    | `mqtt_client.password`, `oidc_provider.client_secret`, `ca_certificate.key_pem`, `ssh_host.private_key` |

See also: [Secrets Handling and Encryption](../security/secrets-and-encryption.md) for security properties of `SecretString` and `MaskedEmail`.

## Database Query Patterns

Paginated list endpoints must never issue per-record queries. Violating this rule produces O(N) database round-trips that make the API unusable at
scale.

### Batch loading rule

After fetching a page of N records, collect all unique foreign-key IDs from the page and load related entities in a single `is_in(ids)` query. Build a
`HashMap<Uuid, …>` for O(1) lookup during response construction. Never call `find_by_id` inside a loop.

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

Use `Expr::in_subquery(…)` or a JOIN for tenant-scoping tables that reference `host_id` (e.g. `update_history`). Loading all host IDs into application
memory and passing them to `is_in(Vec<Uuid>)` is only acceptable when the tenant is guaranteed to have fewer than ~100 rows; for unbounded collections
use a subquery:

```rust
let host_subquery = Query::select()
    .column(host::Column::Id)
    .from(host::Entity)
    .and_where(Expr::col(host::Column::TenantId).eq(tenant_id))
    .to_owned();
Entity::find().filter(Column::HostId.in_subquery(host_subquery))
```

### Scope pre-loaded sets tightly

When pre-loading a lookup set to avoid per-item queries, scope it to the narrowest key available. For example, pre-load software ignore rules per
`(tenant_id, plugin_config_id)`, not per `tenant_id` alone — the per-config set is bounded to what a user has explicitly configured for that one
plugin, while the per-tenant set can be unbounded.

### Avoid `unwrap_or(0)` on count queries

A silently-zero count on DB failure hides errors. Propagate DB errors with `?` or log and return an explicit error response. Do not use
`count.unwrap_or(0)` as a silent default.

### SQLite transactions that read before writing must use `BEGIN IMMEDIATE`

<!-- prettier-ignore -->
**Background — SQLITE_BUSY_SNAPSHOT:** SQLite WAL mode allows one writer and many concurrent
readers. When a `BEGIN DEFERRED` transaction reads a row, it establishes a snapshot at the WAL
position at that moment. If a separate connection commits a write before the first transaction
tries to write, SQLite detects that the snapshot is stale and returns `SQLITE_BUSY_SNAPSHOT`
(extended result code 517, primary code 5) **immediately** — it bypasses `busy_timeout` entirely
because retrying cannot help: the snapshot can never become current without restarting the
transaction. The extended code is what distinguishes it from an ordinary `SQLITE_BUSY` (5) lock
wait; `crates/shared/db-tx/src/lib.rs` pins both codes in its two-connection tests.

The symptom is a `database is locked` error with a 2–5 ms latency on an operation that is supposed to wait up to 5 seconds. It is easy to miss in
testing because it only triggers under concurrent load.

**Rule:** Every transaction in the workspace opens via `begin_immediate()` (the `uptrakit-db-tx` leaf crate,
re-exported as `uptrakit_shared_db::begin_immediate`) — not only transactions that read before writing. This
acquires the write lock at `BEGIN` time, before any reads establish a snapshot, so the snapshot-staleness race
cannot occur, and it is a mode no-op on other backends and on nested (savepoint) transactions, so it is safe to
call unconditionally regardless of backend or nesting. All eleven `sea_orm` transaction-opening method paths are
banned via `clippy.toml`'s `disallowed-methods`: `TransactionTrait::{begin, begin_with_config,
begin_with_options, transaction, transaction_with_config}`, plus the inherent `transaction_async` /
`transaction_with_config_async` pair on each of `DatabaseTransaction`, `DatabaseConnection`, and
`DatabaseExecutor`.

```rust
use uptrakit_shared_db::begin_immediate;

// ✓ Correct — the sole sanctioned transaction opener
let txn = begin_immediate(db).await.context_to()?;

let row = Entity::find().one(&txn).await?;   // read
// … some logic …
active_model.update(&txn).await?;            // write — safe, no BUSY_SNAPSHOT

// ✗ Wrong — banned via clippy.toml disallowed-methods; BEGIN DEFERRED opens a
//   snapshot on first read that a concurrent commit can invalidate before the
//   write → SQLITE_BUSY_SNAPSHOT (517) instead of an ordinary busy_timeout wait
let txn = db.begin().await.context_to()?;

// ✗ Also wrong — same mode as begin_immediate(), but hand-rolled: banned so the
//   helper stays the single opener the canary and the escape hatch can police
let txn = db
    .begin_with_options(TransactionOptions {
        sqlite_transaction_mode: Some(SqliteTransactionMode::Immediate),
        ..Default::default()
    })
    .await
    .context_to()?;
```

**Escape hatch:** a call site that genuinely needs a non-default `TransactionOptions` field, or that can justify
staying on a plain writer-writer lock, may bypass the ban with `#[expect(clippy::disallowed_methods, reason =
"...")]`, where the reason names either the write-only rationale or the specific `TransactionOptions` field
needed. The only current users of this escape hatch are `begin_immediate()`'s own internal
`begin_with_options()` call and the canary/negative-control call sites in the `uptrakit-db-tx` test modules
`tests` and `busy_snapshot_tests` (see below) — no production call site outside `uptrakit-db-tx` opts out of `begin_immediate()`.

**Canary:** `crates/shared/db-tx/src/lib.rs`'s `mod tests` exercises all eleven banned paths, each under its own
`#[expect(clippy::disallowed_methods, ...)]`. If a future `sea_orm` upgrade renames or relocates a banned
method, the unresolvable `clippy.toml` path degrades to a config warning that `-D warnings` does not catch — but
the corresponding `#[expect]` then goes unfulfilled, and the workspace's `unfulfilled_lint_expectations = "deny"`
lint turns that into a hard build failure instead of a silent gap.

### Active partial-index predicates must track the active status set

Partial unique indexes that hand-maintain a SQL literal status set (the `WHERE status IN (…)` predicate) must stay in sync with the corresponding
`UpdateStatus` grouping helper. Two such indexes exist and differ **by design**:

- `uix_update_history_host_active` → `('pending','in_progress','awaiting_restart')` = `UpdateStatus::host_blocking()` (excludes `Queued`: a queued
  item does not block the host slot).
- `uix_update_history_host_software_item_active` → `('queued','pending','in_progress','awaiting_restart')` = `UpdateStatus::unfinished()` (`Queued`
  counts as active per (host, item) to prevent duplicate triggers).

The `active_indexes_match_enum_sets` test (`crates/ui/web-api-queries/src/queries/update_history.rs`) introspects `sqlite_master` and maps **each index
to its own helper set** (host index ↔ `host_blocking()`, item index ↔ `unfinished()`), failing CI if a future variant is added without reconciling the
matching index. It deliberately does **not** assert one shared set across both indexes — that would wrongly flag the intentional `Queued` difference.

**New terminal statuses go in neither set.** A terminal status (e.g. `Interrupted`) must be absent from both helpers and both index predicates; a
terminal status that leaks into an active index would pin the host/item slot forever and block the user's re-trigger with a 409 — a compiler-invisible
failure mode the test exists to catch.

### Database Pool Migration

`DbPoolReloadable` owns a `tokio::sync::watch` channel that publishes replacement `Arc<DbConnHandle>` values when the pool is reloaded. Two patterns
apply:

**Watch-driven re-read** (for long-lived polling consumers):

```rust
// Receive a watch::Receiver<Arc<DbConnHandle>> from DbPoolReloadable::subscribe().
// Re-read the current handle on every iteration — never clone it outside the loop.
let handle = db_rx.borrow().clone(); // Arc clone; releases read lock immediately
let rows = MyEntity::find().all(handle.conn()).await?;
```

**Initial-handle** (for startup components that construct once):

```rust
// Clone the connection from the initial handle. This site uses the boot-time
// pool until the process restarts. Annotate with a TODO for future migration.
// TODO: migrate to watch::Receiver<Arc<DbConnHandle>> for live pool updates.
let db = db_rx.borrow().conn().clone();
```

Never hold `db_rx.borrow()` across an `.await` point — the read lock blocks `watch::Sender::send()`. Clone the `Arc<DbConnHandle>` first, then drop
the borrow.

### UpdateStatus grouping helpers

Use `UpdateStatus::unfinished()` and `UpdateStatus::host_blocking()` for status filters — do not inline the status arrays at call sites.

- `unfinished()` — all four non-terminal statuses (Queued, Pending, InProgress, AwaitingRestart). Use for: "does an active row exist for this (host,
  item)?", state reporting queries.
- `host_blocking()` — excludes Queued. Use for: "is this host currently occupied by an in-flight update?", host-level serialisation checks.

### Per-item policy override pattern

Use the **three-state override** model for per-item policy configuration:

- **Inherit** — no override row exists; effective policy comes from global defaults.
- **Disable** — override row with the "none"/"disabled" mode; item opts out regardless of global.
- **Configure** — override row with a real mode + field values; item has an explicit policy.

Row-level inheritance is signalled by the absence of a row, not by null field values.

Within a configured override row, a null dimension value inherits the global default for that dimension (per-field cascade). For example: an item
override with `scaling_mode = delta`, `delta_cores = 2`, and `delta_memory_mb = NULL` will use the global default's `delta_memory_mb` at runtime. The
UI should communicate this by labeling null/empty fields "inherit from global".

When implementing surfaces for three-state policies: use a 4-value `scaling_mode` selector (`inherit` / `none` / mode-specific values) so
`FormVisibleWhen`'s single-field condition can gate dimension fields without compound logic. Cross-mode field inheritance is forbidden: if the
effective mode is `delta`, only `delta_*` dimensions cascade from global; `absolute_*` dimensions are cleared even if the global has them set.

## Exhaustive Enum Dispatch

Wildcard arms in dispatch functions are forbidden. A function that maps enum variants to domain values (timeout, routing key, HTTP status code) must
enumerate every known variant explicitly — a new variant must not silently inherit an arbitrary default.

Extend the `#[non_exhaustive]` rule from the "Public Enum Extensibility" section:

- **Closed enum**: remove the wildcard entirely. The compiler enforces exhaustiveness at compile time.
- **`#[non_exhaustive]` enum** (e.g., `Capability`, `UpdateFinalStatus`): a wildcard is required in external crates, but it must never be silent.
  Replace `_ => some_default` with a `tracing::warn!` + a documented safe fallback, and replace `_ => unreachable!()` with `tracing::warn!` + early
  return. Never use `unreachable!()` on values that come from wire or database state.

```rust
// ✓ Correct — unknown profile logged; safe fallback chosen explicitly
match profile {
    ServiceProfile::Agent | ServiceProfile::Unknown => Some(AGENT_SHUTDOWN_TIMEOUT_SECS),
    ServiceProfile::Scheduler => Some(SCHEDULER_SHUTDOWN_TIMEOUT_SECS),
    ServiceProfile::UpdateTracker => None,
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

Functions must not require `#[allow(clippy::too_many_arguments)]`. No Clippy suppression is approved in this codebase
(AGENTS.md rule "Do not add any `#[allow()]`"). When a function's non-`self` parameter count exceeds Clippy's
threshold (7), introduce a named grouped struct to batch related scalar or reference parameters:

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

Name the struct after its semantic role (`ProcessDiscoveryArgs`, `CreateServiceArgs`), not a generic label like `Params`. The struct should be private
to the module unless it is part of a public API.

## Security Audit Logging

Security-relevant mutations must emit semantic audit entries through `uptrakit-audit-log` APIs. Do not add new `target: "security_audit"` tracing
producers.

See [Logging — `security_audit` Target](logging.md#security_audit-target) for legacy/deprecation notes and runtime filtering guidance.

### When to use

Emit semantic audit entries for operations that:

- Creates, modifies, or deletes plugin configs containing command-bearing fields (`version_command`, `update_command`, `post_pull_command`, hook
  `commands`)
- Modifies access grants or role assignments
- Changes credential-bearing settings (SMTP passwords, OIDC secrets, NATS URLs)
- Approves or revokes services with credential capabilities

### Required audit fields

| Field          | Type                                                    | Description                                                     |
| :------------- | :------------------------------------------------------ | :-------------------------------------------------------------- |
| `action_type`  | `AuditActionType`                                       | Canonical action constant (for example `PLUGIN_CONFIG_UPDATE`)  |
| `outcome`      | `AuditOutcome`                                          | `Success`, `Denied`, `ValidationFailed`, `Failed`, or `Partial` |
| Scope          | `tenant_scope(...)` or `system_scope()`                 | Choose the correct audit table                                  |
| Actor          | `actor(...)`, `actor_service(...)`, or `actor_system()` | Who performed the action                                        |
| Target         | `target(...)` / `target_opt(...)`                       | Optional semantic target identity                               |
| `details_json` | JSON (optional)                                         | Minimal, non-secret metadata                                    |

### Example

```rust
let entry = uptrakit_audit_log::AuditEntry::builder(
    uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_CREATE,
)
.tenant_scope(tenant_db.tenant_id)
.actor(actor_type, actor_id)
.target("plugin_config", id.to_string(), Some(req.name.clone()))
.outcome(uptrakit_audit_log::AuditOutcome::Success)
.details(serde_json::json!({
    "plugin_type": req.plugin_type,
    "command_fields": fields,
}))
.build()?;

state.audit_emitter.emit_best_effort(entry);
```

### Log level rationale

Severity is modeled in `AuditOutcome` and action semantics, not by forcing a dedicated tracing target/level convention. `JournaldBackend` emits
structured audit events to `uptrakit_audit`.

### V2 deferrals

- `audit_log.filter` and `audit_log.retention_days` remain configuration keys but are not yet a complete per-tenant enforcement surface for semantic
  producers.

---

## Visibility and Module Boundaries

Rust provides four visibility levels. Use the narrowest level that satisfies the actual call-site requirements:

| Scope             | Keyword      | Use when                                                    |
| ----------------- | ------------ | ----------------------------------------------------------- |
| Private (default) | _(none)_     | Item is only used in the same module                        |
| Crate-internal    | `pub(crate)` | Item is used across modules/files **within the same crate** |
| Parent module     | `pub(super)` | Item is used only in the parent module                      |
| Fully public      | `pub`        | Item crosses a **crate boundary** (called by another crate) |

### The `unreachable_pub` lint

The workspace enforces this table via the `unreachable_pub` lint (set to `deny` in `[workspace.lints.rust]`). The lint fires whenever a `pub` item is
**not reachable** through the crate's public module chain. It is self-selecting:

- `pub fn foo()` inside a `mod bar { … }` where `bar` is not re-exported → **fires** (downgrade to `pub(crate)` or `pub(super)`)
- `pub fn foo()` inside a `pub mod bar { … }` that is re-exported from `lib.rs` → **does not fire** (the item is genuinely reachable from outside the
  crate)

No `#![allow(unreachable_pub)]` (or any other crate-wide or module-wide `#![allow(...)]`) should appear anywhere in the workspace.

#### Using `#[allow(...)]` — policy and requirements

Item-level `#[allow(...)]` is permitted **only** when a lint produces a false positive that cannot be resolved structurally. The one approved cause in
this codebase is **feature-gating**: an item that is used only when a specific Cargo feature is enabled will appear unused or unreachable in builds
that omit that feature, even though it is correctly public/used in the intended build.

```rust
// ✅ Approved — the field is only read when the "metrics" feature is compiled in.
// Without the allow, the lint fires on --no-default-features builds.
#[allow(dead_code)] // used only with feature = "metrics"
pub counter: u64,
```

**Every `#[allow(...)]` must be accompanied by a comment** on the same line or the line immediately above it explaining precisely why the suppression
is necessary. A bare `#[allow(...)]` with no explanation is not permitted and will be rejected in review.

**`#[allow(...)]` must be placed at the smallest possible scope.** Do not suppress a lint on a function, struct, or module when only a single field,
binding, or expression triggers it. Place the attribute on that specific item instead:

```rust
// ❌ Suppresses the lint for the entire function
#[allow(unused_variables)] // cfg-gated param
fn handle(ctx: &Context, payload: Payload) {
    ctx.process();
}

// ✅ Suppress only the specific binding that is unused
fn handle(ctx: &Context, #[allow(unused_variables)] payload: Payload) { // cfg-gated param
    ctx.process();
}

// ✅ Or use the underscore prefix convention when no attribute is needed at all
fn handle(ctx: &Context, _payload: Payload) {
    ctx.process();
}
```

### Concrete examples

```rust
// ❌ BEFORE — `pub` inside a private module
mod queries {
    pub fn fetch_all(db: &Db) -> Vec<Item> { … }
}

// ✅ AFTER — narrowed to pub(crate) because the function does not cross a crate boundary
mod queries {
    pub(crate) fn fetch_all(db: &Db) -> Vec<Item> { … }
}
```

```rust
// ✅ Correct — pub is justified because another crate calls this
// lib.rs re-exports the module:  pub mod routes;
pub mod routes {
    pub fn health() -> &'static str { "ok" }
}
```

```rust
// ❌ BEFORE — pub helper only used inside the same file
pub fn build_filter(id: Uuid) -> Condition { … }

// ✅ AFTER
fn build_filter(id: Uuid) -> Condition { … }   // or pub(crate) if used in a sibling module
```

### Plugin crate rule

Trait implementation methods are implicitly `pub` when required by the trait — do not annotate them with an explicit `pub`. Only freestanding helper
functions in plugin submodules need visibility tightening:

```rust
// ✅ Trait impl — no explicit pub needed
impl ReleasePlugin for GitHubPlugin {
    async fn fetch_latest(&self) -> Result<Release> { … }
}

// ✅ Freestanding helper — must use pub(crate) if not part of the public API
pub(crate) fn parse_semver_tag(tag: &str) -> Option<semver::Version> { … }
```

### Cross-reference

- [Error Handling](error-handling.md) — public error types follow the same rule: use `pub(crate)` for errors that never cross a crate boundary.
- [Security](../security/README.md) — avoid leaking internal types through `pub` that could expose security-sensitive implementation details.

## Reloadable Trait

Every long-lived subsystem that participates in config reload must implement `Reloadable` (in `uptrakit_config_reload::reloadable`). Use the
`reloadable_erased_impl!` macro to generate the `ReloadableErased` adapter for dynamic dispatch:

```rust
uptrakit_config_reload::reloadable_erased_impl!(MyReloadable, RuntimeConfigDelta::MySection);
```

Rules:

- `validate()` is pure — no side effects, only check invariants.
- `apply()` takes an `Arc<Config>` snapshot and saves the pre-apply state internally for `revert()`.
- `revert()` restores the pre-apply state without calling external services.
- `health_check()` verifies the subsystem accepted the new config and is operating normally.
- `rollback_window()` returns the maximum duration the watchdog waits for `health_check()`.

## Reloadable Subsystems Must Have a Live Consumer

A `Reloadable` whose `apply()` has no live consumer must not exist. If a config section changes but
nothing in the running process reads the new value — because the subsystem holds a client, connection,
or resource captured once at boot with no swap seam — that is not a candidate for a silent `apply()`
that republishes to a channel nobody reads. Choose one of:

- **Wire a real subscriber or swap mechanism.** If the value can be hot-applied, `apply()` must mutate
  the actual live state the subsystem uses (an `arc_swap`, a `watch` channel a consumer polls, etc.), not
  just an in-memory snapshot next to the real one.
- **Validate-reject the change.** If the subsystem has no swap seam, `validate()` must return an error
  for any delta to that field, forcing the coordinator to report the reload as rejected rather than
  applied.

Never let `apply()` return `Ok(())` for a change it did not actually apply. The coordinator's success
report is a promise to the operator that the new value is live; a `Reloadable` that accepts a delta it
silently ignores breaks that promise — the coordinator reports `ConfigReloadApplied` while nothing
changed. See the NATS URL reload gate (`crates/core/controller-runtime/src/reload/nats.rs`) for a
validate-reject implementation of this rule, and [Operator Runbook — Graceful
Reload](../end-user/operator-runbook-reload.md) for the operator-facing behavior this rule produces.

## Reexec Hook Pattern

When a config reload detects an irreversibly-bound key change (e.g. `db.url`, `master_key`, `log.path`, embedded-service topology), the coordinator
delegates the decision to a `ReexecHook` implementation registered at startup.

**Rules:**

- The `uptrakit-config-reload` crate defines `ReexecHook` and `ReexecOutcome`; it must not import `triage::decide` or `perform_reexec` from
  `controller-runtime`. This boundary keeps the shared crate ignorant of process-exec internals.
- The `controller-runtime` crate implements `ControllerReexecHook` (which calls `triage::decide` and `perform_reexec`) and registers it via
  `coordinator.set_reexec_hook(...)` before spawning `coordinator.run()`.
- Capture `current_exe` via `std::env::current_exe()` at startup (before the hook is constructed) and propagate any error through `run_server()`'s
  `Result` return. Never call `current_exe()` inside the hook — it may fail after a process name change.
- Listener FDs for `perform_reexec` are captured by pre-binding HTTPS (and PKI when configured) sockets in `run_server()` before spawning server
  tasks. The raw FD integer is valid after the socket is moved into the server task; `clear_cloexec_raw` uses the integer, not the Rust wrapper.
- When no `ReexecHook` is registered (e.g. in tests), the coordinator skips the reexec check and proceeds with in-process apply.

## Boot Phase Pattern

**Mandatory:** New controller subsystems must be added as a new `boot/<phase>.rs` file with a free
async function that returns a typed output struct. Never add an inline block to an existing phase
function or to `run_server`; doing so re-monolithizes the orchestrator that ADR 0023 decomposed.

Rules:

- One file per phase. The phase function signature is `async fn <phase>(deps…) -> Result<<Output>>`.
- The output struct carries only the values this phase produces. High-fan-out values already in
  `Arc<AppState>` must not be duplicated into phase structs.
- Post-assembly phases (`recovery`, `serve`) take `Arc<AppState>` as their primary argument, not an
  accumulation of phase structs.
- If a phase has five or more distinct sub-concerns (e.g. OAuth + PKI + TLS + JWT + cert_signer),
  start it as a sub-module directory (`boot/<phase>/mod.rs` + per-concern files) rather than a
  single file that will immediately need splitting.
- The consecutive-FD atomicity invariant: inherited-FD claim, HTTPS bind, PKI bind, and both
  `clear_cloexec` calls must remain in one atomic function (`boot::listeners::claim`). No
  fd-allocating call may run between the HTTPS and PKI binds.

See [ADR 0023](../adr/0023-controller-boot-phase-decomposition.md) for the full rationale, module
layout, and guard rails against re-forming a god-struct.

## Per-Section Watch Pattern

Config changes flow through `tokio::sync::watch<Arc<SectionConfig>>` channels. Inject receivers at construction time; never pass config values
directly through function arguments for long-lived subsystems:

```rust
// In constructor:
let (tx, rx) = watch::channel(Arc::new(initial_config));
// Consumer holds rx and calls:
let config = rx.borrow().clone();
```

Consumers that react to changes (instead of reading on each request) use `rx.changed().await`.

## No Static-Init Config

Do **not** use `lazy_static!` or `OnceLock`/`OnceCell` for configuration values. All config that can change at runtime must flow through watch
channels. Static init for config creates a stale snapshot that bypasses the reload path.

## Plugin Constructor Budget

Plugin `from_config()` constructors are called every time the plugin's config changes (drop-and-recreate model). Constructors must be O(small):

- Allocate only cheap state (`String`, `Vec<String>`, parsed scalar values).
- Move expensive resources (`reqwest::Client`, SMTP sessions, compiled regexes) into `Arc`/`OnceLock` outside the plugin struct so they survive plugin
  replacement.
- See `crates/plugins/notifications/email/src/plugin.rs` for the reference implementation.

### InstalledVersionEnricher (controller-only)

When an agent reports an opaque `installed_version` (e.g. a git tree SHA), the controller can enrich it with a human-friendly
`installed_display_version` through the `InstalledVersionEnricher` trait. The role is controller-only and its factory receives an
`InstalledVersionEnrichmentContext` (mirror of `ReleaseFetchContext` from ADR-0015), so the enricher can reach the global GitHub provider or
similar shared resources.

Declare it like any other role:

```rust
roles: [
    // ...
    InstalledVersionEnricher { host_requirements: HostRequirements::CONTROLLER_ONLY },
],
extra_capabilities: [PluginCapability::EnrichInstalledVersion],
installed_version_enricher_create: {
    create: create_installed_version_enricher_my_plugin,
    host_requirements: HostRequirements::CONTROLLER_ONLY,
},
```

Web-api dispatches via descriptor slot lookup; no plugin-type strings appear in the handler (ADR-0018). The trait returns a `Vec` the same
length and order as the input; dispatcher zips by index. See ADR-0021 for the full contract, observability tags, and the 90-commit
operational ceiling.

## File vs. DB Section Assignment

Config key ownership: TOML file owns structural config (listen addresses, DB pool URL, TLS, NATS, zeroconf). The `global_settings` DB table owns
runtime-tuneable values (audit filter, retention). Per-tenant settings remain in the `settings` DB table. The migration
`m20260512_000001_drop_file_keys` removes the DB rows that moved to TOML.

## Service Binary/Runtime Boundary

Every Service binary crate (`agent-ssh`, `mqtt`, `scheduler`, …) is a **thin launch shell**. All business logic, DB entities, migrations, protocol
handling, and crypto helpers live in the corresponding `-runtime` crate. See [ADR-0005](../adr/0005-service-binary-runtime-boundary.md).

### What belongs where

| Binary crate (`*`)       | Runtime crate (`*-runtime`)                         |
| ------------------------ | --------------------------------------------------- |
| `main.rs` — process init | `ServiceHandler` implementation                     |
| `cli.rs` — clap structs  | DB entities and migrations (`service_migrations()`) |
| Subcommand dispatch      | Business logic, surface handlers, crypto helpers    |
| _Nothing else_           | Protocol implementation, transport adapters         |

### service_migrations()

Runtime crates that own a local DB override `ServiceHandler::service_migrations()` (feature-gated via `uptrakit-service-sdk/service-migrations`) to
return their migration list. The controller calls it as a static method on the concrete handler type at startup:

```rust
let migrations = AgentSshHandler::service_migrations();
run_migrations_with_plugins(db, migrations).await?;
```

Services without a DB rely on the default `vec![]`.

### Embedded service construction

The controller constructs the handler with controller-sourced deps (shared DB, state dir, pre-generated ECIES keypair), then passes it to
`run_embedded_service::<H>`. The handler's constructor must not open its own DB connections or read paths from the environment.

```rust
let handler = AgentSshHandler::new(shared_db, state_dir, AgentSshMode::Embedded, Some(keypair));
run_embedded_service(handler, transport, tokens.drain, tokens.abort).await;
```

The standalone binary does the same with `AgentSshMode::Binary` and `None` for the keypair.

## Publishable Crate Dependency Hygiene

Two crates in this workspace are published to crates.io:

- `uptrakit-service-sdk`
- `uptrakit-openapi-client`

Their transitive dep trees (including `[dev-dependencies]` of any crate they reach) must NOT contain any of:

- `uptrakit-audit-log`
- `uptrakit-audit-log-derive`
- `uptrakit-shared-db`
- `uptrakit-tenant-db`
- `uptrakit-crypto`

These five crates are workspace-internal database and encryption plumbing. They have no external consumers and must not be republished to crates.io.

### Why this matters

`cargo publish` (and crates.io's manifest validator) check every named dep entry in the published manifest — including `[dev-dependencies]` that carry
a `version` field, and optional deps — against the registry. A dev-dep on `uptrakit-audit-log` from any crate that the publishable crates transitively
reach is enough to force `audit-log` onto crates.io, and `audit-log` in turn forces `shared-db`, which forces `crypto` and `tenant-db`. The chain is
load-bearing on every edge: cutting any link breaks all of it.

### Enforcement

Two integration tests guard this rule:

- `crates/shared/service-sdk/tests/no_workspace_db_deps.rs`
- `crates/shared/openapi-client/tests/no_workspace_db_deps.rs`

Each test walks the resolved cargo metadata graph (default features and `--all-features`) and panics if any banned name appears, naming the dep chain
back to the publishable crate.

### Why these five and not other internal crates?

Most workspace-internal crates (`uptrakit-build-info`, every plugin, every runtime, etc.) inherit `publish = true` from Cargo's defaults but are kept
off crates.io by `release-plz.toml` declaring `release = false`. That is sufficient because release-plz is the only mechanism that publishes from this
workspace. These five crates additionally carry the belt-and-suspenders `publish = false` in their own `Cargo.toml` because they are the unique
failure case where the squat chain demonstrably reformed once before; locking them in their manifests defends against a contributor running
`cargo publish -p uptrakit-shared-db` directly (bypassing release-plz) and resurrecting the chain.

If you find yourself wanting to add one of these crates to anything in the service-sdk or openapi-client subtree (including dev-deps), stop and think
about what you're actually testing. The wire-side fix for the historical version of this rule replaced two `AuditActionType::*` constants with a
synthetic `TEST_ACTION_TYPE` constant in `crates/shared/wire/src/tests.rs` — the test was asserting serde round-trip shape, not catalog correctness,
so the constant binding added no coverage.

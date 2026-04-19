# Coding Standards

For maintainability-focused Rust conventions beyond the hard rules in this file, see
[Rust Idioms](rust-idioms.md).

## Error Handling

For comprehensive error handling patterns, conventions, and the full decision guide, see
[Error Handling](error-handling.md). Key points:

- Wrap errors in `rootcause::Report` and define a `Result<T>` alias per boundary.
- Use `thiserror::Error` with `#[derive(Debug, Error)]` to describe failures.
- Implement `ReportConversion` (via `impl_report_conversion!`) for all downstream errors and prefer `.context_to()?` to preserve the chain.
- Use `report!(MyError::Variant(…))` for creating new error reports. Never call
  `rootcause::Report::new(…)` directly — the macro additionally captures source location.
- Use `bail!(MyError::Variant(…))` for early returns.
- Avoid `Result<T, String>`; prefer typed enums. **Exception:** in `web-api` route
  handlers and their private validation helpers, `Result<T, String>` is acceptable
  when the string is a user-facing error message that the caller maps to an HTTP
  error response (e.g., via `error_response(StatusCode::BAD_REQUEST, msg)`). This
  avoids `clippy::result_large_err` from returning `Response` directly and keeps
  validation helpers decoupled from HTTP types.
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

## Error Masking Anti-Patterns

Never use `.unwrap_or(N)` or `.unwrap_or_default()` as a silent fallback for database errors.
When the database is unavailable a fallback value produces incorrect program behavior:

- **Security paths:** `count(db).await.unwrap_or(1) > 0` in a registration check treats a DB
  error as "user exists", silently blocking legitimate registrations or skipping the
  registration-token-required path.
- **Data-integrity guards:** `count_linked_hosts(db).await.unwrap_or(0)` treats a DB error as
  "no linked hosts", potentially allowing a soft-delete that would orphan active records.

### Required pattern — route handlers

Use a `match` and return `StatusCode::INTERNAL_SERVER_ERROR` on `Err`, logging the error at the
`error` level:

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

See [AppState Architecture](app-state.md) for the full sub-state pattern, service extractors,
and `db_access_policy.toml` classification rules.

### Required pattern — query functions

Return `Result<T, DbErr>` (or a crate-local `Result`) and propagate errors with `?` at the call
site. Never collapse errors into a default value:

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

The narrower `unwrap_or(0)` rule for count queries in paginated list endpoints is documented in
[Database Query Patterns](#database-query-patterns). This section covers the broader class of
`.unwrap_or(N)` misuse in security and data-integrity code paths.

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
- `Permission`
- `AccessPreset`

**`uptrakit-internal-wire`:**

- `CloseReason`
- `ServiceMessage`, `ControllerMessage`
- `EnrollmentStatus` (with `Other(String)` catch-all)
- `ErrorCode` (with `Other(String)` catch-all)
- `UpdateFinalStatus` (with `Other(String)` catch-all — loses `Copy`)
- `DisconnectReason` (with `Other(String)` catch-all — loses `Copy`)

**`uptrakit-web-api-types`:**

- `AlertSeverity`
- `TriggerUpdateStatus`
- `UpdateStatus`
- `RegistrationMode`
- `NotificationEventType`, `NotificationDeliveryStatus`

**`uptrakit-plugin-infrastructure-core`:**

- `PluginCapability`
- `HostCompatibility`
- `PluginError`

When adding a new public enum, apply `#[non_exhaustive]` by default unless the enum is explicitly guaranteed to be closed (e.g., a two-variant
boolean-like enum).

## Exhaustive Enum Test Coverage

Enum tests (serde round-trips, `Display`/`FromStr` checks, `as_str()` invariants) must automatically cover every
variant. A manually maintained array like `const ALL_VARIANTS: [T; 4]` silently skips any new variant added later.

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

Enums with `#[cfg_attr(feature = "sea-orm", derive(strum::EnumIter, ...))]` already derive `EnumIter` when the
`sea-orm` feature is active. Adding `#[cfg_attr(test, derive(strum::EnumIter))]` on top causes a duplicate
implementation when both `cfg(test)` and `feature = "sea-orm"` are active simultaneously. Use the combined guard:

```rust
#[cfg_attr(all(test, not(feature = "sea-orm")), derive(strum::EnumIter))]
```

This ensures:

- Tests run without `sea-orm` → the `cfg(test)` guard derives `EnumIter`.
- Tests run with `sea-orm` → the sea-orm feature already derives `EnumIter`; the guard is a no-op.

### cfg propagation caveat

`#[cfg(test)]` is **not** propagated to dependency crates. If crate B depends on crate A, the
`#[cfg_attr(test, derive(strum::EnumIter))]` on an enum in crate A will not make `EnumIter` available in crate
B's test code. For enums in external crates, use inline arrays in tests (keeping them complete by always listing
all known variants explicitly).

### Anti-pattern — hardcoded const array

```rust
// ✗ Wrong — silently skips new variants; no compile-time enforcement
const ALL_STATUSES: [MyStatus; 3] = [MyStatus::Pending, MyStatus::Active, MyStatus::Completed];
for status in &ALL_STATUSES { ... }

// ✓ Correct
for status in MyStatus::iter() { ... }
```

### `strum::EnumIter` incompatibility with `Other(String)` variants

`strum::EnumIter` cannot be derived on enums that contain an `Other(String)` catch-all variant
(see [Wire-Safe `Other(String)` Catch-All](#wire-safe-otherstring-catch-all-for-enums) below).
Instead, enumerate known variants explicitly in a `const` array inside the test:

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

Any enum serialised over the wire (WebSocket, NATS, REST body) that may gain new variants in
future releases **must** include an `Other(String)` catch-all variant. This ensures rolling upgrades
are safe: an older peer receiving an unknown variant from a newer peer deserialises it as
`Other("future_variant")` and handles it gracefully instead of returning a
deserialization error.

### When to use

Apply this pattern to every `#[non_exhaustive]` enum that:

- is transmitted over a network protocol (`ServiceMessage`, `ControllerMessage`, `EnrollmentStatus`, `ErrorCode`, etc.),
- is returned as a JSON string in a REST API response (`NotificationEventType`, `NotificationDeliveryStatus`, etc.),
- or is persisted in a column and read back by potentially older software versions.

### Required implementation — use `wire_safe_enum!`

Use the `wire_safe_enum!` macro from `uptrakit-shared-macros` to generate all required boilerplate.
The macro emits: `#[non_exhaustive]` + `Other(String)`, `as_str()`, `Display`, `From<String>`
(infallible with `tracing::debug!` on unknown), `Serialize`, `Deserialize`, a named parse-error
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

For enums whose `From<String>` or `FromStr` impls require custom logic not expressible as a simple
string table (e.g. infallible `FromStr` that maps unknowns to a sentinel rather than `Err`), write
the impls by hand following the pattern in `crates/shared/wire/src/lib.rs`.

### Consequences

- The enum **loses `Copy`** (because `String` is not `Copy`). Any call-site that relied on
  copy semantics must be updated to `.clone()`.
- `strum::EnumIter` cannot be derived. See the
  [test coverage section above](#strumenumiter-incompatibility-with-otherstring-variants).

## `#[non_exhaustive]` on Public Structs

`#[non_exhaustive]` applies to structs as well as enums. Add it to any public struct defined in a
shared crate (`wire`, `shared-types`, `web-api-types`, etc.) that may gain new fields in the future.
This prevents external crates from using struct-literal syntax and breaks at compile time if they try
to match exhaustively.

### Required constructor

Because `#[non_exhaustive]` prevents external callers from constructing the struct with a literal,
every `#[non_exhaustive]` struct **must** expose a constructor or implement `Default`:

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

Internal query functions that write to the database should use typed enums instead of bare `&str`
parameters for discriminator values such as actor type, batch type, and similar classification
fields. Bare strings produce no compile-time guarantees and make it trivial to introduce silent
typos.

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

## Credential-Holding Types and Debug

Any internal struct that contains a credential (password, token, secret key, etc.) **must** store
it as `SecretString` (not `String`). This enforces the masking guarantee at the type level:

- `SecretString`'s `Debug` impl emits `"***"` automatically — no hand-written `Debug` needed.
- The value is zeroed from memory on drop (`ZeroizeOnDrop`).
- `.expose_secret()` is the only way to access the inner value, making every access site explicit
  and auditable.

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

#### Pattern A — gate the entire match arm

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

#### Pattern B — stub + upgrade helper

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

#### Pattern C — always-present tracking field

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

#### Pattern D — conditional early-return inside a guard

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

When a function, type, field, or constant is *only reachable* via a `#[cfg(feature = "X")]`
additive block, the compiler may emit `dead_code` (or a related lint) when that feature is
disabled. Because `#[cfg(not(feature = "X"))]` is prohibited and the item is genuinely needed
under the feature, suppressing the lint with `#[allow(dead_code)]` is the approved solution.

**Requirement:** every such suppression must carry a detailed inline comment that:

1. Names the Cargo feature that gates the sole caller or user of the item.
2. Explains why the item cannot be removed or restructured to avoid the suppression.

```rust
// ✓ Correct — suppression with mandatory explanation
/// Upgrades a no-op stub client to a real bollard client.
// Only called from the `daemon` feature block in `DockerPlugin::new`. Without the `daemon`
// feature the function is unreferenced, but it must remain compiled for the feature to work.
#[allow(dead_code)]
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

Note: when the item is already behind `#[cfg(feature = "X")]`, the dead-code lint only fires
under a build that enables `X` but not the specific caller — which is rare. If the item and
its sole caller are both inside the same `#[cfg(feature = "X")]` block, no suppression is
needed (the compiler sees them together). Use `#[allow(dead_code)]` only after confirming the
lint is genuine.

No other `#[allow()]` suppressions are permitted without explicit approval.

## Atomic Ordering Requirements

Security-critical `AtomicBool` flags (such as `PLAINTEXT_MODE` in `uptrakit-crypto`) must use
`Ordering::Release` for stores and `Ordering::Acquire` for loads. `Ordering::Relaxed` is
incorrect for flags that gate security behavior — on weakly-ordered architectures (ARM), a
thread could see a stale value and either skip encryption or encrypt when plaintext mode was
intended.

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

**Rule:** Any `AtomicBool` or `AtomicU*` that controls a security-sensitive code path must use
at minimum `Release`/`Acquire` ordering. `Relaxed` is only acceptable for pure counters or
statistics where stale reads have no correctness impact.

## Synchronous Locks in Async Code

When a synchronous lock is required anywhere in async code, use `parking_lot::Mutex` or
`parking_lot::RwLock`. **Never use `std::sync::Mutex`, `std::sync::RwLock`,
`tokio::sync::Mutex`, or `tokio::sync::RwLock`.**

- **Sub-microsecond critical sections** with no `.await` across the lock make a sync lock
  correct (no risk of holding across a yield point).
- `parking_lot` primitives are faster under contention and return the guard directly — no
  `Result`/`.unwrap()` needed, which aligns with the workspace panic policy.
- `tokio::sync::Mutex`/`RwLock` are unnecessary overhead for critical sections that do not
  span `.await` points, and their guards are not `Send`, preventing use in `tokio::spawn`
  closures unless the guard is dropped before the first `.await`.
- **Always drop `parking_lot` guards before any `.await` point.** Clone or copy the
  protected value out of the guard, drop the guard, then `.await`.

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

**Amortize expensive operations under the lock.** If the critical section includes cleanup
(e.g., `HashMap::retain()`), do not run it on every call. Use an `AtomicU64` counter to run
cleanup every N calls, keeping per-request lock hold time O(1):

```rust
static CALL_COUNT: AtomicU64 = AtomicU64::new(0);
const CLEANUP_INTERVAL: u64 = 100;

let call_count = CALL_COUNT.fetch_add(1, Ordering::Relaxed);
if call_count.is_multiple_of(CLEANUP_INTERVAL) {
    guard.retain(|_, entry| entry.last_seen >= cutoff);
}
```

## Parallel Broadcast Pattern

When broadcasting messages to multiple consumers via `mpsc::Sender`, use parallel sends with
a per-send timeout. Sequential sends allow a single slow consumer (full channel buffer) to
block all other recipients.

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

| Type | Key validations |
| --- | --- |
| `RegisterRequest` | email format (contains `@`, max 254 chars), `first_name` non-empty, password 8–1024 chars |
| `LoginRequest` | email format, password non-empty |
| `CreateOidcProviderRequest` | name non-empty, slug format (lowercase+digits+hyphens, 1–64), issuer_url scheme, client_id non-empty |
| `UpdateScheduledTaskRequest` | interval_seconds > 0, jitter_seconds >= 0 |
| `UpdateNetworkSettingsRequest` | trusted_proxies items non-empty, real_ip_header non-empty, pki_addr URL format |
| `CreateSoftwareItemRequest` | name non-empty, exactly one of plugin_config_id/plugin_config |
| `CreatePluginConfigRequest` | name non-empty |
| `CreateApiTokenRequest` | `name` non-empty (after trim) |
| `CreateEnrollmentTokenRequest` | `name` non-empty; `max_uses` if present must be > 0; `expires_in_seconds` if present must be > 0 |
| `UpdateServiceRequest` | `ping_interval_seconds` if present must be 0 (sentinel: clear override) or ≥ 5 |
| `CreateSoftwareIgnoreRequest` | `name` or `package_identifier` non-empty (after trim) depending on rule type |

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
    // handler body -- 401/403 already handled by the extractor
}
```

Use the bound variable name `_user` when the `AuthenticatedUser` value is not used in the body, and `user`
when it is (e.g. `user.user_id` for an `actor_id` field).

There are 32 granular permission extractors (e.g. `CanViewServices`, `CanApproveServices`,
`CanCreateSoftware`, `CanTriggerUpdates`, `CanManageUsers`). See
[Authentication and Authorization](../security/auth-and-authorization.md#permission-extractor-reference)
for the full list.

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

### Approved exception: custom authentication paths

Handlers that perform their own token extraction (e.g., reading a `token` query parameter or
`Authorization` header because browser WebSocket connections cannot set custom headers) cannot
use Axum extractors for authentication. In these handlers, `auth_user.has_permission(perm)` is
acceptable **only** when:

1. The token validation is already done manually (JWT or API token, same logic as the standard
   middleware), **and**
2. No typed extractor exists that covers the custom auth path.

Any such handler must include a `// APPROVED: custom auth path — extractor not applicable`
comment alongside the `has_permission` call. The `interactive_ws` WebSocket endpoint is the
canonical example.

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

**Exception:** `Path<String>` is correct for non-UUID path parameters (e.g., base64-encoded OCSP
requests in `ocsp.rs`).

## UUID Query Parameters

Use `Option<Uuid>` (not `Option<String>`) for UUID-typed query parameters. Axum's serde
deserialization automatically rejects malformed UUIDs with `422 Unprocessable Entity`. Manual
`.and_then(|s| Uuid::parse_str(s).ok())` silently swallows invalid values, returning the
"no filter" behaviour instead of an error.

### Required pattern

```rust
use uuid::Uuid;

#[derive(Deserialize)]
struct MyQuery {
    // Axum returns 422 automatically for malformed UUIDs
    plugin_config_id: Option<Uuid>,
}

// In the handler, use params.plugin_config_id directly — no parse needed
```

### utoipa annotations

Declare `Option<Uuid>` in utoipa `params(…)` so the OpenAPI schema emits `format: uuid`:

```rust
#[utoipa::path(
    delete,
    path = "/api/v1/hosts/{id}/discovered",
    params(
        ("id" = Uuid, Path, description = "Host UUID"),
        ("plugin_config_id" = Option<Uuid>, Query, description = "Filter by plugin config UUID"),
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

See also: [Typed Path Extractors](#typed-path-extractors) for the equivalent rule on path
parameters.

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

## Tracing Status Codes

When logging HTTP status codes in `tracing!` macros, use the `%` display format rather than
`.as_u16()`:

```rust
// Correct — uses Display impl, emits "200 OK" or "404 Not Found"
tracing::debug!(status = %response.status(), "request complete");

// Wrong — emits a bare integer with no reason phrase
tracing::debug!(status = response.status().as_u16(), "request complete");
```

The `StatusCode` type's `Display` implementation produces `"<code> <reason>"` (e.g., `"429 Too
Many Requests"`), which is more informative in logs than a bare integer. The `.as_u16()` method
is approved only inside serde serialization helpers where JSON wire compatibility requires a
numeric value.

## Constant-Time Secret Comparison

Externally-provided secrets (webhook tokens, API keys, and similar short-lived credentials) must
**never** be compared using `==` or `!=`. Rust's default `PartialEq` on `&str` short-circuits on
the first differing byte, leaking timing information that an attacker can exploit to infer the
secret one byte at a time.

**Rule:** Whenever code validates a caller-supplied secret against an expected value, use
`subtle::ConstantTimeEq` after normalising both sides to a fixed-length representation.

### Required pattern

Add `subtle = { workspace = true }` to the crate's `Cargo.toml` and use the SHA-256 + `ct_eq`
idiom:

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

Hashing first ensures both inputs are exactly 32 bytes before calling `ct_eq`, making the
comparison unconditionally constant-time regardless of input length differences.

### Anti-pattern table

| Wrong | Right | Reason |
| --- | --- | --- |
| `provided != expected` | `ct_eq` with SHA-256 pre-hashing (see above) | Short-circuit timing leak |
| `provided == expected` | `ct_eq` with SHA-256 pre-hashing (see above) | Short-circuit timing leak |
| `subtle::ConstantTimeEq` directly on `&str` | Hash first, then `ct_eq` | Length difference leaks through variable-time `ct_eq` implementations |

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
pre-load software ignore rules per `(tenant_id, plugin_config_id)`, not per `tenant_id` alone — the
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

## Security Audit Logging

Security-relevant mutations must emit semantic audit entries through
`uptrakit-audit-log` APIs. Do not add new `target: "security_audit"` tracing producers.

See [Logging — `security_audit` Target](logging.md#security_audit-target) for legacy/deprecation
notes and runtime filtering guidance.

### When to use

Emit semantic audit entries for operations that:

- Creates, modifies, or deletes plugin configs containing command-bearing fields
  (`version_command`, `update_command`, `post_pull_command`, hook `commands`)
- Modifies RBAC permissions or role assignments
- Changes credential-bearing settings (SMTP passwords, OIDC secrets, NATS URLs)
- Approves or revokes services with credential capabilities

### Required audit fields

| Field | Type | Description |
| :--- | :--- | :--- |
| `action_type` | `AuditActionType` | Canonical action constant (for example `PLUGIN_CONFIG_UPDATE`) |
| `outcome` | `AuditOutcome` | `Success`, `Denied`, `ValidationFailed`, `Failed`, or `Partial` |
| Scope | `tenant_scope(...)` or `system_scope()` | Choose the correct audit table |
| Actor | `actor(...)`, `actor_service(...)`, or `actor_system()` | Who performed the action |
| Target | `target(...)` / `target_opt(...)` | Optional semantic target identity |
| `details_json` | JSON (optional) | Minimal, non-secret metadata |

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

Severity is modeled in `AuditOutcome` and action semantics, not by forcing a dedicated tracing
target/level convention. `JournaldBackend` emits structured audit events to `uptrakit_audit`.

### V2 deferrals

- `audit_log.filter` and `audit_log.retention_days` remain configuration keys but are not yet a
  complete per-tenant enforcement surface for semantic producers.

---

## Visibility and Module Boundaries

Rust provides four visibility levels. Use the narrowest level that satisfies the actual
call-site requirements:

| Scope | Keyword | Use when |
| --- | --- | --- |
| Private (default) | *(none)* | Item is only used in the same module |
| Crate-internal | `pub(crate)` | Item is used across modules/files **within the same crate** |
| Parent module | `pub(super)` | Item is used only in the parent module |
| Fully public | `pub` | Item crosses a **crate boundary** (called by another crate) |

### The `unreachable_pub` lint

The workspace enforces this table via the `unreachable_pub` lint (set to `deny` in
`[workspace.lints.rust]`). The lint fires whenever a `pub` item is **not reachable**
through the crate's public module chain. It is self-selecting:

- `pub fn foo()` inside a `mod bar { … }` where `bar` is not re-exported → **fires**
  (downgrade to `pub(crate)` or `pub(super)`)
- `pub fn foo()` inside a `pub mod bar { … }` that is re-exported from `lib.rs` → **does not fire**
  (the item is genuinely reachable from outside the crate)

No `#![allow(unreachable_pub)]` (or any other crate-wide or module-wide `#![allow(...)]`)
should appear anywhere in the workspace.

#### Using `#[allow(...)]` — policy and requirements

Item-level `#[allow(...)]` is permitted **only** when a lint produces a false positive that
cannot be resolved structurally. The one approved cause in this codebase is **feature-gating**:
an item that is used only when a specific Cargo feature is enabled will appear unused or
unreachable in builds that omit that feature, even though it is correctly public/used in the
intended build.

```rust
// ✅ Approved — the field is only read when the "metrics" feature is compiled in.
// Without the allow, the lint fires on --no-default-features builds.
#[allow(dead_code)] // used only with feature = "metrics"
pub counter: u64,
```

**Every `#[allow(...)]` must be accompanied by a comment** on the same line or the line
immediately above it explaining precisely why the suppression is necessary. A bare
`#[allow(...)]` with no explanation is not permitted and will be rejected in review.

**`#[allow(...)]` must be placed at the smallest possible scope.** Do not suppress a lint on a
function, struct, or module when only a single field, binding, or expression triggers it. Place
the attribute on that specific item instead:

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

Trait implementation methods are implicitly `pub` when required by the trait — do not
annotate them with an explicit `pub`. Only freestanding helper functions in plugin
submodules need visibility tightening:

```rust
// ✅ Trait impl — no explicit pub needed
impl ReleasePlugin for GitHubPlugin {
    async fn fetch_latest(&self) -> Result<Release> { … }
}

// ✅ Freestanding helper — must use pub(crate) if not part of the public API
pub(crate) fn parse_semver_tag(tag: &str) -> Option<semver::Version> { … }
```

### Cross-reference

- [Error Handling](error-handling.md) — public error types follow the same rule: use
  `pub(crate)` for errors that never cross a crate boundary.
- [Security](../../security/README.md) — avoid leaking internal types through `pub` that
  could expose security-sensitive implementation details.

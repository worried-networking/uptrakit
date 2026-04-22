# Testing Expectations

## Testing Philosophy

### Do not test upstream crate behavior

Upstream/external crates are treated as a black box. Their correctness is
the maintainer's responsibility. Tests should focus exclusively on verifying
**our own** logic, configuration, and contracts.

A test is **upstream crate testing** if it would pass even when the function
body is a direct, unmodified call to the upstream crate. It tests the
dependency, not our code.

A test is **internal logic testing** if it verifies behavior that could
break when our code changes (custom parsing, validation, serde annotations
that define a wire contract, backward compatibility guarantees, custom error
handling paths, etc.).

| Category | Example | Verdict |
| --- | --- | --- |
| `thiserror` `#[error("...")]` Display output | `assert_eq!(err.to_string(), "...")` | Upstream -- remove |
| `serde_json::to_string` / `from_str` roundtrip on a plain `#[derive(Serialize, Deserialize)]` struct with no custom logic | `assert_eq!(deserialized, original)` | Upstream -- remove |
| `argon2` salt uniqueness | Two hashes of the same password differ | Upstream -- remove |
| Custom `#[serde(with = "...")]` module roundtrip | Custom date format serialization | Internal -- keep |
| `skip_serializing_if` annotation | Optional field absent in JSON when `None` | Internal -- keep |
| Backward compatibility (old JSON shape still deserializes) | Missing field defaults correctly | Internal -- keep |
| Wire protocol spec conformance | Serialized JSON matches asyncapi.yaml schema | Internal -- keep |

**Prohibition: `thiserror` Display format string tests are forbidden.**

Tests that construct a `thiserror`-derived error variant and assert `err.to_string()` equals a
string that literally appears in the `#[error("...")]` attribute are testing `thiserror`'s string
interpolation, not application logic. Such tests must be removed. This includes all tests of the
form:

```rust
// Forbidden — tests thiserror, not our code
let err = MyError::Configuration("bad value".to_string());
assert_eq!(err.to_string(), "configuration error: bad value");
```

The `#[error("...")]` format string is a compile-time declaration. If the format string itself
needs to change, the change is intentional and the tests would all need to be updated too —
making them maintenance work rather than safety nets. Tests for custom hand-written `Display`
implementations (not derived from `#[error]`) remain internal logic tests and are kept.

More examples:

```rust
// PROHIBITED — tests upstream thiserror formatting, not our logic
#[test]
fn display_api_error() {
    let err = MyError::ApiError {
        status: reqwest::StatusCode::NOT_FOUND,
        message: "not found".to_string(),
    };
    assert_eq!(err.to_string(), "API error: 404 Not Found not found");
}
```

If you need to verify that a specific error variant is produced (not its Display
string), test the variant itself:

```rust
// OK — tests that the correct error variant is returned
#[test]
fn parse_returns_config_error_on_empty_owner() {
    let result = parse_owner_repo("/repo");
    assert!(matches!(result, Err(ref e) if matches!(e.current_context(), MyError::Configuration(_))));
}
```

### Wire protocol tests: asyncapi.yaml is the source of truth

Spec-conformance tests validate that Rust serialization matches the
[asyncapi.yaml](../../crates/shared/wire/asyncapi.yaml) schema. Each test
constructs a sample message, wraps it in an envelope, serializes it, and
validates required fields, type discriminators, and enum values against the
schema.

Behavioral tests (backward compatibility, field omission, custom serde
modules, exact JSON assertions, envelope/sequence logic) complement spec
tests and are kept as-is.

### Tests must never sleep on real wall-clock time

Use `#[tokio::test(start_paused = true)]` with `tokio::time::advance()` for
deterministic, fast time-dependent tests. Starting with paused time makes
`tokio::time::sleep` and `tokio::time::timeout` resolve via virtual time
advancement instead of wall-clock waiting, and eliminates the small window
between runtime startup and an explicit `tokio::time::pause()` call.

```rust
#[tokio::test(start_paused = true)]
async fn renewal_sleep_does_not_fire_after_one_hour() {
    let mut sleep = create_renewal_sleep(); // 30-day far-future timer
    tokio::time::advance(Duration::from_secs(3600)).await;
    assert!(tokio::time::timeout(Duration::ZERO, &mut sleep).await.is_err());
}
```

Do **not** call `tokio::time::pause()` explicitly inside the test body —
use the `start_paused = true` attribute instead so the runtime starts in
the paused state.

**Exceptions:**

- Docker integration tests (`#[ignore]`) that wait for real external
  processes (e.g., reverse proxy containers) use real delays out of
  necessity.
- Tests that use SQLx database connections (via SeaORM) must **not** use
  `start_paused = true` (or any explicit `tokio::time::pause()` call).
  SQLx's connection pool uses internal Tokio timers for acquire and idle
  timeouts. When time is paused, the Tokio runtime auto-advances to the
  next pending timer, which can fire those pool timers prematurely and
  produce a spurious `ConnectionAcquire(Timeout)` error — especially
  under stress testing (nextest `--stress-count`). Keep DB-backed tests
  on real time and use only short real-time delays (under 200 ms) where
  needed.
- Code that calls `OffsetDateTime::now_utc()` (wall-clock time, not
  Tokio's virtual clock) cannot use `start_paused = true` — it has no
  effect on `time::OffsetDateTime`. See
  [**Wall-Clock Time Injection**](#wall-clock-time-injection) below.

## Wall-Clock Time Injection

`start_paused = true` only affects Tokio's virtual clock — calls to
`tokio::time::sleep`, `tokio::time::Instant::now`, etc. It has **no
effect** on `time::OffsetDateTime::now_utc()`, which always returns real
wall-clock time.

Code that uses `OffsetDateTime::now_utc()` for logic (e.g., rate-limit
windows, expiry checks) must inject a clock to remain deterministic in
tests.

### Canonical pattern

Use `Arc<dyn Fn() -> OffsetDateTime + Send + Sync>` as the clock type
and `parking_lot::Mutex<OffsetDateTime>` (a workspace dependency) to
advance it in tests:

```rust
// --- In production code ---
pub struct MyStore {
    db: DatabaseConnection,
    now: Arc<dyn Fn() -> OffsetDateTime + Send + Sync>,
}

impl MyStore {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db, now: Arc::new(OffsetDateTime::now_utc) }
    }

    #[cfg(test)]
    pub(crate) fn with_clock(
        db: DatabaseConnection,
        now: Arc<dyn Fn() -> OffsetDateTime + Send + Sync>,
    ) -> Self {
        Self { db, now }
    }
}

// Replace every OffsetDateTime::now_utc() call inside the struct with:
let now = (self.now)();

// --- In tests (no start_paused, no DB backdating) ---
let clock = Arc::new(parking_lot::Mutex::new(OffsetDateTime::now_utc()));
let clock_fn: Arc<dyn Fn() -> OffsetDateTime + Send + Sync> = {
    let c = Arc::clone(&clock);
    Arc::new(move || *c.lock())
};
let store = MyStore::with_clock(db, clock_fn);

// Advance the clock past the expiry window:
*clock.lock() += time::Duration::seconds(120);
```

See `RateLimitStore::with_clock` (`crates/ui/web-api-auth/src/auth/rate_limit.rs`)
for the canonical example.

### Rules

- Do **not** add `start_paused = true` to tests that use wall-clock
  injection — those tests do not call Tokio time APIs.
- Do **not** backdate database rows directly (fragile: column rename
  silently breaks the test and does not exercise the production code
  path).
- The `#[cfg(test)] with_clock` constructor is the only approved
  alternative constructor for types with injectable clocks.

## Testing Expectations -- Overview

Changes should be covered by tests, especially if they touch behavior, parsing, or plugin logic. If an integration test is infeasible (e.g., OS
integration) include at least one of:

- Unit tests around decision logic
- Contract tests for serialization/parsing
- Integration tests backed by fixtures or mocks

## Run Tests Locally

```bash
cargo test --all-features
```

If you prefer `nextest`:

```bash
cargo nextest run --all-features
```

### Running web-API tests

The web-API is split across three crates: `uptrakit-web-api`, `uptrakit-web-api-auth`, and
`uptrakit-web-api-queries`. Test each independently or run the workspace together.

Some query tests in `uptrakit-web-api-queries` (specifically in
`crates/ui/web-api-queries/src/queries/hosts.rs` and `crates/ui/web-api-queries/src/queries/autodiscovery.rs`)
use an in-memory SQLite database and are gated behind `#[cfg(all(test, feature = "db-sqlite"))]`.
They are excluded from compilation entirely without the feature, so running
`cargo test -p uptrakit-web-api-queries` alone will not execute them.

Run the full test suite for all three crates:

```bash
cargo test -p uptrakit-web-api-auth --all-features     # auth + settings + registration tests
cargo test -p uptrakit-web-api-queries --features db-sqlite  # DB-backed query tests
cargo test -p uptrakit-web-api --features db-sqlite    # route/integration tests
```

Or run the entire workspace (preferred, mirrors CI):

```bash
cargo test --all-features
```

### What We Test

- Pure logic (unit tests)

- Plugin behavior (parsing, version comparison, metadata mapping)

- API boundaries (request/response types, compatibility)

- REST API integration tests (full HTTP stack with in-memory SQLite)

- WebSocket integration tests (enrollment and reconnection flows)

- Error paths with clear messaging

- Database integration tests (Docker-based for PG, ignored by default):

  ```bash
  cargo test -p uptrakit-integration-tests --test database -- --ignored
  ```

  Tests all REST API flows against SQLite and PostgreSQL using testcontainers.
  See [Database Integration Tests](#database-integration-tests) below.

- Reverse proxy integration tests (Docker-based, ignored by default):

  ```bash
  cargo test -p uptrakit-integration-tests --test reverse_proxy -- --ignored
  ```

  Requires Docker and covers L4/L7 TLS modes, CRL/OCSP revocation, and proxy-specific flows.

- System integration tests (Docker-based, ignored by default):

  ```bash
  docker build -f docker/Dockerfile.test -t uptrakit-test:latest .
  cargo test -p uptrakit-integration-tests -- --ignored
  ```

  Requires Docker and the `uptrakit-test:latest` image. Verifies end-to-end enrollment and
  communication between all binaries.

## REST API Integration Tests

The `test_harness/` module (`crates/ui/web-api/src/test_harness/`) provides a shared
test fixture for exercising the full Axum HTTP stack (request → router → middleware →
handler → database → response) without Docker or external services.

### Architecture

Each test gets its own **in-memory SQLite database** with all migrations applied.
Tests are parallel-safe with no cleanup required.

```text
crates/ui/web-api/src/
  test_harness/
    mod.rs          — TestApp, setup_migrated_db, build_test_state, NoopCertSigner
    http_client.rs  — TestClient (tower::oneshot wrapper with ergonomic API)
    fixtures.rs     — register_user, insert_service, seed_permissions_for_owner, etc.
  integration_tests/
    mod.rs           — module declarations
    auth_flow.rs     — registration, login, refresh, token rotation, logout
    services_crud.rs — service list, approve, reject, deactivate, update
    hosts.rs         — host list, detail, deactivate
    software_items_crud.rs — CRUD lifecycle
    enrollment_tokens.rs   — create, list, revoke
    settings.rs      — registration settings get/update
    notifications.rs — channel and rule CRUD
    plugin_configs.rs — plugin type list, config create/delete
    error_cases.rs   — 401/404/expired JWT systematic testing
    service_ws.rs    — WebSocket enrollment, reconnection, registry send/broadcast
```

All modules are gated behind `#[cfg(all(test, feature = "db-sqlite"))]`.

### Key components

- **`TestApp`**: spins up a fully wired Axum router backed by a migrated SQLite DB
  with a seeded default tenant. Provides `client()` for HTTP requests and direct `db`
  access for fixture insertion.
- **`TestClient`**: thin wrapper around `tower::ServiceExt::oneshot()` with methods
  like `get()`, `post_json()`, `put_json()`, `delete()`, `bearer()`, `send_json()`,
  and `send_status()`.
- **`seed_permissions_for_owner()`**: inserts permissions not present in the initial
  migration (e.g., `view_notifications`, `manage_notifications`) and assigns them to
  the `owner` role. Use this when testing endpoints gated by permissions added in
  later migrations.

### Adding new tests

1. Create a new file in `integration_tests/` and add its `mod` declaration to
   `integration_tests/mod.rs`.
2. Use `TestApp::new().await` to get a fully initialized app.
3. Use `register_and_get_token(&client)` for authenticated requests.
4. For endpoints requiring non-default permissions, call
   `seed_permissions_for_owner()` before registration.
5. WebSocket tests must use `tokio::net::TcpListener` + `axum::serve()` (HTTP
   upgrade does not work via `tower::oneshot`).

### Running

```bash
# All integration tests
cargo test -p uptrakit-web-api --features db-sqlite

# Specific module
cargo test -p uptrakit-web-api --features db-sqlite integration_tests::auth_flow

# Full workspace (includes these automatically)
cargo test --all-features
```

## Database Integration Tests

The `database` test binary in `crates/core/integration-tests/tests/database.rs` exercises the
full REST API against both supported database backends: SQLite and PostgreSQL.

### Architecture

A `TestHarness` builds a fully wired `AppState` + Axum router per backend, mirroring the
in-crate `TestApp` but running from an external crate. PostgreSQL containers are managed by
`testcontainers-modules` — one shared container per process, with a fresh `test_{uuid}`
database per test for isolation.

```text
crates/core/integration-tests/tests/
  database.rs                      -- test binary entry point
  database_helpers/
    mod.rs                         -- module root
    db_providers.rs                -- SQLite/PostgreSQL setup + migrations
    harness.rs                     -- TestHarness (builds AppState + Router)
    http_client.rs                 -- TestClient (tower::oneshot wrapper)
    fixtures.rs                    -- DB insertion + HTTP registration helpers
    macros.rs                      -- db_test! macro
  database/
    migrations.rs                  -- migration smoke tests
    auth_flow.rs                   -- registration, login, refresh, logout
    services.rs                    -- service CRUD + status transitions
    hosts.rs                       -- host list, detail, deactivate
    software_items.rs              -- CRUD lifecycle
    host_tags.rs                   -- tag CRUD + host assignment
    enrollment_tokens.rs           -- create, list, revoke
    system_services.rs             -- system service listing
    system_enrollment_tokens.rs    -- system token CRUD
    notifications.rs               -- channel + rule CRUD
    plugin_configs.rs              -- plugin type list, config create/delete
    settings.rs                    -- registration settings get/update
    users.rs                       -- user list, role update
    api_tokens.rs                  -- token CRUD + authentication
    audit_logs.rs                  -- audit log listing
    update_history.rs              -- update history listing
    health.rs                      -- healthz endpoint
    batch_actions.rs               -- batch deactivation
    error_cases.rs                 -- 401/404/expired JWT
```

### The `db_test!` macro

Each test function is written once as `async fn test_xxx(harness: &TestHarness)` and then
expanded into two `#[ignore]` test functions via the `db_test!` macro:

```rust
async fn test_create_host_tag(harness: &TestHarness) {
    // ... test logic ...
}

db_test!(create_host_tag, test_create_host_tag);
// Generates: create_host_tag_sqlite, create_host_tag_postgres
```

### Running

```bash
# Run all database integration tests (requires Docker for PG)
cargo test -p uptrakit-integration-tests --test database -- --ignored

# Run only SQLite tests (no Docker required)
cargo test -p uptrakit-integration-tests --test database sqlite -- --ignored

# Run only PostgreSQL tests
cargo test -p uptrakit-integration-tests --test database postgres -- --ignored

# Run a specific test across all backends
cargo test -p uptrakit-integration-tests --test database auth_flow -- --ignored
```

A dedicated `database-integration-tests` CI job runs all tests on `ubuntu-latest` (Docker
pre-installed).

## Plugin Unit Test Utilities

Package-manager and release plugins test their logic by injecting mock `CommandExecutor`
implementations. Shared mock types live in `uptrakit-plugin-infrastructure-core` behind the
`testing` feature. Enable them in dev-dependencies:

```toml
[dev-dependencies]
uptrakit-plugin-infrastructure-core = { workspace = true, features = ["testing"] }
```

Then import the types inside your test module:

```rust
use uptrakit_plugin_infrastructure_core::testing::{FixedOutputExecutor, RoutedOutputExecutor};
```

| Type | Constructor | Behaviour |
| --- | --- | --- |
| `FixedOutputExecutor` | `::success(stdout)` | All commands return `Ok` with exit 0 and the given output |
| `FixedOutputExecutor` | `::failure(exit_code)` | All commands return empty output; `execute_quiet` returns `Err(CommandFailed)` |
| `FixedOutputExecutor` | `::new(stdout, exit_code)` | `execute` always `Ok`; `execute_quiet` returns `Err` for non-zero |
| `RoutedOutputExecutor` | `::success(pairs)` | Routes by program name; all routes succeed with exit 0 |
| `RoutedOutputExecutor` | `::new(triples)` | Routes by program name; each route has its own `(output, exit_code)` |

Do not define local mock executor structs in plugin test modules — always use the shared types above.

## Testing Expectations - Detailed

Every behaviour change must include tests. Types of tests used:

- **Unit tests**: pure logic, version comparison, parsing.
- **Plugin tests**: parsing upstream metadata, mapping to internal models.
- **API boundary tests**: request/response (de)serialisation, backwards compatibility.
- **REST API integration tests**: full HTTP stack tests via `TestApp` (see above).
- **Error path tests**: expected failures produce correct error types and messages.
- **Docker integration tests**: reverse proxy tests using real containers (see below).
- **System integration tests**: end-to-end tests with all binaries in Docker containers (see below).
- **Service activity parity tests**: ensure Agent and MQTT service records update `ip_address` and `last_seen_at`
  consistently across connect and ping flows.

Run tests with:

```sh
cargo test --all-features
# or with nextest:
cargo nextest run --all-features
```

### Reverse proxy integration tests

Docker-based integration tests in `crates/core/integration-tests/tests/reverse_proxy/` validate that the controller's
middleware correctly extracts `ServiceIdentity` from forwarded headers when behind real reverse proxies. Each test uses
`testcontainers` to spin up a Docker container.

```text
crates/core/integration-tests/tests/
  reverse_proxy.rs              -- test binary entry point (mod reverse_proxy { ... })
  reverse_proxy/
    pki.rs                      -- TestPki: CA + server cert + agent cert generation (rcgen)
    server.rs                   -- TestServer: lightweight Axum HTTPS server with real middleware
    ocsp_responder.rs           -- OcspResponder: HTTP and HTTPS OCSP responder for testing
    nginx.rs                    -- Nginx L7 test (nginx:latest)
    traefik.rs                  -- Traefik L7 test (traefik:v3)
    caddy.rs                    -- Caddy L7 test (caddy:latest)
    haproxy.rs                  -- HAProxy L7 test (haproxy:latest)
    envoy.rs                    -- Envoy L7 test (envoyproxy/envoy:v1.31-latest)
    nginx_crl.rs                -- Nginx CRL revocation test
    haproxy_crl.rs              -- HAProxy CRL revocation test
    envoy_crl.rs                -- Envoy CRL revocation test
    nginx_ocsp.rs               -- Nginx OCSP revocation tests (HTTP, HTTPS, AIA)
```

All tests are `#[ignore]` with descriptive messages and never run in normal `cargo test`. They require Docker.

```sh
# Run all reverse proxy tests
cargo test -p uptrakit-integration-tests --test reverse_proxy -- --ignored

# Run a single proxy test (by name substring)
cargo test -p uptrakit-integration-tests --test reverse_proxy nginx -- --ignored
```

A dedicated `reverse-proxy-tests` CI job runs these on `ubuntu-latest` (Docker pre-installed).

When validating reverse proxy setups locally, confirm `/api/v1/services` shows expected service IP metadata and
`last_seen_at` movement for both Agent and MQTT services. Cross-check the security model in
[docs/security/reverse-proxy-security.md](../security/reverse-proxy-security.md).

### System integration tests

End-to-end tests in `crates/core/integration-tests/` verify that the actual compiled binaries
(controller, agent, agent-ssh, scheduler, mqtt) can communicate correctly as a system. Each test
uses `testcontainers` to orchestrate Docker containers on a shared network, verifying enrollment
flows and inter-component communication.

See [system-integration-tests.md](system-integration-tests.md) for the full guide.

```sh
# Build the multi-binary test Docker image (required before running tests)
docker build -f docker/Dockerfile.test -t uptrakit-test:latest .

# Run all system integration tests
cargo test -p uptrakit-integration-tests -- --ignored

# Run a specific test
cargo test -p uptrakit-integration-tests -- --ignored agent_enrolls
```

A dedicated `system-integration-tests` CI job builds the Docker image and runs these tests on
`ubuntu-latest`.

## Frontend Testing

The frontend uses [Vitest](https://vitest.dev/) for unit and component tests and
[Playwright](https://playwright.dev/) for end-to-end tests. All tests live inside the `frontend/`
directory.

### Unit and component tests (Vitest)

```bash
# Run all Vitest tests once
cd frontend && npm run test

# Run with coverage report
cd frontend && npm run test:coverage
```

Coverage is collected from `src/lib/**` via the `@vitest/coverage-v8` provider. Minimum thresholds
are enforced for lines (70%), branches (65%), and functions (70%). These thresholds will increase
as coverage grows.

**Test locations:**

| Test file | What it covers |
| --- | --- |
| `src/lib/api.test.ts` | `authenticatedFetch`, token refresh, error extraction, session-expired banner lifecycle |
| `src/lib/auth.test.ts` | `initialize`, `handleLogin`, `handleLogout`, `handleOidcCallback` |
| `src/lib/utils.test.ts` | `isValidLogoUrl`, `formatDate`, `safeRedirect`, `copyToClipboard` |
| `src/routes/services/services.test.ts` | Services page: load, error, empty state, filter buttons |
| `src/routes/hosts/hosts.test.ts` | Hosts page: load, error, empty state |
| `src/lib/components/Pagination.test.ts` | Prev/Next disabled states, click handlers, page number buttons, ellipsis rendering, aria-current, total count display |
| `src/lib/components/ContextMenu.test.ts` | Keyboard navigation (Arrow/Home/End/Enter/Escape) |
| `src/lib/components/ModalBackdrop.test.ts` | Focus trapping (Tab/Shift+Tab), Escape, backdrop click |
| `src/lib/components/ConfirmDialog.test.ts` | Confirm/cancel callbacks, disabled state, labels |

Component tests use [`@testing-library/svelte`](https://testing-library.com/docs/svelte-testing-library/intro/)
with jsdom. The `src/test-setup.ts` file imports `@testing-library/jest-dom` matchers. The `$lib`
path alias is configured in `vitest.config.ts` so component tests can import with `$lib/...`
exactly as production code does.

#### Mocking Svelte 5 rune modules

`$lib/auth.svelte` and `$lib/api` are mocked in route component tests using `vi.mock()`. Because
vitest hoists `vi.mock()` calls, the mock is applied before the component is imported:

```typescript
vi.mock('$lib/auth.svelte', () => ({ getUser: vi.fn(() => null) }));
vi.mock('$lib/api', () => ({ getServices: vi.fn() }));

import ServicesPage from './+page.svelte'; // receives the mocked modules
```

#### Testing components that accept Svelte 5 snippets

Components that take a `children: Snippet` prop (e.g. `ContextMenu`, `ModalBackdrop`) use Svelte
5's `createRawSnippet` to pass static HTML content in tests:

```typescript
import { createRawSnippet } from 'svelte';

const children = createRawSnippet(() => ({
  render: () => '<button role="menuitem">Action</button>'
}));

render(ContextMenu, { top: 0, left: 0, onclose: vi.fn(), children });
```

### End-to-end tests (Playwright)

```bash
# Install Chromium (one-time setup)
cd frontend && npx playwright install --with-deps chromium

# Run all E2E tests (starts the dev server automatically)
cd frontend && npm run test:e2e
```

E2E tests live in `frontend/tests/e2e/` and run against the SvelteKit dev server (started
automatically by Playwright's `webServer` configuration). **No Uptrakit backend is required** —
all API calls are intercepted with `page.route()` inside each test.

**Test files:**

| File | Coverage |
| --- | --- |
| `tests/e2e/auth.test.ts` | Unauthenticated redirect, login form, successful login, wrong credentials |
| `tests/e2e/services.test.ts` | Service list rendering, empty state, type filter |
| `tests/e2e/hosts.test.ts` | Host list rendering, empty state, context menu, deactivate dialog |
| `tests/e2e/public-entry.test.ts` | Public login + registration flow shell |
| `tests/e2e/ui-parity.test.ts` | Desktop UI parity fixtures for built-in and surface-backed patterns (mandatory for any visual change) |
| `tests/e2e/ui-parity-responsive.test.ts` | Mobile UI parity fixtures — bottom navigation, overflow sheet, responsive shell |
| `tests/e2e/parity-config.ts` | Shared fixtures, mock API, scenario builders for the parity suites |

E2E tests are **not** run in CI automatically and are **not** part of the pre-push hook (they
add several minutes per run). Contributors must run them locally before pushing any change that
touches theme tokens, shared primitives, route markup, or parity fixtures — see
[Quality gates — Frontend (SvelteKit)](quality-gates.md#frontend-sveltekit). Snapshot
regeneration must run on macOS + Chromium per the parity-suite guard in `playwright.config.ts`.
To add the suite to CI, install Chromium in the job
(`npx playwright install --with-deps chromium`) and run `npm run test:e2e` after
`npm run build`. See `playwright.config.ts` for the full configuration.

> **Security note:** See [docs/security/auth-and-authorization.md](../security/auth-and-authorization.md)
> for authentication flow details that E2E tests exercise.

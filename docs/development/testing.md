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

See `RateLimitStore::with_clock` (`crates/ui/web-api/src/auth/rate_limit.rs`)
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

### Running `uptrakit-web-api` tests

Some query tests in `uptrakit-web-api` (specifically in
`crates/ui/web-api/src/queries/hosts.rs` and `crates/ui/web-api/src/queries/autodiscovery.rs`)
use an in-memory SQLite database and are gated behind `#[cfg(all(test, feature = "db-sqlite"))]`.
They are excluded from compilation entirely without the feature, so running
`cargo test -p uptrakit-web-api` alone will not execute them.

Run the full `uptrakit-web-api` test suite — including DB-backed tests — with:

```bash
cargo test -p uptrakit-web-api --features db-sqlite
```

Or run the entire workspace (preferred, mirrors CI):

```bash
cargo test --workspace --all-features
```

### What We Test

- Pure logic (unit tests)

- Plugin behavior (parsing, version comparison, metadata mapping)

- API boundaries (request/response types, compatibility)

- Error paths with clear messaging

- Reverse proxy integration tests (Docker-based, ignored by default):

  ```bash
  cargo test -p uptrakit-controller reverse_proxy -- --ignored
  ```

  Requires Docker and covers L4/L7 TLS modes, CRL/OCSP revocation, and proxy-specific flows.

## Testing Expectations - Detailed

Every behaviour change must include tests. Types of tests used:

- **Unit tests**: pure logic, version comparison, parsing.
- **Plugin tests**: parsing upstream metadata, mapping to internal models.
- **API boundary tests**: request/response (de)serialisation, backwards compatibility.
- **Error path tests**: expected failures produce correct error types and messages.
- **Docker integration tests**: reverse proxy tests using real containers (see below).
- **Service activity parity tests**: ensure Agent and MQTT service records update `ip_address` and `last_seen_at`
  consistently across connect and ping flows.

Run tests with:

```sh
cargo test --all-features
# or with nextest:
cargo nextest run --all-features
```

### Reverse proxy integration tests

Docker-based integration tests in `crates/core/controller/tests/reverse_proxy/` validate that the controller's
middleware correctly extracts `ServiceIdentity` (unified identity extractor, replacing the former `AgentIdentity` and
`MqttServiceIdentity`) from forwarded headers when behind real reverse proxies. Each test uses `testcontainers` to spin
up a Docker container.

```text
crates/core/controller/tests/
  reverse_proxy.rs              -- test binary entry point
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
cargo test -p uptrakit-controller reverse_proxy -- --ignored

# Run a single proxy test
cargo test -p uptrakit-controller reverse_proxy::nginx -- --ignored
```

A dedicated `reverse-proxy-tests` CI job runs these on `ubuntu-latest` (Docker pre-installed).

When validating reverse proxy setups locally, confirm `/api/v1/services` shows expected service IP metadata and
`last_seen_at` movement for both Agent and MQTT services. Cross-check the security model in
[docs/security/reverse-proxy-security.md](../security/reverse-proxy-security.md).

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
| `src/lib/api.test.ts` | `authenticatedFetch`, token refresh, error extraction |
| `src/lib/auth.test.ts` | `initialize`, `handleLogin`, `handleLogout`, `handleOidcCallback` |
| `src/lib/utils.test.ts` | `isValidLogoUrl`, `formatDate`, `safeRedirect`, `copyToClipboard` |
| `src/routes/services/services.test.ts` | Services page: load, error, empty state, filter buttons |
| `src/routes/hosts/hosts.test.ts` | Hosts page: load, error, empty state |
| `src/lib/components/Pagination.test.ts` | Prev/Next disabled states, click handlers, page display |
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

E2E tests are **not** run in CI automatically. To add them to CI, install Chromium in the job
(`npx playwright install --with-deps chromium`) and run `npm run test:e2e` after `npm run build`.
See `playwright.config.ts` for the full configuration.

> **Security note:** See [docs/security/auth-and-authorization.md](../security/auth-and-authorization.md)
> for authentication flow details that E2E tests exercise.

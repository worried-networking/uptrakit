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

Use `tokio::time::pause()` with `tokio::time::advance()` for deterministic,
fast time-dependent tests. Paused time makes `tokio::time::sleep` and
`tokio::time::timeout` resolve via virtual time advancement instead of
wall-clock waiting.

**Exceptions:**

- Docker integration tests (`#[ignore]`) that wait for real external
  processes (e.g., reverse proxy containers) use real delays out of
  necessity.
- Tests that use SQLx database connections (via SeaORM) must call
  `tokio::time::pause()` **after** establishing the pool. SQLx's
  connection pool uses internal tokio timers for acquire and idle
  timeouts; pausing time before `Database::connect()` causes
  `PoolTimedOut` because auto-advance expires the timers instantly.
  If the test loop itself performs DB queries (e.g., a scheduler poll
  loop), `tokio::time::pause()` cannot be used at all — keep real-time
  delays minimal (under 200 ms) instead.

## Testing Expectations -- Overview

Changes should be covered by tests, especially if they touch behavior, parsing, or provider logic. If an integration test is infeasible (e.g., OS
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

### What We Test

- Pure logic (unit tests)

- Provider behavior (parsing, version comparison, metadata mapping)

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
- **Provider tests**: parsing upstream metadata, mapping to internal models.
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

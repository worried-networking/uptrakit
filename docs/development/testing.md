# Testing Expectations

## Testing Expectations - Overview

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

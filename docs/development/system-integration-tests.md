# System Integration Tests

End-to-end tests that verify all Uptrakit binaries communicate correctly as a system.

## Overview

Unlike unit tests and REST API integration tests (which use in-memory SQLite and mock
connections), system integration tests build the actual binaries into a Docker image and
orchestrate them using [testcontainers](https://github.com/testcontainers/testcontainers-rs)
on a shared Docker network.

These tests verify:

- Controller starts and serves its health check endpoint
- User registration and login via the REST API
- Agent enrollment with bootstrap tokens (TOFU + mTLS)
- Scheduler enrollment as a system service
- MQTT service enrollment as a system service
- Agent-SSH enrollment with plaintext secrets mode
- All four service types enrolling concurrently with a single controller

## Architecture

```text
Docker network: uptrakit-test-{uuid}
┌─────────────────────────────────┐
│ controller (container name)     │ ◄── test verifies via mapped host port
│ HTTPS :8443                     │     (reqwest + danger_accept_invalid_certs)
│ --allow-plaintext-secrets       │
│ --bootstrap-enrollment-token    │
│ --bootstrap-system-enrollment-  │
│   token                         │
└─────────┬───────────────────────┘
          │ mTLS WebSocket (container DNS)
    ┌─────┼─────┬──────────┐
    │     │     │          │
  agent  mqtt  scheduler  agent-ssh
  --tofu --tofu --tofu    --tofu
  --enrollment-token      --allow-plaintext-secrets
```

Each test creates a unique Docker network (`uptrakit-test-{uuid7}`) so tests can run
in parallel without conflicts. The controller container gets a unique name
(`controller-{uuid7}`) for DNS resolution by the service containers.

## Prerequisites

- Docker daemon running locally
- The `uptrakit-test:latest` Docker image must be pre-built

## Building the Test Image

```bash
docker build -f docker/Dockerfile.test -t uptrakit-test:latest .
```

The `docker/Dockerfile.test` builds all five binaries (controller, agent, agent-ssh,
scheduler, mqtt) into a single image using cargo-chef for dependency caching. It shares
the same build pattern as the production Dockerfile, so Docker BuildKit can share layers
between builds.

The image has no `ENTRYPOINT` — each container specifies its command via
testcontainers `with_cmd()`.

## Running Tests

```bash
# Run all system integration tests
cargo test -p uptrakit-integration-tests -- --ignored

# Run a specific test
cargo test -p uptrakit-integration-tests -- --ignored controller_starts
cargo test -p uptrakit-integration-tests -- --ignored agent_enrolls
cargo test -p uptrakit-integration-tests -- --ignored all_components
```

All tests are marked `#[ignore]` with descriptive messages explaining the Docker image
requirement. They never run during `cargo test --workspace`.

## Crate Structure

```text
crates/core/integration-tests/
  Cargo.toml
  src/
    lib.rs              — re-exports helpers
    containers.rs       — ControllerContainer, ServiceContainer wrappers
    api_client.rs       — REST API client for verification
  tests/
    system.rs           — test binary entry point
    system/
      controller_startup.rs     — health check, user registration/login
      agent_enrollment.rs       — agent enrolls with token
      scheduler_enrollment.rs   — scheduler enrolls as system service
      mqtt_enrollment.rs        — mqtt enrolls as system service
      agent_ssh_enrollment.rs   — agent-ssh enrolls with token
      full_system.rs            — all 4 services enroll concurrently
```

## Key Design Decisions

### Test tokens

Hardcoded test tokens (`test-enrollment-token-do-not-use-in-prod` and
`test-system-token-do-not-use-in-prod`) with high `max_uses` (100) and long
TTL (3600s) so multiple services can enroll in the same test.

### TLS handling

- Controller runs with `--allow-plaintext-secrets` (no master key required)
- All services use `--tofu` to trust the controller's self-signed CA
- API verification uses `reqwest` with `danger_accept_invalid_certs(true)`
- Agent-SSH additionally uses `--allow-plaintext-secrets` for its local store

### Wait strategies

- Controller: waits for `"HTTPS server listening on"` on stderr
- Services: wait for `"enrollment complete, certificate saved to disk"` on stderr
- API readiness: polls `GET /healthz` every 500ms until success

### Cleanup

testcontainers automatically removes containers and networks when the
`ContainerAsync` handle is dropped at the end of each test.

### No `start_paused`

These tests run real processes with real network I/O. Virtual time cannot be
used (per the testing guidelines for Docker integration tests).

## CI

The `system-integration-tests` job in `.github/workflows/ci.yml`:

1. Checks out the repository
2. Builds the frontend (required by the controller)
3. Builds the `uptrakit-test:latest` Docker image
4. Runs `cargo test -p uptrakit-integration-tests -- --ignored`

## Adding New Tests

1. Create a new file in `tests/system/`
2. Add its `mod` declaration to `tests/system.rs`
3. Use `ControllerContainer::start()` and `ServiceContainer::start_*()` to manage containers
4. Use `ApiClient` for REST API verification
5. Mark all tests with the standard `#[ignore]` attribute and descriptive message
   (see existing tests for the exact wording)

## Related Documentation

- [Testing expectations](testing.md) — general testing guidelines
- [Docker deployment](../docker/deployment.md) — production Docker setup
- [Security: TOFU and TLS](../security/tofu-and-tls.md) — TLS trust model

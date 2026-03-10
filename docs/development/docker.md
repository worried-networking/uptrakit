# Docker Development Guide

This document covers building, testing, and publishing Docker images for Uptrakit.

## Dockerfile Architecture

The project uses a single multi-stage `docker/Dockerfile` parameterized via build args:

| Arg | Description | Example |
| --- | --- | --- |
| `PACKAGE` | Cargo package name | `uptrakit-controller` |
| `BINARY` | Output binary name | `uptrakit-controller` |
| `FEATURES` | Comma-separated Cargo features | `embed-frontend,db-all,oidc` |

### Build Stages

| Stage | Base Image | Purpose |
| --- | --- | --- |
| `frontend-builder` | `node:lts-bookworm-slim` | Builds the SvelteKit SPA |
| `planner` | `rust:1-bookworm` + cargo-chef | Generates a dependency recipe |
| `builder` | `rust:1-bookworm` + cargo-chef | Cooks dependencies (cached), then builds the binary |
| `runtime` | `debian:bookworm-slim` | Non-root `uptrakit` user, `/data/{config,state}` |

The builder stage installs `cmake`, `clang`, and `pkg-config` for `aws-lc-sys`. The runtime stage
installs only `ca-certificates`.

## Building Images Locally

```bash
# Build via docker-compose (uses matrix from docker-compose.yml)
docker compose build controller

# Build directly with custom features
docker build -f docker/Dockerfile \
  --build-arg PACKAGE=uptrakit-controller \
  --build-arg BINARY=uptrakit-controller \
  --build-arg FEATURES=embed-frontend,db-all,oidc,embedded-scheduler,nats,notifications-all \
  -t uptrakit-controller .
```

### Build Matrix

These are the standard image configurations built by CI:

| Image suffix | Package | Binary | Features |
| --- | --- | --- | --- |
| `controller` | `uptrakit-controller` | `uptrakit-controller` | `embed-frontend,db-all,oidc,embedded-scheduler,nats,notifications-all` |
| `controller-swagger` | `uptrakit-controller` | `uptrakit-controller` | `embed-frontend,db-all,oidc,embedded-scheduler,nats,notifications-all,swagger-ui` |
| `scheduler` | `uptrakit-scheduler` | `uptrakit-scheduler` | `db-all,oidc` |
| `mqtt` | `uptrakit-mqtt` | `uptrakit-mqtt` | *(none)* |
| `agent-ssh` | `uptrakit-agent-ssh` | `uptrakit-agent-ssh` | *(none)* |
| `cli` | `uptrakit-cli` | `uptrakit` | *(none)* |

## Docker Compose

The `docker-compose.yml` at the repo root provides a complete local deployment. See
[Docker deployment](../end-user/deployment/docker.md) for usage.

### Testing with docker-compose

```bash
# Start controller only (SQLite)
docker compose up -d
docker compose logs -f controller

# Start all services
docker compose --profile full up -d

# Tear down
docker compose --profile full down -v
```

## CI Workflow

The `.github/workflows/docker.yml` workflow:

- **Triggers**: push to `main`, pull requests to `main`, version tags (`v*`)
- **Registry**: `ghcr.io/worried-networking/uptrakit-{name}`
- **Platforms**: `linux/amd64`, `linux/arm64` (via QEMU)
- **Caching**: GitHub Actions cache (`type=gha,mode=max`)
- **PR builds**: build-only (no push) to validate Dockerfiles

### Tag Strategy

| Trigger | Tags |
| --- | --- |
| Push to `main` | `main`, `sha-<commit>` |
| Tag `v1.2.3` | `1.2.3`, `1.2`, `1`, `sha-<commit>` |
| Pull request | `pr-<number>` |

## Enrollment Token Bootstrap

The controller supports bootstrap enrollment tokens for zero-interaction docker-compose startups.
See [Docker deployment — auto-enrollment](../end-user/deployment/docker.md#auto-enrollment) for
the end-user flow.

### Implementation

New CLI flags (with corresponding `UPTRAKIT_*` env vars):

- `--bootstrap-enrollment-token` — pre-shared secret for tenant service enrollment
- `--bootstrap-enrollment-token-max-uses` — max uses (default: 1)
- `--bootstrap-enrollment-token-ttl` — TTL in seconds (default: 300)
- `--bootstrap-system-enrollment-token` — pre-shared secret for system service enrollment
- `--bootstrap-system-enrollment-token-max-uses` — max uses (default: 1)
- `--bootstrap-system-enrollment-token-ttl` — TTL in seconds (default: 300)

At startup the controller hashes the provided value with Argon2id and inserts a token named
"bootstrap" if no active token with that name exists. The operation is idempotent across restarts.

Source: `crates/core/controller/src/startup.rs` (`bootstrap_enrollment_tokens`).

## Test Docker Image

A separate `docker/Dockerfile.test` builds all five binaries (controller, agent, agent-ssh,
scheduler, mqtt) into a single image for system integration tests:

```bash
docker build -f docker/Dockerfile.test -t uptrakit-test:latest .
```

This image has **no `ENTRYPOINT`** — each container specifies its command via testcontainers
`with_cmd()`. It uses the same cargo-chef caching pattern as the production Dockerfile.

See [system-integration-tests.md](system-integration-tests.md) for the full testing guide.

## .dockerignore

The `docker/.dockerignore` file excludes build artifacts, VCS metadata, documentation,
and IDE files from the Docker build context.

## Docker Plugin Architecture

The `uptrakit-plugin-releases-docker` crate (at `crates/plugins/releases/docker/`) provides
container image tracking and updates. Key architectural aspects are documented here for
contributors.

### SSH Proxy and Runtime Detection

When `agent-ssh` manages a remote host, the Docker plugin bridges local bollard connections to
the remote Docker/Podman daemon via a Unix socket proxy:

1. `DockerSocketProxy::start(executor, dial_stdio_cmd)` binds a temporary socket under
   `/tmp/uptrakit/docker-proxy-{pid}-{n}.sock`.
2. Each accepted connection opens a stdio tunnel to `{dial_stdio_cmd}` on the remote host
   and copies bytes bidirectionally.
3. Bollard connects to the local proxy socket via `unix://...`.

The `dial_stdio_cmd` is determined by the `container_runtime` config field:

| Config | Command |
| --- | --- |
| `auto` | Probe remote: `docker` first, then `podman`; restart proxy with winner |
| `docker` | `docker system dial-stdio` |
| `podman` | `podman system dial-stdio` |

Auto-detection runs inside `detect_host_compatibility()` via `detect_and_apply_runtime()`.
Detection uses `command -v docker` / `command -v podman` (with 5-second timeouts). When a
different runtime is detected from the initial proxy assumption, the proxy is torn down and
restarted with the correct command, and a new `BollardDockerClient` is created pointing at
the new socket.

### Rootless Socket Probing

For local (non-SSH) connections, when `docker_host` is not set, `probe_local_socket_path()`
(Unix + `daemon` feature) checks these paths in order and returns the first that exists as a
Unix socket:

```text
/var/run/docker.sock              (rootful Docker)
/run/user/{euid}/docker.sock      (rootless Docker)
/run/user/{euid}/podman/podman.sock  (rootless Podman)
/run/podman/podman.sock           (rootful Podman)
```

On Linux the effective UID is read from `/proc/self/status`. On other Unix platforms
user-scoped paths are skipped.

### Credential Resolution

When `use_system_credentials = true` and no explicit `auth` is set, the plugin reads
`~/.docker/config.json`:

- **Local agent**: reads from the local filesystem.
- **SSH agent**: executes `cat ~/.docker/config.json` over the existing SSH session (reads
  from the **remote** host's config, not the controller's).

Credential helpers are invoked via `printf '%s' {registry} | docker-credential-{helper} get`.
Helper names are validated against `[a-zA-Z0-9_-]+` before invocation. Resolved credentials
are cached per registry in a `parking_lot::Mutex<HashMap>` for the lifetime of the plugin
instance.

See `crates/plugins/releases/docker/src/credentials.rs` for implementation details.

### Label Filtering

`include_labels` and `exclude_labels` filter containers at the discovery and update layers:

- **`include_labels`**: skips containers missing ANY of the required label/value pairs.
- **`exclude_labels`**: skips containers matching ANY of the excluded label/value pairs.
- Applied in `discover_software()`, and to the container list in `execute_update()`.
- An empty map (the default) applies no filter.

## Related Documentation

- [Docker deployment (end-user)](../end-user/deployment/docker.md)
- [Docker plugin (end-user)](../end-user/plugins/docker.md)
- [Embedded frontend](embedded-frontend.md) — the `embed-frontend` feature used by Docker images
- [Setup](setup.md) — local development prerequisites
- [Quality gates](quality-gates.md) — CI checks that run alongside Docker builds
- [System integration tests](system-integration-tests.md) — Docker-based end-to-end tests

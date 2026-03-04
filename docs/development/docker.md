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

## .dockerignore

The `docker/.dockerignore` file excludes build artifacts, VCS metadata, documentation,
and IDE files from the Docker build context.

## Related Documentation

- [Docker deployment (end-user)](../end-user/deployment/docker.md)
- [Embedded frontend](embedded-frontend.md) — the `embed-frontend` feature used by Docker images
- [Setup](setup.md) — local development prerequisites
- [Quality gates](quality-gates.md) — CI checks that run alongside Docker builds

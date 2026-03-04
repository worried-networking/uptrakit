# Releases

Uptrakit uses [release-please](https://github.com/googleapis/release-please) to automate version
bumps, changelog generation, and GitHub releases. Binary artifacts and Docker images are built
automatically when a release is published.

## Release flow

1. Push commits to `main` using [Conventional Commits](commit-messages.md) format.
2. release-please opens (or updates) a release PR that bumps `Cargo.toml` workspace version,
   `Cargo.lock`, `frontend/package.json`, and generates `CHANGELOG.md`.
3. Merge the release PR. release-please creates a GitHub release with a `v0.0.x` tag.
4. The `release-please.yml` workflow builds 7 binaries for 4 targets (28 total), uploads them
   to the GitHub release, and attests provenance for each.
5. The `docker.yml` workflow triggers on the `v*` tag push and builds multi-arch Docker images.

## Version strategy

While the project is pre-1.0 (`0.0.x`):

- `feat` commits bump **patch** (e.g. `0.0.1` -> `0.0.2`).
- `feat!` (breaking) commits bump **minor** (e.g. `0.0.2` -> `0.1.0`).
- Major bumps never happen automatically before `1.0.0`.

This is configured via `bump-minor-pre-major` and `bump-patch-for-minor-pre-major` in
`.github/release-please-config.json`.

## Binary artifacts

Each release includes pre-built binaries for 4 targets:

| Target | Runner | Method |
| --- | --- | --- |
| `x86_64-unknown-linux-gnu` | `ubuntu-latest` | native |
| `aarch64-unknown-linux-gnu` | `ubuntu-latest` | `cross` |
| `x86_64-apple-darwin` | `macos-13` | native |
| `aarch64-apple-darwin` | `macos-latest` | native |

### Binaries per release

| Artifact name | Package | Features |
| --- | --- | --- |
| `uptrakit-controller` | uptrakit-controller | embed-frontend,db-all,oidc,embedded-scheduler,nats,notifications-all |
| `uptrakit-controller-swagger` | uptrakit-controller | same + swagger-ui |
| `uptrakit-agent` | uptrakit-agent | (default) |
| `uptrakit-agent-ssh` | uptrakit-agent-ssh | (default) |
| `uptrakit-mqtt` | uptrakit-mqtt | (default) |
| `uptrakit-scheduler` | uptrakit-scheduler | db-all,oidc (no-default-features) |
| `uptrakit-cli` | uptrakit-cli | (default) |

Asset naming: `{artifact}-v{version}-{target}` (raw binaries, no tarballs).

### ARM64 Linux cross-compilation

ARM64 Linux builds use [cross](https://github.com/cross-rs/cross). The `Cross.toml` file
installs `cmake`, `clang`, and `pkg-config` in the build container so that `aws-lc-sys` compiles
successfully.

## Build provenance attestation

Every binary is attested using
[`actions/attest`](https://github.com/actions/attest) for SLSA build provenance. This allows
consumers to verify that binaries were built by the official CI pipeline.

### Verifying attestation

```sh
gh attestation verify <binary-file> --repo worried-networking/uptrakit
```

Example:

```sh
gh attestation verify uptrakit-controller-v0.0.2-x86_64-unknown-linux-gnu \
  --repo worried-networking/uptrakit
```

## Installing from source

All binary crates support `cargo install --git`:

```sh
# Controller (with all features)
cargo install --git https://github.com/worried-networking/uptrakit \
  uptrakit-controller \
  --features embed-frontend,db-all,oidc,embedded-scheduler,nats,notifications-all

# Agent
cargo install --git https://github.com/worried-networking/uptrakit uptrakit-agent

# SSH agent
cargo install --git https://github.com/worried-networking/uptrakit uptrakit-agent-ssh

# MQTT bridge
cargo install --git https://github.com/worried-networking/uptrakit uptrakit-mqtt

# Scheduler
cargo install --git https://github.com/worried-networking/uptrakit \
  uptrakit-scheduler --no-default-features --features db-all,oidc

# CLI
cargo install --git https://github.com/worried-networking/uptrakit uptrakit-cli
```

The `embed-frontend` feature requires the `frontend/build/` directory to exist at compile time.
Build the frontend first (`cd frontend && npm ci && npm run build`) or omit the feature to serve
the frontend from a separate directory via `--static-dir`.

## Configuration files

| File | Purpose |
| --- | --- |
| `.github/release-please-config.json` | release-please package configuration |
| `.github/.release-please-manifest.json` | Current version tracked by release-please |
| `.github/workflows/release-please.yml` | Release workflow (version bump + artifact builds) |
| `.github/workflows/docker.yml` | Docker image builds (triggered by `v*` tags) |
| `Cross.toml` | Cross-compilation settings for ARM64 Linux |

## Related documentation

- [Commit Messages](commit-messages.md) — Conventional Commits format required by release-please
- [Docker](docker.md) — Docker image build process
- [Quality Gates](quality-gates.md) — CI checks that must pass before merging

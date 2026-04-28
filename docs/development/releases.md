# Releases

Uptrakit uses [release-plz](https://github.com/release-plz/release-plz) to automate version
bumps, changelog generation, and GitHub releases. Binary artifacts and Docker images are built
automatically when a release is published.

## Release flow

1. Push commits to `main` using [Conventional Commits](commit-messages.md) format.
2. release-plz opens (or updates) a release PR that bumps `Cargo.toml` workspace version,
   `Cargo.lock`, `frontend/package.json`, and generates `CHANGELOG.md`.
3. Merge the release PR. release-plz creates a GitHub release with a `v0.0.x` tag.
4. The `release-plz.yml` workflow builds 7 binaries for 4 targets (28 total), uploads them
   to the GitHub release, and attests provenance for each.
5. The `docker.yml` workflow triggers on the `v*` tag push and builds multi-arch Docker images.

## Version strategy

While the project is pre-1.0 (`0.0.x`):

- `feat` commits bump **patch** (e.g. `0.0.1` -> `0.0.2`).
- `feat!` (breaking) commits bump **minor** (e.g. `0.0.2` -> `0.1.0`).
- Major bumps never happen automatically before `1.0.0`.

This is configured via `bump-minor-pre-major` and `bump-patch-for-minor-pre-major` in
`release-plz.toml`.

## Binary artifacts

Each release includes pre-built binaries for 4 targets:

| Target | Runner | Method |
| --- | --- | --- |
| `x86_64-unknown-linux-gnu` | `ubuntu-latest` | native |
| `x86_64-unknown-linux-musl` | `ubuntu-latest` | `cross` |
| `aarch64-unknown-linux-gnu` | `ubuntu-latest` | `cross` |
| `aarch64-apple-darwin` | `macos-latest` | native |

### Binaries per release

| Artifact name | Package | Features |
| --- | --- | --- |
| `uptrakit-controller` | uptrakit-controller | embedded-frontend,db-all,nats,oidc,zeroconf,notifications-all (no embedded services) |
| `uptrakit-controller-standalone` | uptrakit-controller-standalone | embedded-frontend,db-all,nats,oidc,zeroconf,notifications-all,embedded-scheduler,embedded-mqtt,embedded-agent,embedded-ssh-agent |
| `uptrakit-agent` | uptrakit-agent | (default) |
| `uptrakit-agent-ssh` | uptrakit-agent-ssh | (default) |
| `uptrakit-mqtt` | uptrakit-mqtt | (default) |
| `uptrakit-scheduler` | uptrakit-scheduler | db-all,oidc (no-default-features) |
| `uptrakit-cli` | uptrakit-cli | (default) |

Asset naming: `{artifact}-{version}-{target}.tar.gz` (archived; single binary at archive root).

### cross-compiled Linux targets

Both `aarch64-unknown-linux-gnu` and `x86_64-unknown-linux-musl` builds use
[cross](https://github.com/cross-rs/cross). The `Cross.toml` file installs `cmake`, `clang`,
and `pkg-config` in the build container via a pre-build hook so that `aws-lc-sys` compiles
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
gh attestation verify uptrakit-controller-0.0.2-x86_64-unknown-linux-gnu.tar.gz \
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
| `release-plz.toml` | release-plz package configuration (bump rules, changelog) |
| `.github/workflows/release-plz.yml` | Release workflow (version bump + artifact builds) |
| `.github/workflows/docker.yml` | Docker image builds (triggered by `v*` tags) |
| `Cross.toml` | Cross-compilation settings for cross-compiled Linux targets |

## Related documentation

- [Commit Messages](commit-messages.md) — Conventional Commits format required by release-plz
- [Docker](docker.md) — Docker image build process
- [Quality Gates](quality-gates.md) — CI checks that must pass before merging

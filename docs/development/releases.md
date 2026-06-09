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

| Target                      | Runner          | Method  |
| --------------------------- | --------------- | ------- |
| `x86_64-unknown-linux-gnu`  | `ubuntu-latest` | native  |
| `x86_64-unknown-linux-musl` | `ubuntu-latest` | `cross` |
| `aarch64-unknown-linux-gnu` | `ubuntu-latest` | `cross` |
| `aarch64-apple-darwin`      | `macos-latest`  | native  |

### Binaries per release

| Artifact name                    | Package                        | Features                                                                                                                         |
| -------------------------------- | ------------------------------ | -------------------------------------------------------------------------------------------------------------------------------- |
| `uptrakit-controller`            | uptrakit-controller            | embedded-frontend,db-all,nats,oidc,zeroconf,notifications-all (no embedded services)                                             |
| `uptrakit-controller-standalone` | uptrakit-controller-standalone | embedded-frontend,db-all,nats,oidc,zeroconf,notifications-all,embedded-scheduler,embedded-mqtt,embedded-agent,embedded-ssh-agent |
| `uptrakit-agent`                 | uptrakit-agent                 | (default)                                                                                                                        |
| `uptrakit-agent-ssh`             | uptrakit-agent-ssh             | (default)                                                                                                                        |
| `uptrakit-mqtt`                  | uptrakit-mqtt                  | (default)                                                                                                                        |
| `uptrakit-scheduler`             | uptrakit-scheduler             | db-all,oidc (no-default-features)                                                                                                |
| `uptrakit-cli`                   | uptrakit-cli                   | (default)                                                                                                                        |

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

| File                                | Purpose                                                     |
| ----------------------------------- | ----------------------------------------------------------- |
| `release-plz.toml`                  | release-plz package configuration (bump rules, changelog)   |
| `.github/workflows/release-plz.yml` | Release workflow (version bump + artifact builds)           |
| `.github/workflows/docker.yml`      | Docker image builds (triggered by `v*` tags)                |
| `Cross.toml`                        | Cross-compilation settings for cross-compiled Linux targets |

## Changelog scoping and `changelog_include`

Binary CHANGELOGs roll up commits from their dependency crates via
`changelog_include = [...]` in `release-plz.toml`. For each included
crate, release-plz walks commits backwards and stops at one of two
anchors:

1. **crates.io tarball equality** — for crates with `publish = true`,
   release-plz fetches the most recent tarball from crates.io and walks
   back until the in-repo tree matches it.
2. **last matching git tag** — for crates with `git_only = true`, the
   `published_at_sha1` is the commit the matching `<name>-v<version>`
   tag points at.

Plain `publish = false` library/plugin crates have neither anchor, so
release-plz walks the entire repo history and re-dumps it into every
binary CHANGELOG cycle after cycle.

**Therefore: every crate listed in any `changelog_include` array must
have exactly one of those two anchors** — either `publish = true` or
`git_only = true`. **Mixing both on one crate is forbidden**:
`git_only` silently wins and disables `cargo publish`, which is the
exact wedge that stalled releases from `v0.0.2` through `v0.0.5` (see
PR #136 and the surrounding release-plz cycle). The
[`release_plz_config_is_self_consistent`](../../crates/core/functional-tests/tests/release_config_invariants.rs)
test enforces this.

### When adding a new `changelog_include` member

Pick the anchor by asking "does this crate ship to crates.io?":

- **Yes — transitive dep of `uptrakit-service-sdk` or
  `uptrakit-openapi-client`.** Set `publish = true`,
  `git_release_enable = false`. The workspace default
  `git_tag_enable = true` still gives the crate a per-version tag, but
  the crates.io tarball is the authoritative anchor. Current members:
  `uptrakit-shared-types`, `uptrakit-wire`, `uptrakit-surfaces`,
  `uptrakit-web-api-types`, `uptrakit-shared-macros`,
  `uptrakit-build-info`.
- **No — binary-internal infrastructure, plugin, or runtime.** Set
  `git_only = true`, `git_release_enable = false`, `publish = false`.
  The git tag is the anchor. Current members:
  `uptrakit-controller-runtime`, `uptrakit-controller-core`,
  `uptrakit-service-connections`, every `uptrakit-plugin-*`, every
  `uptrakit-*-runtime`.

`release = false` crates are excluded from analysis and cannot be in
any `changelog_include` array — convert them to the appropriate anchor
shape first.

### Guardrails

Two test files in the workspace lock this rule in:

- [`crates/core/functional-tests/tests/release_config_invariants.rs`](../../crates/core/functional-tests/tests/release_config_invariants.rs)
  — asserts binary releasability flags (`BINARY_TARGETS` constant —
  match this list when adding a new binary crate) and the
  `git_only + publish = true` self-consistency rule.
- [`crates/shared/service-sdk/tests/no_workspace_db_deps.rs`](../../crates/shared/service-sdk/tests/no_workspace_db_deps.rs)
  and [`crates/shared/openapi-client/tests/no_workspace_db_deps.rs`](../../crates/shared/openapi-client/tests/no_workspace_db_deps.rs)
  — assert the five workspace-internal db/crypto/audit crates
  (`uptrakit-audit-log`, `uptrakit-audit-log-derive`,
  `uptrakit-shared-db`, `uptrakit-tenant-db`, `uptrakit-crypto`) never
  re-enter the publishable resolve graph. See [`docs/development/coding-standards.md`
  § Publishable Crate Dependency Hygiene](coding-standards.md).

Historical note: prior to the two-anchor model being documented, the
public-API libs were `git_only = true` + `publish = false`, which meant
they could never reach crates.io. `uptrakit-service-sdk` and
`uptrakit-openapi-client` therefore wedged at the `local > registry`
short-circuit (see PR #136). Earlier still, plain `[[package]]` entries
(no `git_only`) caused binary CHANGELOGs to re-print the full repo
history on every release (~5000 stale bullets per PR, see
PRs #131 and #132). Backfill tags were created at commit `87bb3e85e` (the
`uptrakit-controller-v0.0.4` release) to establish the baseline going
forward.

## Backfilling release assets

Use this when a release page on GitHub has missing or partial assets — for example after a
`release-plz` `Release` job hit HTTP 422 mid-loop and left some tags assetless, or after a
release was created via the GitHub UI rather than the workflow.

Trigger the `release-plz` workflow via `workflow_dispatch`, passing a comma-separated list of
tags:

```sh
gh workflow run release-plz.yml \
  -f backfill_tags=uptrakit-controller-v0.0.3,uptrakit-controller-standalone-v0.0.3
```

Each tag must match
`^uptrakit-(controller-standalone|controller|agent-ssh|agent|mqtt|scheduler|cli)-v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$`.
The longer prefixes (`controller-standalone`, `agent-ssh`) come first so they parse correctly;
the optional suffix allows SemVer pre-release tags such as `v0.1.0-rc.1`.

For each tag the workflow:

1. Validates the tag against the regex above and confirms the release already exists on
   GitHub. Tags that don't match or don't exist fail the run; backfill never synthesises a
   release page.
2. Builds the binary for all 4 targets (`x86_64-unknown-linux-gnu`,
   `aarch64-unknown-linux-gnu`, `x86_64-unknown-linux-musl`, `aarch64-apple-darwin`).
3. Packages each binary into `${pkg}-${version}-${target}.tar.gz` and computes a local
   sha256.
4. Compares the local sha256 against the GitHub asset's stored `digest`. If they match, the
   asset is already correct and the workflow skips attest + upload for that archive (logged
   as `::notice::`).
5. Otherwise runs `actions/attest@v4` to register SLSA build provenance for the changed
   archives — **before** publishing — and then `gh release upload --clobber` to write the
   archive and its `.sha256` sidecar to the release.

Re-running the workflow against the same tags is safe: digest-equal archives no-op,
digest-different archives are replaced. Sigstore's attestation log is append-only, so
backfilled releases can accumulate multiple attestations over time;
`gh attestation verify` always picks the matching one for the currently-published bytes.

The `docker.yml` workflow is **not** retriggered by backfill — `gh release upload --clobber`
only touches release-asset rows, not git tags. Docker images that were already built for the
tag remain unchanged.

## Related documentation

- [Commit Messages](commit-messages.md) — Conventional Commits format required by release-plz
- [Docker](docker.md) — Docker image build process
- [Quality Gates](quality-gates.md) — CI checks that must pass before merging

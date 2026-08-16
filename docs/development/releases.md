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

### Strict frontend gate

`frontend/build.rs` embeds the SvelteKit SPA from `frontend/build/` (gitignored). Its behaviour is keyed on cargo's
`PROFILE` build-script env var:

- **Debug profile** (plain `cargo build`/`check`/`test`/`clippy`): if `frontend/build/index.html` is absent, a stub
  page is embedded and a `cargo::warning` is emitted. CI lint/test jobs, pre-push, and local workspace builds need no
  frontend assets.
- **Any profile inheriting `release`** (`--release`, `--profile release-fast`; unknown/missing values are also
  strict — fail-closed): if `frontend/build/index.html` is absent, the build **fails** with an actionable
  `cargo::error` message. Every ship path (release-plz binary builds, backfill, all docker images, cross targets,
  `cargo install --path` from a clone with built assets) builds `--release`, so none can silently ship the stub UI
  (see [Installing from source](#installing-from-source) — `cargo install --git` of the controller binaries cannot
  satisfy the gate). Profiles inheriting `dev` report `debug` and keep the stub.

Why the surrounding release machinery never trips the gate spuriously:

- **release-plz never compiles this crate:** `ci/release-plz/cargo-wrapper.sh` injects `--no-verify` into
  `cargo package --workspace` and `semver_check = false` disables semver builds. The public-API library crates that
  ship to crates.io (`publish = true` in `release-plz.toml`) neither include nor depend on `uptrakit-frontend` — its
  only dependent, `uptrakit-controller-runtime`, is `git_only` and unpublished — and `cargo publish` verification and
  docs.rs both compile in the debug profile, which the gate treats leniently.
- **`cargo chef cook` (docker) runs dummy build scripts** generated by cargo-chef's skeleton; the real
  `cargo build --release` runs after the frontend `COPY`, with assets present.

**Convention made explicit:** the gate encodes "release profile ⇒ shippable ⇒ real assets required". A future
release-profile CI job that compiles the controller graph (e.g. `--release` coverage or a smoke test) must supply
`frontend/build/` first (`npm run build`, or download the `frontend-build` artifact) or it fails at
`uptrakit-frontend`'s build script. That failure is loud CI breakage — the safe direction; never a shipped stub.

**Break-glass (emergency releases):** if an urgent backend-only release must ship while the frontend build is broken
for unrelated reasons, an operator can deliberately supply a placeholder:

```sh
mkdir -p frontend/build && echo '<!doctype html><title>UI unavailable</title>' > frontend/build/index.html
```

Same degraded-UI outcome as the old silent stub, but as a visible, deliberate act. The placeholder is **transient and
must never be committed** — `frontend/build/` stays gitignored; a committed placeholder would satisfy the gate for
every future build and silently reinstate the hazard the gate exists to kill.

**Scope / residual:** the gate keys only on `index.html` presence. A `frontend/build/` with a present-but-drifted
`index.html` and partial sibling assets passes; content/completeness validation is deliberately out of scope.

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

All binary crates support `cargo install --git`, except the controller, which needs a local clone (see below):

```sh
# Controller — requires the built frontend, so install from a local clone
# (oidc, notifications-all, and the embedded frontend are always on):
git clone https://github.com/worried-networking/uptrakit
cd uptrakit/frontend && npm ci && npm run build && cd ..
cargo install --path crates/core/controller \
  --features db-all,embedded-scheduler,nats

# All-in-one (embedded scheduler/mqtt/agent/ssh-agent are already on):
# cargo install --path crates/core/controller-standalone --features db-all,nats
```

```sh
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

`cargo install` builds in the release profile, so installing either controller binary requires `frontend/build/`
to exist at compile time — the build fails otherwise (see
[Strict frontend gate](#strict-frontend-gate)). Direct `cargo install --git` of the controller binaries is
therefore unsupported (cargo's temp checkout can never contain the built frontend) — install from a local clone
as above, or use the release binaries / docker images. Release controller binaries always serve the embedded
copy, with no runtime override; only debug builds auto-detect `./frontend/build` (or `./frontend`) relative to
the working directory.

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

## AI-polished release notes

The `polish-notes` job in `release-plz.yml` rewrites each new release's raw git-cliff commit
list into an operator-facing summary. Pipeline shape:

1. **Filter** the releases from this cycle down to packages with a GitHub release page
   (`git_release_enable = true` in `release-plz.toml`) — `git_only` crates that only get a tag
   are skipped.
2. **Generate**, per release, via a credential-less, read-only opencode agent: it reads the
   package's `CHANGELOG.md`, the full commit messages in the release range, and a `git diff`
   scoped to the package's directories, and prints a notes document between sentinel markers.
3. **Extract** the document from between the sentinels.
4. **Validate** it deterministically (starts with `## Summary`, contains `## Highlights`, links
   the changelog, length and fence-balance checks).
5. **Publish** via `gh release edit`.

The opencode CLI is installed via `npm install -g` and pinned to an exact version in the
workflow step. The model is `google/gemini-3.6-flash`, overridable without a workflow edit
via the repo variable `POLISH_NOTES_MODEL` — swap the variable to move to a replacement
model (retirement, price change) without touching the workflow.

`polish-notes.sh` re-exports the `GEMINI_API_KEY` secret as `GOOGLE_GENERATIVE_AI_API_KEY` before
invoking opencode. This is not redundant: opencode only injects an API key for providers that
declare exactly one environment-variable name, and `google` declares three — so it enables the
provider but delegates authentication to `@ai-sdk/google`, which reads
`GOOGLE_GENERATIVE_AI_API_KEY` and nothing else. Swapping `POLISH_NOTES_MODEL` to a non-Google
provider means supplying that provider's own key variable, which the script leaves untouched if
already set. The secret is mandatory in CI only — a local run can omit it and rely on
`opencode auth login`, whose stored credential takes precedence over both variables.

The `GEMINI_API_KEY` secret should be a dedicated CI-only key with a quota/budget cap, not a
personal key. Rotate it if you suspect it's been exposed: the agent's bash tool inherits the
key (it needs it to call the model), so a prompt-injected commit message could in principle try
to exfiltrate it. Repo integrity is unaffected either way — the agent holds no GitHub
credential (`GH_TOKEN` is stripped from its environment, `persist-credentials: false` on
checkout, and it runs as opencode's read-only `plan` agent), so the residual risk is limited to
the billable key, not the repository.

A red `polish-notes` job means partial or zero edits were applied, or the key/model is broken —
it never means releases or their assets are affected, since the job runs after those already
exist. For a transient failure (model timeout, rate limit), re-run it from the Actions UI; it
resumes idempotently, because release bodies that already start with `## Summary` are skipped.

A re-run checks out the original run's commit, so it cannot pick up a workflow or script fix
landed afterwards. When the job itself was broken, backfill the affected releases locally once the
fix is on `main` — the same idempotent skip applies, so listing already-polished tags is harmless:

```sh
git fetch --tags --force   # prev-tag lookup walks the full tag list
export GH_TOKEN=$(gh auth token)          # needs contents:write
export MODEL=google/gemini-3.6-flash      # no default outside the workflow
export RELEASES='[{"package_name":"uptrakit-controller","tag":"uptrakit-controller-v0.0.6","version":"0.0.6"}]'
ci/release-plz/polish-notes.sh
```

`uptrakit-controller` and `uptrakit-controller-standalone` share most of their changes but are
polished as independent releases, so their notes can end up worded differently even when
describing the same underlying change — this divergence is accepted, not a bug.

To dry-run the script locally without publishing:

```sh
RELEASES='[{"package_name":"uptrakit-controller","tag":"uptrakit-controller-v0.0.6","version":"0.0.6"}]' \
  POLISH_NOTES_SKIP_PUBLISH=1 ci/release-plz/polish-notes.sh
```

Per [AI Guidelines](ai-guidelines.md), only public repository content (commits, diffs, and
changelogs of this public repo) is sent to the Gemini API — no sensitive data.

## Related documentation

- [Commit Messages](commit-messages.md) — Conventional Commits format required by release-plz
- [Docker](docker.md) — Docker image build process
- [Quality Gates](quality-gates.md) — CI checks that must pass before merging

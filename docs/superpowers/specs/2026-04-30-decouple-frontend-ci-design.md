# Decouple Frontend Build from Backend CI Jobs

**Date:** 2026-04-30
**Status:** Approved

## Problem

Four backend CI jobs (`backend-lint`, `backend-test`, `system-integration-tests`,
`database-integration-tests`) run `npm ci && npm run build` before any Rust work. A broken
SvelteKit build blocks backend-only hotfixes from landing, even when no frontend code changed.

## Root Cause

The root cause differs per job:

- **`backend-lint` / `backend-test`**: Run `cargo check --all-features` and
  `cargo test --all-features`. Two independent activation vectors pull in `uptrakit-frontend`:
  (1) `embedded-frontend` is in `uptrakit-controller-runtime`'s `default` feature set, which
  activates `dep:uptrakit-frontend` on any build that does not suppress defaults on that crate
  directly; (2) `uptrakit-controller` and `uptrakit-controller-standalone` unconditionally list
  `"embedded-frontend"` in their `[dependencies]` feature vector for `uptrakit-controller-runtime`
  — features declared this way are additive and cannot be suppressed by `--no-default-features`
  on the dependent crate. Either vector alone is sufficient to pull in `uptrakit-frontend`.
  Without `frontend/build/`, the naive path would fail — so CI runs npm first.

- **`system-integration-tests`**: Builds `Dockerfile.test` then runs
  `cargo test -p uptrakit-integration-tests -- --ignored`. The integration-tests crate has no
  dependency on `uptrakit-frontend`; the Docker build uses its own internal `frontend-builder`
  stage. The host-side npm step was defensive and is not needed for either step.

- **`database-integration-tests`**: Runs
  `cargo test -p uptrakit-integration-tests --test database -- --ignored`. Same crate — no
  `uptrakit-frontend` dep. Host-side npm step is not needed.

## Why the Fix Is Safe

`frontend/build.rs` already has a stub fallback: if `frontend/build/index.html` is absent, it
embeds a single-page placeholder. This path was intentionally added for release-plz git_only
mode, which runs `cargo package --workspace` in a fresh worktree where `frontend/build/` does
not exist. The stub is sufficient for all backend CI jobs because none of them test frontend
asset delivery. `frontend/build/` is gitignored and therefore absent on every fresh CI checkout
— the stub runs on every CI run today; npm currently pre-empts it before Cargo starts.

The stub does not cause `cargo clippy` to fail. Build-script `cargo::warning` output appears in
Cargo's build log but is not a `rustc` diagnostic and is not subject to `-D warnings`. No
compiler lint is emitted.

`Dockerfile.test` already has its own `frontend-builder` stage that runs `npm ci && npm run build`
inside Docker. The Docker image is self-contained; it does not rely on any host-built artifacts.

## Change

One file changed: `.github/workflows/ci.yml`

Remove `actions/setup-node` and `npm ci && npm run build` from:

- `backend-lint`
- `backend-test`
- `system-integration-tests`
- `database-integration-tests`

### What stays unchanged

| Item | Reason |
| --- | --- |
| `frontend` job in `ci.yml` | Sole npm health gate; runs lint, format check, type check, tests, and build |
| `reverse-proxy-tests` | Already has no npm steps |
| `markdown` job | Uses npm only for `markdownlint-cli` global install; unrelated to frontend assets |
| `frontend/build.rs` stub | Already handles absent `frontend/build/`; no Rust changes needed |
| `docker/Dockerfile.test` | Already self-contained with its own `frontend-builder` stage |
| `release-plz.yml` `release-pr` job | Runs `npm run build` to populate `frontend/build/` so `cargo package` embeds real assets; load-bearing even though the CI wrapper injects `--no-verify`, because the embedded content must be present for a valid package |
| `release-plz.yml` `build-frontend` job | Builds frontend artifact uploaded to `build-artifacts`; feeds the release binary pipeline; intentionally retained |
| `docker.yml` `build-frontend` job | Production Docker images embed the real frontend; this coupling is intentional and out of scope |

### Known remaining coupling

`docker.yml` triggers on `pull_request` against `main` and has a `build-frontend` job that all
Docker build matrix jobs depend on. After this change, backend-only PRs will no longer be
blocked by frontend failures in `ci.yml`, but will still be blocked by frontend failures in
`docker.yml`. Fully decoupling `docker.yml` requires either restructuring the production
Dockerfile to embed its own npm stage (like `Dockerfile.test` does) or accepting that production
image builds are a legitimate gate on frontend health. That decision is out of scope here.

## Pre-Merge Prerequisites

1. **Verify the stub under full-feature Clippy via CI, not locally.** The stub path has been
   validated by `cargo package --workspace` (release-plz git_only) but never by
   `cargo clippy --all-targets --all-features -- -D warnings` or `cargo test --all-features` with
   npm absent. Local toolchain versions differ from CI's `dtolnay/rust-toolchain@stable` — a lint
   that is `allow` locally may be `warn` on the CI runner, and `-D warnings` promotes it to error.
   Push the decoupling change to a branch, observe `backend-lint` and `backend-test` complete
   green in CI, then merge.

2. **Enforce `frontend` as a required status check before opening the PR.** After this change,
   `frontend` is the sole `ci.yml` gate preventing a broken SvelteKit build from reaching `main`.
   If it is not a required check, a broken frontend can merge, trigger `release-plz`, produce a
   GitHub release and crates.io publish, then cause `build-frontend` in `release-plz.yml` to fail
   — leaving a release with no binary assets attached. Verify and enforce this in branch
   protection *before* opening the decoupling PR, not concurrently.

3. **Grep for `Assets::` usage in test modules.** The stub embeds a single placeholder HTML file.
   Any test that calls `Assets::get(...)` and asserts on file count, content, content-type, or
   path structure will pass with the stub but diverge from production. Run:
   `grep -r "Assets::" crates/ --include="*.rs" -l` and confirm no test module exercises
   asset content. If found, gate those tests behind a feature flag or skip them when
   `frontend/build/` is absent.

## Implementation Notes

- The Rust cache written by `backend-lint` (`Swatinem/rust-cache`, `shared-key: "rust-all-features"`,
  no explicit `save-if` — defaults to write) will be invalidated on the first run after this
  change: the compiled `uptrakit-frontend` artifact changes from real-asset to stub. All other
  jobs using this cache key (`backend-test`, `backend-deny`, `reverse-proxy-tests`,
  `system-integration-tests`, `database-integration-tests`) have `save-if: "false"` and only
  read it. Expect one slow `backend-lint` run; subsequent runs restore the stub-based cache
  normally.

- **Pre-existing release pipeline risk (not introduced by this change):** `release-plz` creates
  tags and publishes crates before `build-artifacts` runs. If `build-frontend` fails,
  `build-artifacts` is skipped and the release has no binary downloads. This risk existed before;
  this change raises its likelihood by removing the implicit gate (a broken frontend currently
  blocks `backend-lint`, which prevents `main` from advancing). Covered by prerequisite 2 above.

## Success Criteria

- `backend-lint`, `backend-test`, `system-integration-tests`, and `database-integration-tests` in
  `ci.yml` complete without running `npm ci` or `npm run build`.
- A broken SvelteKit build surfaces in the `frontend` job (and `docker.yml` `build-frontend`),
  but not in the four `ci.yml` jobs listed above.
- `cargo check --all-features`, `cargo clippy --all-targets --all-features`, and
  `cargo test --all-features` all pass without a pre-built `frontend/build/` (verified via CI,
  per prerequisite 1).

## Out of Scope

- Decoupling `docker.yml` `build-frontend` from backend Docker builds (separate decision)
- Per-crate change detection in CI (independent improvement)
- sccache or build matrix optimizations (independent improvement)
- Frontend repo extraction (independent decision)

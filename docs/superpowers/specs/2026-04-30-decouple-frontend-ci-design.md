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
  (1) `--all-features` on the workspace activates every workspace member including
  `uptrakit-frontend` directly; (2) `uptrakit-controller` and `uptrakit-controller-standalone`
  hardcode `"embedded-frontend"` in their `[dependencies]` feature vector for
  `uptrakit-controller-runtime`, making it unconditional — `--no-default-features` cannot
  suppress it. Without `frontend/build/`, the naive path would fail — so CI runs npm first.

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

Single file: `.github/workflows/ci.yml`

Remove `actions/setup-node` and `npm ci && npm run build` from:

- `backend-lint`
- `backend-test`
- `system-integration-tests`
- `database-integration-tests`

### What stays unchanged

| Item | Reason |
| --- | --- |
| `frontend` job | Sole npm health gate; runs lint, format check, type check, tests, and build |
| `reverse-proxy-tests` | Already has no npm steps |
| `markdown` job | Uses npm only for `markdownlint-cli` global install; unrelated to frontend assets |
| `frontend/build.rs` stub | Already handles absent `frontend/build/`; no Rust changes needed |
| `docker/Dockerfile.test` | Already self-contained with its own `frontend-builder` stage |

## Implementation Notes

- The shared Rust cache (`Swatinem/rust-cache`, `shared-key: "rust-all-features"`) will be
  invalidated on the first run after this change because the compiled `uptrakit-frontend`
  artifact changes from real-asset to stub. Expect one slow CI run; subsequent runs hit the
  cache normally.

## Success Criteria

- Backend-only PRs land without `npm ci` or `npm run build` running in any backend job.
- A broken SvelteKit build surfaces only in the `frontend` job, not in any backend job.
- `cargo check --all-features`, `cargo clippy --all-targets --all-features`, and
  `cargo test --all-features` all pass without a pre-built `frontend/build/`.

## Out of Scope

- Per-crate change detection in CI (independent improvement)
- sccache or build matrix optimizations (independent improvement)
- Frontend repo extraction (independent decision)

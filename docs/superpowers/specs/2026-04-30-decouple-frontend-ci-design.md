# Decouple Frontend Build from Backend CI Jobs

**Date:** 2026-04-30
**Status:** Approved

## Problem

Four backend CI jobs (`backend-lint`, `backend-test`, `system-integration-tests`,
`database-integration-tests`) run `npm ci && npm run build` before any Rust work. A broken
SvelteKit build blocks backend-only hotfixes from landing, even when no frontend code changed.

## Root Cause

`uptrakit-controller-runtime` has `embedded-frontend` in its default feature set, which pulls in
`uptrakit-frontend` as an optional dep. `cargo check --all-features` and `cargo test --all-features`
activate this feature. Without a pre-built `frontend/build/`, Cargo panics — so CI runs npm first.

## Why the Fix Is Safe

`frontend/build.rs` already has a stub fallback: if `frontend/build/index.html` is absent, it
embeds a single-page placeholder and emits `cargo::warning` (not an error). This path was
intentionally added for release-plz git_only mode, which runs `cargo package --workspace` in a
fresh worktree where `frontend/build/` does not exist. The same stub is sufficient for backend CI
jobs that do not test frontend asset delivery.

`Dockerfile.test` already has its own `frontend-builder` stage (`npm ci && npm run build`) that
runs inside Docker. The host-side npm step in `system-integration-tests` is therefore redundant.

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
| `frontend/build.rs` stub | Already handles absent `frontend/build/`; no Rust changes needed |
| `docker/Dockerfile.test` | Already self-contained with its own `frontend-builder` stage |

## Success Criteria

- A backend-only PR lands successfully when `frontend/build/` is absent from the workspace.
- A broken SvelteKit build surfaces only in the `frontend` job, not in any backend job.
- `cargo clippy -D warnings` passes with the stub (stub emits `cargo::warning`, not a compiler error).

## Out of Scope

- Per-crate change detection in CI (independent improvement)
- sccache or build matrix optimizations (independent improvement)
- Frontend repo extraction (independent decision)

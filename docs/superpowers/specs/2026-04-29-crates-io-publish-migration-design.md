# crates.io Publish Migration — Design Spec

**Date:** 2026-04-29
**Status:** Approved

## Problem

`uptrakit-service-sdk` and `uptrakit-openapi-client` were designed for external plugin authors but are
not actually usable externally. The current implementation vendors/snapshots copies of four workspace
crates (`uptrakit-wire`, `uptrakit-shared-types`, `uptrakit-web-api-types`, `uptrakit-surfaces`) into
`src/generated/` directories, with a `workspace-internal` feature that switches between vendored types
(external) and real workspace types (internal). This creates:

- Two xtask sync commands (`sync-sdk`, `sync-openapi-client`) that must be run after every type change
- ~100 generated files that drift from their sources
- Six `#[cfg(not(feature = "workspace-internal"))]` non-additive feature gates
- Passthrough stub features (`sea-orm = []`, `http-ssrf = []`, etc.) only present to suppress
  `unexpected_cfgs` from vendored copies

The correct solution is publishing the shared type crates to crates.io directly, making both SDK
crates proper crates.io consumers.

## Scope

### Crates published to crates.io (7 total)

All start at version `0.0.1` with independent versioning (not `version.workspace = true`):

| Crate                     | Path                            |
| ------------------------- | ------------------------------- |
| `uptrakit-shared-macros`  | `crates/shared/macros`          |
| `uptrakit-surfaces`       | `crates/shared/surfaces`        |
| `uptrakit-shared-types`   | `crates/shared/types`           |
| `uptrakit-wire`           | `crates/shared/wire`            |
| `uptrakit-web-api-types`  | `crates/shared/web-api-types`   |
| `uptrakit-service-sdk`    | `crates/shared/service-sdk`     |
| `uptrakit-openapi-client` | `crates/shared/openapi-client`  |

The 5 shared crates have zero internal path deps outside this set and depend only on public crates —
they are publishable as-is once `publish` is configured correctly.

### Out of scope

- Internal crates (controller, agents, scheduler, mqtt, plugins, web-api, etc.) stay `git_only = true`,
  no crates.io publish
- No new features added to any crate in this migration
- The crates inlined into `service-sdk` (backoff, build-info, dirs, tracing-init) stay inlined — they
  are internal implementation details, not re-exported

## Architecture

### Why no `[patch.crates-io]` is needed

Workspace builds already use `{ workspace = true }` path deps for all 5 shared crates. Cargo
auto-converts path+version deps to registry deps on `cargo package`. This means:

- **Workspace builds**: all crates consuming `uptrakit-wire` etc. resolve to the same local path →
  type identity preserved
- **Published crates**: `cargo package` rewrites `{ path = "...", version = "0.0.1" }` to
  `uptrakit-wire = "0.0.1"` (crates.io registry dep) in the published manifest
- **External consumers**: get crates.io versions, consistent types throughout

No `[patch]` entry required.

### release-plz root cause (be8fc14b)

Workspace `publish = false` caused release-plz to query crates.io for every member. Empty results
tripped `registry_package_exists = false` and short-circuited ALL version bumps — no release PR
opened after 0.0.1. The workaround was `git_only = true` everywhere. The fix is publishing the 7
crates so crates.io has them as a real baseline for subsequent release-plz runs.

## Changes (5 commits, single PR)

### Commit 1 — 5 shared crates: independent versions + publish

For each of `uptrakit-shared-macros`, `uptrakit-surfaces`, `uptrakit-shared-types`, `uptrakit-wire`,
`uptrakit-web-api-types`:

- Set `version = "0.0.1"` explicitly in `Cargo.toml` (replacing `version.workspace = true`)
- Add `publish = true` per-crate (overrides workspace `publish = ["uptrakit-private"]`)

All 5 already have `version = "0.0.1"` alongside `path = "..."` in `[workspace.dependencies]` —
required for Cargo to rewrite path deps to registry deps on `cargo package`. No change needed there.

No code changes — these crates are already clean.

### Commit 2 — service-sdk: remove workspace-internal + generated/

**`Cargo.toml` changes:**

- Remove `workspace-internal` feature
- Change `uptrakit-wire` and `uptrakit-shared-types` from `optional = true` to hard non-optional deps
- Remove passthrough stub features: `sea-orm = ["dep:sea-orm"]`, `openapi = ["dep:utoipa"]`,
  `http-ssrf = []`. These existed only to suppress `unexpected_cfgs` from vendored copies. If
  `sea-orm`/`openapi` forwarding is still needed post-migration, replace with
  `sea-orm = ["uptrakit-shared-types/sea-orm"]` etc. Evaluate at implementation time.
- Set `version = "0.0.1"` explicitly, `publish = true`

**`src/lib.rs` changes:**

Replace alternating cfg mod blocks with single direct re-exports:

```rust
pub(crate) mod wire_api { pub(crate) use uptrakit_wire::*; }
pub(crate) mod shared_types_api { pub(crate) use uptrakit_shared_types::*; }
```

**Deleted:** `src/generated/` (entire directory)

### Commit 3 — openapi-client: remove workspace-internal + generated/, restore shared-macros dep

**`Cargo.toml` changes:**

- Remove `workspace-internal` feature
- Change `uptrakit-web-api-types` and `uptrakit-shared-types` from `optional = true` to hard
  non-optional deps
- Add `uptrakit-shared-macros = { workspace = true }` as hard non-optional dep
- Remove passthrough stub features: `sea-orm = []`, `http-ssrf = []`, `openapi = []`,
  `test-support = []`. If openapi forwarding is meaningful, replace with
  `openapi = ["uptrakit-web-api-types/openapi"]`. Evaluate at implementation time.
- Set `version = "0.0.1"` explicitly, `publish = true`

**`src/lib.rs` changes:**

Replace alternating cfg mod blocks with single direct re-exports.

**`src/macros.rs` changes:**

- Remove inlined copies of `impl_report_conversion!` and `wire_safe_enum!`
- Re-export from `uptrakit-shared-macros` (adjust to match actual macro export style)

**Deleted:** `src/generated/` (entire directory)

### Commit 4 — xtask: delete sync commands

- Delete `xtask/src/sync_sdk.rs`
- Delete `xtask/src/sync_openapi_client.rs`
- Update `xtask/src/main.rs`: remove `sync-sdk` and `sync-openapi-client` subcommands and all
  related imports/dispatch

### Commit 5 — release-plz: enable publish for 7 crates

**`release-plz.toml` changes:**

Add `[[package]]` entries for the 5 shared crates (no `git_only`, defaults to publish to crates.io):

```toml
[[package]]
name = "uptrakit-shared-macros"

[[package]]
name = "uptrakit-surfaces"

[[package]]
name = "uptrakit-shared-types"

[[package]]
name = "uptrakit-wire"

[[package]]
name = "uptrakit-web-api-types"
```

For `uptrakit-service-sdk` and `uptrakit-openapi-client`: remove `git_only = true` from their
existing `[[package]]` entries. Keep `git_release_enable = true` and `git_tag_enable = true` —
GitHub releases remain useful for SDK consumers tracking releases.

## Bootstrap (post-merge, manual)

First publish must be done manually in topological order since these crates don't exist on crates.io:

```bash
cargo publish -p uptrakit-shared-macros
cargo publish -p uptrakit-surfaces
cargo publish -p uptrakit-shared-types
cargo publish -p uptrakit-wire
cargo publish -p uptrakit-web-api-types
cargo publish -p uptrakit-service-sdk
cargo publish -p uptrakit-openapi-client
```

After first publish, release-plz handles all subsequent version bumps and publishes automatically.
`CARGO_REGISTRY_TOKEN` is already configured in the CI workflow — no new secrets needed.

## Quality Gates

Run after each commit:

```bash
cargo fmt --all
cargo check --no-default-features --features db-sqlite
cargo check --all-features
cargo clippy --all-targets --no-default-features --features db-sqlite
cargo clippy --all-targets --all-features
cargo test --all-features
cargo deny check
```

## What Gets Deleted

- `crates/shared/service-sdk/src/generated/` — replaced by real dep
- `crates/shared/openapi-client/src/generated/` — replaced by real dep
- `xtask/src/sync_sdk.rs` — no generated/ to produce
- `xtask/src/sync_openapi_client.rs` — no generated/ to produce
- `workspace-internal` feature in both SDK crates — no alternation needed
- Passthrough stub features (~6 total) — no vendored code requiring cfg suppression
- Six `#[cfg(not(feature = "workspace-internal"))]` gates — non-additive gates eliminated
- Inlined macros in `openapi-client/src/macros.rs` — restored to `uptrakit-shared-macros` dep

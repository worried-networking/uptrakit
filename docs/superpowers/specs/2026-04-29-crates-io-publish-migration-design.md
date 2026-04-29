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
- A pre-commit hook and CI workflow enforcing generated file freshness

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
- `service-sdk`'s own `macros.rs` (containing `impl_report_conversion!`) stays inlined — it is not
  migrated to `uptrakit-shared-macros` in this migration

## Architecture

### Why no `[patch.crates-io]` is needed

Workspace builds already use `{ workspace = true }` path deps for all 5 shared crates. Cargo
auto-converts path+version deps to registry deps on `cargo package`. This means:

- **Workspace builds**: all crates consuming `uptrakit-wire` etc. resolve to the same local path →
  type identity preserved
- **Published crates**: `cargo package` rewrites `{ path = "...", version = "0.0.1" }` to
  `uptrakit-wire = "0.0.1"` (crates.io registry dep) in the published manifest. Deps declared
  `{ workspace = true }` undergo the same rewrite since the workspace entry carries both `path` and
  `version`.
- **External consumers**: get crates.io versions, consistent types throughout

No `[patch]` entry required.

### Cascade versioning after bootstrap

The 5 shared crates use independent versioning. When a lower-level crate (e.g.,
`uptrakit-shared-types`) releases a new version, dependent crates (`uptrakit-wire`,
`uptrakit-web-api-types`) still declare the old version in their Cargo.toml files on disk (the
`{ workspace = true }` entry carries the pinned version from `[workspace.dependencies]`). To
propagate a new version downstream:

1. Bump the version pin in `[workspace.dependencies]` for the updated crate
2. Commit — release-plz detects the Cargo.toml change in each dependent crate and opens version
   bump PRs for them in the next release-pr cycle

This cascade is one `[workspace.dependencies]` edit per breaking change, not per dependent crate.

### release-plz root cause (be8fc14b)

Workspace `publish = false` caused release-plz to query crates.io for every member. Empty results
tripped `registry_package_exists = false` and short-circuited ALL version bumps — no release PR
opened after 0.0.1. The workaround was `git_only = true` everywhere. The fix is publishing the 7
crates so crates.io has them as a real baseline for subsequent release-plz runs.

### release-plz `[[package]]` entries and workspace-level defaults

`release-plz.toml` has a workspace-level `publish = false`. Crates **not** listed in `[[package]]`
inherit this default and are never published. Crates listed in `[[package]]` without `git_only = true`
or `publish = false` **will** publish to crates.io. Adding `[[package]]` entries for the 5 shared
crates (with no `git_only`) is sufficient to opt them into crates.io publishing.

### Transition window (merge → bootstrap)

After this PR merges, the 7 crates do not yet exist on crates.io. The `release-plz release-pr`
CI job fires on the merge push and may fail because `cargo package` cannot resolve transitive deps
not yet on crates.io. This leaves **no harmful state** — the `release-pr` job only opens GitHub
PRs, it never creates tags or publishes to crates.io. The failure shows as a red CI check on the
migration merge commit; this is expected and can be documented in the PR description. After
bootstrap, the next `main` push triggers a clean `release-pr` run. **Merge and bootstrap in the
same session** to minimize the window.

## Changes (5 commits, single PR)

### Commit 1 — 5 shared crates: independent versions + publish

For each of `uptrakit-shared-macros`, `uptrakit-surfaces`, `uptrakit-shared-types`, `uptrakit-wire`,
`uptrakit-web-api-types`:

- Set `version = "0.0.1"` explicitly in `Cargo.toml` (replacing `version.workspace = true`)
- Add `publish = true` per-crate (overrides workspace `publish = ["uptrakit-private"]`)

All 5 already have `version = "0.0.1"` alongside `path = "..."` in `[workspace.dependencies]` —
required for Cargo to rewrite path deps to registry deps on `cargo package`.

Also move `rootcause` in `uptrakit-shared-macros/Cargo.toml` from `[dependencies]` to
`[dev-dependencies]`. The crate's `#[macro_export]` macros reference `rootcause::` paths at the
call site's scope, not within the crate itself; runtime consumers should not inherit `rootcause`
as a transitive dependency.

No `.rs` source changes — these crates are already clean.

### Commit 2 — service-sdk: remove workspace-internal + generated/

**`Cargo.toml` changes:**

- Remove `workspace-internal` feature
- Change `uptrakit-wire` and `uptrakit-shared-types` from `optional = true` to hard non-optional deps
- Remove `http-ssrf = []` (no-op stub — plugin authors needing SSRF protection should depend on
  `uptrakit-shared-types` with `http-ssrf` feature directly)
- Replace `sea-orm = ["dep:sea-orm"]` with `sea-orm = ["uptrakit-shared-types/sea-orm"]` (real
  forwarding feature — allows plugin authors to enable sea-orm derives via service-sdk)
- Replace `openapi = ["dep:utoipa"]` with `openapi = ["uptrakit-shared-types/openapi"]` (real
  forwarding feature)
- Set `version = "0.0.1"` explicitly, `publish = true`

**`src/lib.rs` changes:**

Replace alternating cfg mod blocks with single direct re-exports:

```rust
pub(crate) mod wire_api { pub(crate) use uptrakit_wire::*; }
pub(crate) mod shared_types_api { pub(crate) use uptrakit_shared_types::*; }
```

Remove the `pub mod generated;` declaration and delete the `src/generated/` directory.

**Deleted:** `src/generated/` (entire directory)

### Commit 3 — openapi-client: remove workspace-internal + generated/, restore shared-macros dep

**`Cargo.toml` changes:**

- Remove `workspace-internal` feature
- Change `uptrakit-web-api-types` and `uptrakit-shared-types` from `optional = true` to hard
  non-optional deps
- Add `uptrakit-shared-macros = { workspace = true }` as hard non-optional dep
- Remove `http-ssrf = []` and `test-support = []` (no-op stubs with no meaningful forwarding target)
- Replace `sea-orm = []` with `sea-orm = ["uptrakit-shared-types/sea-orm"]` (real forwarding feature)
- Replace `openapi = []` with `openapi = ["uptrakit-web-api-types/openapi"]` (real forwarding feature)
- Set `version = "0.0.1"` explicitly, `publish = true`

**`src/lib.rs` changes:**

Replace alternating cfg mod blocks with single direct re-exports. Remove the `pub mod generated;`
declaration.

**`src/macros.rs` changes:**

`uptrakit-shared-macros` exports `impl_report_conversion!` and `wire_safe_enum!` as
`#[macro_export]` declarative macros. These are accessible to downstream crates via the crate
root path — no intermediate re-export module needed, but call sites must reference them explicitly.
The correct action:

- Delete `src/macros.rs`
- Remove `#[macro_use] mod macros;` from `src/lib.rs`
- The dep on `uptrakit-shared-macros` makes the macros available to call sites inside
  `openapi-client`. Update each call site to use the qualified path
  `uptrakit_shared_macros::impl_report_conversion!(...)` or add `use uptrakit_shared_macros::impl_report_conversion;`
  at the top of each file that uses the macro. Note: `service-sdk` has its own `macros.rs`
  with an inlined copy of `impl_report_conversion!` that intentionally stays — do not delete it.

**Deleted:** `src/generated/` (entire directory)

### Commit 4 — xtask + pre-commit hook + CI: delete sync infrastructure

- Delete `xtask/src/sync_sdk.rs`
- Delete `xtask/src/sync_openapi_client.rs`
- Update `xtask/src/main.rs`: remove `sync-sdk` and `sync-openapi-client` subcommands and all
  related imports/dispatch
- Update `.husky/pre-commit`: remove the two sync blocks at the end of the file — each block is
  an `echo` line followed by a `cargo xtask sync-*` line, plus a blank line between them (5 lines
  total). Delete everything from `echo '[pre-commit] Regenerating service-sdk...'` to the end.
- Delete `.github/workflows/generated-check.yml`: this workflow checks that `src/generated/`
  matches the sync output; with `generated/` gone it is obsolete

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

First publish must be done manually in topological order. Use the script below — it is idempotent
(skips already-published crates) and polls the crates.io index before each dependent publish to
avoid "no matching package" errors from index propagation lag:

```bash
#!/usr/bin/env bash
set -euo pipefail

publish_crate() {
  local name="$1"
  if cargo search "$name" --limit 1 2>/dev/null | grep -qE "^$name[[:space:]]*="; then
    echo "$name already on crates.io — skipping"
    return
  fi
  cargo publish -p "$name"
  echo "Waiting for $name 0.0.1 to appear in crates.io index..."
  until cargo search "$name" --limit 1 2>/dev/null | grep -qE "^$name[[:space:]]*="; do
    sleep 10
  done
  echo "$name indexed."
}

publish_crate uptrakit-shared-macros
publish_crate uptrakit-surfaces
publish_crate uptrakit-shared-types
publish_crate uptrakit-wire
publish_crate uptrakit-web-api-types
publish_crate uptrakit-service-sdk
publish_crate uptrakit-openapi-client
```

If a publish fails mid-sequence, re-run the script — it will skip already-completed steps and
resume from where it left off.

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

Run `cargo package --dry-run` for the 3 leaf crates after Commit 1. These have no internal deps
and validate cleanly pre-bootstrap:

```bash
cargo package --dry-run -p uptrakit-shared-macros
cargo package --dry-run -p uptrakit-surfaces
cargo package --dry-run -p uptrakit-shared-types
```

**Note:** `uptrakit-wire`, `uptrakit-web-api-types`, `uptrakit-service-sdk`, and
`uptrakit-openapi-client` each depend on other crates in the set. `cargo package --dry-run`
resolves their deps from crates.io — and those deps don't exist there yet. These 4 crates
can only be dry-run validated after bootstrap. Run the full 7-crate dry-run as post-bootstrap
verification before considering the migration complete.

## What Gets Deleted

- `crates/shared/service-sdk/src/generated/` — replaced by real dep
- `crates/shared/openapi-client/src/generated/` — replaced by real dep
- `crates/shared/openapi-client/src/macros.rs` — inlined macros restored via uptrakit-shared-macros dep
- `xtask/src/sync_sdk.rs` — no generated/ to produce
- `xtask/src/sync_openapi_client.rs` — no generated/ to produce
- `.github/workflows/generated-check.yml` — no generated/ to verify
- `workspace-internal` feature in both SDK crates — no alternation needed
- Passthrough stub features (~7 total) — no vendored code requiring cfg suppression
- Six `#[cfg(not(feature = "workspace-internal"))]` gates — non-additive gates eliminated
- Two sync blocks (5 lines) at the end of `.husky/pre-commit` that invoke sync commands

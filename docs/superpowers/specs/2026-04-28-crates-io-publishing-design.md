# crates.io Publishing & Binary Distribution Design

**Date:** 2026-04-28

## Goals

1. Allow external developers to create services via `uptrakit-service-sdk`
2. Allow external developers to build HTTP API clients via `uptrakit-openapi-client`
3. Allow installing all 6 binaries without building from source

## Published Crates (crates.io)

Exactly **2 crates** are published. Everything else stays `publish = false` permanently.

Both crates are currently **not** self-contained — they have workspace path deps that must be removed
as part of this work. The target state is zero workspace **path** dependencies in both. Workspace
metadata fields (`edition`, `license`, `authors`, `repository` using `.workspace = true`) are fine
and do not need to change.

No backwards compatibility guarantees. Both crates are `0.x`; breaking changes are allowed in minor
bumps and will be communicated in changelogs.

### `uptrakit-service-sdk`

Target state: self-contained, zero workspace path dependencies. Protocol types (wire message types,
surface contracts, shared primitives) owned inline under `src/generated/`. Only third-party
crates.io deps.

Current workspace deps to eliminate: `uptrakit-backoff`, `uptrakit-build-info`, `uptrakit-crypto`,
`uptrakit-directories`, `uptrakit-internal-wire`, `uptrakit-shared-macros`, `uptrakit-shared-types`,
`uptrakit-tracing-init`.

**Notes on specific deps:**

- `uptrakit-backoff` — `Backoff` struct is currently re-exported in the SDK's public API. Must be
  inlined (copied verbatim), not dropped, to preserve the public interface.
- `uptrakit-build-info` — used two ways: (a) runtime `BuildInfo::current()` in `main_helper.rs` —
  inline into service-sdk; (b) build-script entry point used by binary crates' `build.rs` — binaries
  keep direct path dep on `uptrakit-build-info` unchanged. Independent usages.
- `uptrakit-shared-macros` — contains only `macro_rules!` helpers (not proc macros). Two options:
  (a) inline needed macros by copy-paste; (b) publish `uptrakit-shared-macros` as a separate
  crates.io dep — it has **zero workspace path deps** (only `rootcause`) so publishing it adds just
  one extra crate with no cascade. Prefer (b) if the macro surface needed in the published crate is
  non-trivial.
- `uptrakit-crypto` — only ECIES sealed-box decrypt used. Moves behind `sensitive-params` feature;
  dep dropped.

**Codegen note:** Wire and surface types reference `uptrakit_shared_types::` and
`uptrakit_surfaces::` paths internally. A simple text copy of those source files into
`src/generated/` produces uncompilable code. The codegen must perform **AST-level path rewriting**
(using `syn`) to replace all internal crate paths with the SDK's own inline type paths. This is
non-trivial — scope carefully to only the types that appear in the SDK's public API surface.

**Alternative to AST rewriting:** `uptrakit-wire` (post-rename) and `uptrakit-surfaces` have zero
workspace path dependencies — they depend only on third-party crates (`serde`, `uuid`, `time`,
etc.). Publishing them on crates.io is trivially safe and would eliminate the codegen requirement
entirely. This path is **not chosen** because it forces external consumers to take on all wire
protocol types and surfaces API (a large, unstable surface), and because it would require
`publish = true` on crates that are intentionally internal. The xtask codegen approach keeps the
published surface minimal and explicitly curated.

**Features:**

- `sensitive-params` (off by default, **does not exist yet — must be added**) — gates ECIES
  sealed-box decryption. The `decrypt_sensitive_params` function (currently in
  `src/sensitive_params.rs`, unconditionally exported) moves behind this feature. Adds `aws-lc-rs`
  as a direct dep. Note: `aws-lc-rs` is already transitive via rustls in most dep trees, but a
  `default-features = false` build may not have it — document in crate README. `aws-lc-rs` also
  has non-trivial build prerequisites (C toolchain, NASM on some platforms) — document prominently.
- `zeroconf` (existing) — mDNS service discovery via `mdns-sd`
- `cli` (existing) — tracing init for CLI entry points
- `test-support` (existing) — test helpers

**Stability:** `0.x` — no API stability guarantee. MSRV tracks workspace MSRV (documented in root
`Cargo.toml`).

**Cargo.toml requires:** explicit `publish = true` (required to override workspace default
`publish = false`), independent `version` field (not `version.workspace`).

### `uptrakit-openapi-client`

Target state: self-contained, zero workspace path dependencies. HTTP request/response types owned
inline under `src/generated/`. Only third-party crates.io deps.

Current workspace deps to eliminate: `uptrakit-shared-macros`, `uptrakit-shared-types`,
`uptrakit-web-api-types` (which transitively pulls `uptrakit-internal-wire`). Same codegen note
applies — path rewriting required for any types referencing other internal crates.

**Stability:** `0.x` — no API stability guarantee. MSRV tracks workspace MSRV.

**Cargo.toml requires:** explicit `publish = true`, independent `version` field (not
`version.workspace`).

### What is NOT published

All internal crates stay `publish = false`:

- `uptrakit-wire` (renamed from `uptrakit-internal-wire`)
- `uptrakit-surfaces`
- `uptrakit-shared-types`, `uptrakit-backoff`, `uptrakit-build-info`, `uptrakit-tracing-init`,
  `uptrakit-directories`
- `uptrakit-shared-macros` — unless chosen as option (b) above, in which case it gains
  `publish = true`
- `uptrakit-crypto` and all cryptographic internals
- `uptrakit-frontend` — owns the rust-embed `Assets` struct; `git_tag_enable = true` so
  release-plz can cascade frontend version bumps to the controller, but no crates.io publish
- `uptrakit-agent-runtime`, `uptrakit-agent-ssh-runtime`, `uptrakit-mqtt-runtime`,
  `uptrakit-scheduler-runtime` — `git_tag_enable = true` for version cascade to binary crates;
  no crates.io publish
- All plugins, web-api stack, DB layer, agent-core, scheduler engine

## Binary Distribution

**No `cargo install` from crates.io for binaries.** Instead:

- The existing `build-artifacts` job in `release-plz.yml` handles cross-platform builds and uploads
  to GitHub Releases. No migration to cargo-dist.
- **`cargo-binstall`** is the recommended install method for end users — downloads pre-built binary,
  falls back to source compile. Requires five fixes to the existing workflow (see below) plus
  `[package.metadata.binstall]` in each binary crate's `Cargo.toml`.
- **`cargo install --git`** remains as a developer escape hatch.

**Note:** The asset rename (fix #1 below) is a **breaking change** for any external scripts,
Dockerfiles, or CI pipelines that currently download assets by exact name. No migration period —
clean break at next release.

**Five workflow fixes required:**

1. **Asset naming** — rename outputs from `{name}-{target}` to `{name}-{version}-{target}.tar.gz`.
   binstall pattern matching requires a versioned archive format. Contents: single binary at the
   root of the archive.
2. **Checksums** — generate `.sha256` files (`sha256sum` on Linux, `shasum -a 256` on macOS; both
   produce `<hex-hash>  <filename>` format) alongside each asset before upload. These files are for
   human verification and script integrity checks — **binstall does not verify sidecar `.sha256`
   files**; binstall integrity is handled by the `pkg-url` HTTPS transport and optional manifest
   checksums only.
3. **`cross` pin** — replace
   `cargo install cross --git https://github.com/cross-rs/cross` (compiles from HEAD on every run)
   with a pinned release tag and `actions/cache` on `~/.cargo/bin/cross`.
4. **musl builds** — add `x86_64-unknown-linux-musl` target to the build matrix. GNU-linked
   binaries fail on systems with older glibc (Ubuntu 20.04, RHEL 8). musl produces
   statically-linked binaries with no runtime libc dependency. Use `cross` for musl builds.
5. **Per-package version extraction** — asset filenames embed the version (fix #1), but release-plz
   creates independent per-package releases with independent versions. The `upload_if_released`
   helper already extracts the tag per package from `release-plz.outputs.releases`; it must also
   extract the `version` field and use it when constructing the filename (e.g.
   `uptrakit-agent-${VERSION}-${TARGET}.tar.gz`). Without this, the filename cannot be constructed
   before knowing which packages were actually released in this run.

**Binaries exposed to binstall** (6 packages):

- `uptrakit-controller`, `uptrakit-agent`, `uptrakit-agent-ssh`, `uptrakit-mqtt`,
  `uptrakit-scheduler`, `uptrakit-cli`

**Binstall metadata — each crate needs its own stanza** pointing to its own release tag
(release-plz creates per-package tags like `uptrakit-agent-v0.0.1`):

```toml
# Example for uptrakit-agent
[package.metadata.binstall]
pkg-url = "{ repo }/releases/download/uptrakit-agent-v{ version }/uptrakit-agent-{ version }-{ target }.tar.gz"
bin-dir = "{ bin }{ binary-ext }"
pkg-fmt = "tgz"

# Example for uptrakit-cli (binary name differs from package name)
[package.metadata.binstall]
pkg-url = "{ repo }/releases/download/uptrakit-cli-v{ version }/uptrakit-cli-{ version }-{ target }.tar.gz"
pkg-fmt = "tgz"
[[package.metadata.binstall.overrides]]
bin-name = "uptrakit"
bin-dir = "uptrakit{ binary-ext }"
```

**Platforms:** Linux (x86_64-gnu, x86_64-musl, aarch64-gnu) and macOS (x86_64, aarch64). Windows
is **out of scope**.

`uptrakit-controller-standalone` is a build artifact (same crate, different feature flags) with no
distinct Cargo package name — not binstall-installable. Available as manual GitHub Release download
only. Resolving this is deferred.

**Frontend embedding:** Already handled. `uptrakit-frontend` is a proper workspace crate
(`publish = false`) that owns the rust-embed `Assets` struct. The `build-frontend` CI job runs
`npm run build` and uploads the static assets; `build-artifacts` downloads them to `frontend/build/`
before building the controller. The controller's `embed-frontend` feature pulls in
`uptrakit-frontend` as an optional dep — the Cargo dep edge lets release-plz cascade frontend
version bumps to the controller automatically.

## Wire/Surface Type Sync

The generated type modules in service-sdk and openapi-client are kept in sync with internal sources
via an xtask workflow. **The xtask crate and `src/generated/` directories do not yet exist — both
must be created as part of this work.**

**CI is the authoritative gate** for sync correctness. The pre-commit hook is fast local feedback
only — it can be bypassed (`--no-verify`, GUI clients, direct pushes). Never rely on the hook for
correctness.

Pre-commit hook installed via `husky-rs` (already a workspace dev-dependency). The xtask binary is
compiled by cargo; subsequent runs use incremental build cache and are fast.

### service-sdk

Source of truth: `crates/shared/wire/` and `crates/shared/surfaces/`
Generated output: `crates/shared/service-sdk/src/generated/`

Note: `uptrakit-surfaces` is currently referenced by `uptrakit-internal-wire` via
`path = "../surfaces"` but is **not declared in `[workspace.dependencies]`** — fix this (add to
workspace deps) in the wire rename PR.

The codegen uses `syn` to parse internal wire and surface types and emits equivalent Rust
structs/enums with paths rewritten to the SDK's namespace. Generated types carry the same field
names, serde attributes, and `#[non_exhaustive]` annotations as their internal counterparts. Module
is `pub mod generated` re-exported at crate root. Scope codegen to types appearing in the public
API surface only — not all wire internals.

Commands:

- `cargo xtask sync-sdk` — regenerates generated modules, leaves staging to developer
- `cargo xtask sync-sdk --commit` — regenerates + commits in one shot

Pre-commit hook behavior: runs `cargo xtask sync-sdk`; if any generated files changed, **aborts
the commit** with a clear message:

```text
🔄 Wire/surface types changed — service-sdk generated types updated:
   crates/shared/service-sdk/src/generated/wire.rs

Commit aborted. Review changes, then:
  git add crates/shared/service-sdk/src/generated/ && git commit
  — or —
  cargo xtask sync-sdk --commit
```

CI check: runs `cargo xtask sync-sdk`, fails if any generated file differs from committed state.
This is the real gate.

### openapi-client

Source of truth: the OpenAPI JSON spec committed to `openapi/spec.json`. Generated from the
controller's utoipa annotations via a dedicated binary target or `--features dump-openapi`
(mechanism TBD — neither exists yet; must be implemented as part of this work). No DB required at
spec-dump time. The canonical spec must be generated with the **full feature set**
(`db-all,oidc,nats,notifications-all,zeroconf`) so all endpoints are included.

Generated output: `crates/shared/openapi-client/src/generated/`

Commands:

- `cargo xtask sync-openapi-client` — regenerates generated modules from `openapi/spec.json`
- `cargo xtask sync-openapi-client --commit` — regenerates + commits

Same pre-commit + CI pattern as service-sdk. CI also runs a separate check: regenerate
`openapi/spec.json` from the controller and diff against committed — fail on any difference.

## Internal Wire Rename

`uptrakit-internal-wire` → `uptrakit-wire` throughout the workspace. Not published — clarity
improvement for internal contributors. Mechanical find/replace across ~30 crates in a single
atomic PR. Also add `uptrakit-surfaces` to `[workspace.dependencies]` in the same PR.

## Versioning Strategy

- `uptrakit-service-sdk` and `uptrakit-openapi-client` already have independent
  `version = "0.0.1"` fields (not `version.workspace`). Only `version` is independent; other
  metadata fields may keep `.workspace = true`.
- `release-plz.toml` already has `[[package]]` entries for both crates with
  `git_release_enable = true`, `git_tag_enable = true`, and the correct per-crate tag/release
  name patterns. No changes to `release-plz.toml` required.
- release-plz tracks them per-crate: commits touching `crates/shared/service-sdk/**` or
  `crates/shared/openapi-client/**` (including `src/generated/`) trigger a version bump for that
  crate only.
- Generated type changes are **breaking changes** unless purely additive. Codegen commits must use
  conventional commit format (`feat!` or `BREAKING CHANGE` footer) so release-plz emits a minor
  bump (not patch) under `0.x`.
- All other workspace crates continue using `version.workspace`.
- Semver policy: `0.x` — breaking changes communicated in changelog. Stable `1.0` deferred.

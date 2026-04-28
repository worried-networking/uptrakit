# Controller Crate Split Design

**Date:** 2026-04-28

## Goals

1. Enable `cargo binstall uptrakit-controller` and
   `cargo binstall uptrakit-controller-standalone` as distinct installable packages.
2. Resolve the deferred binstall problem for `uptrakit-controller-standalone`
   (previously a build artifact with no distinct Cargo package name).
3. Rename `embed-frontend` feature to `embedded-frontend` for consistency with
   other `embedded-*` features.

## Workspace Layout

Current `crates/core/controller/` is renamed to `crates/core/controller-runtime/`
and converted to a library crate. Two new thin binary wrapper crates are created
alongside it:

```text
crates/core/
  controller-runtime/   ← renamed from controller/, lib crate (publish = false)
  controller/           ← new thin binary wrapper
  controller-standalone/ ← new thin binary wrapper
```

## controller-runtime

**Crate type:** `[lib]` — no `[[bin]]` target.

**Package name:** `uptrakit-controller-runtime`

**publish:** `false`

Contains all real implementation: startup phases, embedded service framework, PKI,
database layer, server setup, CLI argument parsing, build script (`build.rs`), and
all feature-gated modules (`scheduler/`, `agent/`, `ssh_agent/`, `mqtt/`,
`embedded_frontend.rs`, `zeroconf.rs`).

Exposes a single public entry point:

```rust
pub fn main() { /* current main() body */ }
```

All feature flags live here. No feature flags are declared in the wrapper crates.

## controller

**Crate type:** `[[bin]]`

**Package name:** `uptrakit-controller`

**Activated features on `controller-runtime`:**

- `embedded-frontend` (renamed from `embed-frontend`)
- `db-sqlite`
- `oidc`
- `zeroconf`
- `interactive`
- `notifications-all`
- `reset-data`
- `dashboard-icons`

No embedded service features (`embedded-scheduler`, `embedded-mqtt`,
`embedded-agent`, `embedded-ssh-agent`).

`src/main.rs`:

```rust
fn main() {
    controller_runtime::main();
}
```

## controller-standalone

**Crate type:** `[[bin]]`

**Package name:** `uptrakit-controller-standalone`

Identical structure to `controller`. Activates all features, including all
embedded service features:

- Everything `controller` activates, plus:
- `embedded-scheduler`
- `embedded-mqtt`
- `embedded-agent`
- `embedded-ssh-agent`

`src/main.rs`:

```rust
fn main() {
    controller_runtime::main();
}
```

## Feature Rename

`embed-frontend` is renamed to `embedded-frontend` in
`controller-runtime/Cargo.toml`. All references across the workspace are updated:

- `controller-runtime/Cargo.toml` — feature declaration and `#[cfg(feature = ...)]`
  guards
- `controller-runtime/build.rs` — feature check
- `controller/Cargo.toml` — activated feature
- `controller-standalone/Cargo.toml` — activated feature
- Any CI scripts or documentation referencing `embed-frontend`

## build.rs

Stays in `controller-runtime/`. The wrapper crates have no `build.rs`.

## Versioning

All three crates use `version.workspace = true`. They are tightly coupled — runtime
API changes always require wrapper bumps, and independent version cadences would add
overhead with no benefit.

## Changelog

`controller-runtime/` inherits the existing `CHANGELOG.md` from the current
`controller/`. It is the single source of truth for release history.

Both binary wrapper crates point to it via `changelog_path` but do not write to it
(`changelog_update = false`). Their GitHub Release bodies are populated from the
runtime's changelog.

## release-plz.toml Changes

Replace the existing `uptrakit-controller` block with:

```toml
[[package]]
name = "uptrakit-controller-runtime"
changelog_update = true
changelog_include = ["frontend"]

[[package]]
name = "uptrakit-controller"
git_release_enable = true
git_tag_enable = true
git_tag_name = "uptrakit-controller-v{{ version }}"
git_release_name = "uptrakit-controller v{{ version }}"
changelog_path = "crates/core/controller-runtime/CHANGELOG.md"
changelog_update = false

[[package]]
name = "uptrakit-controller-standalone"
git_release_enable = true
git_tag_enable = true
git_tag_name = "uptrakit-controller-standalone-v{{ version }}"
git_release_name = "uptrakit-controller-standalone v{{ version }}"
changelog_path = "crates/core/controller-runtime/CHANGELOG.md"
changelog_update = false
```

Key behaviors:

- `controller-runtime` updates `CHANGELOG.md` on every release despite
  `publish = false` — `changelog_update` and `publish` are independent settings
  in release-plz.
- Both binary crates create GitHub Releases; the runtime does not.
- `changelog_path` is workspace-root-relative and verified to work for reading
  release body content from another crate's changelog.

## Workspace Cargo.toml

Add `controller-runtime`, `controller`, and `controller-standalone` to
`[workspace.members]`. Remove `controller` (old path). Any internal workspace
dependency on `uptrakit-controller` is updated to `uptrakit-controller-runtime`.

## Out of Scope

- `[package.metadata.binstall]` configuration — deferred to CI/binstall setup work.
- CI artifact naming changes — deferred.
- Independent version cadences for binary crates — deferred to a separate spec.

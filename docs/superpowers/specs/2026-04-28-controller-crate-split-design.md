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

If the workspace `Cargo.toml` already uses a `"crates/core/*"` glob in
`[workspace.members]`, the new crates are auto-discovered. If it uses explicit
paths, add all three.

## controller-runtime

**Crate type:** `[lib]` — no `[[bin]]` target.

**Package name:** `uptrakit-controller-runtime`

**publish:** `false`

Contains all real implementation: startup phases, embedded service framework, PKI,
database layer, server setup, CLI argument parsing, build script (`build.rs`), and
all feature-gated modules (`scheduler/`, `agent/`, `ssh_agent/`, `mqtt/`,
`embedded_frontend.rs`, `zeroconf.rs`).

The current `src/main.rs` becomes `src/lib.rs`. It exposes a single public entry
point:

```rust
#[doc(hidden)]
pub fn run() -> std::process::ExitCode { /* current main() body */ }
```

Named `run` (not `main`) to avoid confusion with binary entry points.

All feature flags live here. No feature declarations exist in the wrapper crates.

## controller

**Crate type:** `[[bin]]`

**Package name:** `uptrakit-controller`

**Binary output name:** `uptrakit-controller` (Cargo default — single binary crate
uses package name; no explicit `[[bin]]` `name` field needed).

**Cargo.toml dependency on controller-runtime:**

```toml
[dependencies]
uptrakit-controller-runtime = { workspace = true, features = [
  "embedded-frontend",
  "db-sqlite",
  "oidc",
  "zeroconf",
  "interactive",
  "notifications-all",
  "reset-data",
  "dashboard-icons",
] }
```

No embedded service features (`embedded-scheduler`, `embedded-mqtt`,
`embedded-agent`, `embedded-ssh-agent`).

**Breaking change:** `embedded-scheduler` and `embedded-mqtt` are currently in the
`controller` crate's default feature set. This split intentionally drops them from
the lean `controller` binary. Users who relied on the all-in-one behavior should
use `controller-standalone` instead. This is the primary behavioral difference
introduced by the split.

Note: `embedded-agent` and `embedded-ssh-agent` were never in the existing default
feature set — they remain opt-in.

The features `nats`, `journald`, `swagger-ui`, and `db-postgres` are intentionally
omitted from wrapper defaults — they were not in the existing `default = [...]` set
and remain opt-in. Both wrapper crates must declare thin forwarding features for all
features of `controller-runtime` that are not already activated by the wrapper's
dependency spec, so that `cargo build -p uptrakit-controller --features nats` works
correctly instead of silently building without the feature (Cargo ignores unknown
`--features` with only a warning, not an error).

`controller` forwarding features (features not activated by its dep spec):

```toml
[features]
embedded-scheduler = ["uptrakit-controller-runtime/embedded-scheduler"]
embedded-mqtt = ["uptrakit-controller-runtime/embedded-mqtt"]
embedded-agent = ["uptrakit-controller-runtime/embedded-agent"]
embedded-ssh-agent = ["uptrakit-controller-runtime/embedded-ssh-agent"]
nats = ["uptrakit-controller-runtime/nats"]
journald = ["uptrakit-controller-runtime/journald"]
swagger-ui = ["uptrakit-controller-runtime/swagger-ui"]
db-postgres = ["uptrakit-controller-runtime/db-postgres"]
db-all = ["uptrakit-controller-runtime/db-all"]
notifications-telegram = ["uptrakit-controller-runtime/notifications-telegram"]
notifications-email = ["uptrakit-controller-runtime/notifications-email"]
```

`controller-standalone` forwarding features (embedded service features already in
its dep spec; only the remaining opt-ins need forwarding):

```toml
[features]
nats = ["uptrakit-controller-runtime/nats"]
journald = ["uptrakit-controller-runtime/journald"]
swagger-ui = ["uptrakit-controller-runtime/swagger-ui"]
db-postgres = ["uptrakit-controller-runtime/db-postgres"]
db-all = ["uptrakit-controller-runtime/db-all"]
notifications-telegram = ["uptrakit-controller-runtime/notifications-telegram"]
notifications-email = ["uptrakit-controller-runtime/notifications-email"]
```

`src/main.rs`:

```rust
fn main() -> std::process::ExitCode {
    uptrakit_controller_runtime::run()
}
```

## controller-standalone

**Crate type:** `[[bin]]`

**Package name:** `uptrakit-controller-standalone`

**Binary output name:** `uptrakit-controller-standalone`

Identical structure to `controller`. Activates all features on
`controller-runtime`, including all embedded service features:

```toml
[dependencies]
uptrakit-controller-runtime = { workspace = true, features = [
  "embedded-frontend",
  "db-sqlite",
  "oidc",
  "zeroconf",
  "interactive",
  "notifications-all",
  "reset-data",
  "dashboard-icons",
  "embedded-scheduler",
  "embedded-mqtt",
  "embedded-agent",
  "embedded-ssh-agent",
] }
```

`src/main.rs`:

```rust
fn main() -> std::process::ExitCode {
    uptrakit_controller_runtime::run()
}
```

## Feature Rename

`embed-frontend` is renamed to `embedded-frontend` throughout. All affected sites:

- `controller-runtime/Cargo.toml` — feature declaration
  (`embedded-frontend = ["dep:uptrakit-frontend"]`; `uptrakit-frontend` is the dep,
  not `rust-embed` directly — the frontend crate owns the `Assets` struct)
- `controller-runtime/src/lib.rs` — `#[cfg(feature = "embed-frontend")]`
- `controller-runtime/src/server.rs` — `#[cfg(feature = "embed-frontend")]`
- `controller-runtime/src/cli.rs` — `#[cfg(feature = "embed-frontend")]` and any
  doc comments referencing the name
- `controller-runtime/src/startup/validation.rs` — `cfg!()` macro usage
- `controller-runtime/src/embedded_frontend.rs` — module-level doc comment
  (the file imports `Assets` from `uptrakit_frontend`; no `RustEmbed` derive to update)
- `controller/Cargo.toml` — activated feature in dependency spec
- `controller-standalone/Cargo.toml` — activated feature in dependency spec
- `.github/workflows/docker.yml` — passes `embed-frontend` via `--features`
- `.github/workflows/release-plz.yml` — passes `embed-frontend` via `--features`

`controller-runtime/build.rs` does **not** need updating — the frontend build
validation that previously checked `CARGO_FEATURE_EMBED_FRONTEND` now lives in
`frontend/build.rs` (moved during the frontend-crate promotion). The runtime's
`build.rs` only emits `UPTRAKIT_RELEASE_NAME`.

The module file `src/embedded_frontend.rs` is not renamed — the module name is
unrelated to the feature name.

## build.rs

Stays in `controller-runtime/`. The wrapper crates have no `build.rs`. Cargo
propagates build-time environment variables (including `UPTRAKIT_RELEASE_NAME`
set by CI) to all build scripts in the dependency graph, so
`controller-runtime/build.rs` receives them correctly when building either wrapper.

## Versioning

All three crates use `version.workspace = true`. They are tightly coupled — runtime
API changes always require wrapper bumps, and independent version cadences would add
overhead with no benefit.

## Changelog

`controller-runtime/CHANGELOG.md` is the existing `controller/CHANGELOG.md` moved
via `git mv` to preserve git history. It is the single source of truth for release
history.

Both binary wrapper crates point to it via `changelog_path` (workspace-root-relative
per release-plz documentation) but do not write to it (`changelog_update = false`).
Their GitHub Release bodies are populated from the runtime's changelog.

## release-plz.toml Changes

Replace only the existing `[[package]] name = "uptrakit-controller"` block. All other
blocks (`uptrakit-frontend`, `uptrakit-agent-runtime`, etc.) are already present in the
file and must not be touched.

```toml
[[package]]
name = "uptrakit-controller-runtime"
changelog_update = true
changelog_include = [
  "uptrakit-frontend",
  "uptrakit-agent-runtime",
  "uptrakit-agent-ssh-runtime",
  "uptrakit-mqtt-runtime",
  "uptrakit-scheduler-runtime",
]

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

- `controller-runtime` updates `CHANGELOG.md` despite `publish = false` —
  `changelog_update` and `publish` are independent release-plz settings.
- Both binary crates create GitHub Releases; the runtime does not (workspace default
  `git_release_enable = false` applies).
- `changelog_path` values are workspace-root-relative per release-plz documentation.
- `controller-runtime` does NOT use `git_tag_enable = true` (unlike other `*-runtime`
  crates in this repo). Those crates version independently and use tags to propagate
  bumps. These three crates share `version.workspace = true` and always bump together,
  so a separate runtime tag is redundant.
- `changelog_include` mirrors what the old `uptrakit-controller` block had, transferred
  to the runtime. Package names must match exactly — `"uptrakit-frontend"` not `"frontend"`.

## CI Build Commands

Existing CI commands that pass `--features embed-frontend,...` directly to
`-p uptrakit-controller` will break after the split because wrapper crates declare
no features. Specific known breakage points:

- Build steps using `--no-default-features --features embed-frontend,...` on
  `-p uptrakit-controller` — must be replaced with `-p uptrakit-controller` or
  `-p uptrakit-controller-standalone` with no `--features` flag.
- `upload_if_released "uptrakit-controller" "uptrakit-controller-standalone-${TARGET}"`
  — **required change, not deferred**: after split, standalone has its own release tag
  (`uptrakit-controller-standalone-v{{ version }}`); this call must use
  `"uptrakit-controller-standalone"` as the package name argument, otherwise standalone
  binaries are silently not uploaded to releases.
- `select(.package_name == "uptrakit-controller")` gates in `release-plz.yml`
  (frontend build trigger, `check any binary released` filter) — **required change,
  not deferred**: must also include `"uptrakit-controller-standalone"` or be
  refactored to detect either package, otherwise standalone releases ship with no
  binary artifacts and `cargo binstall uptrakit-controller-standalone` fails.
- The feature rename (`embed-frontend` → `embedded-frontend`) and the CI workflow
  updates that reference `embed-frontend` must land in the same commit. If the rename
  lands first, CI silently builds binaries without the embedded frontend (Cargo treats
  unknown `--features` values as a no-op, not an error).

All other CI command updates (artifact naming, cross-compilation flags) are deferred
to the CI/binstall setup work.

After implementing, run `release-plz release-pr --dry-run` to confirm:

- all three crates bump atomically
- wrapper GitHub Release bodies are populated from the runtime changelog (not empty)

## Out of Scope

- `[package.metadata.binstall]` configuration — deferred to CI/binstall setup work.
- CI artifact naming changes — deferred.
- Independent version cadences for binary crates — deferred to a separate spec.

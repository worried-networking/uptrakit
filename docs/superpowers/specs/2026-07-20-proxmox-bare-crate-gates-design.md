# Proxmox Plugin: Fix Bare Per-Crate Test/Clippy Gates — Design

**Date:** 2026-07-20
**Status:** Approved for planning
**Scope:** `crates/plugins/infrastructure/proxmox/Cargo.toml`, `docs/development/dependency-policy.md`

## Problem

`cargo test -p uptrakit-plugin-infrastructure-proxmox` and
`cargo clippy --all-targets -p uptrakit-plugin-infrastructure-proxmox` (no feature flags) fail to
compile on `main` with 15× E0308 + 1× E0599. This is a pre-existing bug, independently confirmed to
reproduce on a clean checkout. It bites every developer or agent who runs the natural per-crate gate
command, and it has already produced one mis-specified "Expected: PASS" gate in a delivered plan
(recorded in the common-mistakes ledger).

No CI gate is red today: every full-workspace build unifies `uptrakit-plugin-infrastructure-proxmox/migrations`
ON via unconditional normal deps (`crates/ui/web-api-queries/Cargo.toml`, `crates/ui/surface-proxy/Cargo.toml`,
`crates/ui/web-api/Cargo.toml` dev-deps). The failure is reachable only in isolated `-p` builds, which
no CI job runs. This fix is therefore developer-ergonomics: it makes the obvious command mean what it
says.

## Root cause (verified empirically this session)

Three interacting facts:

1. **Feature-switched type aliases in infra-core.** `crates/plugins/infrastructure/core/src/descriptor.rs`
   defines `MigrationsFn`, `ResetTenantDataFn`, and `DbMigrateTablesFn` with _different signatures_
   depending on core's `migrations` feature (real fn-pointer types vs `fn()` placeholders). This makes
   the feature non-additive by construction — enabling it changes types.
2. **Mirrored stubs in proxmox keyed on proxmox's own feature.** `reset.rs`, `db_migrate.rs`, and
   `plugin.rs` (`__proxmox_migrations` / `__proxmox_agent_migrations`) pair real impls under
   `#[cfg(feature = "migrations")]` with `fn() {}` stubs under `#[cfg(not(feature = "migrations"))]`,
   wired unconditionally into `declare_plugin!`. The stubs only type-check when _core's_ alias world
   matches _proxmox's_ feature state.
3. **Dev-dep feature unification desynchronizes the two.** Proxmox's dev-dep on infra-core requests
   `features = ["testing", "agent-infra"]`, and core's `agent-infra` implies core's `migrations`. Under
   any test/`--all-targets` build, resolver v3 unifies dev-dep features → core compiles with the _real_
   alias signatures while proxmox's own `migrations` stays OFF → the `fn() {}` stubs mismatch (E0308).
   Bare `cargo check -p` passes because lib-only builds do not unify dev-dep features.

The E0599 is secondary: test code in `surfaces.rs` calls `ProxmoxPlugin::controller_migrations`, which
is gated behind proxmox's `migrations` feature.

**The stub (migrations-OFF) world is load-bearing in production**: `agent-core` and `scheduler-runtime`
consume the registry with its default `daemon` feature only, compiling proxmox _without_ `migrations`
so agent binaries do not carry `sea-orm-migration` or controller migration code. Any fix must leave
that world intact.

## Chosen approach: self dev-dependency enabling the crate's own features

Add one dependency line (plus explanatory comment) to
`crates/plugins/infrastructure/proxmox/Cargo.toml`:

```toml
[dev-dependencies]
# Self dev-dependency (cargo explicitly permits dev-dep cycles onto the package
# itself): enables this crate's own features for its test/bench targets only.
# Required because the dev-dep on infrastructure-core below enables core's
# `agent-infra` -> `migrations`, which switches core's descriptor fn-pointer
# aliases to their real signatures; without the matching proxmox features the
# `#[cfg(not(feature = "migrations"))]` stubs in reset.rs/db_migrate.rs/plugin.rs
# fail to type-check (E0308). Deleting this line re-breaks every bare
# `cargo test|clippy -p uptrakit-plugin-infrastructure-proxmox` invocation.
# Lib-only builds (production consumers, agent binaries, `cargo check -p`) are
# unaffected: resolver v3 does not unify dev-dep features for them.
uptrakit-plugin-infrastructure-proxmox = { workspace = true, features = ["agent-infra", "plugin-ops", "db-sqlite", "testing"] }
```

The manifest comment is a required deliverable, not decoration: the pattern has no precedent in this
workspace and an unexplained self-reference is a prime "cleanup" deletion target.

### Why this feature list

Contrarian-reviewed and measured this session:

- `["migrations", "db-sqlite", "testing"]` (the previously documented "correct command" world) runs
  164 tests but never compiles `pub mod agent` (`#[cfg(feature = "agent-infra")]`, `src/lib.rs`) nor
  the `plugin-ops`-gated `resource_scaling` tests — silently skipping ~15% of the crate's tests. That
  is a milder instance of the green-on-empty false signal this project's ledger bans.
- `["agent-infra", "plugin-ops", "db-sqlite", "testing"]` runs the full suite (193 tests at time of
  writing — treat the count as evidence, not a gate; the gate is "compiles and all suites run") and
  matches clippy-clean. `agent-infra` already implies `migrations` in proxmox's feature graph, so
  `migrations` is redundant and intentionally omitted.

### What changes and what does not

| Build                                                                  | Before             | After                  |
| ---------------------------------------------------------------------- | ------------------ | ---------------------- |
| `cargo test -p uptrakit-plugin-infrastructure-proxmox`                 | E0308 ×15, E0599   | full suite green       |
| `cargo clippy --all-targets -p uptrakit-plugin-infrastructure-proxmox` | same errors        | clean                  |
| `cargo check -p uptrakit-plugin-infrastructure-proxmox` (lib only)     | green (stub world) | unchanged (stub world) |
| Agent/scheduler binary builds (proxmox without `migrations`)           | stub world, green  | unchanged              |
| Full-workspace gates (`--all-features`, minimal db-sqlite set)         | green              | unchanged              |

Known accepted semantics change: `cargo test --no-default-features -p uptrakit-plugin-infrastructure-proxmox`
can no longer exercise a "no-features test world" — dev-dep features are additive and not suppressed
by `--no-default-features`. This is not a regression (that world never compiled; that is the bug), but
the manifest comment and the dependency-policy note must state it: stub-world verification lives in the
agent binary build (`cargo check -p uptrakit-plugin-infrastructure-proxmox` / `cargo build -p uptrakit-agent`),
not in any test target.

## Alternatives considered and rejected

- **Document-the-command only (no code change):** zero blast radius, but leaves the bare command a
  15-error trap and leaves the ledger-recorded mis-gate class open for every future plan touching this
  crate. Rejected in favor of making the obvious command correct.
- **Trim dev-dep features + `#![cfg(feature = "migrations")]`-gate the test modules:** bare
  `cargo test` would compile but run ~0 tests — exactly the green-on-empty anti-pattern the ledger
  bans. Rejected.
- **Make proxmox's normal dep on core always enable `migrations` (drop the duality):** drags
  `sea-orm-migration` + `uptrakit-shared-db/db-migrate` into agent binaries via the registry's
  featureless dep. Violates thin-agent dependency hygiene. Rejected.
- **Root-cause restructure of core's feature-switched aliases into additive, cfg-gated optional
  fields:** the honest fix for the non-additive-feature disease (the aliases violate the spirit of the
  "Feature flags are additive only" invariant), but it touches core's descriptor, `macros.rs`, seven
  catalog struct literals, and every plugin. Disproportionate for a dev-gate bug. **Deferred** (see
  Out of scope).

## Verification (implementation gates)

All commands from repo root; every gate names its feature world explicitly:

1. `cargo test -p uptrakit-plugin-infrastructure-proxmox` — bare, no flags. Expected: compiles, all
   test suites execute and pass (full suite; 193 at time of writing).
2. `cargo clippy --all-targets -p uptrakit-plugin-infrastructure-proxmox` — bare. Expected: clean
   (the workspace-wide `proc-macro-error2` future-incompat _warning_ is pre-existing and out of scope).
3. `cargo check -p uptrakit-plugin-infrastructure-proxmox` — lib-only stub world. Expected: green,
   proving the migrations-OFF lib still compiles.
4. `cargo build -p uptrakit-agent` — proves the production stub-world consumer is unaffected.
5. Canonical workspace gates for a manifest change:
   `cargo clippy --all-targets --no-default-features --features db-sqlite` and
   `cargo clippy --all-targets --all-features` (frontend build required first), `cargo test --all-features`,
   `cargo deny check`.
6. **Release-pipeline probe (named, non-optional):** the self-dev-dep is a first-in-repo shape and the
   release pipeline cannot be validated read-only. With the change applied, run the release-plz cargo
   wrapper's package step (`ci/release-plz/cargo-wrapper.sh` invocation of
   `cargo package --allow-dirty --workspace --no-verify`) and a `release-plz update` dry run; confirm
   both succeed and proxmox's next-version computation is unaffected.
7. Commit `Cargo.lock` if it changes (the lock records per-package dependency edges).

Commit type: `build(proxmox): …` — this is a build-system/dev-gate fix with zero runtime change;
`fix:` would trigger an unwarranted release-plz version bump (the pipeline releases on `feat`/`fix`).

## Documentation deliverables

- **Inline manifest comment** in `crates/plugins/infrastructure/proxmox/Cargo.toml` (shown above) —
  co-located rationale so the precedent-free pattern survives future cleanup.
- **`docs/development/dependency-policy.md`** — add a short subsection under the existing
  dev-dependencies guidance ("If a dep is used only in tests…"): when a crate's _test targets_ require
  the crate's own features (typically because dev-deps force a dependency's features that must stay in
  lockstep), the sanctioned pattern is a self dev-dependency via `workspace = true` with the needed
  features; cite the proxmox manifest as the worked example, and state the `--no-default-features`
  test-target caveat.
- **No ADR:** this is a dependency-policy mechanic, not an architecture decision; the deferred core
  alias restructure would be ADR-worthy if picked up.
- **No AGENTS.md / quality-gates change:** those documents define workspace-level gates, which are
  unchanged; per-crate command behavior is corrected, not redefined.

## Out of scope / deferred

- Root-cause restructure of infra-core's feature-switched type aliases (`MigrationsFn`,
  `ResetTenantDataFn`, `DbMigrateTablesFn`, and the `agent-infra`/`plugin-ops` siblings in
  `descriptor.rs`) into additive optional fields — deferred debt; would also allow deleting the
  `#[cfg(not(feature = "migrations"))]` stub sites in proxmox.
- The pre-existing `#[cfg(not(feature))]` sites themselves: they remain load-bearing for thin agent
  builds and are only removable via the deferred restructure.
- Repo-wide audit of other crates whose bare `-p` gates may be similarly broken by dev-dep feature
  unification (only proxmox is confirmed).
- The standards-snapshot staleness-script false positive (collapsed source-path notation) — session
  tooling housekeeping, not project code; handled outside this spec.

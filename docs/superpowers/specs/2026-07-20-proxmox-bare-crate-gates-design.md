# Proxmox Plugin: Fix Bare Per-Crate Test/Clippy Gates — Design

**Date:** 2026-07-20
**Status:** Approved (user-approved after a 3-round adversarial generator–critic review + independent verification pass)
**Scope:** `crates/plugins/infrastructure/proxmox/Cargo.toml`, `.github/workflows/ci.yml`,
`ci/verify_no_new_cfg_not_feature.sh` (new) + allowlist, `.husky/pre-push`,
`docs/development/dependency-policy.md`, `docs/development/quality-gates.md`,
`docs/development/coding-standards.md`, `AGENTS.md` (quick-start gate list)

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
not in any test target. Stated plainly as a coverage trade (contrarian-surfaced): proxmox's test
targets can now ONLY run in the maximal feature world — per-feature-subset test execution
(cargo-hack-style) is structurally impossible for this crate until the deferred infra-core
restructure makes its features additive; until then, feature-subset verification is compile-only
(lib checks + agent binary build).

## Long-term prevention (in scope; approved via adversarial review)

The self-dev-dep alone fixes the instance. Two CI gates prevent the class, each in the job that
matches its shape: Gate A (cargo-compiling) joins `backend-lint` (no new job — avoids a `rust-cache`
shared-key save race; `backend-lint` is the sole writer of the `rust-all-features` key, `backend-test`
already sets `save-if: "false"` for exactly this reason). Gate B (pure grep script) joins the
`semantic-boundary` job, which already installs ripgrep and runs the sibling `verify_*.sh` scripts —
NOT `backend-lint`, which has no ripgrep step and never runs script gates.

### Gate A — bare-crate clippy sweep over `crates/plugins/**`

One `cargo clippy --all-targets -p <crate>` per plugin crate, added to `backend-lint` (precedent: the
package-isolation `cargo check -p uptrakit-controller-runtime …` step there, which carries its own
justifying comment). The crate list MUST be derived dynamically — a shell loop over the two-level
plugin layout (`crates/plugins/*/*/Cargo.toml` glob, reading each `name` field) — never a
hand-maintained enumeration, so a future plugin crate is covered the day it lands. Single
command per crate: clippy `--all-targets` type-checks lib+tests+benches, so it surfaces both the E0308
alias-desync shape and the E0599 shape (test code calling own-feature-gated items — a shape no
`cfg(not)` grep can predict). Verified: no other plugin crate currently fails this gate (structural
check of all plugin crates + empirical bare-clippy spot-check of registry and skills, both clean).
Contingency if the pre-wiring end-to-end sweep (verification step 8) reddens a crate the spot-checks
missed: fix that crate in-scope (same self-dev-dep pattern if it has the desync shape, or the direct
compile fix) — Gate A deliberately has NO per-crate skip/allowlist; an all-green sweep is the
precondition for wiring it, not something to except around.
Cost note for the implementing PR: the sweep shares `backend-lint`'s target dir but bare-default
resolution differs from `--all-features`, so it reduces — not eliminates — recompilation; measure CI
minutes in the PR rather than trusting estimates. Explicit cost decision rule (not a silent
every-PR default): if the measured sweep adds more than ~2 minutes to `backend-lint`, demote it —
path-filter to PRs touching `crates/**`, or move the full sweep to a nightly/merge-gate job keeping
only touched-crate checks per PR. The sweep also grows linearly with plugin count by design; revisit
placement when it does.

Rejected for this role: `cargo hack --each-feature` — dev-dependency features activate in every
test-mode leg, so post-fix all legs resolve identically (moot), and in `check` mode dev-deps never
activate (structurally cannot catch the class). A full feature powerset across the workspace is pure
cost.

### Gate B — attribute-grep gate on negated feature cfgs

New `ci/verify_no_new_cfg_not_feature.sh` + checked-in allowlist, mirroring the
`ci/verify_no_security_audit.sh` + `_allowlist.txt` pattern (`rule|path|regex` format), wired into
both the `semantic-boundary` CI job and `.husky/pre-push` alongside its three `verify_*.sh` siblings
(all three run in both places; a gate that only fires in CI breaks the local-first pattern).

Match shape — the gate must catch a _negated feature test anywhere inside a `cfg` attribute_, not
just the leading `cfg(not(` spelling. Three real shapes exist in-tree today:
`#[cfg(not(feature = "x"))]`, `#[cfg(not(any(feature = "a", feature = "b")))]`
(`controller-runtime/src/db/mod.rs`), and `#[cfg(all(feature = "a", not(feature = "b")))]`
(`proxmox/src/plugin.rs` middle branch — the same paired-stub risk class this gate exists for; a
bare `cfg(not(`-only anchor is blind to it). Prescribed detection: slurp-mode matching per the
in-repo precedent `ci/verify_handler_state_contract.sh` (`perl -0777` handles attributes rustfmt
wraps across lines — long `any(...)` lists), with the pattern
`^\s*#!?\[cfg\([^\]]*not\s*\(\s*(any\s*\(\s*)?feature` under the `/gsm` flags. Load-bearing pieces,
each empirically verified this review: the `^\s*` line-start anchor with `/m` is what excludes the
two prose mentions (`crates/shared/audit-log/src/emitter.rs` doc-comment,
`crates/core/agent-ssh-runtime/src/client.rs` inline comment — the unanchored form of this same
pattern false-positives on BOTH, so the anchor is not optional); the `#!?` alternation also covers
the inner-attribute form `#![cfg(...)]` (an established module-gating idiom here; no negated inner
attribute exists today, but the gate must not be blind to one). Verified behavior of the anchored
pattern: 0 matches in `emitter.rs`, exactly the two real attributes in `client.rs`, and all three
in-tree shapes matched. Still prove both directions at implementation time: RED on a temporary
unlisted addition of each of the three shapes, silent on the prose mentions.

Allowlist keying: follow `verify_no_security_audit.sh`'s `is_allowlisted()` semantics — entries match
on (rule, path, text-regex); line numbers are display-only, so slurp-mode matching (which has byte
offsets, not line numbers) composes with the allowlist format; derive display line numbers by
counting newlines up to the match offset, or omit them.

Seed the allowlist from a fresh run of the final pattern at implementation time — the widened shape
finds MORE than the 24 top-level `cfg(not(` attributes counted earlier (the `cfg(all(…, not(…)))`
sites were invisible to that count); the checked-in allowlist file is the source of truth and prose
must never restate its counts. Carve out `crates/core/controller-runtime/src/db/mod.rs` as a distinct
permanently-allowed category: its `cfg(not(any(feature…)))` guards a `compile_error!` ("pick a DB
backend"), a benign different class from feature-switched signatures.

Allowlist semantics — a ratchet, not an exception pathway: it grandfathers the existing sites so the
prohibition (until now unenforced prose in coding-standards) becomes CI-enforced for all new code;
entries are expected only to be removed (each deletion via the deferred restructure below), and adding
one requires explicit maintainer sign-off in review with an in-file justification comment. The gate
freezes the violation count; full conformance is owned by the deferred restructure, not by this spec.

Why Gate B is not redundant with Gate A (contrarian-tested): Gate A compiles only `crates/plugins/**`
— the five crates with negated-feature cfgs outside it (agent-runtime, agent-ssh-runtime,
service-sdk, web-api, controller-runtime) are Gate B's only coverage; Gate B also catches a new
type-switching site that happens to compile green in every CI feature world (this bug was exactly
that for months); and its cost is a sub-second grep. Rare benign uses exist (the `compile_error!`
backend guard is one — hence its permanent carve-out), so friction lands on a population that is
almost entirely prohibited code plus the occasional carve-out-worthy guard, each a deliberate
review decision. Retirement coupling: Gate B and the allowlist exist because the restructure is
deferred — the restructure spec, when picked up, must shrink the allowlist toward the carve-out-only
state and then decide whether Gate B survives as a cheap tripwire or retires.

### Policy rule (with the CI gates, completes prevention)

`docs/development/dependency-policy.md` gains a requirement — framed as a REQUIRED INTERIM
MITIGATION with an explicit expiry condition ("retire this rule and the proxmox self-dev-dep when
the infra-core feature-switched aliases become additive; see the deferred restructure spec"), never
as a standing pattern to reach for by default — the compliant long-term answer to feature desync is
additive features, not more self-dev-deps. The rule: a self dev-dependency is REQUIRED whenever a
crate (a) carries `#[cfg(not(feature))]` fallback code whose
shape depends on a shared dependency's feature state, and (b) any dev-dependency can transitively
force that foreign feature on. State explicitly that this rule targets the type-switching/fallback
shape only; Gate A's bare-crate sweep is the catch-all for the E0599-only shape (feature-gated test
code with no fallback involved). Thin-binary constraint, stated precisely: `uptrakit-agent` is the
binary that must stay free of `sea-orm-migration` (no registry/migration dependency at all);
`uptrakit-agent-ssh` already carries it unconditionally via `agent-ssh-runtime` — policy text must not
imply otherwise.

## Alternatives considered and rejected

- **Document-the-command only (no code change):** zero blast radius, but leaves the bare command a
  15-error trap and leaves the ledger-recorded mis-gate class open for every future plan touching this
  crate. Rejected in favor of making the obvious command correct — BUT retained as the named fallback:
  if verification step 6 (release-plz probe) comes back red against the self-dev-dep, ship the doc-only
  stance plus Gates A/B (which are independent of the manifest change) instead of forcing the novel
  mechanism through.
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
   both succeed and proxmox's next-version computation is unaffected. Honest scope of this probe: it
   validates release-plz's dependency-graph walk and tarball _emission_; `--no-verify` deliberately
   skips the extract-and-rebuild step — the one path that would resolve the self-dev-dep from a
   packaged tarball. Acceptable residual: the pipeline is `git_only` (crates never published or
   rebuilt from tarballs), so that unexercised path has no consumer.
7. Commit `Cargo.lock` if it changes (the lock records per-package dependency edges).
8. For the CI additions: `actionlint` on `.github/workflows/ci.yml` (already in pre-commit for staged
   workflow files); run `ci/verify_no_new_cfg_not_feature.sh` locally — expected: exit 0 on main with
   the seeded allowlist, non-zero for a temporary unlisted addition of EACH of the three in-tree
   shapes (`cfg(not(feature))`, `cfg(not(any(feature…)))`, `cfg(all(…, not(feature…)))`) — prove the
   RED cases, not just the green; run the full `.husky/pre-push` once (the script joins its siblings
   there); run the Gate-A sweep loop locally once end-to-end to confirm every plugin crate is clean
   before wiring it into CI.

Commit type: `build(proxmox): …` — this is a build-system/dev-gate fix with zero runtime change;
`fix:` would trigger an unwarranted release-plz version bump (the pipeline releases on `feat`/`fix`).

## Documentation deliverables

- **Inline manifest comment** in `crates/plugins/infrastructure/proxmox/Cargo.toml` (shown above) —
  co-located rationale so the precedent-free pattern survives future cleanup.
- **`docs/development/dependency-policy.md`** — add a subsection under the existing dev-dependencies
  guidance ("If a dep is used only in tests…") carrying the REQUIRED self-dev-dep rule from
  "Long-term prevention" above: trigger conditions (a)+(b), the proxmox manifest as worked example,
  the `--no-default-features` test-target caveat, and the cross-reference that Gate A covers the
  E0599-only shape the rule does not.
- **`docs/development/quality-gates.md` + AGENTS.md quick-start block** — add
  `bash ci/verify_no_new_cfg_not_feature.sh` to the canonical gate list (quality-gates.md is the
  canonical source; AGENTS.md quick-start updates in the same commit per its own maintenance rule).
- **`docs/development/coding-standards.md#feature-flags`** — the substantive home of the additive-only
  rule: one line noting it is now CI-enforced by the new script, with the allowlist as the
  grandfathered ratchet (`feature-flags.md` only cross-references this section; do not put the note
  there).
- **`.husky/pre-push`** — add the new script invocation alongside its three `verify_*.sh` siblings.
- **No ADR for this spec:** the self-dev-dep + CI gates are dependency-policy/CI mechanics. The
  deferred core alias restructure (below) IS ADR-worthy and gets its own spec + ADR when picked up.

## Out of scope / deferred

- **Root-cause restructure of infra-core's feature-switched signatures — deferred to its own
  spec + ADR**, with these adversarially-reviewed design constraints as the starting point:
  - `MigrationsFn` is fully solvable via an always-defined erasure trait: `trait PluginMigration:
Send` with a `#[cfg(feature = "migrations")]` `into_sea_orm()` method,
    `type MigrationsFn = fn() -> Vec<Box<dyn PluginMigration>>` (never switches), adapter
    `impl PluginMigration for Box<dyn sea_orm_migration::MigrationTrait>` matching the
    already-erased vec proxmox returns. The `Send` bound is CONFIRMED against the pinned dep:
    `sea-orm-migration 2.0.0-rc.41` declares `trait MigrationTrait: MigrationName + Send + Sync`.
  - `ResetTenantDataFn`/`DbMigrateTablesFn` and the LIVE `InfraSlot`/`InfraBundle` family are NOT
    erasure-compatible — their signatures embed concrete `sea_orm::DatabaseTransaction`/
    `DatabaseConnection` types (`descriptor.rs`, `roles.rs` `on_plugin_config_reported`/
    `has_infra_state`). Erasing those means "no direct sea_orm types in any shared descriptor
    signature" — a materially larger project the ADR must scope honestly. Mitigation until then:
    Gate B, plus the fact that `agent-infra` co-activates `migrations` in both core and proxmox, so
    the Phase-0 self-dev-dep already shields the InfraSlot family from this exact desync.
  - The proxmox `#[cfg(not(feature = "migrations"))]` stub sites remain load-bearing for thin
    `uptrakit-agent` builds and are only removable via this restructure.
- Repo-wide audit of non-plugin crates' bare `-p` gates (Gate A covers `crates/plugins/**`; the 5
  crates with negated-feature cfg sites outside it — agent-runtime, agent-ssh-runtime, service-sdk,
  web-api, controller-runtime — are not swept and only proxmox was confirmed broken).
- The standards-snapshot staleness-script false positive (collapsed source-path notation) — session
  tooling housekeeping, not project code; handled outside this spec.

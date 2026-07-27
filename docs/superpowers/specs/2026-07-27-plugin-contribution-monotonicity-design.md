# Plugin Contribution Monotonicity — Feature-Unification-Proof Descriptor Contributions

**Date:** 2026-07-27 **Status:** Approved for planning

## Problem

The same defect class has shipped twice in `crates/plugins/infrastructure/proxmox/src/plugin.rs`,
three months apart:

- **2026-04-25, commit `800975bea`** — `controller_update_protection` was gated on
  `#[cfg(not(feature = "agent-infra"))]`. Cargo feature unification enabled `agent-infra` when the
  standalone controller built with `embedded-ssh-agent`, so the shipped binary lost update
  protection. Fixed by deleting the gate; no rule or gate shipped with the fix.
- **2026-07 (current)** — `descriptor_plugin_surfaces()` (`plugin.rs:64-69`) returns `vec![]` when
  `cfg!(feature = "agent-infra")` is true. The unification chain
  `controller-standalone` (`embedded-ssh-agent`) → `controller-runtime` → `agent-ssh-runtime`
  (registry with `agent-infra`) → registry → proxmox enables the feature in the shipped standalone
  controller, so `proxmox.hosts` and every other proxmox surface never registers there. The lean
  `uptrakit-controller` binary is unaffected, which made the loss read as intermittent.

Both are one class: **a plugin expressed "this contribution is controller-only" as a compile-time
feature predicate, in a workspace where features unify across members.** A crate cannot observe
which member enabled its feature, so the producing crate's local reasoning is meaningless at the
final link. The existing regex gate (`ci/verify_no_new_cfg_not_feature.sh`) bans only the
`#[cfg(not(feature = ...))]` attribute form; the second occurrence used the _permitted_ `cfg!()`
expression form with subtractive semantics and sailed through.

Aggravators, all verified:

- Proxmox's self dev-dep (`proxmox/Cargo.toml`, `features = ["agent-infra", ...]`) forces
  `agent-infra` ON for the crate's own tests, so its tests exercised the empty branch for their
  entire life. A test (`plugin.rs`, `unified_registrations_pair_every_interaction_with_plugin_handled_delivery`)
  explicitly asserts `descriptor_plugin_surfaces().is_empty()` under `agent-infra` — the bug is
  codified as expected behavior.
- The existing catalog guard (`crates/plugins/infrastructure/registry/tests/surface_id_naming_guard.rs:6-10`)
  documents that it skips proxmox rows "when its registrations are populated in the compiled
  feature set" — an honest punt that leaves exactly this hole.

## Decision 1 — the field split is the process-scope model; no new type

**No `ProcessRole`/`ContributionSite` enum is added.** `PluginDescriptor` already encodes process
scope as data selected by the consumer:

| Field                                                                     | Sole production consumer                                                                                                                             |
| ------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------- |
| `surfaces` (`descriptor.rs:563`)                                          | controller boot — `crates/core/controller-runtime/src/boot/components.rs:277` via `PluginSurfaceOps::surface_registrations()` (`catalog.rs:440-451`) |
| `agent_surfaces` (`descriptor.rs:582`)                                    | agent runtime — `crates/core/agent-ssh-runtime/src/surface_runtime.rs:192-197`                                                                       |
| `migrations`                                                              | controller-side migrator                                                                                                                             |
| `agent_migrations`                                                        | agent-ssh runtime migrator                                                                                                                           |
| role slots' `HostRequirements` (e.g. `HostRequirements::CONTROLLER_ONLY`) | dispatch-time compatibility check                                                                                                                    |

The agent runtime never calls `surface_registrations()`; a populated `surfaces` field in an
agent-only build is inert data. The merged (embedded) process consumes both sides of each split
once — no double registration, no conflict. The `cfg!` gate also saves zero binary size: it is a
runtime branch, both arms compile and link regardless.

**The defect is therefore "producer second-guessing the consumer."** The deliverable is an
invariant, enforcement, and the exemplar fix — not new descriptor machinery.

## Decision 2 — the contribution-monotonicity invariant

> **Contribution monotonicity:** enabling a Cargo feature may only **add** descriptor
> contributions; it must never remove or alter contributions that exist without it. Descriptor and
> registration builder functions must produce the same-or-more data for every superset of enabled
> features.

The line between legitimate and banned feature gating:

- **Legitimate — "this code does not exist without the feature."** `#[cfg(feature = "X")]` on
  modules, dependencies, and items whose code cannot compile without the feature. Registration
  stays monotonic via the two established shapes, both present in `plugin.rs` today:
  - additive chain: `std::iter::empty().chain(#[cfg(feature = "agent-infra")] ...)`
    (`proxmox_agent_surfaces`, `plugin.rs:1338-1343`);
  - paired stubs: a `#[cfg(feature = "X")]` real fn plus a `#[cfg(not(feature = "X"))]` stub
    returning empty, when the fn body cannot compile without the feature
    (`__proxmox_agent_migrations`, `plugin.rs:1319-1334`). These stubs are monotone (the feature
    only ever adds data) and stay grandfathered in the existing regex-gate allowlist. Each
    permitted stub carries an inline comment naming why the split exists (approval co-located with
    the code, not a detached list).
- **Banned — "this data is suppressed although the code exists."** Any branch — `cfg!(feature)`,
  `if !cfg!`, let-bound predicates, helper indirection — that returns _less_ registration data when
  a feature is ON. All spellings of the shape are banned by rule; enforcement is behavioral
  (Decision 3), not textual, precisely because the spellings are unbounded.

## Decision 3 — enforcement: three behavioral layers, regex gate untouched

`ci/verify_no_new_cfg_not_feature.sh` keeps its single existing rule unchanged. No regex can
distinguish subtractive from additive `cfg!` use; the class is enforced where it manifests — the
compiled catalog.

### Layer C — unconditional critical-surface presence (drift-free instance killer)

New guard test `crates/plugins/infrastructure/registry/tests/contribution_monotonicity_guard.rs`:

- Assert the catalog produced by `all_descriptors()` contains a surface with
  `surface.descriptor.surface_id.as_str() == "proxmox.hosts"` **unconditionally** — proxmox is a
  mandatory (non-optional, non-cfg-gated) registry dependency, so the assertion is valid in every
  feature configuration. This single assertion catches **both** historical shapes: the 2026-04
  whole-descriptor suppression (`surfaces: None`) and the 2026-07 empty-registrations suppression
  both manifest as "`proxmox.hosts` absent". Same load-bearing-lane note as Layer A: the assertion
  is _valid_ in every lane, but it only _catches the bug_ in an `agent-infra`-ON build — in the
  default lane nothing is suppressed. The catching lane is `cargo test --all-features`.
- Update `surface_id_naming_guard.rs`: replace the "checked only when populated" punt comment with
  a pointer to the new guard. The unconditional presence assertion lives **only** in
  `contribution_monotonicity_guard.rs` — one assertion site, not two files restating the same fact
  (the same drift argument that killed the manifest alternative). Feature-scoped `saw_*`
  assertions for genuinely optional plugins stay where they are.

### Layer A — declaring ⇒ non-empty (list-free class check for the populated-but-empty shape)

Same guard file: for every descriptor where `desc.surfaces` is `Some(ops)`, assert
`(ops.registrations)()` is non-empty **and** each `PluginSurfaceRegistration.surfaces` is
non-empty. Declaring the ops block while yielding nothing is always a defect (a plugin with no
surfaces omits the block).

**Guarantee scope, stated honestly (per the vacuous-guard lesson):** Layer A is meaningful in an
`agent-infra`-ON build; in the default lane it passes trivially because nothing is suppressed. The
load-bearing lane is the existing canonical workspace gate `cargo test --all-features`
(quality-gates.md), which unifies `agent-infra` ON — the exact shape where the bug lives.

**RED command** (TDD sequencing: the guard file is itself a deliverable of this spec — write it
first, run against the still-buggy proxmox code to observe the assertion failure, then apply the
exemplar fix and observe green):

```sh
cargo test -p uptrakit-plugin-infrastructure-registry --features agent-infra \
  --test contribution_monotonicity_guard
```

### Layer B — cross-build superset diff (class closure: content thinning, future plugins, any axis)

New gate `cargo xtask contribution-monotonicity-check`. The xtask umbrella and CI/pre-push wiring
follow the `audit-coverage-check`/`openapi-client-check` pattern, but the execution model is
genuinely new plumbing: the existing xtask gates are static analyses over checked-in artifacts,
while this gate must **execute compiled code** (the catalog is runtime-computed by
`declare_plugin!`) and therefore orchestrates two nested cargo builds itself — no in-tree
precedent, and the build cost is owned explicitly below.

**Sequencing: Layer B is severable insurance, not part of the merge-blocking bar.** Layers A + C
plus the exemplar fix and docs fully cover both twice-shipped bug shapes; Layer B's marginal
coverage is future plugins with no dedicated assertion and content thinning — neither has yet
occurred. Layer B stays a committed deliverable of this spec (per the scoping decision), but it is
planned as its own phase/plan that can land after A + C, and may be downgraded (e.g. CI-only, or
nightly) if its build cost lands badly — without touching the A + C bar.

1. **Dump entry point (new plumbing, named deliverable):** an example binary in the registry crate
   (`crates/plugins/infrastructure/registry/examples/dump_contributions.rs`) that walks
   `all_descriptors()` and prints one JSON document to stdout: per `type_id` — surface IDs, per
   surface interaction `(id, effective_http_method)` pairs and data-source IDs; agent-interaction
   IDs from `agent_surfaces`; migration names from `migrations`/`agent_migrations` (via
   `MigrationName::name()`). Plus a metadata header: the build's **feature fingerprint** — a map
   over every feature the registry crate declares, each entry a literal
   `cfg!(feature = "...")` bool. Target choice: an _example_ (first in the workspace) rather than
   a `[[bin]]` (would need `required-features` or break bare `cargo build -p`) or a `#[test]`
   (libtest harness noise interleaves with the JSON payload); examples are not built by default
   `cargo build`, so the target is inert outside the gate.
2. **Two builds, the two shipped controller shapes:**
   - baseline = lean controller: `cargo run -p uptrakit-plugin-infrastructure-registry
--example dump_contributions --no-default-features --features <baseline list>`. **Derivation
     mechanism is pinned here, not deferred — it is the correctness pivot of the whole layer.**
     The list is derived from **package-scoped** resolution:
     `cargo tree -p uptrakit-controller -e features`, which correctly omits `agent-infra`.
     Deriving it from unscoped `cargo metadata` is **forbidden**: `cargo metadata` reports the
     workspace-unified resolve, where `agent-infra` is ON — that baseline would silently equal the
     union and make the diff vacuous forever (verified against this workspace; the two tools give
     opposite answers). The list is derived **once and pinned** in the xtask (deterministic; avoids
     parsing `cargo tree`'s indent-tree text on every run), with staleness guarded twice: the
     fingerprint-key exhaustiveness diff (step 4) fails loud when the registry gains a feature,
     forcing a conscious re-pin decision, and the pinned expected fingerprint map is hand-authored,
     independent of any derivation tooling. Current derived set includes at least `plugin-ops`,
     `migrations`, the `notifications` family, and `dashboard-icons` — and not `agent-infra`.
     `--no-default-features` is always passed so registry's `default` cannot drift in unnoticed.
     Note the deliberate asymmetry: the derivation describes registry-inside-lean-controller, but
     the build applies it to registry-as-root (the example target); residual dev-dep unification
     differences between those two shapes are exactly what the fingerprint exact-match catches.
     Registry `[dev-dependencies]` were checked at spec time and pull no `agent-infra`;
   - union: same command with `--all-features`.
3. **Compare in xtask:** for every plugin and every contribution field present in the baseline,
   the union set must be a **superset**. Any feature that subtracts _anything_ — a whole
   descriptor, a surface, a single interaction, a migration — fails the gate, for every plugin
   including ones added later with no dedicated assertion. Plugins present only in the union
   (optionally compiled, e.g. notification plugins) are additive and pass by construction.
4. **Self-verifying lanes:** xtask compares the baseline dump's feature fingerprint for **exact
   equality** against the pinned expected map (not just `agent_infra: false` — a future registry
   dev-dependency silently widening the baseline with any enumerated feature trips the gate), and
   asserts the union fingerprint is all-`true`. If dev-dep or example-target feature unification
   ever shifts either lane, the gate fails loud instead of self-punting — closing the "guarantee
   scope quietly narrows to the compiled feature set" trap that got the last two guards. The
   fingerprint map itself is **not** allowed to become an unenforced `Cargo.toml` mirror: the xtask
   diffs the map's keys against the registry package's `features` table from `cargo metadata` and
   fails loud on any missing or extra key, so adding a registry feature without extending the
   fingerprint is itself a gate failure. Additionally, a **non-vacuity canary**: the xtask asserts
   the baseline dump is a _proper_ subset of the union (at least one known union-only contribution
   present — e.g. the `agent-infra`-gated proxmox agent interactions). This closes the residual
   axis the fingerprint cannot see: inflation of the baseline via a transitive/dev-dep feature
   that is not a registry-declared feature could otherwise silently collapse baseline == union and
   make the diff pass vacuously forever.

CI wiring: new step in `ci.yml` and a `docs/development/quality-gates.md` entry. Honest cost
accounting: these are two **full** dependency-graph builds, and because Cargo's build cache is
feature-fingerprint-keyed, alternating the two feature sets against a target dir shared with other
gates thrashes rather than reuses the cache — the plan must place the step where the union build
can reuse the `rust-all-features` cache and give the baseline build its own cache key (or a
dedicated `--target-dir`) instead of assuming "two quick builds". Pre-push inclusion decided at
plan time against that real cost; default expectation is CI-only.

Error handling: xtask check follows the existing xtask error conventions; the example binary
returns `Result` and reports via stderr + non-zero exit (workspace lints deny
`unwrap`/`expect`/`panic!` in examples too).

## Decision 4 — exemplar fix and workspace sweep

1. **Proxmox fix:** delete `descriptor_plugin_surfaces()` (`plugin.rs:64-69`); wire
   `proxmox_plugin_surfaces` directly into `declare_plugin!(surfaces: { registrations: ... })`.
2. **Fix the codifying test:** `unified_registrations_pair_every_interaction_with_plugin_handled_delivery`
   loses its `if cfg!(feature = "agent-infra") { assert empty; return; }` head and asserts the real
   pairing content unconditionally; sibling content tests keep calling the raw builder.
3. **Workspace sweep (one-time audit task):** grep every plugin crate's **and**
   `crates/plugins/infrastructure/core`'s (the framework crate's alias/placeholder stubs are
   in scope, not just crates that are themselves plugins) descriptor/registration builder paths
   for `cfg!` and `#[cfg]` use; classify each site as
   (a) subtractive — fix like proxmox, or (b) monotone additive-chain/paired-stub — keep, ensure
   the inline comment naming the reason exists. Known classifications from spec-time reading:
   `descriptor_plugin_surfaces` (a); the `plugin.rs:1941` test head (a);
   `__proxmox_agent_migrations` / `__proxmox_migrations` / `proxmox_agent_surfaces` (b). The sweep
   result table lands in the implementation plan, produced by running the grep at plan time (no
   counts asserted here).

### How a plugin author tests process-scoped contributions (answers brief Q4)

- **Content** is tested in the plugin crate against the raw builder functions
  (`proxmox_plugin_surfaces()`-style), which are feature-independent by the invariant.
- **Presence under unification** is _never_ testable from `cargo test -p <plugin>` (the plugin
  cannot see other members' feature choices; proxmox's self dev-dep even pins the union shape).
  It belongs to the registry guards (Layers A/C) and the cross-build diff (Layer B). The plugin
  authoring guide states this split explicitly.
- The proxmox self-dev-dep restructure (making infra-core's feature-switched fn-pointer aliases
  additive so bare `cargo test -p` works without forcing `agent-infra`) remains a separate
  deferred spec, cross-referenced from the ADR; this spec does not depend on it.

## Documentation deliverables

| File                                                           | Change                                                                                                                                                                                                         |
| -------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `docs/adr/0032-plugin-contribution-monotonicity.md`            | new ADR: invariant, model decision, enforcement layers, permitted/banned shapes                                                                                                                                |
| `docs/development/coding-standards.md` (Feature Flags section) | contribution-monotonicity rule, the legitimate/banned line, both permitted shapes with the proxmox examples                                                                                                    |
| `AGENTS.md` (root)                                             | extend the "**Feature flags are additive only.**" rule by one clause: descriptor contributions are feature-monotonic; link ADR-0032 (stays within size budget)                                                 |
| `crates/plugins/AGENTS.md`                                     | authoring rule: registration builders are predicate-free; how to express feature-gated contributions; where presence is tested                                                                                 |
| `docs/development/feature-flags.md`                            | cross-reference from the additive-only note to ADR-0032                                                                                                                                                        |
| `docs/development/plugin-system.md`                            | note on descriptor contribution fields and consumer-side selection                                                                                                                                             |
| `docs/development/surfaces.md`                                 | update the catalog-guard paragraph (new guard file, removed punt)                                                                                                                                              |
| `docs/development/quality-gates.md`                            | add `cargo xtask contribution-monotonicity-check` (note: the file lists no xtask gate today — this is its first xtask row; backfilling the two existing xtask gates there is a nice-to-have, not a dependency) |
| `CONTEXT.md`                                                   | glossary entry: **Contribution Monotonicity**                                                                                                                                                                  |

No new external dependencies (JSON via existing workspace `serde_json`; xtask uses existing
utilities), so no version pins are introduced.

## Alternatives considered

- **`ProcessRole`/`ContributionSite` enum on the descriptor** — rejected: the field split already
  carries process scope as consumer-selected data; a parallel enum would be a second source of
  truth that can disagree with the fields.
- **Generalizing the regex CI script to subtractive `cfg!`** — rejected: the expression form's
  spellings are unbounded (inverted predicates, let-bindings, helpers); a text gate catches the
  committed shape, not the shape class. The script keeps its one job.
- **Per-plugin expected-contributions manifest in the guard test** — rejected after contrarian
  review: it is the mirror-table drift pattern root `AGENTS.md` explicitly bans, it is opt-in (new
  plugins unprotected), and it cannot see content thinning without becoming a full hand-maintained
  catalog copy. Layer C keeps only the drift-free slice (a handful of unconditional critical-surface
  assertions); Layer B covers the generic slice list-free.
- **Runtime boot-time assertion in the controller** — rejected as primary enforcement: fails at
  deploy time per installation, not at merge time. Not included; may be revisited as a canary
  independently.

## Non-goals

- Plugin identity/naming (separate spec, ADR-0031 series).
- Surface visibility and authorization (separate spec).
- `declare_plugin!` redesign.
- Proxmox self-dev-dep / infra-core fn-pointer-alias restructure (existing deferred spec).
- Retiring the existing regex gate or its allowlist ratchet.

## Success criteria

Phase boundary (mirrors Decision 3's severability): criteria 1, 4, 5 define the **merge-blocking
phase** (Layers A + C, exemplar fix, sweep, docs, ADR); criteria 2, 3 belong to the **Layer B
phase**, planned separately.

1. The RED command above, run with the new guard file in place against the still-unfixed proxmox
   code, fails on the assertions; it passes after the exemplar fix.
2. `cargo xtask contribution-monotonicity-check` fails if any plugin's contribution set under
   `--all-features` is not a superset of its baseline set — including plugins and features that do
   not exist yet (class-level, not instance-level).
3. Both lane fingerprints are asserted by the gate itself; a mis-featured lane fails loud.
4. A plugin author reading `crates/plugins/AGENTS.md` finds exactly one documented way to express
   "controller-only": populate the controller-consumed field and let the consumer select — with the
   two permitted shapes for genuinely feature-gated code.
5. All existing quality gates stay green (`cargo fmt/check/clippy` in both canonical feature sets,
   `cargo test --all-features`, markdownlint).

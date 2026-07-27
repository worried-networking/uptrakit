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
  both manifest as "`proxmox.hosts` absent".
- Strengthen `surface_id_naming_guard.rs`: replace the "checked only when populated" punt comment
  and add the same unconditional proxmox presence expectation, mirroring its existing
  `saw_notifications_email` pattern (feature-scoped assertions stay for genuinely optional
  plugins).

### Layer A — declaring ⇒ non-empty (list-free class check for the populated-but-empty shape)

Same guard file: for every descriptor where `desc.surfaces` is `Some(ops)`, assert
`(ops.registrations)()` is non-empty **and** each `PluginSurfaceRegistration.surfaces` is
non-empty. Declaring the ops block while yielding nothing is always a defect (a plugin with no
surfaces omits the block).

**Guarantee scope, stated honestly (per the vacuous-guard lesson):** Layer A is meaningful in an
`agent-infra`-ON build; in the default lane it passes trivially because nothing is suppressed. The
load-bearing lane is the existing canonical workspace gate `cargo test --all-features`
(quality-gates.md), which unifies `agent-infra` ON — the exact shape where the bug lives.

**RED command (fails on current `main`, passes after the exemplar fix):**

```sh
cargo test -p uptrakit-plugin-infrastructure-registry --features agent-infra \
  --test contribution_monotonicity_guard
```

### Layer B — cross-build superset diff (class closure: content thinning, future plugins, any axis)

New gate `cargo xtask contribution-monotonicity-check` (xtask precedent:
`audit-coverage-check`, `openapi-client-check`):

1. **Dump entry point (new plumbing, named deliverable):** an example binary in the registry crate
   (`crates/plugins/infrastructure/registry/examples/dump_contributions.rs`) that walks
   `all_descriptors()` and prints one JSON document to stdout: per `type_id` — surface IDs, per
   surface interaction `(id, effective_http_method)` pairs and data-source IDs; agent-interaction
   IDs from `agent_surfaces`; migration names from `migrations`/`agent_migrations` (via
   `MigrationName::name()`). Plus a metadata header with the build's feature fingerprint, at
   minimum `{"agent_infra": cfg!(feature = "agent-infra"), "migrations": cfg!(feature = "migrations")}`.
2. **Two builds, the two shipped controller shapes:**
   - baseline ≈ lean controller: `cargo run -p uptrakit-plugin-infrastructure-registry
--example dump_contributions --features plugin-ops,migrations` (exact baseline feature set
     pinned at plan time to mirror the lean `uptrakit-controller` registry shape; registry
     `[dev-dependencies]` were checked at spec time and pull no `agent-infra`, so the example
     target builds in the true baseline shape);
   - union: same command with `--all-features`.
3. **Compare in xtask:** for every plugin and every contribution field present in the baseline,
   the union set must be a **superset**. Any feature that subtracts _anything_ — a whole
   descriptor, a surface, a single interaction, a migration — fails the gate, for every plugin
   including ones added later with no dedicated assertion. Plugins present only in the union
   (optionally compiled, e.g. notification plugins) are additive and pass by construction.
4. **Self-verifying lanes:** xtask asserts the baseline dump's fingerprint has
   `agent_infra: false` and the union's has `agent_infra: true`. If dev-dep or example-target
   feature unification ever silently turns the baseline into the union shape, the gate fails loud
   instead of self-punting — closing the "guarantee scope quietly narrows to the compiled feature
   set" trap that got the last two guards.

CI wiring: new step in `ci.yml` (reuses the `rust-all-features` cache for the union build) and a
`docs/development/quality-gates.md` entry. Pre-push inclusion decided at plan time (two extra
builds; acceptable if the baseline build is cheap enough, otherwise CI-only).

Error handling: xtask check follows the existing xtask error conventions; the example binary
returns `Result` and reports via stderr + non-zero exit (workspace lints deny
`unwrap`/`expect`/`panic!` in examples too).

## Decision 4 — exemplar fix and workspace sweep

1. **Proxmox fix:** delete `descriptor_plugin_surfaces()` (`plugin.rs:64-69`); wire
   `proxmox_plugin_surfaces` directly into `declare_plugin!(surfaces: { registrations: ... })`.
2. **Fix the codifying test:** `unified_registrations_pair_every_interaction_with_plugin_handled_delivery`
   loses its `if cfg!(feature = "agent-infra") { assert empty; return; }` head and asserts the real
   pairing content unconditionally; sibling content tests keep calling the raw builder.
3. **Workspace sweep (one-time audit task):** grep every plugin crate's descriptor/registration
   builder paths for `cfg!` and `#[cfg]` use; classify each site as
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

| File                                                           | Change                                                                                                                                                         |
| -------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `docs/adr/0032-plugin-contribution-monotonicity.md`            | new ADR: invariant, model decision, enforcement layers, permitted/banned shapes                                                                                |
| `docs/development/coding-standards.md` (Feature Flags section) | contribution-monotonicity rule, the legitimate/banned line, both permitted shapes with the proxmox examples                                                    |
| `AGENTS.md` (root)                                             | extend the "**Feature flags are additive only.**" rule by one clause: descriptor contributions are feature-monotonic; link ADR-0032 (stays within size budget) |
| `crates/plugins/AGENTS.md`                                     | authoring rule: registration builders are predicate-free; how to express feature-gated contributions; where presence is tested                                 |
| `docs/development/feature-flags.md`                            | cross-reference from the additive-only note to ADR-0032                                                                                                        |
| `docs/development/plugin-system.md`                            | note on descriptor contribution fields and consumer-side selection                                                                                             |
| `docs/development/surfaces.md`                                 | update the catalog-guard paragraph (new guard file, removed punt)                                                                                              |
| `docs/development/quality-gates.md`                            | add `cargo xtask contribution-monotonicity-check` (canonical command home)                                                                                     |
| `CONTEXT.md`                                                   | glossary entry: **Contribution Monotonicity**                                                                                                                  |

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

1. The RED command above fails on current `main` and passes after the exemplar fix.
2. `cargo xtask contribution-monotonicity-check` fails if any plugin's contribution set under
   `--all-features` is not a superset of its baseline set — including plugins and features that do
   not exist yet (class-level, not instance-level).
3. Both lane fingerprints are asserted by the gate itself; a mis-featured lane fails loud.
4. A plugin author reading `crates/plugins/AGENTS.md` finds exactly one documented way to express
   "controller-only": populate the controller-consumed field and let the consumer select — with the
   two permitted shapes for genuinely feature-gated code.
5. All existing quality gates stay green (`cargo fmt/check/clippy` in both canonical feature sets,
   `cargo test --all-features`, markdownlint).

# 0032 — Plugin Contribution Monotonicity

**Date:** 2026-07-27 **Status:** Accepted

## Context

The same defect class shipped twice in `crates/plugins/infrastructure/proxmox/src/plugin.rs`,
three months apart. On 2026-04-25, commit `800975bea` gated `controller_update_protection` behind
`#[cfg(not(feature = "agent-infra"))]`; Cargo feature unification turned `agent-infra` on whenever
the standalone controller built with `embedded-ssh-agent`, so the shipped binary silently lost
update protection. The fix deleted the gate but shipped no rule or guard against the underlying
pattern. In 2026-07 the same pattern resurfaced one level removed: `descriptor_plugin_surfaces()`
returned an empty vector whenever `cfg!(feature = "agent-infra")` was true. The feature-unification
chain — standalone controller (`embedded-ssh-agent`) → controller-runtime → agent-ssh-runtime
(which depends on the registry with `agent-infra`) → registry → proxmox — turns the feature on in
exactly the shipped standalone controller, so `proxmox.hosts` and every other proxmox surface
silently failed to register there. The lean `uptrakit-controller` binary was unaffected, which made
the loss read as intermittent rather than systemic.

Both incidents are one class: a plugin expressed "this contribution is controller-only" as a
compile-time feature predicate, inside a workspace where features unify across members. A crate
cannot observe which other workspace member turned its feature on, so the producing crate's local
reasoning about "am I in the controller or the agent" is meaningless once linked into a merged
binary. The existing regex gate, `ci/verify_no_new_cfg_not_feature.sh`, bans only the
`#[cfg(not(feature = ...))]` attribute form; the second incident used the permitted `cfg!()`
expression form with subtractive semantics inside ordinary Rust control flow, which no regex over
attribute syntax can distinguish from a legitimate boolean check. The class is unbounded in
spelling (inverted predicates, let-bound booleans, helper indirection), so a textual gate can only
ever catch the exact shape it was written against.

See `docs/superpowers/specs/2026-07-27-plugin-contribution-monotonicity-design.md` for the full
audit, alternatives considered, and phased plan.

## Decision

### The field split is the process-scope model; no `ProcessRole` enum

`PluginDescriptor` already encodes process scope as data selected by its consumer, not as a
predicate evaluated by its producer: `surfaces` is read only by controller boot
(`PluginSurfaceOps::surface_registrations()`), `agent_surfaces` only by the agent runtime,
`migrations`/`agent_migrations` split the same way for the respective migrators, and role-slot
`HostRequirements` (e.g. `HostRequirements::CONTROLLER_ONLY`) is checked at dispatch time. A
plugin author expresses "controller-only" or "agent-only" by populating only the field its
intended consumer reads — never by adding a new enum or a runtime branch that tries to guess which
process it is running in. The merged (embedded) process consumes both sides of every split exactly
once, so a populated field that its own process never reads is simply inert data, not a conflict.
No `ProcessRole`/`ContributionSite` enum is introduced: a parallel enum would be a second source of
truth for process scope that could disagree with which fields are actually populated.

### Contribution monotonicity

> **Contribution monotonicity:** enabling a Cargo feature may only **add** descriptor
> contributions; it must never remove or alter contributions that exist without it. Descriptor and
> registration builder functions must produce the same-or-more data for every superset of enabled
> features.

The line between legitimate and banned feature gating follows directly from that definition.
Legitimate use is "this code does not exist without the feature": `#[cfg(feature = "X")]` on
modules, dependencies, and items that genuinely cannot compile without it, expressed through one of
two monotone shapes — an additive chain (`std::iter::empty().chain(#[cfg(feature = "X")] ...)`) or
a pair of stubs (a real `#[cfg(feature = "X")]` function alongside a `#[cfg(not(feature = "X"))]`
stub returning empty), each carrying an inline comment naming why the split exists. Banned use is
"this data is suppressed although the code exists": any branch — `cfg!(feature)`, `if !cfg!`,
let-bound predicates, or helper indirection — that returns less registration data when a feature is
turned on. Because that shape has unbounded spellings, it is banned by rule rather than by pattern
match, and caught behaviorally rather than textually.

### Enforcement: three behavioral layers; the regex gate is unchanged

`ci/verify_no_new_cfg_not_feature.sh` keeps its one existing rule exactly as-is — it still bans the
`#[cfg(not(feature = ...))]` attribute form and nothing more. The subtractive-`cfg!` class is
enforced instead where it actually manifests: the compiled catalog produced by `all_descriptors()`.

Two of the three layers are behavioral tests already landed in
`crates/plugins/infrastructure/registry/tests/contribution_monotonicity_guard.rs`, both run by the
canonical `cargo test --all-features` workspace gate (the lane that turns `agent-infra` on and is
therefore the only lane where either assertion can catch a real suppression):

- **Layer C — unconditional critical-surface presence.** Asserts the catalog contains a surface
  with `surface_id == "proxmox.hosts"` in every feature configuration, since proxmox is a
  mandatory, non-cfg-gated registry dependency. This single assertion catches both historical
  suppression shapes — whole-descriptor `None` (2026-04) and empty-registrations (2026-07) — because
  both manifest as the same surface going missing.
- **Layer A — declaring implies non-empty.** For every descriptor whose `surfaces` field is
  populated, asserts its registrations function returns at least one registration and every
  registration's `surfaces` list is non-empty. A plugin that declares the ops block while yielding
  nothing is always a defect, since a plugin with no surfaces simply omits the block.

The third layer, **Layer B — cross-build superset diff**, is `cargo xtask
contribution-monotonicity-check`: it builds the registry twice (the lean controller's derived
feature baseline, and `--all-features`) and asserts every plugin's union-build contributions are a
superset of its baseline-build contributions, closing the class for plugins and features that do
not have a dedicated Layer A/C assertion. It is wired into the backend lint CI job
(`.github/workflows/ci.yml`). Its design, including the package-scoped baseline derivation and the
fingerprint self-verification it needs to avoid a vacuous diff, is recorded in
`docs/superpowers/specs/2026-07-27-plugin-contribution-monotonicity-design.md` (Decision 3, Layer
B).

## Consequences

- A plugin author expresses "controller-only" or "agent-only" by populating only the field its
  intended consumer reads, and never by adding a predicate that tries to detect which process it is
  linked into — process scope is consumer-selected data, not producer-computed logic.
- "Content" (what a builder function returns) stays testable per-plugin with `cargo test -p
  <plugin>`; "presence under feature unification" is not, and was never testable that way — the
  plugin cannot observe another workspace member's feature choices, and proxmox's own dev-dependency
  even pins the union shape for its own tests. That guarantee now belongs to the registry-level
  guards in `contribution_monotonicity_guard.rs`, with the cross-build diff in Layer B covering
  plugins with no dedicated assertion.
- Layers A and C are meaningful only in a build where the guarded feature is enabled; in the default
  feature lane both assertions pass trivially because nothing is being suppressed. `cargo test
  --all-features` is the load-bearing lane, consistent with the rest of the quality-gates catalog.
- Layer B (`cargo xtask contribution-monotonicity-check`, CI-enforced in the backend lint job) now
  provides automated protection against contribution thinning under unification for plugins and axes
  without a dedicated Layer A/C assertion — closing the gap that previously relied on the exemplar
  fix, the workspace sweep, and code review alone.
- The proxmox self dev-dependency restructure that would let `cargo test -p
  uptrakit-plugin-infrastructure-proxmox` exercise the lean feature shape instead of always pinning
  the union remains a separate, deferred design:
  `docs/superpowers/specs/2026-07-20-proxmox-bare-crate-gates-design.md`. This ADR does not depend on
  it.

## Cross-references

- Spec: `docs/superpowers/specs/2026-07-27-plugin-contribution-monotonicity-design.md`
- Deferred proxmox restructure spec:
  `docs/superpowers/specs/2026-07-20-proxmox-bare-crate-gates-design.md`
- Behavioral guard (Layers A + C):
  `crates/plugins/infrastructure/registry/tests/contribution_monotonicity_guard.rs`
- Prior incident: commit `800975bea` (`fix(proxmox): fix update protection never running in
  embedded builds`)
- Regex gate (unchanged): `ci/verify_no_new_cfg_not_feature.sh`
- `AGENTS.md` — "Feature flags are additive only" rule

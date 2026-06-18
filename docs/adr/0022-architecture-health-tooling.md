# ADR-0022: Architecture-Health Tooling — Hybrid Cargo Gates + Advisory CodeScene

Date: 2026-06-18

## Status

Accepted.

## Context

Sentrux was the project's architecture-health tool, wired into CI (a `sentrux:` job that
installed via `curl … | sh` from an unmaintained repo), the pre-push hook, `quality-gates.md`,
and a Claude Code MCP plugin. It graded structural health across ~14 dimensions and checked
custom rules in `.sentrux/rules.toml`.

Sentrux is abandoned — no maintained release cadence, and the `curl | sh` install from an
unmaintained source is a supply-chain liability. The governance capability is worth keeping; the
tool is not. We need a replacement built on durable, team-controlled tooling so an upstream
abandonment cannot take the gates down again.

Research (see `docs/superpowers/specs/2026-06-17-architecture-health-tooling-design.md`) found no
single Rust tool reproduces Sentrux. The plan was a hybrid: enforced cargo gates plus advisory
CodeScene. During implementation, two of the candidate cargo gates proved a poor fit for this
codebase and the plan was revised accordingly.

## Decision

Remove every Sentrux touchpoint (`sentrux:` CI job + installer, pre-push block, `.sentrux/rules.toml`,
MCP plugin, doc/review references). Architecture is governed by:

- **Plugin boundary** — `python3 ci/check_plugin_semantic_boundary.py` (blocking, `semantic-boundary:`
  job). Unchanged; the most durable custom-rule enforcer because the team owns it.
- **Licenses / advisories / bans** — `cargo deny check` (blocking, `backend-deny:` job). Unchanged.
- **Unused dependencies** — `cargo machete` runs **advisory** (non-blocking, `unused-deps:` CI job).
- **Coverage + behavioral health** — `cargo-llvm-cov` → Codecov + CodeScene, and the CodeScene
  dashboard (hotspots, change/temporal coupling, code-health grade), all **advisory**. Deferred to a
  follow-up PR.

Static **Dependency Structure Matrix** and **afferent/efferent coupling** are deferred: no turnkey
Rust tool exists and they are derivable only via custom code.

### Rejected during implementation

- **`cargo modules --acyclic` as a module-cycle gate.** Empirically unusable here: it analyses the
  *item* graph, so any idiomatic `Debug`/`Clone`/`Display`/`Default`/`fn new() -> Self` impl reads as a
  `Type ↔ Type::method` cycle. A full sweep flagged **66 of 71 crates** with **zero genuine cycles**,
  and no flag suppresses the false positives. Rust's resolver already forbids circular *crate*
  dependencies at build time, so the real risk this gate was meant to cover does not exist. Dropped
  entirely — including the `cargo metadata`-derived crate-allowlist helper it required.
- **`cargo machete` as a *blocking* gate.** This workspace is macro- and feature-heavy (its weak
  spot): a baseline run produced ~32–45 findings dominated by false positives (macro crates,
  feature-gated TLS, build-side-effect dev-deps). Keeping it green would require an ongoing per-crate
  ignore list, fighting the zero-maintenance goal. It runs **advisory** instead — useful signal,
  no gate.

## Consequences

- The abandoned `curl | sh` supply-chain liability is gone.
- Enforcement rests on tooling the team controls (`check_plugin_semantic_boundary.py`) or that is
  mature and community-owned (`cargo deny`); the abandonable SaaS (CodeScene) is strictly advisory.
- No new blocking gate is added in this PR — `cargo machete` is advisory and the cycle gate is dropped.
  Net enforced surface is unchanged; net advisory surface improves (machete + CodeScene replace
  advisory Sentrux).
- There is no module-cycle gate. This is acceptable: crate cycles are impossible by construction and
  intra-crate module cycles are rare and not worth a false-positive-ridden gate.
- A second custom-rule enforcer (cargo-pup / dylint) remains deferred until a concrete rule needs it.

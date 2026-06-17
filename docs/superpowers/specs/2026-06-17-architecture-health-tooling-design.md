# Architecture-Health Tooling — Replacing Sentrux with a Hybrid Cargo + CodeScene Stack

**Date:** 2026-06-17 **Status:** Proposed

## Problem

Sentrux is the project's current architecture-health tool. It is wired into four places:

- `.github/workflows/ci.yml` — the `sentrux:` job (`sentrux check . || true`, advisory).
- `.husky/pre-push` — `sentrux check .` (advisory, prints "advisory only, not blocking push").
- `docs/development/quality-gates.md` — documented in the pre-push hook table.
- Claude Code MCP — the `mcp__plugin_sentrux_sentrux__*` toolset (scan, check_rules, dsm, git_stats, health, rescan, test_gaps).

Sentrux graded structural health A–F across ~14 dimensions (coupling, dependency cycles, cohesion, dead code, test coverage), produced a Dependency
Structure Matrix (DSM), git churn/hotspot stats, custom architecture-rule checking, and test-gap detection.

**Sentrux is now abandoned** — installer pulls from `raw.githubusercontent.com/sentrux/sentrux/main`, and there is no maintained release cadence. An
abandoned tool in the toolchain is dead weight at best and a supply-chain liability at worst (the CI job `curl … | sh` from an unmaintained repo).
The architecture-governance capability it provided is worth keeping; the tool is not.

This spec selects a replacement. The grilling decisions that bound it:

1. **Hybrid stack** — durable local cargo tooling for the enforced gates, plus CodeScene (SaaS, free OSS tier) for health grading, hotspots, and
   Claude Code MCP.
2. **Drop DSM + static afferent/efferent coupling** — no turnkey Rust tool exists; only derivable via custom code on `guppy`. Deferred (see
   Non-goals). This removes `guppy` from the stack entirely.
3. **MCP integration is nice-to-have** — CodeScene ships one as a bonus; not a selection driver.
4. **Repo is public OSS** — CodeScene / SonarCloud / Qlty free-for-OSS tiers apply.

## Goals

1. Remove every Sentrux touchpoint (CI job, pre-push hook, quality-gates doc, MCP plugin) and its `curl | sh` installer.
2. Stand up a replacement whose **enforced** dimensions run on mature, community-owned cargo tooling the team controls — so the "tool got abandoned"
   failure cannot take the blocking gates down again.
3. Keep CodeScene (the one piece that can also be abandoned or repriced) strictly **advisory and non-load-bearing**: if it disappears, the blocking
   gates survive untouched.
4. Cover Sentrux's dimensions that have a credible modern Rust tool; explicitly defer the ones that do not (DSM, static coupling).
5. Pin every tool to its latest stable version, verified 2026-06-17.
6. Leave a future maintainer one place (a section in `docs/development/quality-gates.md`) explaining what each tool checks and how to run it.

## Non-goals (YAGNI)

- **No Dependency Structure Matrix.** No off-the-shelf Rust DSM renderer exists; building one on `guppy` is custom analysis code to maintain.
  Deferred until the gap actually bites. (Note: crate-DAG _layering_ — the dimension DSM governed — is enforced by
  `check_plugin_semantic_boundary.py` (and a deferred second enforcer if ever needed), **not** by `cargo-modules --acyclic`; see below.)
- **No static afferent/efferent coupling or instability metric (`I = Ce/(Ca+Ce)`).** Same reason — derivable from `guppy` only via custom code.
  Deferred. CodeScene's _change_ coupling (behavioral, from git history) partially covers the intent.
- **`cargo-modules --acyclic` is not a workspace-layering gate.** It detects _intra-crate module_ cycles only. Cross-_crate_ cycles are already
  impossible — Cargo's resolver rejects circular crate deps at build time — so the crate-DAG and its layering are governed by the boundary checker +
  the `check_plugin_semantic_boundary.py` checker, a separate concern from this gate. Do not conflate the two.
- **No single consolidated A–F scorecard across 14 named dimensions.** Sentrux's one-report format is not reproduced. CodeScene gives one health
  score (1–10); the static gates report pass/fail per tool. Aggregating them into one artifact is deferred (a thin CI summary script if ever
  wanted).
- **No replacement for `cargo-deny`.** It is already in CI and pre-push, already covers advisories/licenses/bans, and is unaffected. Kept as-is.
- **No new workspace `[dependencies]`.** Every tool here is a CLI binary installed in CI / locally, not a library dependency. `Cargo.toml` and
  `deny.toml` are untouched (no license-allowlist impact).

## The stack

### Tier 1 — Enforced, local, durable (cargo-native)

These run in CI and pre-push. All are mature, multi-maintainer, community-owned, free OSS.

| Sentrux dimension                           | Tool                                                                                   | Latest stable (2026-06-17) | Gate role                  |
| ------------------------------------------- | -------------------------------------------------------------------------------------- | -------------------------- | -------------------------- |
| Dependency cycles (module-level)            | **cargo-modules** `dependencies --acyclic`                                             | 0.26.0 (2026-04-18)        | **Blocking** — new gate    |
| Item-level dead code                        | clippy `dead_code` (already `all = deny`)                                              | —                          | Already blocking           |
| Unused dependencies                         | **cargo-machete**                                                                      | 0.9.2 (2026-04-15)         | Blocking — new gate        |
| Test coverage                               | **cargo-llvm-cov** → **Codecov**                                                       | 0.8.7 (2026-05-13)         | Report → upload to Codecov |
| Advisories / licenses / bans                | **cargo-deny** (already present)                                                       | 0.19.9 (2026-06-15)        | Already blocking           |
| Custom architecture rules (plugin boundary) | **`ci/check_plugin_semantic_boundary.py`** (already present, `semantic-boundary:` job) | —                          | Already blocking           |

`cargo-machete` chosen over `cargo-shear`/`cargo-udeps`: no nightly required, fastest, most established (1.3k★). `cargo-llvm-cov` chosen over
`cargo-tarpaulin`: modern default, emits LCOV that **Codecov** ingests. **Codecov** (codecov.io) is the coverage platform — free for OSS, repo is
public, `CODECOV_TOKEN` secret already added. Upload via `codecov/codecov-action@v7` (7.0.0, 2026-06-07). Codecov owns the coverage report, PR
delta, and badge. The **same `lcov.info` is also pushed to CodeScene** so its test-gap dimension is coverage-backed — one `cargo-llvm-cov` run, two
consumers (no double compile).

If `--acyclic` is red on day one (pre-existing intra-crate cycles), fix them in the same PR, or land it advisory and flip to blocking in the PR that
clears the last one — no open-ended advisory period.

### Tier 1.5 — Custom architecture rules (the Sentrux `check_rules` role)

The repo **already owns** a custom architecture-rule enforcer: `ci/check_plugin_semantic_boundary.py`, run as the blocking `semantic-boundary:` CI
job, guarding the plugin/production boundary. This is the most durable possible answer for custom rules — code the team controls, already wired and
blocking. For the highest-value layering rule, **no new tool is needed**, and no new tool is added. Enforcing further boundaries (e.g. the
controller↔core split of ADR 0003) is **deferred** — extend the Python checker, or adopt **cargo-pup** (Datadog), when a concrete second rule is
actually needed. See Deferred.

### Tier 2 — Advisory health brain (CodeScene SaaS, free OSS tier)

| Sentrux dimension    | CodeScene capability                           |
| -------------------- | ---------------------------------------------- |
| A–F health grades    | Code Health score (1–10), per-file + aggregate |
| Git churn / hotspots | Behavioral code analysis (its core competency) |
| Cohesion             | Low-cohesion detection (X-Ray / Code Health)   |
| Coupling (over time) | Change/temporal coupling from git history      |
| Test-gap             | Coverage-backed (ingests the same `lcov.info`) |

- **Free for open source** (repo is public). Polyglot — also analyzes the TypeScript/Svelte `frontend/` (a bonus Sentrux did not cover).
- **Load-bearing value is the dashboard**, not a per-PR check: the hotspot + change/temporal-coupling views (reviewed periodically, e.g. monthly)
  are the part with no cargo equivalent and the reason CodeScene earns its place. This needs no PR wiring.
- **Advisory only**: any PR delta is a non-blocking status, never a required gate. The optional per-PR status is **deferrable** — an advisory check
  nobody owns becomes ceremony. If kept, name an owner who reviews it; otherwise ship the dashboard alone first.
- **Coverage is uploaded to CodeScene too** (in addition to Codecov), from the same `cargo-llvm-cov` `lcov.info`, so its test-gap dimension is
  coverage-backed. CodeScene otherwise analyzes the whole repo from source + git history, so it onboards new crates with zero config.
- **MCP (nice-to-have):** CodeScene MCP server `MCP-1.3.4` (2026-06-14) runs locally against the repo (no source upload for the MCP path),
  `claude mcp add`. Documented as an **opt-in** developer step, not required infrastructure.

### Dropped from the stack vs research

- **guppy** — only purpose was DSM + static coupling, both deferred. Removed.
- **cargo-depgraph** (stale 2023), **cargo-geiger** / **cargo-public-api** / **cargo-semver-checks** — useful but out of scope for "keep the
  architecture in check"; not Sentrux dimensions. Noted for a future spec if wanted.

## Integration changes (the rip-out + wire-in)

1. **`.github/workflows/ci.yml`**
   - **Delete** the `sentrux:` job (lines ~156–163), including the `curl … | sh` installer.
   - **Add** an `architecture:` job. Install tools with `taiki-e/install-action@v2` (`tool:` list, version-pinned) — the **same mechanism the
     existing `backend-deny` job uses** (`tool: cargo-deny@0.19.4`); do **not** introduce `cargo binstall`, which is not used anywhere in this
     repo's CI and would add a bootstrap dependency. The `install-action` `tool:` list is the **single source of truth** for pinned versions. Steps:
     run `cargo modules dependencies --lib --acyclic --all-features` (blocking) per **lib-bearing** workspace member — `cargo-modules` has no
     `--workspace` flag (runs once per package, `-p <crate> --lib`) and resolves only the active cfg, so `--all-features` is required to see
     feature-gated modules (same posture as the `--all-features` test/coverage gates; verify cargo-modules honors it at implementation time, else
     record the default-only gap in the ADR). **Derive the target list as an allowlist from `cargo metadata`** — select members that expose a
     `lib`/`rlib`/`proc-macro` target — **not** a hardcoded blocklist; `--lib` errors on any member without a library target, so an allowlist
     self-maintains as crates are added/removed (the next new binary crate cannot silently break the gate). The bin-only members fall out
     automatically (today that is `controller`, `agent`, `agent-ssh`, `scheduler`, `mqtt`, `controller-standalone` — thin shims over their
     `*-runtime` libs where the real module graph lives; illustrative, not the gate definition), and `frontend` never appears (not a cargo package).
     Then `cargo machete` (blocking).
   - **Extend `backend-test:` for coverage, then upload to Codecov + CodeScene** (do not add a separate `coverage:` job — a separate job recompiles
     and re-runs the whole suite, doubling CI time and cache cost). Replace only its `cargo test --all-features` **step** with
     `cargo llvm-cov --workspace --all-features --lcov --output-path lcov.info` (one run produces `lcov.info`), then add two upload steps consuming
     that same file: (a) `codecov/codecov-action@v7` (`token: ${{ secrets.CODECOV_TOKEN }}`, `files: lcov.info`); (b) upload the same `lcov.info` to
     CodeScene's coverage import (its CLI / coverage API — exact mechanism + `CS_*` secret confirmed at implementation time). **Leave the subsequent
     `verify_handler_state_contract.sh` and `verify_db_access_policy.py` steps intact** (separate blocking gates in the same job). Single compile,
     same feature set as the existing test gate (`--all-features` is mandatory so coverage matches what CI actually tests). Cache note:
     `cargo llvm-cov` builds with `-C instrument-coverage`, a different artifact set than the cached `cargo test` build — but `backend-test:`
     already sets `save-if: "false"` (it only **reads** the `rust-all-features` cache), so the instrumented build cannot poison that cache; it just
     won't fully hit it. No new `shared-key` needed.
   - **CodeScene** otherwise needs no CI job change; its GitHub integration runs independently. An optional advisory PR-delta status may be enabled
     later (deferred to PR 2, see sequencing).

2. **`.husky/pre-push`**
   - **Replace** the `# --- Sentrux architectural rules ---` block (lines ~75–82) with `cargo modules dependencies --lib --acyclic --all-features`
     per lib-bearing member (blocking; same `cargo metadata`-derived lib-bearing allowlist as the CI `architecture:` job) and `cargo machete`
     (blocking). These are blocking gates: **fail the push if the tool is absent**
     (`command -v cargo-modules >/dev/null || { echo "install: …"; exit 1; }`), matching how the existing `markdownlint` gate hard-fails — **not**
     the Sentrux skip-if-absent pattern, which is exactly the failure mode this spec removes (a missing tool silently meant no check ran).

3. **`.husky/pre-commit`** — unchanged (fast staged-file tier; architecture checks belong in pre-push).

4. **Claude Code MCP config** — ✅ **done**: the Sentrux MCP plugin (`sentrux@sentrux-marketplace`, the `mcp__plugin_sentrux_sentrux__*` provider)
   has already been uninstalled by the user. No further action; optionally add the CodeScene MCP server (`codescene-oss/codescene-mcp-server`, tag
   `MCP-1.3.4`) via `claude mcp add` as an opt-in developer step.

5. **Tool installation** — CI uses `taiki-e/install-action@v2` with a version-pinned `tool:` list (the repo's established pattern); that list is the
   single source of truth for pinned versions. Local dev documented in `setup.md` (`cargo install <tool> --locked`, or `cargo binstall` as an
   optional fast path). No tool becomes a workspace dependency.

## Zero-maintenance on new crates (binding requirement)

Adding a new internal or public crate to the workspace must require **zero manual edits to any gate**. Every gate must auto-discover the workspace.
This is a hard design constraint, not a nicety — per-crate config is exactly the rot that makes governance tooling get bypassed.

| Gate                                | New-crate behavior                                                                                                                                                                                      |
| ----------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `cargo modules --acyclic`           | `cargo metadata`-derived lib-bearing allowlist auto-includes new lib crates; bin-only auto-excluded                                                                                                     |
| `cargo machete`                     | Workspace-wide, auto. Only possible manual touch: a `machete.toml` ignore entry for a genuine FP                                                                                                        |
| `cargo llvm-cov --workspace`        | Workspace flag covers new crates automatically                                                                                                                                                          |
| Codecov + CodeScene coverage upload | Consume the whole `lcov.info`; new crates appear with no config                                                                                                                                         |
| `cargo deny`                        | Workspace-wide, auto                                                                                                                                                                                    |
| `check_plugin_semantic_boundary.py` | Rules **must** be path/layer globs (e.g. `crates/plugins/** must not import crates/core/**`), **never** per-crate enumerations, so a new crate dropped into an existing layer is governed automatically |
| CodeScene dashboard                 | Analyzes the whole repo; new crates appear automatically                                                                                                                                                |

The single tolerated manual step in the entire stack is a rare `machete.toml` false-positive entry. Authoring boundary rules as per-crate lists is a
**banned pattern** — it reintroduces the maintenance burden this requirement forbids. `quality-gates.md` must document this rule for future rule
authors.

## Implementation sequencing

Land in two PRs so a flaky new gate cannot hold the urgent supply-chain rip-out hostage:

- **PR 1 (urgent — the actual ask):** remove the remaining Sentrux touchpoints (the `curl … | sh` CI job, the pre-push block, the quality-gates doc
  row — the MCP plugin is already uninstalled) and land the two genuine 1:1 durable replacements — `cargo modules … --acyclic` and `cargo machete` —
  in CI + pre-push. This kills the unmaintained `curl | sh` immediately. Budget for `cargo-machete` false positives on a feature-gated 77-crate
  workspace (deps used only under one feature or via macro re-export) → a `machete.toml` ignore-list may be needed before it goes green.
- **PR 2 (health layer):** coverage (`cargo llvm-cov` extending `backend-test:`) uploaded to **both Codecov and CodeScene**, plus the CodeScene
  dashboard (+ optional per-PR status). None of these block the supply-chain fix.

## Snapshot conformance

- **deny.toml license allowlist** — not engaged: tools are installed binaries, not workspace `[dependencies]`. No `Cargo.toml` change. ✅
- **Conventional commits / release-please** — the implementation PR follows the existing convention. ✅
- **markdownlint (line_length 150, MD051 off)** — new docs conform. ✅
- **Additive-only feature-flag rule, async-lock rules, panic policy, etc.** — N/A; this spec adds no Rust production code.
- **Quality-gates structure** — new gates slot into the existing pre-push/CI tier model; documented in `quality-gates.md`.

## Documentation deliverables (non-optional — this changes CI, hooks, and architecture governance)

- **NEW ADR** `docs/adr/0022-architecture-health-tooling.md` — records the decision: drop abandoned Sentrux; adopt hybrid (durable cargo gates +
  advisory CodeScene); explicitly defer DSM/static-coupling; rationale for keeping the SaaS layer non-load-bearing. (ADR 0021 is reserved by the
  in-progress skills-version-display work; this is 0022. Confirm the next free number at implementation time.)
- **UPDATE** `docs/development/quality-gates.md` — replace the `sentrux check .` row in the pre-push table; add the new `architecture:` job and the
  `backend-test:` coverage + Codecov/CodeScene upload steps; add a short section listing each tool, what it covers, how to run it, and the
  zero-maintenance rule for rule authors (path/layer globs, never per-crate lists). (No separate `architecture-health.md` — it would duplicate this
  file's structure.)
- **UPDATE** `docs/development/setup.md` — local install instructions for the new tools (`cargo install <tool> --locked`; `cargo binstall` optional
  fast path); note CI is the authoritative version source.
- **UPDATE** `AGENTS.md` — any quality-gates / architecture-rules mention of Sentrux → new stack.
- **UPDATE** `README.md` — replace any Sentrux reference; add a **Codecov coverage badge** (repo is public). Grep `CONTRIBUTING.md` for Sentrux;
  update or state "no reference found".
- **UPDATE** any MCP-related doc that references Sentrux (grep at implementation time; `docs/development/oauth-mcp.md` does not; update or state "no
  reference found") — optionally document CodeScene MCP opt-in as a developer tool.
- **OPTIONAL** `codecov.yml` at repo root — only if non-default coverage targets/flags are wanted; default Codecov behavior needs no config file.

## Verification

- CI green with the Sentrux job removed and new jobs added.
- `cargo modules dependencies --lib --acyclic --all-features` passes for every `cargo metadata`-derived lib-bearing member, **or** any existing
  intra-crate module cycles are fixed in-PR (else land advisory, flip in the PR that clears the last one).
- `cargo machete` passes (or flagged unused deps removed) — blocking.
- `cargo llvm-cov --workspace --all-features` produces `lcov.info`; **Codecov and CodeScene both ingest it** on a test PR (Codecov report appears,
  CodeScene test-gap is coverage-backed).
- `.husky/pre-push` runs the new checks locally and reports clearly.
- Adding a throwaway empty crate to the workspace requires **no gate-config edits** and CI stays green (validates the zero-maintenance requirement).
- No remaining grep hit for `sentrux` outside this spec and the new ADR's history note.

## Risks & mitigations

- **CodeScene SaaS could be abandoned or repriced.** Mitigation: it is strictly advisory; all blocking gates are local cargo tooling. Its loss
  degrades insight, not enforcement.
- **Dual coverage upload (Codecov + CodeScene) doubles the integration surface.** Mitigation: both consume the one `lcov.info` (no extra compile);
  **Codecov is the primary coverage home** (badge, PR delta) and the CodeScene push is best-effort — if CodeScene's coverage-import mechanism is
  awkward, ship Codecov first and add the CodeScene push as a follow-up step within PR 2. Neither is a blocking gate.
- **Existing intra-crate module cycles may make `--acyclic` red on day one.** Mitigation: run it first in report mode; fix in-PR, or land advisory
  and flip to blocking in the PR that resolves the last cycle, rather than a big-bang block.
- **Tool version drift.** Mitigation: the CI `taiki-e/install-action` `tool:` list is the single source of truth for pins; `quality-gates.md`
  **references** it rather than duplicating version strings (no second place to drift). Note `cargo-deny` is already pinned at `0.19.4` in the
  existing `backend-deny` job — bumping it is out of scope for this spec. **Local caveat:** pre-push installs (`cargo install`/`binstall`) are
  **not** version-locked to the CI list, so a contributor's local tool can differ from CI; **CI is authoritative** and pre-push is best-effort.
  `setup.md` states this and lists the same versions as a courtesy.

## Deferred / out of scope

- Dependency Structure Matrix (DSM) rendering for Rust.
- Static afferent/efferent coupling + instability metric (`I = Ce/(Ca+Ce)`).
- Single consolidated A–F multi-dimension scorecard artifact.
- `cargo-geiger` (unsafe surface), `cargo-public-api` + `cargo-semver-checks` (API drift) — separate future spec.
- **A second custom-rule enforcer** beyond `check_plugin_semantic_boundary.py`. Add when a concrete rule (e.g. ADR 0003 controller↔core) is actually
  needed — extend the Python checker, or adopt `cargo-pup` (Datadog, 0.1.8) / `dylint` (Trail of Bits, 6.0.1) then. Not built speculatively.
- `cargo-modules orphans` (dead/unlinked-file detection) — add if orphan files become a real problem; rides the already-installed `cargo-modules`.

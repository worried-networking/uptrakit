# Track C Production Semantic Gate Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: use `superpowers:subagent-driven-development` or `superpowers:executing-plans` to implement this plan
> task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** land the Track C production semantic-boundary gate as a shared local + CI quality check, clean the remaining production violations, and keep
plugin-specific knowledge out of production consumers other than the registry/catalogue boundary.

**Architecture:** keep [`ci/check_plugin_semantic_boundary.sh`](/Users/andreyyantsen/Development/uptrakit/ci/check_plugin_semantic_boundary.sh) as the
stable public entrypoint, but preserve the current shell rules until the Python helper reaches rule parity. Build the structured checker in shadow
mode first, then cut the shell entrypoint over once parity is proven by tests and real-repo comparison. Clean production consumers by using the
registry crate’s existing public surface, removing direct `uptrakit_plugin_infrastructure_core` manifest/code dependencies, and only adding small
feature-gated registry re-exports if a real missing symbol is discovered. Fix the frontend semantic leak generically by attaching extension owner
metadata to extension responses, then filtering host-row context-menu extensions against the row’s plugin set instead of hardcoding Docker IDs.

**Tech Stack:** Bash (`ci/check_plugin_semantic_boundary.sh`, `.husky/pre-push`), Python 3.11+ (`argparse`, `dataclasses`, `pathlib`, `re`,
`subprocess`, `tomllib`, `unittest`), Rust workspace crates (`uptrakit-plugin-infrastructure-registry`, `uptrakit-plugin-infrastructure-core`,
`uptrakit-agent-core`, `uptrakit-scheduler-engine`, `uptrakit-agent-ssh`, `uptrakit-web-api`, `uptrakit-web-api-types`), Svelte/TypeScript
(`frontend/src/lib`, `frontend/src/routes/software/[id]/+page.svelte`), GitHub Actions, Markdown docs.

---

## File Structure

Implementation units for Track C:

- [`docs/superpowers/specs/2026-04-14-track-c-production-semantic-gate-design.md`](/Users/andreyyantsen/Development/uptrakit/docs/superpowers/specs/2026-04-14-track-c-production-semantic-gate-design.md)
  Responsibility: canonical Track C policy and acceptance criteria. Read-only reference during implementation.
- [`docs/internal/changes/TASK-0007/ST-0032-track-c-production-semantic-inventory.md`](/Users/andreyyantsen/Development/uptrakit/docs/internal/changes/TASK-0007/ST-0032-track-c-production-semantic-inventory.md)
  Responsibility: living inventory of representative and then exact production findings during migration; must end with `Remaining: None`.
- [`ci/check_plugin_semantic_boundary.sh`](/Users/andreyyantsen/Development/uptrakit/ci/check_plugin_semantic_boundary.sh) Responsibility: stable
  entrypoint for local hooks and CI; initially keeps the existing shell rules authoritative, then delegates to Python after parity.
- [`ci/check_plugin_semantic_boundary.py`](/Users/andreyyantsen/Development/uptrakit/ci/check_plugin_semantic_boundary.py) Responsibility: structured
  repository scan, canonical plugin ID extraction, syntax-aware literal detection, manifest scanning, optional allowlist parsing, and deterministic
  rule reporting.
- [`ci/test_check_plugin_semantic_boundary.py`](/Users/andreyyantsen/Development/uptrakit/ci/test_check_plugin_semantic_boundary.py) Responsibility:
  checker tests over fixture trees plus real-rule coverage for allowlist handling, inline-test exclusion, fully-qualified crate references, raw
  strings, and manifest edge cases.
- [`ci/testdata/plugin_semantic_boundary/**`](/Users/andreyyantsen/Development/uptrakit/ci/testdata/plugin_semantic_boundary) Responsibility: passing
  and failing fixture repos, including minimal canonical plugin ID definitions via `plugin_ids::ALL`.
- [`ci/plugin_semantic_boundary_allowlist.example.toml`](/Users/andreyyantsen/Development/uptrakit/ci/plugin_semantic_boundary_allowlist.example.toml)
  Responsibility: documented example only; no real allowlist file should exist unless a temporary exception is intentionally approved.
- [`ci/plugin_semantic_boundary_allowlist.toml`](/Users/andreyyantsen/Development/uptrakit/ci/plugin_semantic_boundary_allowlist.toml) Responsibility:
  optional temporary exception file; should remain absent in the final state for this rollout.
- [`/.husky/pre-push`](/Users/andreyyantsen/Development/uptrakit/.husky/pre-push) Responsibility: invoke the shared semantic-boundary check before
  push without duplicating rule logic.
- [`/.github/workflows/ci.yml`](/Users/andreyyantsen/Development/uptrakit/.github/workflows/ci.yml) Responsibility: run the semantic-boundary gate as
  a blocking CI job or step.
- [`docs/development/quality-gates.md`](/Users/andreyyantsen/Development/uptrakit/docs/development/quality-gates.md) Responsibility: document the
  semantic-boundary gate in maintainer quality gates.
- [`docs/development/setup.md`](/Users/andreyyantsen/Development/uptrakit/docs/development/setup.md) Responsibility: document that `pre-push` runs the
  semantic-boundary gate.
- [`crates/shared/agent-core/Cargo.toml`](/Users/andreyyantsen/Development/uptrakit/crates/shared/agent-core/Cargo.toml)
- [`crates/shared/scheduler-engine/Cargo.toml`](/Users/andreyyantsen/Development/uptrakit/crates/shared/scheduler-engine/Cargo.toml)
- [`crates/core/agent-ssh/Cargo.toml`](/Users/andreyyantsen/Development/uptrakit/crates/core/agent-ssh/Cargo.toml) Responsibility: remove direct
  `uptrakit-plugin-infrastructure-core` production dependencies once code imports are migrated.
- [`crates/plugins/infrastructure/registry/src/lib.rs`](/Users/andreyyantsen/Development/uptrakit/crates/plugins/infrastructure/registry/src/lib.rs)
  Responsibility: keep the registry as the approved production consumer surface; only add narrowly-scoped feature-gated re-exports here if the
  migration discovers a real missing type.
- [`crates/shared/agent-core/src/client.rs`](/Users/andreyyantsen/Development/uptrakit/crates/shared/agent-core/src/client.rs)
- [`crates/shared/agent-core/src/update.rs`](/Users/andreyyantsen/Development/uptrakit/crates/shared/agent-core/src/update.rs)
- [`crates/shared/agent-core/src/version_check.rs`](/Users/andreyyantsen/Development/uptrakit/crates/shared/agent-core/src/version_check.rs)
- [`crates/shared/scheduler-engine/src/executors/fetch_releases.rs`](/Users/andreyyantsen/Development/uptrakit/crates/shared/scheduler-engine/src/executors/fetch_releases.rs)
- [`crates/core/agent-ssh/src/runtime_support.rs`](/Users/andreyyantsen/Development/uptrakit/crates/core/agent-ssh/src/runtime_support.rs)
- [`crates/core/agent-ssh/src/operations/bootstrap.rs`](/Users/andreyyantsen/Development/uptrakit/crates/core/agent-ssh/src/operations/bootstrap.rs)
  Responsibility: remove direct `uptrakit_plugin_infrastructure_core` usage from non-plugin production code.
- [`crates/plugins/infrastructure/core/src/plugin_ops.rs`](/Users/andreyyantsen/Development/uptrakit/crates/plugins/infrastructure/core/src/plugin_ops.rs)
- [`crates/plugins/infrastructure/core/src/catalog.rs`](/Users/andreyyantsen/Development/uptrakit/crates/plugins/infrastructure/core/src/catalog.rs)
- [`crates/core/controller/src/main.rs`](/Users/andreyyantsen/Development/uptrakit/crates/core/controller/src/main.rs)
- [`crates/ui/web-api/src/extension_registry.rs`](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api/src/extension_registry.rs)
- [`crates/ui/web-api/src/routes/extensions.rs`](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api/src/routes/extensions.rs)
- [`crates/ui/web-api/src/routes/service_ws/handler/messages.rs`](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api/src/routes/service_ws/handler/messages.rs)
- [`crates/shared/web-api-types/src/extensions.rs`](/Users/andreyyantsen/Development/uptrakit/crates/shared/web-api-types/src/extensions.rs)
- [`frontend/src/lib/types.ts`](/Users/andreyyantsen/Development/uptrakit/frontend/src/lib/types.ts)
- [`frontend/src/lib/extensions.svelte.ts`](/Users/andreyyantsen/Development/uptrakit/frontend/src/lib/extensions.svelte.ts)
- [`frontend/src/routes/software/[id]/+page.svelte`](/Users/andreyyantsen/Development/uptrakit/frontend/src/routes/software/[id]/+page.svelte)
  Responsibility: attach owner plugin metadata to extension responses and use it to filter host-row context menu extensions generically instead of
  hardcoding `releases_docker`.

Current representative residue confirmed in the repo:

- direct `uptrakit_plugin_infrastructure_core` imports in `agent-core`, `scheduler-engine`, and `agent-ssh`
- direct `uptrakit-plugin-infrastructure-core` manifest deps in the same three crates
- hardcoded host-row Docker extension gate in
  [`frontend/src/routes/software/[id]/+page.svelte`](/Users/andreyyantsen/Development/uptrakit/frontend/src/routes/software/[id]/+page.svelte)
- no CI wiring yet for the semantic-boundary checker in
  [`/.github/workflows/ci.yml`](/Users/andreyyantsen/Development/uptrakit/.github/workflows/ci.yml)
- current shell checker still enforces legacy helper/dashboard rules that the Python cutover must not drop

Treat that list as representative, not exhaustive. Task 1 inventory must capture the exact remaining set before cleanup proceeds.

---

### Task 1: Inventory And High-Coverage Fixtures

**Files:**

- Create: `docs/internal/changes/TASK-0007/ST-0032-track-c-production-semantic-inventory.md`
- Create: `ci/test_check_plugin_semantic_boundary.py`
- Create: `ci/testdata/plugin_semantic_boundary/pass/**`
- Create: `ci/testdata/plugin_semantic_boundary/fail/**`

- [ ] **Step 1: Create the living inventory doc**

Create
[`ST-0032-track-c-production-semantic-inventory.md`](/Users/andreyyantsen/Development/uptrakit/docs/internal/changes/TASK-0007/ST-0032-track-c-production-semantic-inventory.md)
with these sections:

- shared entrypoints
- permanent production scope
- permanent exclusions
- representative confirmed residue
- exact findings discovered by the structured checker
- resolved findings
- remaining findings

Seed it with the confirmed residue above, but clearly label that section as representative only.

- [ ] **Step 2: Build fixture trees that include canonical plugin IDs**

Create pass/fail fixture repos under `ci/testdata/plugin_semantic_boundary/` with a minimal copy of:

- `crates/shared/types/src/plugin_type_id.rs`
- a `plugin_ids` module containing a minimal `ALL` array

The fixture helper must derive canonical plugin IDs from `plugin_ids::ALL`, not by scraping every `PluginTypeId::from_static(...)` literal in the
file.

Minimum fail fixtures:

- direct `use uptrakit_plugin_infrastructure_core`
- fully-qualified inline reference like `uptrakit_plugin_infrastructure_core::BatchFetchResult`
- production file named like `config_test.rs` that must still be scanned because filename suffix alone is not a valid test-only exclusion
- `plugin_ids::...` use in production Rust
- forbidden helper callsite
- forbidden helper definition in `plugin_type_id.rs`
- hardcoded plugin-type string literal in Rust
- hardcoded plugin-type string literal in Svelte/TS
- manifest dependency on `uptrakit-plugin-infrastructure-core`
- manifest dependency on a concrete plugin crate
- target-specific non-dev manifest dependency table
- workspace dependency indirection (`workspace = true`)

Minimum pass fixtures:

- registry imports in production code
- docs/comments mentioning plugin IDs without executable/plugin-identity context
- inline `#[cfg(test)] mod tests` and `*_test.rs` references
- genuinely test-only `_test.rs` file that should be excluded
- examples/migrations/docs trees containing plugin IDs
- `dev-dependencies` and `target.'cfg(test)'.dependencies`
- raw strings unrelated to plugin identity

- [ ] **Step 3: Write tests before the Python helper exists**

Create [`ci/test_check_plugin_semantic_boundary.py`](/Users/andreyyantsen/Development/uptrakit/ci/test_check_plugin_semantic_boundary.py) with failing
tests for:

- pass fixture succeeds
- each rule family produces its rule ID
- forbidden helper definition in `plugin_type_id.rs` is rejected
- allowlist suppresses exactly one finding
- inline-test/module exclusion works
- production `_test.rs`-style filenames are still scanned when they live in production paths
- raw-string parsing works for both Rust and frontend
- manifest scanning covers plain, target-specific, and workspace-driven dependencies

The tests should invoke `python3 ci/check_plugin_semantic_boundary.py --root <fixture-root>`.

- [ ] **Step 4: Run the tests and confirm the pre-implementation failure mode**

Run:

```bash
python3 -m unittest discover -s ci -p 'test_check_plugin_semantic_boundary.py' -v
```

Expected: failure because the Python helper does not exist yet.

---

### Task 2: Stage The Structured Checker Without Losing Existing Enforcement

**Files:**

- Create: `ci/check_plugin_semantic_boundary.py`
- Modify later: `ci/check_plugin_semantic_boundary.sh`
- Test: `ci/test_check_plugin_semantic_boundary.py`

- [ ] **Step 1: Add the Python helper skeleton**

Create [`ci/check_plugin_semantic_boundary.py`](/Users/andreyyantsen/Development/uptrakit/ci/check_plugin_semantic_boundary.py) with:

- CLI args: `--root`, `--allowlist`, `--format`
- stable rule IDs
- `Finding` dataclass
- deterministic renderer
- exit codes: `0` clean, `1` violations, `2` usage/config error

Do not change the shell entrypoint yet.

- [ ] **Step 2: Make the helper testable before cutover**

Implement enough scaffolding that the unit test file can execute the helper and receive deterministic empty/failing outputs, even before all rules are
complete.

- [ ] **Step 3: Keep the shell checker authoritative during migration**

Leave [`ci/check_plugin_semantic_boundary.sh`](/Users/andreyyantsen/Development/uptrakit/ci/check_plugin_semantic_boundary.sh) functionally intact in
this task. If helpful, add an opt-in shadow call to the Python helper behind an env var, but do not remove or bypass the existing shell rules until
Task 6 parity validation passes.

- [ ] **Step 4: Re-run checker tests**

Run:

```bash
python3 -m unittest discover -s ci -p 'test_check_plugin_semantic_boundary.py' -v
```

Expected: only rule-assertion tests still fail; harness-level execution should now work.

---

### Task 3: Implement Rule Families, Allowlist, And Parity Logic

**Files:**

- Modify: `ci/check_plugin_semantic_boundary.py`
- Modify: `ci/test_check_plugin_semantic_boundary.py`
- Create: `ci/plugin_semantic_boundary_allowlist.example.toml`
- Optional only if needed later: `ci/plugin_semantic_boundary_allowlist.toml`

- [ ] **Step 1: Implement canonical plugin ID extraction from `plugin_ids::ALL`**

In the Python helper, read
[`crates/shared/types/src/plugin_type_id.rs`](/Users/andreyyantsen/Development/uptrakit/crates/shared/types/src/plugin_type_id.rs) and derive the
canonical plugin ID set from the `plugin_ids::ALL` array membership, not from all `from_static(...)` occurrences.

This is a two-phase parse:

- extract which constant names appear in `plugin_ids::ALL`
- resolve each referenced constant name to its `PluginTypeId::from_static("...")` value

The helper may use a small parser/regex combo, but the contract must be “IDs present in `plugin_ids::ALL` and only those IDs”.

- [ ] **Step 2: Implement explicit production scope and exclusions**

The helper must scan:

- production Rust under `crates/**`
- production frontend source under `frontend/src/**`
- in-scope `Cargo.toml` manifests

The helper must exclude:

- `docs/**`
- tests
- examples
- migrations
- plugin crates under `crates/plugins/**`
- registry implementation under `crates/plugins/infrastructure/registry/**`
- only the canonical constant definitions in `crates/shared/types/src/plugin_type_id.rs`

For Rust, support both path-based exclusions and inline `#[cfg(test)] mod tests` prefix scanning where needed.

- [ ] **Step 3: Implement all rule families**

Implement these Track C rules with stable IDs:

- `concrete-plugin-import`
- `plugin-core-import`
- `plugin-ids-reference`
- `forbidden-plugin-helper`
- `manifest-plugin-dependency`
- `hardcoded-plugin-type-literal`

Requirements:

- Rust scanning must catch `use ...` and fully-qualified inline references.
- Literal scanning must be syntax-aware enough to skip comments while still catching normal strings, raw Rust strings, and TS/Svelte strings.
- Hardcoded plugin-type literals must only fire in plugin-identity contexts, not on arbitrary prose strings.
- `forbidden-plugin-helper` must catch both callsites and helper definitions, including `fn is_package_manager(` and `fn display_name(` in
  `plugin_type_id.rs`.

Also port the currently enforced legacy shell-only guardrails before cutover so the Python checker does not weaken enforcement within the overlapping
in-scope surface:

- dashboard-icons bespoke surface
- helper definition/callsite checks currently enforced by the shell script
- identity-specific helper checks currently enforced by the shell script

- [ ] **Step 4: Implement manifest scanning correctly**

Manifest scanning must cover:

- `[dependencies]`
- target-specific non-dev dependency tables
- `workspace = true` indirection by resolving workspace package bindings

Manifest scanning must ignore:

- `[dev-dependencies]`
- test-only target dependency tables

- [ ] **Step 5: Implement external allowlist support**

Create
[`ci/plugin_semantic_boundary_allowlist.example.toml`](/Users/andreyyantsen/Development/uptrakit/ci/plugin_semantic_boundary_allowlist.example.toml)
documenting the approved shape:

- `rule_id`
- `path`
- `match_kind`
- `match_value`
- `reason`

The real allowlist file is optional and absent by default. Matching should be narrow and exact enough that one exception suppresses one known finding
class in one file.

- [ ] **Step 6: Prove parity against the current shell checker**

Run the Python helper and the current shell checker against the real repo and compare outputs. Until parity is reached for the overlapping legacy
rules, do not cut over the shell entrypoint.

Parity here means:

- every finding emitted by the current shell checker must also be emitted by the Python checker for the overlapping legacy rule set after excluding
  boundary-owner paths that Track C intentionally removes from scope
- the Python checker may emit additional findings because its scope is broader

Because the current shell checker fails fast, add a temporary migration-only `UPTRAKIT_SEMANTIC_BOUNDARY_REPORT_ALL=1` mode inside
[`ci/check_plugin_semantic_boundary.sh`](/Users/andreyyantsen/Development/uptrakit/ci/check_plugin_semantic_boundary.sh). Implement it by refactoring
the current direct-`exit 1` helpers into collector-aware helpers that append findings to a shared buffer in report-all mode and preserve the existing
fail-fast behavior by default.

The overlapping parity surface is the current shell checker’s `crates/`-scoped legacy rule set. Frontend findings emitted only by the Python checker
are out of scope for parity comparison.

Run:

```bash
python3 -m unittest discover -s ci -p 'test_check_plugin_semantic_boundary.py' -v
python3 ci/check_plugin_semantic_boundary.py
UPTRAKIT_SEMANTIC_BOUNDARY_REPORT_ALL=1 bash ci/check_plugin_semantic_boundary.sh
```

Expected:

- unit tests pass
- Python helper reports at least the known real-repo residue
- shell checker still enforces the current narrower rules

Update the inventory doc with the exact findings now emitted by the Python helper.

---

### Task 4: Backend Consumer And Manifest Cleanup Using The Existing Registry Surface

**Files:**

- Modify: `crates/shared/agent-core/Cargo.toml`
- Modify: `crates/shared/scheduler-engine/Cargo.toml`
- Modify: `crates/core/agent-ssh/Cargo.toml`
- Modify: `crates/plugins/infrastructure/registry/src/lib.rs` only if a real symbol is missing
- Modify: `crates/shared/agent-core/src/client.rs`
- Modify: `crates/shared/agent-core/src/config_test.rs` because it is a confirmed production module despite the filename
- Modify: `crates/shared/agent-core/src/update.rs`
- Modify: `crates/shared/agent-core/src/version_check.rs`
- Modify: `crates/shared/scheduler-engine/src/executors/fetch_releases.rs`
- Modify: `crates/core/agent-ssh/src/runtime_support.rs`
- Modify: `crates/core/agent-ssh/src/operations/bootstrap.rs`

- [ ] **Step 1: Migrate code imports from core to registry**

Before editing imports, audit the symbols used by the target files against the current registry surface so the migration starts from facts, not
speculative re-export work.

Replace direct `uptrakit_plugin_infrastructure_core` imports in the listed production files with registry-qualified imports from
[`uptrakit-plugin-infrastructure-registry`](/Users/andreyyantsen/Development/uptrakit/crates/plugins/infrastructure/registry).

Use the existing crate-root re-exports first. Only if a genuinely required symbol is missing should
[`registry/src/lib.rs`](/Users/andreyyantsen/Development/uptrakit/crates/plugins/infrastructure/registry/src/lib.rs) gain a narrow additive re-export,
and any `agent_infra` symbol must remain feature-gated.

- [ ] **Step 2: Remove manifest-level direct core deps**

After production code imports compile against the registry surface, remove `uptrakit-plugin-infrastructure-core` from:

- [`crates/shared/agent-core/Cargo.toml`](/Users/andreyyantsen/Development/uptrakit/crates/shared/agent-core/Cargo.toml)
- [`crates/shared/scheduler-engine/Cargo.toml`](/Users/andreyyantsen/Development/uptrakit/crates/shared/scheduler-engine/Cargo.toml)
- [`crates/core/agent-ssh/Cargo.toml`](/Users/andreyyantsen/Development/uptrakit/crates/core/agent-ssh/Cargo.toml)

If any remaining `uptrakit_plugin_infrastructure_core` references are test-only after the production migration, either migrate those tests too or move
the crate to `[dev-dependencies]` only. Do not leave it in production `[dependencies]`.

- [ ] **Step 3: Do not perform stale cleanup**

Do not add work for [`connection_context.rs`](/Users/andreyyantsen/Development/uptrakit/crates/shared/agent-core/src/connection_context.rs) unless the
checker finds a production violation there. Its current `plugin_ids` usage is test-only.

- [ ] **Step 4: Run focused backend verification**

Run:

```bash
cargo fmt --all -- --check
cargo check --no-default-features --features db-sqlite
cargo check --all-features
python3 ci/check_plugin_semantic_boundary.py
```

Expected:

- `cargo fmt` passes
- both `cargo check` invocations pass
- the direct core import/dependency findings for the migrated files disappear
- any remaining findings are inventoried

Update the inventory doc by moving resolved files into a `Resolved` section.

---

### Task 5: Generic Frontend Fix, CI Wiring, And Local Hook Integration

**Files:**

- Modify: `crates/plugins/infrastructure/core/src/plugin_ops.rs`
- Modify: `crates/plugins/infrastructure/core/src/catalog.rs`
- Modify: `crates/core/controller/src/main.rs`
- Modify: `crates/ui/web-api/src/extension_registry.rs`
- Modify: `crates/ui/web-api/src/routes/extensions.rs`
- Modify: `crates/ui/web-api/src/routes/service_ws/handler/messages.rs`
- Modify: `crates/shared/web-api-types/src/extensions.rs`
- Modify: `frontend/src/lib/types.ts`
- Modify: `frontend/src/lib/extensions.svelte.ts`
- Modify: `frontend/src/routes/software/[id]/+page.svelte`
- Modify: `.husky/pre-push`
- Modify: `.github/workflows/ci.yml`
- Modify: `docs/development/quality-gates.md`
- Modify: `docs/development/setup.md`

- [ ] **Step 1: Add owner plugin metadata to extension responses**

Take the trait-signature-change path explicitly. Extend the plugin extension path so each plugin-backed extension response carries its owning plugin
type ID. The exact shape may be `owner_plugin_type: Option<String>` or an equivalent field, but the data flow must be concrete:

- `PluginExtensionOps::extension_manifests_and_actions()` returns the owning `PluginTypeId` together with each plugin-backed manifest/action bundle
- `PluginCatalog` populates that owner from each descriptor when collecting extension manifests
- controller startup and any mock/test implementations updated for the new trait shape continue to compile
- `ExtensionRegistry` stores the owner on plugin-backed resolved entries
- the list-extensions route serializes it into the HTTP response type
- the frontend TS type exposes it to callers

`PluginCatalog` is the only non-test implementor, so this trait-shape change is acceptable. Service-provided extensions should continue to have no
plugin owner.

- [ ] **Step 2: Replace the hardcoded Docker gate with generic owner-based filtering**

In [`frontend/src/routes/software/[id]/+page.svelte`](/Users/andreyyantsen/Development/uptrakit/frontend/src/routes/software/[id]/+page.svelte), stop
checking for `'releases_docker'`.

Instead:

- collect host-row context-menu extensions
- filter them generically with an explicit predicate equivalent to
  `!ext.owner_plugin_type || host.plugins.some((p) => p.plugin_type === ext.owner_plugin_type)`
- show only extensions whose `owner_plugin_type` is present in the row’s plugin set, or that are otherwise explicitly row-applicable

Do not replace the current code with `getContextMenuExtensions('software-item-host').length > 0`; that would surface Docker actions for non-Docker
rows.

- [ ] **Step 3: Wire the semantic gate into pre-push**

Update [`/.husky/pre-push`](/Users/andreyyantsen/Development/uptrakit/.husky/pre-push) to invoke:

```sh
echo "[pre-push] Running plugin semantic boundary check..."
bash ci/check_plugin_semantic_boundary.sh
```

Keep the rule logic in the shared script only. This step is additive: preserve the existing fmt, markdownlint, cargo, sentrux, and frontend checks
already in the hook.

- [ ] **Step 4: Wire the semantic gate into CI**

Update [`/.github/workflows/ci.yml`](/Users/andreyyantsen/Development/uptrakit/.github/workflows/ci.yml) so the semantic-boundary check runs as a
blocking CI job or named step. Use the shared shell entrypoint:

```yaml
- run: bash ci/check_plugin_semantic_boundary.sh
```

- [ ] **Step 5: Update maintainer docs**

Update [`docs/development/quality-gates.md`](/Users/andreyyantsen/Development/uptrakit/docs/development/quality-gates.md) and
[`docs/development/setup.md`](/Users/andreyyantsen/Development/uptrakit/docs/development/setup.md) so they explicitly document:

- the semantic-boundary gate
- that it runs from `pre-push`
- that docs/tests/examples/migrations are excluded from enforcement
- that production code is not exempt

- [ ] **Step 6: Run focused frontend + integration verification**

Run:

```bash
bash ci/check_plugin_semantic_boundary.sh
(cd frontend && npm run lint)
(cd frontend && npm run format:check)
(cd frontend && npm run check)
(cd frontend && npm run test)
cargo check --no-default-features --features db-sqlite
markdownlint --config .markdownlint.json 'docs/development/quality-gates.md' 'docs/development/setup.md'
```

Expected:

- boundary gate passes or only reports still-inventoried production residue
- frontend checks/tests pass
- docs lint passes

---

### Task 6: Cutover, Final Inventory Closeout, And Full Verification

**Files:**

- Modify: `ci/check_plugin_semantic_boundary.sh`
- Modify: `ci/check_plugin_semantic_boundary.py`
- Modify: `docs/internal/changes/TASK-0007/ST-0032-track-c-production-semantic-inventory.md`

- [ ] **Step 1: Cut the shell entrypoint over to Python**

Once Task 3 parity is proven and Tasks 4-5 cleanup is in place, simplify
[`ci/check_plugin_semantic_boundary.sh`](/Users/andreyyantsen/Development/uptrakit/ci/check_plugin_semantic_boundary.sh) to a thin stable wrapper
around the Python helper. Do this only after the parity comparison and real-repo validation pass.

Keep the shell script’s in-code allowlist arrays empty throughout this migration. Any temporary suppression needed during parity work must use the
TOML allowlist mechanism instead of shell-only exceptions so cutover does not drop hidden allowlist state.

- [ ] **Step 2: Run the real checker and close the inventory**

Run:

```bash
python3 ci/check_plugin_semantic_boundary.py
```

If any findings remain, resolve them and rerun until clean. End the inventory doc with:

```md
## Remaining

- None.
```

- [ ] **Step 3: Keep the real allowlist absent**

Verify:

```bash
test ! -f ci/plugin_semantic_boundary_allowlist.toml
```

Expected: exit `0`.

- [ ] **Step 4: Run the full verification suite**

Run:

```bash
python3 -m unittest discover -s ci -p 'test_check_plugin_semantic_boundary.py' -v
bash ci/check_plugin_semantic_boundary.sh
cargo fmt --all -- --check
markdownlint --config .markdownlint.json '**/*.md'
cargo deny check
cargo check --no-default-features --features db-sqlite
cargo clippy --all-targets --no-default-features --features db-sqlite -- -D warnings
cargo test --no-default-features --features db-sqlite
(cd frontend && npm run lint)
(cd frontend && npm run format:check)
(cd frontend && npm run check)
(cd frontend && npm run test)
(cd frontend && npm run build)
cargo check --all-features
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

Expected: all commands exit `0`, and the semantic-boundary gate reports no unallowlisted production violations.

---

## Self-Review

- Spec alignment: this plan matches the spec’s zero-tolerance production gate, explicit exclusions, external optional allowlist, registry-only
  production boundary, CI integration, and pre-push integration.
- Reviewer fixes applied: the plan no longer drops existing shell enforcement prematurely, no longer invents a redundant `consumer_api.rs`, includes
  Cargo manifest cleanup, includes CI wiring, uses canonical plugin IDs from `plugin_ids::ALL`, expands fixture/test coverage, and avoids the unsafe
  frontend `hostExtensions.length > 0` regression.
- Risk posture: the highest implementation risk remains the syntax-aware literal scanner and the extension-owner metadata path for frontend filtering,
  both of which are now called out explicitly rather than hidden behind oversimplified steps.

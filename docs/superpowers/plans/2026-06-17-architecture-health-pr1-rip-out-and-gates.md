# Architecture-Health PR 1 — Sentrux Rip-Out + Durable Cargo Gates Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement
> this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove every Sentrux touchpoint and replace its enforced role with two durable, team-controlled cargo gates (`cargo-modules --acyclic` for
module cycles, `cargo-machete` for unused deps) wired into CI and pre-push, both auto-discovering the workspace.

**Architecture:** A single shell helper (`ci/arch_lib_members.sh`) derives the lib-bearing crate allowlist from `cargo metadata` so both CI and the
pre-push hook stay zero-maintenance as crates are added. Tools install in CI via the repo's existing `taiki-e/install-action@v2` pattern (never
`cargo binstall`). Pre-push gates hard-fail if a tool is missing, matching the existing `markdownlint` gate — the opposite of Sentrux's silent skip.

**Tech Stack:** GitHub Actions YAML, Husky bash hooks, `cargo metadata` + `jq`, `cargo-modules` 0.26.0, `cargo-machete` 0.9.2, markdown docs.

**Spec:** `docs/superpowers/specs/2026-06-17-architecture-health-tooling-design.md` (PR 1 scope of the Implementation sequencing section).

**Snapshot rules in scope** (`.superpowers/standards-snapshot.md`): conventional commits required for PRs; markdownlint line_length 150;
quality-gates pre-push/CI tier model; husky-rs hooks (`NO_HUSKY_HOOKS=1` disables); no new workspace `[dependencies]`.

---

## File structure

| File                                           | Responsibility                                                                       | Action |
| ---------------------------------------------- | ------------------------------------------------------------------------------------ | ------ |
| `ci/arch_lib_members.sh`                       | Print lib-bearing workspace crate names (allowlist), one per line                    | Create |
| `.github/workflows/ci.yml`                     | Add `architecture:` job; delete `sentrux:` job                                       | Modify |
| `.husky/pre-push`                              | Replace Sentrux block with hard-fail acyclic + machete gates                         | Modify |
| `.sentrux/rules.toml`                          | Sentrux rule config (68 KB)                                                          | Delete |
| `docs/development/quality-gates.md`            | Replace Sentrux rows/section with new gates + tool reference + zero-maintenance rule | Modify |
| `AGENTS.md`                                    | Replace Sentrux MCP session-start block                                              | Modify |
| `CODEREVIEW.md`, `crates/ui/cli/CODEREVIEW.md` | Reword Sentrux-metric references                                                     | Modify |
| `docs/adr/00NN-architecture-health-tooling.md` | Record the decision                                                                  | Create |

---

## Task 1: Lib-bearing allowlist helper

**Files:**

- Create: `ci/arch_lib_members.sh`

- [ ] **Step 1: Confirm `cargo-modules` flags exist as assumed**

Install locally and inspect the CLI (this is the "verify it fails / verify assumptions" step):

```bash
cargo install cargo-modules --version 0.26.0 --locked
cargo modules dependencies --help | grep -E -- '--acyclic|--lib|--package|--all-features|--workspace|--no-default-features'
```

Expected: `--acyclic`, `--lib`, `--package`, `--all-features` present. If any flag name differs in 0.26.0, record the actual name and use it
everywhere below. **Also check for a workspace-wide mode:** if `--workspace` (or any single-invocation-over-all-crates mode) exists, prefer it over
the per-crate loop in Tasks 2-4 — one analysis pass is far cheaper than ~70. If no such flag exists, the per-crate loop stands (and Task 2 Step 1
times it).

- [ ] **Step 2: Write the helper script**

Derive the allowlist from `cargo metadata` — never hardcode crate names. A crate is "lib-bearing" if it has a target of kind `lib`, `rlib`, or
`proc-macro`. The six bin-only core crates fall out automatically. **Note:** `uptrakit-frontend` IS a lib crate (it embeds built frontend assets via
a `build.rs`) so it appears in the allowlist — that is correct and it is analyzed. The optional `SKIP_CRATES` list below exists only for crates that
cannot be analyzed by `cargo-modules` without external build artifacts (e.g. if `uptrakit-frontend`'s `build.rs` needs `frontend/build` present);
leave it empty unless Task 2 proves a crate cannot compile under the gate — that is the single documented exception.

```bash
#!/usr/bin/env bash
# Print lib-bearing workspace member crate names, one per line.
# Used by CI (architecture: job) and .husky/pre-push so the cycle gate
# auto-discovers crates — adding a new lib crate needs no edit here.
set -euo pipefail

# Crates that cannot be analyzed by cargo-modules without external build
# artifacts. Keep empty unless a crate provably fails the gate at compile time
# (see Task 2). Space-separated exact package names.
SKIP_CRATES="${SKIP_CRATES:-}"

command -v jq >/dev/null 2>&1 || { echo "arch_lib_members: jq is required" >&2; exit 1; }

cargo metadata --no-deps --format-version 1 \
  | jq -r '
      .packages[]
      | select(any(.targets[];
          (.kind | index("lib")) or
          (.kind | index("rlib")) or
          (.kind | index("proc-macro"))))
      | .name
    ' \
  | { if [ -n "$SKIP_CRATES" ]; then grep -vxF -f <(printf '%s\n' $SKIP_CRATES); else cat; fi; } \
  | sort
```

- [ ] **Step 3: Make it executable and run it**

```bash
chmod +x ci/arch_lib_members.sh
./ci/arch_lib_members.sh
```

Expected: a sorted list of crate names. It MUST include known lib crates (e.g. `uptrakit-shared-types`, `uptrakit-wire`) and MUST NOT include the
six bin-only core crates (`controller`, `agent`, `agent-ssh`, `scheduler`, `mqtt`, `controller-standalone`). `uptrakit-frontend`,
`uptrakit-functional-tests`, and `uptrakit-integration-tests` WILL appear (they are real lib crates) — Task 2 confirms they analyze cleanly.

- [ ] **Step 4: Add a runnable self-check**

Create the check inline and run it (no framework — a bash assert). It asserts the six bin-only crates are excluded — it does NOT assert
`uptrakit-frontend` is excluded, because that crate is a real lib and is legitimately in the allowlist:

```bash
out="$(./ci/arch_lib_members.sh)"
echo "$out" | grep -qx uptrakit-wire || { echo "FAIL: expected uptrakit-wire in allowlist"; exit 1; }
for binonly in controller agent agent-ssh scheduler mqtt controller-standalone; do
  echo "$out" | grep -qx "$binonly" && { echo "FAIL: bin-only '$binonly' must be excluded"; exit 1; }
done
echo "OK: all six bin-only crates excluded"
```

Expected: `OK: all six bin-only crates excluded`. If `uptrakit-wire` is not the exact package name, substitute a real lib crate name from Step 3
output.

- [ ] **Step 5: Commit**

```bash
git add ci/arch_lib_members.sh
git commit -m "ci(architecture): add lib-bearing crate allowlist helper"
```

---

## Task 2: Establish a green baseline for the two gates

This task does NOT change CI — it confirms the gates pass today (or makes them pass) so they can ship blocking. Per spec graduation rule: ship
blocking if already clean; otherwise fix in-PR.

**Files:** none created; possibly `crates/*/Cargo.toml` metadata edits if machete reports false positives, or a `SKIP_CRATES` entry in
`ci/arch_lib_members.sh` if a crate cannot compile under the gate.

- [ ] **Step 1: Run the cycle gate across the allowlist**

These `cargo install` lines are **local-only** for this baseline check — CI installs via `taiki-e/install-action` (Task 3). Never copy
`cargo install` into a workflow file.

```bash
cargo install cargo-modules --version 0.26.0 --locked
# warm the build first so the timing below reflects steady-state, not a cold compile:
cargo check --workspace --all-features >/dev/null 2>&1 || true
fail=0
start=$SECONDS
while IFS= read -r pkg; do
  echo "== $pkg =="
  cargo modules dependencies --package "$pkg" --lib --acyclic --all-features >/dev/null || fail=1
done < <(./ci/arch_lib_members.sh)
echo "cycle-gate exit: $fail  wall-clock: $((SECONDS - start))s"
```

Expected: `cycle-gate exit: 0`. **Record the wall-clock.** Per-crate `cargo modules` runs a compiler front-end pass per crate (~70 crates); if a
workspace-wide mode was found in Task 1 Step 1, use it instead and this concern disappears. Pre-push latency budget: if the warm wall-clock exceeds
**~60s**, do NOT run the full loop in pre-push — in Task 4 run the gate **CI-only** and have pre-push print an advisory pointer
(`echo "[pre-push] architecture gate runs in CI"`), since a slow pre-push is what drives developers to `NO_HUSKY_HOOKS=1` and silently defeats the
gate. Record the chosen tier (pre-push vs CI-only) in the ADR.

Two distinct failure modes:

- **A module cycle** (cargo-modules reports an above-diagonal edge) → fix the cycle in this PR (default). If large, record it and land the gate
  advisory (drop `|| fail=1`, print a warning).
- **A crate fails to COMPILE** under cargo-modules (most likely `uptrakit-frontend`, whose `build.rs` may need `frontend/build` assets, or a
  test-harness crate needing infra) → add that exact package name to `SKIP_CRATES` in `ci/arch_lib_members.sh` with a one-line comment naming why.
  This is the single documented allowlist exception; do NOT skip a crate that merely has a cycle.

- [ ] **Step 2: Install and run the unused-deps gate**

`cargo install` here is **local-only** (see Step 1); CI uses `taiki-e/install-action`.

```bash
cargo install cargo-machete --version 0.9.2 --locked
cargo machete
```

Expected: exit 0, "Done!". If it reports unused deps:

- If genuinely unused → remove them from that crate's `Cargo.toml` (real cleanup).
- If a **false positive** (dep used only via macro re-export or under one feature) → add to that crate's `Cargo.toml`:

  ```toml
  [package.metadata.cargo-machete]
  ignored = ["the-crate-name"]
  ```

  (This is the per-crate ignore mechanism — there is no `machete.toml`. Metadata is not a `[dependencies]` entry, so it does not engage the
  `deny.toml` allowlist.)

- [ ] **Step 3: Re-run both gates clean, then commit any cleanup**

```bash
cargo machete && echo "machete clean"
# Stage only the files this task could have touched (Cargo.toml metadata + the helper). Never `git add -A`.
changed=$(git diff --name-only -- '**/Cargo.toml' ci/arch_lib_members.sh)
[ -n "$changed" ] && git commit --only $changed -m "chore(deps): resolve cargo-machete findings for architecture gate" || echo "nothing to commit"
```

Expected: `machete clean`. If no cleanup was needed, the commit is skipped — fine.

---

## Task 3: Add the `architecture:` CI job

**Files:**

- Modify: `.github/workflows/ci.yml` (add a new job near the existing `backend-deny:` job, ~line 39; do NOT touch the `sentrux:` job yet — Task 5
  removes it)

- [ ] **Step 1: Add the job**

Mirror the `backend-deny:` job's install pattern (`taiki-e/install-action@v2`). The `tool:` list is the single source of truth for pinned versions.

```yaml
architecture:
  runs-on:
    - ubuntu-latest
  steps:
    - uses: actions/checkout@v6
    - uses: dtolnay/rust-toolchain@stable
    - uses: Swatinem/rust-cache@v2
      with:
        shared-key: "rust-all-features"
        save-if: "false"
    - uses: taiki-e/install-action@v2
      with:
        tool: cargo-modules@0.26.0,cargo-machete@0.9.2
    - name: Module acyclicity (per lib-bearing crate)
      shell: bash
      run: |
        fail=0
        while IFS= read -r pkg; do
          echo "== $pkg =="
          cargo modules dependencies --package "$pkg" --lib --acyclic --all-features >/dev/null || fail=1
        done < <(bash ci/arch_lib_members.sh)
        exit $fail
    - name: Unused dependencies
      run: cargo machete
```

`shell: bash` is explicit on the loop step because `< <(…)` is a bashism — GitHub's Ubuntu runners default to bash, but stating it removes any
ambiguity for the process substitution.

- [ ] **Step 2: Validate the YAML and the job logic locally**

```bash
python3 -c "import yaml,sys; yaml.safe_load(open('.github/workflows/ci.yml')); print('yaml ok')"
# re-run the same loop the job runs, to confirm green:
fail=0; while IFS= read -r p; do cargo modules dependencies --package "$p" --lib --acyclic --all-features >/dev/null || fail=1; done < <(bash ci/arch_lib_members.sh); echo "exit $fail"
cargo machete
```

Expected: `yaml ok`, loop `exit 0`, machete clean.

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/ci.yml
git commit -m "ci(architecture): add cargo-modules acyclic + cargo-machete gates"
```

---

## Task 4: Add the pre-push gate (hard-fail)

**Files:**

- Modify: `.husky/pre-push` (replace the `# --- Sentrux architectural rules ---` block, currently ~lines 74-82)

- [ ] **Step 1: Replace the Sentrux block**

**First check the Task 2 Step 1 timing decision.** If the warm cycle-gate loop exceeded the ~60s budget (and no cheaper workspace-wide mode exists),
do NOT put the per-crate loop in pre-push — instead replace the Sentrux block with `cargo machete` only plus
`echo "[pre-push] module-cycle gate runs in CI"`, and skip adding the acyclic loop below. Otherwise (loop is fast) use the full replacement below.

Hard-fail if a tool is absent — matching the existing `markdownlint` gate (`command -v … || { …; exit 1; }`), NOT Sentrux's skip-if-absent.

Replace:

```bash
# --- Sentrux architectural rules ---
if command -v sentrux >/dev/null 2>&1; then
  echo "[pre-push] Running sentrux check (advisory)..."
  sentrux check . || echo "[pre-push] sentrux check failed — advisory only, not blocking push."
else
  echo "[pre-push] sentrux not found, skipping architectural check."
  echo "[pre-push] Install: curl -fsSL https://raw.githubusercontent.com/sentrux/sentrux/main/install.sh | sh"
fi
```

with:

```bash
# --- Architecture gates (blocking) ---
echo "[pre-push] Running module acyclicity check..."
command -v cargo-modules >/dev/null 2>&1 || { echo "[pre-push] cargo-modules required: cargo install cargo-modules --version 0.26.0 --locked" >&2; exit 1; }
while IFS= read -r pkg; do
  cargo modules dependencies --package "$pkg" --lib --acyclic --all-features >/dev/null || { echo "[pre-push] module cycle in $pkg" >&2; exit 1; }
done < <(bash ci/arch_lib_members.sh)

echo "[pre-push] Running cargo machete..."
command -v cargo-machete >/dev/null 2>&1 || { echo "[pre-push] cargo-machete required: cargo install cargo-machete --version 0.9.2 --locked" >&2; exit 1; }
cargo machete
```

- [ ] **Step 2: Run the hook body directly to confirm it passes**

```bash
bash -c '
set -euo pipefail
while IFS= read -r pkg; do
  cargo modules dependencies --package "$pkg" --lib --acyclic --all-features >/dev/null || { echo "cycle in $pkg"; exit 1; }
done < <(bash ci/arch_lib_members.sh)
cargo machete
echo "pre-push arch gates OK"
'
```

Expected: `pre-push arch gates OK`.

- [ ] **Step 3: Commit**

```bash
git add .husky/pre-push
git commit -m "ci(pre-push): hard-fail architecture gates, drop sentrux"
```

---

## Task 5: Remove all remaining Sentrux touchpoints

**Files:**

- Modify: `.github/workflows/ci.yml` (delete `sentrux:` job, ~lines 156-163)
- Delete: `.sentrux/rules.toml`
- Modify: `AGENTS.md` (session-start MCP block, ~lines 318-333)
- Modify: `CODEREVIEW.md`, `crates/ui/cli/CODEREVIEW.md`

- [ ] **Step 1: Delete the `sentrux:` CI job**

Remove this block from `.github/workflows/ci.yml`:

```yaml
sentrux:
  runs-on:
    - ubuntu-latest
  steps:
    - uses: actions/checkout@v6
    - name: Install sentrux
      run: curl -fsSL https://raw.githubusercontent.com/sentrux/sentrux/main/install.sh | sh
    - run: sentrux check . || true
```

- [ ] **Step 2: Delete the Sentrux rule config**

```bash
git rm .sentrux/rules.toml
rmdir .sentrux 2>/dev/null || true
```

- [ ] **Step 3: Replace the AGENTS.md session-start block**

The block at ~lines 318-333 instructs sessions to call `mcp__plugin_sentrux_sentrux__*` tools (plugin already uninstalled — dead instructions).
Replace the Sentrux-MCP instructions with a pointer to the new gates. Open `AGENTS.md`, find the section starting
`- **At the start of every session**, call \`mcp**plugin_sentrux_sentrux**scan\`` and replace that bullet group with:

```markdown
- **Architecture is enforced in CI and pre-push**, not via an MCP tool. The blocking gates are `cargo modules dependencies --lib --acyclic` (per
  lib-bearing crate, via `ci/arch_lib_members.sh`) and `cargo machete`. Run them locally before pushing; see `docs/development/quality-gates.md`.
- Behavioral health (hotspots, change-coupling, code-health grade) lives in the **CodeScene** dashboard (advisory, not a gate).
```

- [ ] **Step 4: Reword CODEREVIEW.md Sentrux references**

These are point-in-time review reports. Reword live-tool references so no stale tool name remains. In `CODEREVIEW.md` and
`crates/ui/cli/CODEREVIEW.md`, replace `Sentrux` mentions with the equivalent live concept:

- `Sentrux metrics` / `Sentrux Snapshot` → `structural metrics` / `Structural Snapshot`
- `per Sentrux` / `Sentrux reports …` → `per cargo-modules` (for cycle/acyclicity claims) or `structural analysis` (for coupling/size heuristics)
- `Sentrux modularity score drag` → `modularity drag`

Keep the numbers; only the tool attribution changes.

- [ ] **Step 5: Verify completeness — no Sentrux refs remain outside spec/plans**

```bash
git grep -il sentrux -- ':!docs/superpowers/*'
```

Expected: **no output**. Any hit must be cleaned before committing.

- [ ] **Step 6: Confirm CI YAML still valid and commit**

```bash
python3 -c "import yaml; yaml.safe_load(open('.github/workflows/ci.yml')); print('yaml ok')"
git rm .sentrux/rules.toml 2>/dev/null || true
git add .github/workflows/ci.yml AGENTS.md CODEREVIEW.md crates/ui/cli/CODEREVIEW.md
git commit -m "chore: remove abandoned Sentrux (CI job, rules, MCP refs, review notes)"
```

Expected: `yaml ok`. (Explicit paths — never `git add -A` in an agent context, per repo feedback rule.)

---

## Task 6: Update `quality-gates.md`

**Files:**

- Modify: `docs/development/quality-gates.md` (pre-push table ~line 28; "Architectural rules (sentrux)" section ~lines 209-225)

- [ ] **Step 1: Replace the pre-push table row**

Replace:

```markdown
| `sentrux check .` | Skipped gracefully if `sentrux` is not installed |
```

with two rows (each on its own line — do not let a formatter join them):

```markdown
| `cargo modules … --lib --acyclic` (per lib-bearing crate) | Blocking; crate list from `ci/arch_lib_members.sh`; hard-fails if `cargo-modules`
absent | | `cargo machete` | Blocking; unused-dependency check; hard-fails if `cargo-machete` absent |
```

(markdownlint exempts table rows from the 150-char line limit, so these long rows are fine.)

- [ ] **Step 2: Replace the "Architectural rules (sentrux)" section**

Replace the entire `## Architectural rules (sentrux)` section (the `sentrux check .` / `.sentrux/rules.toml` / install-via-curl prose) with:

```markdown
## Architecture health

Enforced, blocking gates (CI `architecture:` job + pre-push):

- **Module cycles** — `cargo modules dependencies --package <crate> --lib --acyclic --all-features`, run per lib-bearing crate. The crate list is
  derived from `cargo metadata` by `ci/arch_lib_members.sh`, so **adding a new crate needs no edit** — a new lib crate is picked up automatically;
  bin-only crates and `frontend` are excluded automatically.
- **Unused dependencies** — `cargo machete`. False positives are silenced per-crate via `[package.metadata.cargo-machete] ignored = [...]` in that
  crate's `Cargo.toml` (the only tolerated manual step in the stack).
- **Plugin boundary** — `python3 ci/check_plugin_semantic_boundary.py` (unchanged).
- **Licenses / advisories / bans** — `cargo deny check` (unchanged).

Tool versions are pinned in the `architecture:` job's `taiki-e/install-action` `tool:` list (the single source of truth). Install locally with
`cargo install <tool> --locked`.

**Rule-author rule:** any boundary rule (plugin checker, or a future enforcer) must be a path/layer glob (e.g.
`crates/plugins/** must not import crates/core/**`), **never** a per-crate enumeration — per-crate lists reintroduce maintenance the stack forbids.

Behavioral health (code-health grade, hotspots, change-coupling) is advisory and lives in the **CodeScene** dashboard — see PR 2.
```

- [ ] **Step 3: Add local-install instructions to `setup.md`**

Contributors need the new pre-push tools installed. In `docs/development/setup.md`, find the pre-commit/pre-push hooks section and add:

````markdown
The pre-push architecture gates need two cargo tools. Install the versions pinned in the `architecture:` CI job's `taiki-e/install-action` `tool:`
list (that list is the single source of truth — do not duplicate version numbers here):

> `cargo install cargo-modules --locked` and `cargo install cargo-machete --locked` (append `--version <X>` to match the CI-pinned version exactly;
> CI is authoritative, local installs are best-effort).

(Write the two install commands as a real fenced `sh` code block; the version literals stay only in the CI YAML, never here, to avoid a drift trap.)

- [ ] **Step 4: Lint and commit**

```bash
npx prettier --prose-wrap always --print-width 148 --write docs/development/quality-gates.md docs/development/setup.md
markdownlint --config .markdownlint.json docs/development/quality-gates.md docs/development/setup.md
git add docs/development/quality-gates.md docs/development/setup.md
git commit -m "docs: document cargo architecture gates in quality-gates + setup"
```
````

Expected: markdownlint exits clean (line_length 150 per snapshot).

---

## Task 7: Add the ADR

**Files:**

- Create: `docs/adr/00NN-architecture-health-tooling.md`

- [ ] **Step 1: Compute the next free ADR number**

```bash
next=$(printf '%04d' $(( $(ls docs/adr | grep -oE '^[0-9]{4}' | sort -n | tail -1 | sed 's/^0*//') + 1 )))
echo "computed next ADR: $next"
ls docs/adr/0021-* 2>/dev/null && echo "0021 exists on disk"
```

**Important — coordinate the number.** On disk today the highest ADR is `0020`, so the formula computes `0021`. But the in-flight
skills-version-display work (`docs/superpowers/plans/2026-06-17-skills-version-display.md`, Task 14) creates
`0021-installed-version-enrichment-role.md` and may not be merged yet. To avoid a collision, use **`0022`** for this ADR regardless of the formula
(the spec already names `0022`). If by implementation time `0022` is also taken, use the next free number and update the spec's reference. Set
`next=0022` before Step 2 unless `0022` already exists.

- [ ] **Step 2: Write the ADR**

Create `docs/adr/<next>-architecture-health-tooling.md` following the format of existing ADRs (e.g. `docs/adr/0020-service-merge-redirect.md` —
match its heading structure):

```markdown
# <next>. Architecture-health tooling: hybrid cargo gates + advisory CodeScene

Date: 2026-06-17

## Status

Accepted

## Context

Sentrux (the previous architecture-health tool) is abandoned — its CI job installed via `curl … | sh` from an unmaintained repo, a supply-chain
liability. We still want its governance: module-cycle prevention, unused-dependency detection, coverage, and behavioral health (hotspots, coupling).
No single Rust tool reproduces it.

## Decision

Hybrid stack. Enforced, blocking gates run on mature, team-controlled cargo tooling so an upstream abandonment cannot take the gates down again:

- `cargo modules --acyclic` — intra-crate module cycles (per lib-bearing crate, auto-discovered).
- `cargo machete` — unused dependencies.
- `cargo llvm-cov` → Codecov + CodeScene — coverage (PR 2).
- Existing `cargo deny` and `ci/check_plugin_semantic_boundary.py` are unchanged.

Behavioral health (code-health grade, hotspots, change/temporal coupling) uses **CodeScene** (SaaS, free for OSS) strictly **advisory** — if it is
abandoned or repriced, enforcement is untouched.

Static **Dependency Structure Matrix** and **afferent/efferent coupling** are deferred: no turnkey Rust tool exists and they are derivable only via
custom code. `cargo modules --acyclic` governs intra-crate cycles only — crate-DAG layering is the plugin boundary checker's job, not this gate's.

## Consequences

- Adding a crate needs zero gate-config edits (allowlist derived from `cargo metadata`).
- The only recurring manual step is a rare per-crate `cargo-machete` ignore entry.
- A second custom-rule enforcer (cargo-pup / dylint) is deferred until a concrete rule needs it.
```

- [ ] **Step 3: Lint and commit**

```bash
npx prettier --prose-wrap always --print-width 148 --write docs/adr/<next>-architecture-health-tooling.md
markdownlint --config .markdownlint.json docs/adr/<next>-architecture-health-tooling.md
git add docs/adr/<next>-architecture-health-tooling.md
git commit -m "docs(adr): record architecture-health tooling decision"
```

---

## Self-review checklist (run before handing off)

- [ ] `git grep -il sentrux -- ':!docs/superpowers/*'` returns nothing.
- [ ] `ci/arch_lib_members.sh` excludes the 6 bin-only crates + `frontend`, includes real lib crates.
- [ ] Both gates pass locally (`exit 0`, machete clean).
- [ ] `.github/workflows/ci.yml` parses as YAML; `architecture:` present, `sentrux:` gone.
- [ ] All new/changed markdown passes `markdownlint --config .markdownlint.json`.
- [ ] Every commit uses Conventional Commits (snapshot rule).

# Architecture-Health PR 2 — Coverage + CodeScene Health Layer Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement
> this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Produce workspace test coverage with `cargo-llvm-cov` by replacing the existing `cargo test` step (one instrumented compile for unit +
integration tests, plus a separate doctest compile), upload the one `lcov.info` to **both Codecov and CodeScene**, and stand up the advisory
CodeScene behavioral dashboard.

**Architecture:** The existing `backend-test:` CI job gains coverage — its `cargo test --all-features` step becomes
`cargo llvm-cov … --all-features` (one instrumented compile that runs unit + integration tests), plus a separate `cargo test --doc` step (doctests
can't share llvm-cov's instrumented artifacts on stable, so one extra doc compile — this preserves the doctest execution the old `cargo test` did).
The single `lcov.info` is consumed by two upload steps. The job's existing `save-if: "false"` cache config means the instrumented build cannot
poison the shared `rust-all-features` cache. CodeScene runs independently on source + git history; coverage upload just enriches its test-gap
dimension. Nothing here is a blocking gate.

**Tech Stack:** GitHub Actions YAML, `cargo-llvm-cov` 0.8.7, `codecov/codecov-action@v7` (7.0.0), CodeScene SaaS + its coverage import, markdown
docs.

**Spec:** `docs/superpowers/specs/2026-06-17-architecture-health-tooling-design.md` (PR 2 scope).

**Depends on:** PR 1 plan merged (Sentrux removed; `architecture:` job exists). This plan is independent of PR 1's gates and can be developed in
parallel, but should merge after.

**Snapshot rules in scope:** conventional commits; markdownlint line_length 150; quality-gates tier model; no new workspace `[dependencies]`
(cargo-llvm-cov is an installed binary).

**Prerequisite secrets:** `CODECOV_TOKEN` (already added by the user). A CodeScene access token/secret for coverage upload — confirm exact name
during Task 3.

---

## File structure

| File                                | Responsibility                                                         | Action            |
| ----------------------------------- | ---------------------------------------------------------------------- | ----------------- |
| `.github/workflows/ci.yml`          | `backend-test:` job: llvm-cov step + Codecov upload + CodeScene upload | Modify            |
| `README.md`                         | Codecov coverage badge; remove any Sentrux mention (if PR 1 missed it) | Modify            |
| `codecov.yml`                       | Optional non-default coverage config                                   | Create (optional) |
| `docs/development/quality-gates.md` | Document the coverage step + dual upload                               | Modify            |

---

## Task 1: Replace `cargo test` with `cargo llvm-cov` (+ separate doctest step)

**Files:**

- Modify: `.github/workflows/ci.yml` `backend-test:` job (~lines 91-105)

- [ ] **Step 1: Confirm `cargo-llvm-cov` invocation locally**

```bash
cargo install cargo-llvm-cov --version 0.8.7 --locked
cargo llvm-cov --workspace --all-features --lcov --output-path lcov.info
ls -l lcov.info && head -1 lcov.info
```

`cargo llvm-cov` **runs the full test suite under instrumentation** — it wraps `cargo test`, so swapping the `cargo test --all-features` step for it
PRESERVES the mandatory test gate (snapshot: `cargo test --all-features`); coverage is a byproduct. It also **auto-installs `llvm-tools-preview`**
on first run, so no separate `rustup component add` step is needed.

**Prove the gate is equivalent, not just "should be".** The existing step is `cargo test --all-features` with no `--workspace`; the new one adds
`--workspace`. In a virtual workspace these select the same members **only if `default-members` is unset**. Confirm and compare test counts:

```bash
grep -q 'default-members' Cargo.toml && echo "WARN: default-members set — --workspace may change the test set; reconcile flags" || echo "default-members unset: --workspace == default selection"
a=$(cargo test --all-features 2>&1 | grep -c 'test result: ok')
b=$(cargo llvm-cov --workspace --all-features 2>&1 | grep -c 'test result: ok')
echo "test-binary count: bare=$a llvm-cov=$b"
```

Expected: `default-members unset`, and `bare == llvm-cov`. If they differ, the swap changed the enforced gate — add/remove `--workspace` so the test
set matches before proceeding.

Expected: `lcov.info` exists and starts with an LCOV record (e.g. `SF:` or `TN:` line). Delete the local `lcov.info` afterward (`rm lcov.info`).

- [ ] **Step 2: Edit the job — install the tool and swap the test step**

The current job has `- run: cargo test --all-features` between the cache step and the two `verify_*` steps. Add the install step (after
`Swatinem/rust-cache`) and replace only the `cargo test` step. Leave the `verify_handler_state_contract.sh` and `verify_db_access_policy.py` steps
untouched.

Add after the `Swatinem/rust-cache@v2` step (no `rustup component add` — `cargo-llvm-cov` self-installs the component):

```yaml
- uses: taiki-e/install-action@v2
  with:
    tool: cargo-llvm-cov@0.8.7
```

Replace:

```yaml
- run: cargo test --all-features
```

with two steps — coverage (which runs the unit + integration tests), then doctests separately (doctests are NOT executed by `cargo llvm-cov` on
stable, so keep running them to preserve the existing `cargo test` behavior; they are simply excluded from the coverage report):

```yaml
- name: Test + coverage
  run: cargo llvm-cov --workspace --all-features --lcov --output-path lcov.info
- name: Doctests
  run: cargo test --workspace --all-features --doc
```

- [ ] **Step 3: Validate YAML and the verify steps remain**

```bash
python3 -c "import yaml; j=yaml.safe_load(open('.github/workflows/ci.yml')); s=j['jobs']['backend-test']['steps']; \
names=[x.get('run','')+x.get('name','') for x in s]; \
assert any('llvm-cov' in n for n in names), 'llvm-cov step missing'; \
assert any('--doc' in n for n in names), 'doctest step missing'; \
assert not any(n.strip()=='cargo test --all-features' for n in names), 'old plain cargo test step still present'; \
assert any('verify_handler_state_contract' in n for n in names), 'handler-state verify dropped'; \
assert any('verify_db_access_policy' in n for n in names), 'db-policy verify dropped'; \
print('backend-test job ok')"
```

Expected: `backend-test job ok`.

- [ ] **Step 4: Commit**

```bash
git add .github/workflows/ci.yml
git commit -m "ci(coverage): produce workspace coverage via cargo-llvm-cov"
```

---

## Task 2: Upload coverage to Codecov

**Files:**

- Modify: `.github/workflows/ci.yml` `backend-test:` job (after the coverage step)

- [ ] **Step 1: Add the Codecov upload step**

After the `Test + coverage` step (and before or after the `verify_*` steps — order does not matter for upload):

```yaml
- name: Upload coverage to Codecov
  uses: codecov/codecov-action@v7
  with:
    token: ${{ secrets.CODECOV_TOKEN }}
    files: lcov.info
    fail_ci_if_error: false
```

`fail_ci_if_error: false` keeps coverage upload advisory — it must not block the merge (spec: coverage is non-blocking).

- [ ] **Step 2: Validate YAML**

```bash
python3 -c "import yaml; j=yaml.safe_load(open('.github/workflows/ci.yml')); \
s=j['jobs']['backend-test']['steps']; \
assert any(x.get('uses','').startswith('codecov/codecov-action@v7') for x in s), 'codecov step missing'; \
print('codecov step ok')"
```

Expected: `codecov step ok`.

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/ci.yml
git commit -m "ci(coverage): upload lcov to Codecov"
```

---

## Task 3: Upload the same coverage to CodeScene

**Files:**

- Modify: `.github/workflows/ci.yml` `backend-test:` job

- [ ] **Step 1: Confirm CodeScene's coverage-import mechanism and secret name (do this BEFORE writing the step)**

CodeScene ingests external LCOV. Confirm the exact mechanism for this project's CodeScene plan — current options are the `cs` CLI
(`cs check`/coverage import) or the cloud coverage REST endpoint — and the secret name. **Do not write a step that calls a script or command that
does not yet exist.** If the mechanism cannot be confirmed now, per spec this upload is best-effort: ship Codecov (Task 2) and land this step in a
follow-up PR — do not commit a no-op step. Record the confirmed command before Step 2.

- [ ] **Step 2: Add the CodeScene upload step (inline the confirmed command; consumes the same `lcov.info`)**

Inline the single confirmed command directly in `run:` — do not introduce a wrapper script for a one-line command. Fill in the real command + secret
name from Step 1; the shape is:

```yaml
- name: Upload coverage to CodeScene
  env:
    CODESCENE_TOKEN: ${{ secrets.CODESCENE_TOKEN }}
  run: |
    [ -n "$CODESCENE_TOKEN" ] || { echo "CODESCENE_TOKEN unset — skipping (e.g. fork/dependabot PR)"; exit 0; }
    <confirmed cs CLI or curl command> lcov.info
  continue-on-error: true
```

The in-`run:` token guard (`[ -n "$CODESCENE_TOKEN" ] || exit 0`) skips cleanly on PRs where the secret is unset (forks, dependabot) —
runner-portable, unlike `secrets.*` in a step-level `if:`, and avoids a spurious red step on every such run. `continue-on-error: true` keeps it
advisory regardless.

`continue-on-error: true` plus the `if:` secret-guard keep it advisory and skip cleanly if the secret is unset. (`secrets.*` is not valid in a
step-level `if:` on all runners — if the guard errors, drop the `if:` and rely on `continue-on-error` alone.)

- [ ] **Step 3: Validate YAML and commit**

```bash
python3 -c "import yaml; yaml.safe_load(open('.github/workflows/ci.yml')); print('yaml ok')"
git add .github/workflows/ci.yml
git commit -m "ci(coverage): upload lcov to CodeScene (advisory)"
```

Expected: `yaml ok`. (Explicit path — never `git add -A` in an agent context, per repo feedback rule.)

---

## Task 4: README Codecov badge

**Files:**

- Modify: `README.md`

- [ ] **Step 1: Add the badge near the top (with existing badges, if any)**

```markdown
[![codecov](https://codecov.io/gh/worried-networking/uptrakit/branch/main/graph/badge.svg)](https://codecov.io/gh/worried-networking/uptrakit)
```

The slug is `worried-networking/uptrakit` (confirmed via `git remote get-url origin`). If the remote differs at implementation time, re-run
`git remote get-url origin` and substitute.

- [ ] **Step 2: Confirm no Sentrux reference remains in README**

```bash
grep -i sentrux README.md && echo "FIX: sentrux still in README" || echo "README clean"
```

Expected: `README clean` (PR 1 should already have removed it; this is a backstop).

- [ ] **Step 3: Lint and commit**

```bash
npx prettier --prose-wrap always --print-width 148 --write README.md
markdownlint --config .markdownlint.json README.md
git add README.md
git commit -m "docs(readme): add Codecov coverage badge"
```

---

## Task 5: Optional `codecov.yml`

**Files:**

- Create (optional): `codecov.yml`

- [ ] **Step 1: Decide if non-default behavior is wanted**

Default Codecov needs no config. Add `codecov.yml` ONLY if non-default targets/flags are wanted (e.g. informational status, no PR-failure
threshold). YAGNI — skip this task entirely unless a concrete need exists. If skipping, check the box and move on.

- [ ] **Step 2 (only if needed): Minimal advisory config**

```yaml
coverage:
  status:
    project:
      default:
        informational: true
    patch:
      default:
        informational: true
```

- [ ] **Step 3 (only if created): Commit**

```bash
git add codecov.yml
git commit -m "ci(coverage): codecov informational-only config"
```

---

## Task 6: Document coverage in `quality-gates.md` + CodeScene dashboard onboarding

**Files:**

- Modify: `docs/development/quality-gates.md`

- [ ] **Step 1: Add a coverage note under the Architecture health section**

This section is created by PR 1. If PR 1 has not merged yet (out-of-order), create the heading first so the append target exists:

```bash
grep -q '^## Architecture health' docs/development/quality-gates.md || printf '\n## Architecture health\n' >> docs/development/quality-gates.md
```

Then append to the `## Architecture health` section:

```markdown
### Coverage (advisory)

`cargo llvm-cov --workspace --all-features --lcov` runs in the `backend-test:` CI job (replacing the plain `cargo test` step; a separate
`cargo test --doc` step keeps doctests running) and the resulting `lcov.info` is uploaded to **both Codecov** (the coverage home — report, PR delta,
README badge) **and CodeScene** (so its test-gap dimension is coverage-backed). Neither upload blocks merges.
```

- [ ] **Step 2: Add a one-time CodeScene onboarding note**

Append:

```markdown
### CodeScene dashboard (advisory)

CodeScene (SaaS, free for OSS) provides the behavioral health view — code-health grade, hotspots, change/temporal coupling — that no cargo tool
reproduces. One-time setup: connect the public repo at codescene.io. It analyzes source + git history; the per-PR delta status is optional and only
worth enabling if someone owns reviewing it. An opt-in local MCP server (`codescene-oss/codescene-mcp-server`) exposes it to Claude Code for
developers who want it.
```

- [ ] **Step 3: Lint and commit**

```bash
npx prettier --prose-wrap always --print-width 148 --write docs/development/quality-gates.md
markdownlint --config .markdownlint.json docs/development/quality-gates.md
git add docs/development/quality-gates.md
git commit -m "docs(quality-gates): document coverage + CodeScene dashboard"
```

---

## Self-review checklist (run before handing off)

- [ ] `backend-test:` job: `cargo test` step is gone, `cargo llvm-cov` produces `lcov.info`, both `verify_*` steps still present.
- [ ] Codecov upload step uses `codecov/codecov-action@v7` with `CODECOV_TOKEN`, `fail_ci_if_error: false`.
- [ ] CodeScene upload consumes the same `lcov.info`, is `continue-on-error: true` (advisory).
- [ ] `.github/workflows/ci.yml` parses as YAML.
- [ ] README badge uses the real repo slug; no Sentrux reference in README.
- [ ] All changed markdown passes `markdownlint --config .markdownlint.json`.
- [ ] Every commit uses Conventional Commits.

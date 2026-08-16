# CI / Workflow Reliability Hardening — Design

**Date:** 2026-07-12
**Status:** Design (pending plan)
**Scope:** `.github/workflows/` only. No Rust/frontend code, no ADR, no dependency, no doc rewrite.

## Problem

Four verified `ci-tooling` findings from `.superpowers/audit-2026-07-11.md` (Immediate actions / Medium)
share one root theme: **CI/release enforcement does not match its documented or intended behavior.** Each
is an effort-S GitHub Actions YAML fix. Bundling them into one coherent editing pass over the workflow
files is more maintainable than four piecemeal PRs and gives one reviewer one mental model ("workflow
reliability") instead of four.

| # | Audit | File:line | Class | Hazard |
| - | ----- | --------- | ----- | ------ |
| 1 | L775 | `ci.yml:112` | stability | Minimal-features (`db-sqlite`) config is only compile/clippy-checked in CI; its **tests** run only in the bypassable pre-push hook. Runtime regressions behind `cfg(feature)` gates land CI-green. |
| 2 | L791 | `ci.yml:4` | maintainability | Full suite runs **twice** per PR commit (`push:['**']` + `pull_request`); no `concurrency:` group, so superseded runs never cancel. Same in `agent-core-batch-ci.yml`; `docker.yml` also lacks concurrency. |
| 3 | L807 | `release-plz.yml:474` | stability | Normal release path **uploads before attesting** (publishes unattested bytes if Sigstore fails) and lacks `--clobber` (non-idempotent on retry). The backfill path is already hardened the other way. |
| 4 | L823 | `ci.yml:28` | stability | Blocking `markdown:` job installs `markdownlint-cli` **unpinned** — a rule-changing release reddens every open PR with no code change. Every other gate tool is pinned. `node-version: lts/*` here vs `22` in release-plz. |

## Verified current reality (byte-checked against the tree, 2026-07-12)

- **`ci.yml`**: `on: push: branches:['**']` + `pull_request` (top of file); no `concurrency:`. Minimal-features
  is `cargo check`/`cargo clippy` only (`:84`, `:87`). Test execution is all-features
  `cargo llvm-cov --workspace --all-features` (`:112`) + all-features doctests (`:118`). No
  `nextest`/minimal-features test run anywhere. `markdown:` job: `npm install -g markdownlint-cli` (`:28`,
  unpinned), `node-version: lts/*` (`:27`; also `:134/218/247/264/305`).
- **`.husky/pre-push:88`**: `cargo nextest run --no-default-features --features db-sqlite` (the missing CI run;
  falls back to `cargo test` when nextest absent, `:90`).
- **`agent-core-batch-ci.yml`**: `on: push` + `pull_request` (`:3-5`), no `concurrency:`.
- **`docker.yml`**: `on: push: branches:[main]` **+ `push: tags:[uptrakit-*-v*]` (6 patterns)** +
  `pull_request: branches:[main]`, no `concurrency:`. The `tags:` trigger is the **release image-publish** path —
  it must never be cancelled. Already main-scoped `push` on branches — no feature-branch double-run; needs
  concurrency only, with cancellation **disabled** (see Fix 2).
- **`release-plz.yml` / `website.yml`**: both use `concurrency:` (grep-confirmed ci/docker/agent-core-batch are
  the three that lack it).
- **`release-plz.yml` build-artifacts job**: `package_and_upload()` (`:443`) **couples** tar-package + `gh
  release upload` (`:474`, no `--clobber`) in one function, called 7× inline (`:477`–`:508`); the single trailing
  attest step (`:513`) runs `actions/attest@v4` with `subject-path: "uptrakit-*-${{ matrix.target }}.tar.gz"`
  (a glob covering all 7 archives) **after** all uploads.
- **`release-plz.yml` backfill job**: attest step (`:800`) runs **before** upload (`:821`, with `--clobber`),
  rationale documented in-file (`:796-797`): "Attest BEFORE upload: if the attestation step fails (e.g. Sigstore
  outage) we do not publish unattested bytes."
- **markdownlint-cli pin location**: none exists as a package. `.husky/pre-commit`/`pre-push` invoke a
  locally-installed `markdownlint` binary via `command -v` (no version); there is **no** `package.json`
  devDependency for it. The only version-bearing reference is `ci.yml`'s global install.

## Approach (chosen — YAGNI, reuse the repo's own proven patterns)

Straight workflow-YAML corrections. No new CI machinery beyond the single missing test invocation. Each fix
reuses a pattern the repo already trusts: the pre-push test command, the pin-everything convention, the
backfill job's attest→upload+clobber ordering.

### Fix 1 — minimal-features tests in CI (`ci.yml`)

Add a CI step that runs the same command the pre-push hook already runs, so hook and CI enforce the **same
matrix**:

```sh
cargo nextest run --no-default-features --features db-sqlite
```

Placement: **own job** `backend-test-minimal` mirroring pre-push (keeps `backend-lint` a pure check/clippy job;
a test failure is diagnostically distinct from a lint failure, matching the existing `backend-lint`/`backend-test`
split). A separate job runs on its own runner — it does **not** inherit `backend-lint`'s in-progress compile
artifacts — so it declares `dtolnay/rust-toolchain@stable` and restores via `Swatinem/rust-cache` (the same
`shared-key` pattern every other job uses) to avoid a cold `db-sqlite` build. Install `cargo-nextest` via
`taiki-e/install-action` pinned to a chosen current stable version (matching Fix 4's pin-everything convention).
Note: there is **no** existing repo pin to "match" — `.husky/pre-push:87` runs whatever `cargo-nextest` is
locally installed (`command -v` guard, falling back to `cargo test`), and `setup.md` lists it as optional with no
version. The plan-writer picks a current stable and pins it; do not leave it floating.

- **Coverage stays all-features.** Do **not** add a minimal-features `llvm-cov` job — a plain `nextest run`
  mirroring pre-push is sufficient (YAGNI). Coverage instrumentation is an all-features concern.
- **Cross-spec handoff (do not act here):** this adds a named CI gate. The `quality-gates.md` gate-list
  reconciliation is owned by `2026-07-12-developer-docs-drift-sweep-design.md`; note the new job there so its
  gate list can include the row. **This spec does not edit `quality-gates.md`.**

### Fix 2 — kill the double-run and cancel stale runs (`ci.yml`, `agent-core-batch-ci.yml`, `docker.yml`)

Two independent hazards, two changes:

1. **Double-run** (`push:['**']` + `pull_request` both run the full suite per PR commit). A `concurrency:`
   group keyed on `github.ref` does **not** dedup a push-run vs its PR-run — they carry different refs
   (`refs/heads/branch` vs `refs/pull/N/merge`), so concurrency alone cannot fix this. **Restrict `push:` to
   `branches: [main]`.** PR events already cover feature branches.
   - *Tradeoff (stated explicitly):* this removes CI on branches that have **no** open PR. Acceptable — a PR is
     the merge gate; a branch with no PR is not merging, and the author can open a draft PR to get CI early.
     `push` on `main` still runs (post-merge validation + release triggers).
2. **Stale-run cancellation** (rapid successive pushes to `main`, or force-pushes to a PR branch, stack full
   runs). Add:

   ```yaml
   concurrency:
     group: ci-${{ github.ref }}
     cancel-in-progress: ${{ github.ref != 'refs/heads/main' }}
   ```

   Group name follows the repo's existing static-purpose-prefix convention (`release-plz-${{ github.ref }}`,
   `release-pr-${{ github.ref }}`, `website.yml`'s `pages`) — a static `ci-` prefix, **not** `${{ github.workflow
   }}` (redundant at workflow scope; no existing repo group interpolates it). `cancel-in-progress` guarded off for
   `main` so post-merge/release-relevant runs are never cancelled mid-flight; superseded PR/branch runs cancel
   freely. (`actionlint` confirms the conditional `cancel-in-progress` expression is valid GHA syntax.)

Apply **both** changes to `agent-core-batch-ci.yml` (same `push`+`pull_request` double trigger).

`docker.yml` needs the **concurrency group only** (no push-restriction — its branch `push` is already
`[main]`). **Critical:** docker.yml also triggers on release `tags:`, which is the image-publish path. Do **not**
copy ci.yml's `cancel-in-progress: ${{ github.ref != 'refs/heads/main' }}` guard here — a cancelled tag run =
a release with binaries but missing/partial container images. Set docker.yml's `cancel-in-progress: false`
outright (a ref-keyed group already isolates each distinct tag/branch ref; disabling cancellation guarantees a
release publish is never interrupted). Its group key: `group: docker-${{ github.ref }}`.

(Ledger, sibling-completeness: grep confirmed ci/docker/agent-core-batch are the three lacking `concurrency`;
`release-plz`/`website` already have it — do not touch those.)

- **Branch-protection note:** required status checks (e.g. the `frontend` job, configured as a required check per
  bead epic `uptrakit-spec-2026-04-30-decouple-frontend-ci-design`, retired at the beads migration
  2026-08-16; full text at `pre-beads-archive`) are satisfied by the **`pull_request`-triggered**
  run, not the `push` run — so restricting `push` to `[main]` does not affect merge gating. `push` on `main`
  still fires, so the post-merge `frontend-e2e-parity` canary (`if: github.ref == 'refs/heads/main'`) is
  unaffected.

### Fix 3 — attest before upload + idempotent retry (`release-plz.yml` build-artifacts job)

Mirror the **already-hardened backfill job** exactly. This is supply-chain integrity — attest coverage must not
shrink.

- Split `package_and_upload()` into a **package-only** function (tar + sha256, no `gh release upload`).
  **Preserve the existing tag derivation** — the function already resolves the tag correctly via a
  select-by-package lookup, `jq -r --arg p "$pkg" '.[] | select(.package_name==$p) | .tag'` over `$RELEASES`
  (release-plz.yml `:447-449`). Keep exactly that; do **not** reverse-parse a `prefix-version-target.tar.gz`
  filename (fragile — `version` can contain dots/dashes).
- **Do not thread state across steps via a shell array** (a `bash` array dies at the `run:` boundary). Instead,
  both the package and upload steps **re-derive independently from `$RELEASES`**, which is a **job-level `env:`**
  (`:328`, `RELEASES: ${{ needs.release-plz.outputs.releases }}`) and therefore re-readable in every step of the
  `build-artifacts` job. This is the same re-derive-per-step trick the backfill job uses — note backfill reads
  its own job-env var **`$PLAN`** (`:605`, iterated at `:747/780/823`), not `$RELEASES`; the normal path's
  source of truth is `$RELEASES`. Do not import backfill's `$PLAN` machinery; reuse the normal path's proven
  `$RELEASES` select-by-package derivation.
- Run all 7 package calls → all archives on disk.
- Run the existing attest step (its `subject-path: "uptrakit-*-${{ matrix.target }}.tar.gz"` glob already covers
  all 7) **before** any upload.
- Add a final upload step that, for each released package, re-derives `(tag, archive)` from `$RELEASES` (same
  lookup as packaging) and runs `gh release upload "$tag" "$archive" "${archive}.sha256" --clobber`.

Result ordering: **package → attest → upload (`--clobber`)** — identical semantics to backfill. `--clobber` makes
the **upload** idempotent (no 422 on already-uploaded assets), removing the forced manual-backfill dispatch.

- **Attestation is not idempotent — and that is accepted.** `actions/attest@v4` publishes to the append-only
  Sigstore/Rekor transparency log; a retried job (upload failed after attest succeeded) re-attests the same bytes
  and writes a **second** Rekor entry for the same digest. This is cosmetic (both entries are valid, same digest
  and identity; a verifier asserting "a valid attestation exists" still passes) and strictly better than the
  current order's failure mode (attest-fails-after-upload → **unattested bytes published**). Do not claim full
  idempotency; the reorder trades a duplicate-log-entry nuisance for closing the unattested-publish hole.

- **Invariant to preserve:** every one of the 7 archives that reaches `gh release upload` must have been in the
  attest subject set. Because attest is glob-over-disk and now runs after all packaging but before all uploads,
  coverage is structurally guaranteed. Planning asserts archive-count uploaded == archive-count packaged == 7 and
  that no archive is uploaded outside the accumulated pair list.

### Fix 4 — pin markdownlint-cli + align Node major (`ci.yml`)

Collapse the separate `npm install -g` + `markdownlint` steps into a single pinned `npx` invocation, matching the
repo's existing npm-tool idiom (`website.yml:57` runs `npx -y pagefind@1 …` rather than a global install):

```yaml
- run: npx -y markdownlint-cli@0.49.0 --config .markdownlint.json '**/*.md'
```

`markdownlint-cli@0.49.0` is the current npm-latest stable (verified 2026-07-12). `npx -y` with a pinned version
avoids mutating global npm state on the ephemeral runner and is one fewer step than `npm install -g`.

- **No single-source-of-truth file exists** to centralize the pin into — husky invokes a locally-installed
  `markdownlint` binary with no version, and there is no `package.json` devDependency. Do **not** create a
  `package.json` solely to hold this pin (YAGNI); a pinned `npx` invocation in `ci.yml` matches the repo's
  per-install-site pin convention (and its `pagefind` npx precedent).
- **Align Node major:** change `ci.yml` `node-version: lts/*` → `node-version: "22"` to match `release-plz.yml`,
  so the frontend CI validates under the same Node major that builds the released frontend. Change every
  `lts/*` occurrence in `ci.yml` (there are several; grep during planning — do not pin only the `markdown:`
  job). Ledger, sibling-completeness: enumerate all `node-version:` lines in `ci.yml` and change the set, not
  the one the finding named.

## Deliverables

- `.github/workflows/ci.yml` — Fixes 1, 2, 4.
- `.github/workflows/agent-core-batch-ci.yml` — Fix 2.
- `.github/workflows/docker.yml` — Fix 2 (concurrency group only; `push` is already `branches:[main]`, no
  push-restriction needed).
- `.github/workflows/release-plz.yml` — Fix 3.

No Rust/frontend code, no `Cargo.toml` change (markdownlint-cli is a CI tool version, not a workspace
dependency), no ADR (CI mechanics, not an architectural decision), no wire/OpenAPI change.

**Delivery / commit granularity (bundle on blast-radius, not effort):** one spec, but **Fix 3 lands as its own
standalone commit/PR**, independently reviewable. It rewrites the signed-provenance path for every published
binary — a subtle attestation-coverage regression is a supply-chain defect that ships to users, so it must get
proportional review scrutiny, not be diluted in the same diff as a markdownlint pin. Fixes 1/2/4 (low-stakes CI
convenience — worst case is wasted minutes or a red PR) may land together. This also keeps reverts scoped: a
problem in the attestation reorder reverts without dragging the CI-convenience fixes with it.

### Documentation deliverables

- **None in this spec.** The YAML fixes carry their own in-file comments (the double-run tradeoff, the
  attest-before-upload rationale — copy the backfill job's comment style). `quality-gates.md` gate-list
  reconciliation is explicitly **out of scope**, owned by `2026-07-12-developer-docs-drift-sweep-design.md`;
  this spec only flags the new `backend-test-minimal` job as a handoff note for that spec's gate list.

## Verification

- **Fix 1:** after adding the job, grep `ci.yml` for `no-default-features --features db-sqlite` and confirm a
  `nextest run` (not just `check`/`clippy`) matches; confirm `cargo-nextest` is installed via
  `taiki-e/install-action` with an explicit version pin (no floating version).
- **Fix 2:** `actionlint` over all three workflows (already a CI/pre-commit gate); confirm each edited workflow
  has a `concurrency:` block and that only `ci`/`agent-core-batch` had their `push:` restricted; confirm
  `cancel-in-progress` is guarded off for `main`.
- **Fix 3:** confirm in the rewritten build-artifacts job that (a) no `gh release upload` appears before the
  attest step, (b) the attest `subject-path` glob is unchanged, (c) every upload carries `--clobber`, (d) the
  archive count uploaded == archive count packaged == 7 (planning asserts none is uploaded outside the
  accumulated list). Diff the reordered job against the backfill job to confirm ordering parity.
- **Fix 4:** grep `ci.yml` for `markdownlint-cli@` (pinned) and confirm zero `node-version: lts/*` remain.
- **Whole change:** `actionlint` clean; no YAML the workflow parser rejects.

## Alternatives considered

- **Concurrency-only for the double-run** (no push restriction) — *rejected*: a ref-keyed concurrency group
  cannot dedup push-vs-PR (different refs), so the double-run survives. Restriction is the only fix that
  actually removes it.
- **Keep `push:['**']`, dedup via `github.event_name` in the concurrency key** — *rejected*: more complex, and
  still runs both to the point of cancellation (wasted minutes) rather than never starting the duplicate.
- **A shared `scripts/quality-gates.sh` runner** invoked by hook + CI + doc to prevent gate-list drift —
  *rejected (deferred-not-dismissed)*: already rejected in the developer-docs-drift-sweep spec on
  scope/reversibility grounds; not re-introduced here.
- **A minimal-features `llvm-cov` coverage job** — *rejected*: YAGNI; a `nextest run` mirroring pre-push closes
  the regression gap. Coverage stays all-features.
- **Create `package.json` to centralize the markdownlint-cli pin** — *rejected*: YAGNI; no such file exists and
  husky doesn't consume one. A pinned global install matches the repo's per-install-site pin convention.

## Out of scope

Other unspecced Immediate-Medium findings in different subsystems — core-agent L859 (silent ExecuteUpdate
drop), core-agent-ssh L876 (bootstrap key-loss), core-controller L894 (CA-reload swallow), core-mqtt-scheduler
L911 (hard-abort claim-release interval), plugins-infra L1042, ui-cli-surface-proxy L1093/1110/1126,
web-api-routes L1226 — are separate specs for future iterations. No new CI/guard machinery beyond Fix 1's single
test invocation. No workflow restructuring or composite-action extraction.

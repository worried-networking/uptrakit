# Release Build Matrix Deduplication — Design

Source: audit-2026-07-11 MEDIUM ci-tooling (`.github/workflows/release-plz.yml`, `backfill-build-artifacts`), verified.
Decisions settled 2026-08-06 with the owner after a contrarian review round; contrarian findings are folded in below.

## Context

`release-plz.yml` carries two near-identical ~250-line build jobs: `build-artifacts` (normal release path) and
`backfill-build-artifacts` (manual `workflow_dispatch` backfill). Both duplicate the 4-target matrix
(x86_64/aarch64 gnu, x86_64 musl via cross, aarch64-apple-darwin), the cross install+cache steps, the macOS
LTO workaround, and seven per-binary `cargo build --release && cp` steps. The duplicated `build-frontend` /
`backfill-build-frontend` jobs are byte-similar as well. Per-binary feature lists are additionally repeated in
`docker.yml`'s build matrix and its tag-only `build-swagger` job. The cli "binary is named `uptrakit`"
exception is encoded in three places (`package_and_upload` call, `inner_name_for()`, docker matrix).

The two release-plz copies have diverged in semantics:

| Aspect | `build-artifacts` (normal) | `backfill-build-artifacts` |
| --- | --- | --- |
| Build scope | all 7 binaries, unconditionally | plan members only |
| Package not released | silent skip | n/a (plan pre-filtered) |
| Binary missing on disk | hard error | hard `::error` |
| Existing-asset handling | none (upload fails on dup) | digest pre-check, skip if identical |
| Attestation order | after upload | before upload |
| Upload | plain | `--clobber` |

Contrarian review surfaced three load-bearing facts, verified against sources:

1. **The normal path's "wasteful" all-7 build is the repo's only cross-target compile gate.** `ci.yml` contains
   zero `--target` cross-compilation; musl/aarch64/darwin compile only inside release builds. Plan-only builds
   would silently delete that coverage.
2. **Backfill checks out the dispatch ref, not the tag** (bare `actions/checkout@v7` in
   `backfill-build-artifacts`). Today a backfill rebuilds *main's* source and `--clobber`s it onto an old
   release as that version's asset — a pre-existing provenance bug this refactor must fix, not canonicalize.
3. **The `--no-default-features` axis is where silent artifact-content drift lives.** `docker/Dockerfile`
   derives it from FEATURES-emptiness; release steps hand-write it (scheduler only). It currently coincides
   only because the controller crates declare no `default` feature key while
   `crates/core/scheduler/Cargo.toml` does (`default = ["db-sqlite", "zeroconf"]`).

## Goals

- One home for the target matrix, build/package/attest/upload steps, and the frontend-build job.
- One machine-readable home for per-binary build facts (package, binary name, features,
  `--no-default-features`, frontend dependency, docker participation), shared with `docker.yml` via a drift
  gate.
- Unified upload semantics: the backfill pipeline's stronger shape (digest pre-check, attest-before-upload,
  `--clobber`, hard error on missing binary) wins everywhere; the silent-skip case becomes structurally
  impossible because plans are pre-filtered to released packages.
- Fix the backfill checkout-ref bug (same-commit guard).
- Preserve the cross-target compile gate on every release.
- A verification path that does not require cutting a real release.

## Non-goals

- No change to published artifact names or contents. Archive names stay `{package}-{version}-{target}.tar.gz`
  with `.sha256` sidecars; inner binary names unchanged (cli stays `uptrakit`).
- No change to release-plz tag/version semantics (`git_only` mode untouched; jobs `release-plz` and
  `release-pr` are not modified beyond consuming outputs).
- No generated matrix for `docker.yml` (drift gate instead; revisit only if the gate proves noisy).
- No per-entry mixed-SHA backfill matrix (guard + per-tag-group dispatch instead).

## Component 1 — binaries manifest

New file `ci/release-plz/binaries.json`, the single source for per-binary build facts:

```json
{
  "binaries": [
    { "package_name": "uptrakit-controller",            "binary": "uptrakit-controller",            "features": ["db-all", "nats"], "no_default_features": false, "needs_frontend": true,  "docker": true  },
    { "package_name": "uptrakit-controller-standalone", "binary": "uptrakit-controller-standalone", "features": ["db-all", "nats"], "no_default_features": false, "needs_frontend": true,  "docker": true  },
    { "package_name": "uptrakit-agent",                 "binary": "uptrakit-agent",                 "features": [],                 "no_default_features": false, "needs_frontend": false, "docker": false },
    { "package_name": "uptrakit-agent-ssh",             "binary": "uptrakit-agent-ssh",             "features": [],                 "no_default_features": false, "needs_frontend": false, "docker": true  },
    { "package_name": "uptrakit-mqtt",                  "binary": "uptrakit-mqtt",                  "features": [],                 "no_default_features": false, "needs_frontend": false, "docker": true  },
    { "package_name": "uptrakit-scheduler",             "binary": "uptrakit-scheduler",             "features": ["db-all", "oidc"], "no_default_features": true,  "needs_frontend": false, "docker": true  },
    { "package_name": "uptrakit-cli",                   "binary": "uptrakit",                       "features": [],                 "no_default_features": false, "needs_frontend": false, "docker": true  }
  ],
  "swagger": {
    "package_name": "uptrakit-controller-standalone",
    "binary": "uptrakit-controller-standalone",
    "features": ["db-all", "nats", "swagger-ui"]
  }
}
```

The `swagger` object exists solely for the drift gate to validate `docker.yml`'s tag-only `build-swagger` job;
release builds never read it. `no_default_features` is explicit per entry — never derived from feature-list
emptiness — because the Dockerfile and the release build apply different derivation rules today (Context #3).

## Component 2 — reusable workflow

New file `.github/workflows/build-release-assets.yml`, `on: workflow_call`.

Inputs (all consumed via the typed `inputs.*` context, never `github.event.inputs.*`):

| Input | Type | Meaning |
| --- | --- | --- |
| `plan` | string (required) | JSON array of `{package_name, tag, version}` — the packages to package/attest/upload |
| `build_set` | string (required) | `"manifest"` = build every manifest binary (compile gate); `"plan"` = build plan members only |
| `checkout_ref` | string, default `""` | Ref to build from; empty string means the calling event's default checkout |
| `needs_frontend` | boolean (required) | Whether to build and download the frontend (computed by the plan producer — a job-level `if:` cannot read a file, so this must travel as an input) |
| `dry_run` | boolean, default `false` | Build, package, and digest-check, but skip attest and upload |

The callee declares **no `permissions:` block** and carries a comment stating why: a called workflow inherits
the caller's grants (`contents: write`, `id-token: write`, `attestations: write` are required); declaring any
job-level `permissions:` key would drop every unlisted scope to `none` and break `actions/attest` with an
opaque OIDC error.

Jobs:

**`build-frontend`** — `if: inputs.needs_frontend`. Checkout at `inputs.checkout_ref`, node 22 + npm cache,
`npm ci && npm run build`, upload artifact `frontend-build` (retention 1 day). Replaces both existing
frontend jobs; the two callers are mutually exclusive by trigger, so the artifact name cannot collide.

**`build`** — `strategy.matrix.include` with the four `{target, runner, cross}` rows, defined once.
`concurrency: { group: build-release-assets-${{ github.run_id }}-${{ matrix.target }}, cancel-in-progress: false }`
lives **here** (it references `matrix`, which only exists in the callee; callers must not set `concurrency`
on the calling job — that would serialize all four targets). Steps:

1. Checkout at `inputs.checkout_ref` (empty = event default).
2. `dtolnay/rust-toolchain@stable` with `targets: ${{ matrix.target }}`.
3. Cross binary cache + `cargo install cross --version 0.2.5 --locked` on miss (unchanged from today).
4. Download `frontend-build` artifact — `if: inputs.needs_frontend`.
5. Cargo-command indirection (`cross` vs `cargo`) and the macOS LTO-off env step, unchanged.
6. **Build loop** — one step replacing the seven copy-pasted build steps. Data-driven from the manifest so
   no hand-generalization can drop a flag:

   ```bash
   set -euo pipefail
   manifest=ci/release-plz/binaries.json
   if [ "$BUILD_SET" = "plan" ]; then
     pkgs=$(jq -r '.[].package_name' <<< "$PLAN")
   else
     pkgs=$(jq -r '.binaries[].package_name' "$manifest")
   fi
   for pkg in $pkgs; do
     entry=$(jq -c --arg p "$pkg" '.binaries[] | select(.package_name == $p)' "$manifest")
     if [ -z "$entry" ]; then
       echo "::error::package $pkg not present in $manifest" >&2
       exit 1
     fi
     features=$(jq -r '.features | join(",")' <<< "$entry")
     ndf=$(jq -r '.no_default_features' <<< "$entry")
     bin=$(jq -r '.binary' <<< "$entry")
     args=(build --release --target "$TARGET" -p "$pkg")
     [ "$ndf" = "true" ] && args+=(--no-default-features)
     [ -n "$features" ] && args+=(--features "$features")
     echo "::group::build $pkg"
     "$CARGO_CMD" "${args[@]}"
     cp "target/${TARGET}/release/${bin}" "${pkg}-${TARGET}"
     echo "::endgroup::"
   done
   ```

   `BUILD_SET`, `PLAN`, `TARGET`, `CARGO_CMD` arrive via step `env:`. Staged filenames stay
   `{package}-{target}` — unchanged from both current jobs.
7. **Package** — loop over plan entries (today's backfill `Package archives` step); inner name from the
   manifest's `binary` field (retires `inner_name_for()` and the duplicated call-site encoding). Archive
   name `{package}-{version}-{target}.tar.gz` + `.sha256`, unchanged.
8. **Digest pre-check** — today's backfill step, with the jq filter switched to `--arg` binding
   (`--jq '.assets[] | select(.name == $a) | .digest // empty' --arg a "$archive"`): under unification the
   version string arrives from release-plz JSON without the backfill `TAG_REGEX` validation, so no shell
   interpolation into jq programs.
9. **Attest** — `actions/attest@v4`, `if: ${{ inputs.dry_run == false }}`, subject = **all packaged
   archives** (not just upload-queued ones). Decoupled from the upload gate deliberately: re-attesting
   identical bytes is free and append-only, and it repairs the partial-re-run case where an archive's
   upload succeeded previously but its attestation failed (a digest-match skip would otherwise suppress the
   repair forever).
10. **Upload** — `if: ${{ inputs.dry_run == false }}`, upload-queued archives only,
    `gh release upload "$tag" ... --clobber`. On a fresh release the digest pre-check finds no assets, so
    everything queues and behavior matches today's normal path; on re-runs it is idempotent.

## Component 3 — normal release path

In `release-plz.yml`, `build-frontend` and `build-artifacts` are replaced by:

**`release-plan`** — runs after `release-plz`, gated on `any_binary_released == 'true'` (widened for the
`plan_override` path — exact gate in Component 5). Executes new script `ci/release-plz/plan-from-releases.sh`
(same env/`GITHUB_OUTPUT` contract style as `parse-backfill-tags.sh`):

- Input `RELEASES` (the release-plz `releases` output). **Must** default the empty string to `[]`
  (`${RELEASES:-[]}`) — release-plz emits `""`, not `[]`, when nothing is released, and that is the common
  case on pushes to main.
- Filters to package names present in the manifest, projecting `{package_name, tag, version}`.
- Always emits a well-formed `plan` (`[]` on the empty path), plus scalar `any=true|false` and
  `needs_frontend=true|false` outputs (join of plan members against the manifest's `needs_frontend` flags).

**`build-release-assets`** — `uses: ./.github/workflows/build-release-assets.yml` with
`plan: ${{ needs.release-plan.outputs.plan }}`, `build_set: manifest`,
`needs_frontend: ${{ needs.release-plan.outputs.needs_frontend == 'true' }}`, default `checkout_ref`,
`dry_run` false (or forced true under `plan_override`, below). The job-level `if:` gates on
`needs.release-plan.outputs.any == 'true'` — a **string comparison, never `fromJSON`**, because
`fromJSON('')` raises an uncatchable workflow-expression error if the output is ever unset.

`build_set: manifest` preserves today's behavior of compiling every release binary on every target on every
release — the repo's only cross-target compile gate (Context #1) — while packaging/uploading plan members
only. Cost-neutral versus today.

## Component 4 — backfill path

`ci/release-plz/parse-backfill-tags.sh` is extended:

- For each validated tag, resolve its commit SHA via
  `gh api "repos/${GITHUB_REPOSITORY}/commits/<tag>" --jq .sha` (dereferences annotated tags to commits;
  overridable via a `GH_COMMIT_SHA_CMD`-style env hook mirroring the existing `GH_RELEASE_VIEW_CMD` test
  seam).
- **Same-commit guard**: if the resolved SHAs are not all identical, fail with a `::error` naming each
  tag→SHA pair and instructing the operator to dispatch once per same-SHA tag group. Tags cut by one
  release-plz run share a commit, so the common case passes.
- New output `checkout_ref` (the shared SHA).

`backfill-build-artifacts` and `backfill-build-frontend` are replaced by a single call job:
`uses: ./.github/workflows/build-release-assets.yml` with the plan, `build_set: plan` (backfill stays cheap;
the compile gate belongs to the normal path), `checkout_ref` from the guard, `needs_frontend` from the
existing output, and `dry_run: ${{ inputs.dry_run }}`.

This fixes Context #2: backfilled assets are built from the tag's own commit, never from main.

## Component 5 — dispatch inputs and rollout insurance

`release-plz.yml` `workflow_dispatch` gains:

- `dry_run` — `type: boolean`, default `false`. Passed to the callee as `${{ inputs.dry_run }}` (typed
  context). Never read via `github.event.inputs.*`, which yields strings where `"false"` is truthy.
- `plan_override` — string, default `""`. A releases-shaped JSON array injected in place of the release-plz
  `releases` output, exercising the **normal-path** `release-plan` conversion + call chain without cutting a
  release. A non-empty `plan_override` **forces `dry_run: true`** (call job passes
  `dry_run: ${{ inputs.plan_override != '' || inputs.dry_run }}`) — the override may name real tags and must
  never upload.

Gating consequences of `plan_override` (a dispatch with it set must **not** run a real release):

- The `release-plz` and `release-pr` jobs' skip conditions widen from "dispatch with non-empty
  `backfill_tags`" to "dispatch with non-empty `backfill_tags` **or** non-empty `plan_override`".
- `release-plan` therefore gates as `if: always() && ((needs.release-plz.result == 'success' &&
  needs.release-plz.outputs.any_binary_released == 'true') || inputs.plan_override != '')` — the `always()`
  is required because `release-plz` is *skipped* (not failed) on the override path, and the script consumes
  `plan_override` as its `RELEASES` input when set.

Both old build jobs and both old frontend jobs are **kept behind `if: false`** (with a dated comment) for one
real release cycle; a follow-up commit deletes them. Rollback during that cycle is a one-line edit.

Drive-by: the `any_released` job output of the `release-plz` job has no consumers and is removed.

## Component 6 — drift gate

New `bash ci/verify_release_binaries_manifest.sh`, run in CI and pre-push next to the existing `verify_*`
gates. Bidirectional checks (manifest ↔ workflow files), covering **all four** hand-maintained lists in
`docker.yml`:

1. Build-matrix entries: for every manifest entry with `docker: true`, a matrix row per platform exists with
   matching `package`/`binary`/`features`, and no matrix row exists without a manifest entry.
2. `build-swagger` job args match the manifest's `swagger` object.
3. `on.push.tags` patterns: every `docker: true` package has its `{package}-v*` pattern; no stray patterns.
4. `merge` job matrix `name:` list matches the `docker: true` set (short names derived
   `package_name` minus the `uptrakit-` prefix; `binary` differences don't affect this list).
5. **Feature-semantics equivalence**: render the cargo argv the Dockerfile would produce (its rule:
   `--no-default-features` iff `FEATURES` non-empty) and the argv the manifest produces for each
   `docker: true` package. They must be identical, **except** that a `--no-default-features` present only on
   the docker side is tolerated iff the package's `Cargo.toml` has no `default =` key under `[features]`
   (the flag is then a no-op). Concretely: any `docker: true` entry with non-empty `features` and
   `no_default_features: false` must point at a crate without default features, else the gate fails. This
   closes the trap where adding `default = [...]` to a controller crate silently forks docker vs release
   feature sets.

Sibling test `ci/release-plz/test_verify_release_binaries_manifest.sh` follows the
`test_parse-backfill-tags.sh` fixture-override pattern; it must include RED cases for each check class
(missing matrix row, extra row, feature mismatch, missing tag pattern, missing merge name, default-features
trap) plus an empty/valid-input GREEN case.

## Tests and CI wiring

- `ci/release-plz/test_parse-backfill-tags.sh` — extended: SHA resolution, same-commit pass, mixed-commit
  hard fail, `checkout_ref` output.
- New `ci/release-plz/test_plan-from-releases.sh` — cases: `RELEASES=""` (empty plan, `any=false`),
  `RELEASES="[]"`, lib-crates-only releases (e.g. `uptrakit-openapi-client` — filters to empty plan),
  mixed lib+binary, `needs_frontend` true/false derivation.
- New `ci/release-plz/test_verify_release_binaries_manifest.sh` — as above.
- **CI wiring**: none of the bash gate tests currently run anywhere. Add a step to the `ci.yml` job that
  runs the python checker unittests, executing all three `ci/release-plz/test_*.sh` files.
- **actionlint v1.7.12** (latest stable, pinned) added as a ci.yml step over `.github/workflows/`.

## Verification runbook (documented in releases.md)

CI workflows have no local test gate; verification is staged:

1. `actionlint` green in CI on the branch.
2. All `ci/release-plz/test_*.sh` green (runnable locally).
3. `bash ci/verify_release_binaries_manifest.sh` green (proves manifest == today's docker.yml).
4. **dry_run backfill dispatch** against an existing tag: exercises checkout-at-tag, manifest-driven builds
   on all four real runners (including the macOS LTO leg), packaging, and the digest pre-check — skips
   attest/upload. Expect every archive queued (rebuilt bytes differ from released assets); the run proves
   compile+package, not digest-skip.
5. **plan_override dry dispatch** with a hand-written releases JSON: exercises `plan-from-releases.sh`, the
   `any`/`needs_frontend` outputs, and the caller wiring — the code dry_run backfill cannot reach.
6. First real release runs the new path with the old jobs one `if: false` edit away; it is also the first
   live proof of attest/upload permission propagation through `workflow_call`. If it fails, the repaired
   backfill path (same callee) is the recovery tool.
7. Follow-up commit deletes the `if: false` jobs after one clean release.

What dry_run structurally cannot prove — attest OIDC propagation, real `--clobber` upload — is exactly what
step 6 covers, with rollback in place.

## Error handling

- Plan member missing from the manifest, or its binary missing on disk after build: hard `::error` + exit 1
  (backfill semantics win; the silent-skip branch no longer exists).
- Empty plan on the normal path: `release-plan` emits `any=false`; the call job is skipped by string
  comparison. No red runs on ordinary pushes to main.
- Mixed-SHA backfill: hard fail in `backfill-plan` with per-tag SHAs and dispatch instructions.
- Sigstore outage on the normal path now fails the job **before** upload, leaving a tag + assetless release
  page (previously: assets shipped unattested). This availability-for-integrity trade is deliberate;
  recovery is a backfill dispatch once Sigstore recovers. Documented in releases.md.

## Deliverables

Code:

- `ci/release-plz/binaries.json` (new)
- `.github/workflows/build-release-assets.yml` (new)
- `.github/workflows/release-plz.yml` (plan job, call jobs, dispatch inputs, old jobs behind `if: false`,
  `any_released` output removed)
- `ci/release-plz/plan-from-releases.sh` + `test_plan-from-releases.sh` (new)
- `ci/release-plz/parse-backfill-tags.sh` + `test_parse-backfill-tags.sh` (extended)
- `ci/verify_release_binaries_manifest.sh` + `ci/release-plz/test_verify_release_binaries_manifest.sh` (new)
- `.github/workflows/ci.yml` (bash gate tests step, actionlint v1.7.12 step)
- `.husky/pre-push` (drift gate added alongside existing `verify_*` gates)

Docs (non-optional; externally observable behavior changes):

- `docs/development/releases.md` — backfill same-commit rule + per-tag-group dispatch, `dry_run` and
  `plan_override` inputs, the verification runbook, the Sigstore-outage → assetless-release → backfill
  recovery trade, manifest description, and a note that the Sigstore certificate's `job_workflow_ref` now
  points at `build-release-assets.yml` (relevant only if a `--signer-workflow` verification policy is ever
  adopted).
- `docs/development/quality-gates.md` — new gate command (canonical source), same commit as the
  `AGENTS.md` Quick-start block addition (per AGENTS.md maintenance rules).
- One ADR recording the pipeline unification (plan-driven builds, attest-before-upload everywhere, backfill
  same-commit rule), created via `adrs new` — the number is never hand-allocated and must not be pinned in
  advance by any plan (in-flight ADR-number contention has occurred before).

## Constraints and conformance

- Published artifact names and contents unchanged: staged filenames, archive names, inner binary names, and
  cargo argv per package are all preserved byte-for-byte by construction (manifest values transcribed from
  the current workflow lines; the drift gate pins the docker side).
- Snapshot rules honored: Conventional Commits; ADR via `adrs new` only; markdownlint on touched docs;
  quality-gates.md as canonical command source updated with AGENTS.md in the same commit. No Rust code
  changes, so workspace lint rules are untouched.
- New tooling version pinned: actionlint v1.7.12 (latest stable as of 2026-08-06).

## Deferred / out of scope

- Generated `docker.yml` matrix from the manifest (drift gate chosen instead; revisit if the gate proves
  noisy).
- Per-entry mixed-SHA backfill support (matrix = targets × plan entries); the same-commit guard with
  per-tag-group dispatch covers the realistic case.
- A cross-target `cargo check` job in `ci.yml` (unnecessary while `build_set: manifest` preserves the
  release-time compile gate).
- Deleting the `if: false` legacy jobs — explicitly a follow-up commit after one clean release cycle.
- A non-dry backfill rehearsal against a throwaway pre-release tag (owner declined; first real release +
  fixed backfill is the accepted proof path).

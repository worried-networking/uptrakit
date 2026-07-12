# Strict Frontend Build Gate — fail loud instead of shipping a stub UI

**Date:** 2026-07-12
**Status:** Design
**Audit finding:** audit-2026-07-11 HIGH · stability · ci-tooling · effort S
**Sites:** `frontend/build.rs:22-40`, `.github/workflows/release-plz.yml`,
`.github/workflows/docker.yml`, `docker/Dockerfile`

## Problem

`frontend/build.rs` (crate `uptrakit-frontend`, embedded by
`crates/core/controller-runtime` under the `embed-frontend` feature) embeds the
real SvelteKit SPA when `frontend/build/index.html` exists, and otherwise writes
a **stub** `index.html` and emits only a `cargo::warning`:

```rust
if src_index.is_file() {
    copy_dir_recursive(&src_build, &embed_dir).expect("copy build/ to OUT_DIR");
} else {
    // release-plz git_only mode runs `cargo package --workspace` in a fresh
    // worktree where `frontend/build/` (gitignored) does not exist. Embed a
    // stub so the package verifies. ...
    fs::write(&stub, "<!doctype html>…stub…").expect("write stub index.html");
    println!("cargo::warning=frontend/build/ missing — embedded stub. …");
}
```

The stub path exists for **one legitimate reason**: release-plz `git_only`
packaging runs `cargo package --workspace` in a fresh worktree where the
gitignored `frontend/build/` is absent, and the package must still verify.

The hazard: the **same lenient `build.rs`** is used by every real-ship path, so
a missing/empty/ drifted `frontend/build/` produces a released binary or image
with a stub UI and **no failing gate** — only a build warning nobody reads:

- **release-plz.yml build-artifacts** downloads the `frontend-build` artifact to
  `frontend/build` (only `if released == 'true'`, lines ~348-352), then builds
  `uptrakit-controller` and `uptrakit-controller-standalone` (the
  `Build controller-standalone` step ~382-390).
- **release-plz.yml backfill** path re-downloads `frontend-build` to
  `frontend/build` (~296-298) for the same controller binaries.
- **docker.yml** downloads `frontend-build` to
  `/tmp/frontend-ctx/app/frontend/build` and injects it as the
  `frontend-builder` build-context, consumed by the Dockerfile's
  `COPY --from=frontend-builder /app/frontend/build /app/frontend/build`; the
  `cargo build` runs **inside** the image.

If the artifact path, the SvelteKit adapter output dir, or `npm run build`
output ever drifts (or the build dir is empty), the controller /
controller-standalone release binaries and the ghcr images silently ship a stub
UI. This is precisely the partial-release-integrity hazard class the release
machinery exists to prevent — but here it is invisible.

## Root cause

A single build script serves two audiences with opposite needs (packaging
worktrees _must_ tolerate an absent build dir; real ships _must not_), and it
resolves the conflict by always choosing the lenient branch. There is no signal
that distinguishes "packaging worktree, stub is fine" from "release build, stub
is a defect."

## Chosen approach — opt-in strict gate, lenient default preserved

Add one environment variable, `UPTRAKIT_REQUIRE_FRONTEND`. When it is set to
`1` **and** the real assets are absent, `build.rs` fails the build instead of
embedding the stub. When it is unset (the default), behaviour is byte-for-byte
what it is today. The strict flag is then set at exactly the three real-ship
build sites and nowhere else.

This is the smallest change that closes the hazard: it adds a fail-loud signal
without redesigning the artifact-passing mechanism, the SvelteKit adapter, or
the `embed-frontend` feature. The lenient default keeps `cargo package`
worktrees, local `cargo build`, and `cargo check --all-features` working
unchanged.

### Change 1 — `frontend/build.rs`

In the `else` branch (real assets absent), before writing the stub, consult the
flag and fail hard when strict mode is on:

```rust
// Strict mode: real-ship builds (release-plz binaries, docker images) set
// UPTRAKIT_REQUIRE_FRONTEND=1. If the real assets are absent there, a stub
// would silently ship — fail the build instead. Unset (the default) keeps the
// lenient stub path for release-plz `cargo package` worktrees and local dev.
if std::env::var("UPTRAKIT_REQUIRE_FRONTEND").as_deref() == Ok("1") {
    panic!(
        "UPTRAKIT_REQUIRE_FRONTEND=1 but frontend/build/index.html is missing — \
         refusing to embed the stub UI. Ensure the `frontend-build` artifact was \
         downloaded to frontend/build/ (release-plz) or COPYed to \
         /app/frontend/build (docker) before this build."
    );
}
```

And add, next to the existing `cargo::rerun-if-changed=build`:

```rust
println!("cargo::rerun-if-env-changed=UPTRAKIT_REQUIRE_FRONTEND");
```

**`panic!` is the correct, idiomatic failure here** and does not violate the
"no `panic!` in production code" binding rule: `build.rs` is a build script, not
a production code path, and the file's own `main()` already carries
`#[expect(clippy::expect_used, reason = "build script — panicking on missing
environment variables or I/O errors is the correct behaviour")]`. This `panic!`
is the same sanctioned failure mechanism the existing `.expect()` calls rely on.
If `clippy::panic` (a restriction lint, not part of `clippy::all`) is found to
be denied in the workspace lint set at implementation time, widen the existing
`#[expect(...)]` to cover it with the same reason — do **not** silence it with a
bare `#[allow]`.

`rerun-if-env-changed` is **load-bearing, not cosmetic** (see the Docker
cook→build note in Risks): it forces `build.rs` to re-execute when the variable
flips from unset to `1`, so cargo cannot serve a previously-cached stub embed.

### Change 2 — `.github/workflows/release-plz.yml`

Set `UPTRAKIT_REQUIRE_FRONTEND: 1` as a **step-level** `env:` on the controller
and controller-standalone cargo build steps in:

- the **build-artifacts** job (the `Build controller-standalone` step and the
  controller build step in the same job), and
- the **backfill** job's equivalent controller build steps.

These steps only run/consume the `frontend-build` artifact on a real release
(`released == 'true'`) or an explicit backfill dispatch, so strict mode is
always safe there. **Do not** set it at the workflow or job level — the
release-plz `release`/packaging steps and any `cargo package`/verify worktree
must keep the lenient default.

Add an inline comment on each `env:` line naming why it must stay:

```yaml
env:
  # Fail the build if only the stub frontend would embed — this job ships a
  # real release binary. See docs/development/releases.md#strict-frontend-gate.
  UPTRAKIT_REQUIRE_FRONTEND: "1"
```

### Change 3 — `.github/workflows/docker.yml` + `docker/Dockerfile`

`docker.yml` passes the flag as a build-arg on the `docker/build-push-action`
step:

```yaml
build-args: |
  PACKAGE=${{ matrix.package }}
  BINARY=${{ matrix.binary }}
  FEATURES=${{ matrix.features }}
  # Fail the image build if only the stub frontend would embed.
  UPTRAKIT_REQUIRE_FRONTEND=1
```

docker.yml downloads and COPYs the real `frontend-build` artifact for **every**
image build (PR and release), so strict-on-always is correct here and
additionally catches an empty PR frontend build.

**Dockerfile placement is the critical, traced detail.** The `builder` stage
runs `cargo chef cook` (dependencies only, from a source skeleton) **before**
`COPY --from=frontend-builder /app/frontend/build` and before the real
`cargo build`. During `cargo chef cook`, `/app/frontend/build` does **not yet
exist**, and `uptrakit-frontend`'s `build.rs` runs (it is in the controller's
dependency graph under `embed-frontend`). Therefore the `ENV` must be declared
**after** the frontend COPY and **before** the real `cargo build` — never at the
stage-top `ARG` block, or `cargo chef cook` would panic on the (correct) stub
path and fail the image spuriously.

Concretely, in `docker/Dockerfile` stage `builder`, between the existing
`COPY --from=frontend-builder /app/frontend/build /app/frontend/build`
(line ~72) and the `RUN … cargo build …` (line ~75):

```dockerfile
COPY --from=frontend-builder /app/frontend/build /app/frontend/build

# Strict frontend gate. Declared AFTER `cargo chef cook` (which runs build.rs
# against an absent frontend/build/) and BEFORE the real cargo build, so only
# the real build enforces it. See docs/development/releases.md#strict-frontend-gate.
ARG UPTRAKIT_REQUIRE_FRONTEND=
ENV UPTRAKIT_REQUIRE_FRONTEND=$UPTRAKIT_REQUIRE_FRONTEND

RUN if [ -n "${FEATURES}" ]; then \
      cargo build --release -p "${PACKAGE}" --no-default-features --features "${FEATURES}"; \
    ...
```

`build.rs` checks `CARGO_MANIFEST_DIR/build/index.html` — for `uptrakit-frontend`
that resolves to `/app/frontend/build/index.html`, exactly where the COPY lands
(verified against the current Dockerfile). The `ARG` default is empty so
non-CI/local `docker build` without the arg stays lenient.

### Regression guard — CI-file invariant, scope stated honestly

`build.rs` is a build script: it is not compiled into any test target, so there
is no Rust unit/integration test for it (adding one is impossible, not merely
skipped). The protective logic is a two-line env-gated guard; the real
protection is the flag being present at the ship sites.

The guard is therefore a **repo-file assertion**: `ci/verify_require_frontend.sh`
— a short grep that fails if `UPTRAKIT_REQUIRE_FRONTEND` is not set at all three
ship sites (both controller build steps in release-plz build-artifacts, the
backfill controller steps, and the docker.yml build-args), so a future edit that
drops the flag is caught in review/CI rather than at the next silent stub
release. It follows the existing `ci/verify_*.sh` convention and is added to the
AGENTS.md quick-start block and `docs/development/quality-gates.md` (canonical)
in the same commit. Keep it a pure grep — no parsing framework.

**Guard scope, stated explicitly (no overstatement):** this gate — both the
`build.rs` check and the CI grep — verifies only that **`index.html` is
present** in `frontend/build/` on a real-ship build, which is the identical
signal `build.rs` already keys on. It does **not** validate asset _content_ or
completeness: a `frontend/build/` that contains a valid `index.html` but
drifted, partial, or stale sibling assets would pass. That residual is
deliberately out of scope (see Out of Scope) — closing it would mean
content-hashing the SvelteKit output, which is a different, larger change. The
CI grep's guarantee is narrower still: it proves the _flag is wired_, not that
any given build _had_ real assets.

## Alternatives considered

- **Fail hard unconditionally when the stub would embed (no env flag).**
  Rejected: it breaks the legitimate release-plz `cargo package --workspace`
  packaging worktree, whose whole purpose is to verify with `frontend/build/`
  absent. The flag exists precisely to separate that audience from real ships.
- **Delete the stub path; require `frontend/build/` always.** Rejected: same
  breakage as above, plus it forces every local `cargo check --all-features` to
  first run `npm run build`, which the current lenient default intentionally
  avoids.
- **Validate asset content (hash/manifest), not just `index.html` presence.**
  Rejected as scope creep (YAGNI). The audit's failure mode is a _missing/empty_
  build dir, which `index.html` presence already detects. Content validation is
  noted as a residual, not built.

## Documentation deliverables

- **`docs/development/releases.md`** — new `#strict-frontend-gate` subsection:
  the `UPTRAKIT_REQUIRE_FRONTEND` contract, which jobs set it and why, the
  Dockerfile placement constraint (after chef-cook / after COPY / before real
  build), and why the lenient stub still exists for packaging worktrees. The
  three inline workflow/Dockerfile comments link here.
- **`frontend/build.rs`** — module/`main` doc comment explaining the two modes
  (lenient default vs strict `UPTRAKIT_REQUIRE_FRONTEND=1`).
- **`AGENTS.md` quick-start** + **`docs/development/quality-gates.md`
  (canonical, edited in the same commit)** — list `ci/verify_require_frontend.sh`
  alongside the other `ci/verify_*` gates.
- **No ADR** — CI/build-integrity mechanics, not an architectural decision.
- **No wire-protocol, OpenAPI, or `frontend/src` change** — the frontend build
  _output_ is unchanged; only when-to-fail changes.
- **No new dependency** — `std::env::var` only.

## Out of scope

- Reworking the `frontend-build` artifact-passing mechanism, the SvelteKit
  adapter output dir, or the `embed-frontend` feature.
- Content/completeness validation of the embedded assets beyond `index.html`
  presence (the same signal `build.rs` already uses). A `frontend/build/` with a
  present-but-drifted `index.html` and partial siblings passes the gate — named
  as a known residual, not a regression this spec introduces.
- The other CI/release-integrity audit findings (attest-before-upload,
  double-CI-run, unpinned markdownlint) — separate specs.

## Risks

- **Docker chef-cook / real-build ordering (primary).** Covered in Change 3: the
  `ENV` must sit after the frontend COPY and before the real `cargo build`;
  placing it at stage-top would panic `cargo chef cook`. `rerun-if-env-changed`
  additionally guarantees `build.rs` re-runs at the real build even though cook
  already ran it against an absent build dir.
- **Step-scope in release-plz.** The flag must be step-level `env:` on the
  controller build steps only; a job- or workflow-level setting would leak into
  the release-plz packaging/verify steps and break them. Enforced by the inline
  comment + the CI grep asserting _presence at the ship steps_ (not absence
  elsewhere — the grep is a floor, not a ceiling).
- **CI grep brittleness.** A structural refactor of the workflows (job rename,
  step reordering) could make the grep pass or fail spuriously. Kept minimal and
  documented so a maintainer can adjust it in the same PR as any such refactor.

## Quality gates

- `cargo fmt --all`; `cargo clippy --all-targets --all-features` (exercises the
  edited `build.rs` under the frontend-embedding graph — requires
  `frontend/build/` present, i.e. `npm run build` first, per AGENTS.md).
- `bash ci/verify_require_frontend.sh` (new) passes.
- `markdownlint --config .markdownlint.json` on the edited docs.
- Manual/CI verification at implementation: a docker image build with
  `UPTRAKIT_REQUIRE_FRONTEND=1` and an intentionally-empty `frontend/build/`
  must fail at the `cargo build` layer (not at `cargo chef cook`), and a normal
  build must still succeed and embed real assets.

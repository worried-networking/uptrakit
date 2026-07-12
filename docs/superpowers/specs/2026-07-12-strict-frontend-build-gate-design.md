# Strict Frontend Build Gate — fail loud instead of shipping a stub UI

**Date:** 2026-07-12
**Status:** Design
**Audit finding:** audit-2026-07-11 HIGH · stability · ci-tooling · effort S
**Hazard sites:** `frontend/build.rs:22-40`, `.github/workflows/release-plz.yml`,
`.github/workflows/docker.yml` (`build` + `build-swagger`), `docker/Dockerfile`
**Edited by fix:** `frontend/build.rs`, `.github/workflows/release-plz.yml`,
`docker/Dockerfile`, `ci/verify_require_frontend.sh` (new), `.github/workflows/ci.yml`

## Problem

`frontend/build.rs` (crate `uptrakit-frontend`, embedded by
`crates/core/controller-runtime` under the `embedded-frontend` feature) embeds the
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
  `frontend/build` (in the `backfill-build-artifacts` job, ~633-639, gated by
  the `backfill-plan` output containing `uptrakit-controller` /
  `uptrakit-controller-standalone` — not `released == 'true'`) for the same
  controller binaries.
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

Add one environment variable, `UPTRAKIT_REQUIRE_FRONTEND`. When it is set (to any
non-lenient value — see Change 1) **and** the real assets are absent, `build.rs`
fails the build instead of embedding the stub. When it is unset (the default),
behaviour is byte-for-byte what it is today. The strict flag is then set at the
real-ship build sites and nowhere else: **step-level** on the two release-plz
binary build jobs (Change 2), and a **single unconditional `ENV`** in the
Dockerfile that every image job inherits (Change 3).

This is the smallest change that closes the hazard: it adds a fail-loud signal
without redesigning the artifact-passing mechanism, the SvelteKit adapter, or
the `embed-frontend` feature. The lenient default keeps `cargo package`
worktrees, local `cargo build`, and `cargo check --all-features` working
unchanged (local `docker build` is strict, but its `frontend-builder` stage
always produces real assets, so the gate never fires spuriously there).

### Change 1 — `frontend/build.rs`

In the `else` branch (real assets absent), before writing the stub, consult the
flag and fail hard when strict mode is on:

```rust
// Strict mode: real-ship builds (release-plz binaries, docker images) set
// UPTRAKIT_REQUIRE_FRONTEND. If the real assets are absent there, a stub would
// silently ship — fail the build instead. Unset/empty/"0"/"false" (the default)
// keeps the lenient stub path for release-plz `cargo package` worktrees and
// local dev. Any other non-empty value means strict: a safety gate must
// fail-closed on an ambiguous value, never silently no-op.
let strict = std::env::var("UPTRAKIT_REQUIRE_FRONTEND")
    .map(|v| !v.is_empty() && v != "0" && !v.eq_ignore_ascii_case("false"))
    .unwrap_or(false);
if strict {
    panic!(
        "UPTRAKIT_REQUIRE_FRONTEND is set but frontend/build/index.html is \
         missing — refusing to embed the stub UI. Ensure the `frontend-build` \
         artifact was downloaded to frontend/build/ (release-plz) or COPYed to \
         /app/frontend/build (docker) before this build."
    );
}
```

**Why a permissive predicate, not `== Ok("1")`.** For a safety gate the
dangerous failure direction is "guard silently does nothing," so an unexpected
value (`true`, `yes`, a stray trailing newline) must resolve to strict, not
lenient. The predicate above treats only unset / empty / `0` / `false` as
lenient. The CI grep (below) still pins the literal `=1` at every ship site, so
the two are belt-and-suspenders, not coupled on a single spelling.

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

**`clippy::panic` is denied today — the `#[expect]` MUST be widened (firm step,
not conditional).** The workspace lint table sets `panic = "deny"` in
`[workspace.lints.clippy]` (root `Cargo.toml`), and `frontend/Cargo.toml` opts
in via `[lints] workspace = true`, so a bare `panic!` in `build.rs` fails clippy
now. Change 1 therefore includes widening the existing attribute on `main()`
to cover the new lint, e.g.:

```rust
#[expect(
    clippy::expect_used,
    clippy::panic,
    reason = "build script — panicking on missing environment variables, I/O \
              errors, or an absent frontend under UPTRAKIT_REQUIRE_FRONTEND is \
              the correct behaviour"
)]
fn main() { … }
```

`allow_attributes_without_reason = "deny"` is also set, so the `reason` is
mandatory — do **not** silence it with a bare `#[allow]`.

`rerun-if-env-changed` is **cheap correctness insurance that makes the env-var
contract explicit to cargo** — keep it, but it is not the load-bearing Docker
protection. In the Dockerfile the real `cargo build` runs after `COPY . .`
(source) and the frontend `COPY` (assets), on a distinct layer from
`cargo chef cook`; that layer invalidation is what guarantees `build.rs` runs
fresh against the populated `frontend/build/`. (An earlier draft claimed the
line stops cargo "serving the cook-stage stub embed from cache"; that overstates
it — `cargo chef cook` compiles _dependencies_, and whether or not it runs this
workspace member's `build.rs`, no cook-stage frontend embed is reused at the
real build layer. See the Docker note in Change 3 and Risks.)

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

### Change 3 — `docker/Dockerfile` only (single source of truth for all images)

**The strict flag for images is hardcoded in the Dockerfile, not passed
per-job.** Every image the Dockerfile produces embeds real assets: its
`frontend-builder` stage runs `npm run build` internally (line ~26) and CI
overrides that stage with `--build-context frontend-builder=<prebuilt>`, so the
`COPY --from=frontend-builder /app/frontend/build` **always** lands a populated
`frontend/build/` — for CI _and_ local `docker build`. There is no legitimate
image build with absent assets, so strict-on-always is correct, and a single
unconditional `ENV` covers **every** image job — `build`, `build-swagger`
(`uptrakit-controller-swagger`), and any image job added later — with zero
per-job build-arg wiring. This deliberately rejects the per-job `build-arg`
approach: `docker.yml` has more than one frontend-embedding image job
(`build-swagger` at ~182-239 ships a real published image), and a per-job arg
must be remembered in each `build-args:` block — the exact "a future job forgets
it" failure this spec exists to kill. One Dockerfile line removes that class.

**Placement is the critical, traced detail.** The `builder` stage runs
`cargo chef cook` (dependencies only, from a source skeleton) **before**
`COPY --from=frontend-builder /app/frontend/build` and before the real
`cargo build`. `uptrakit-frontend`'s `build.rs` is in the controller graph
**unconditionally** — `uptrakit-controller`'s manifest hard-declares
`uptrakit-controller-runtime = { features = ["embedded-frontend", …] }`
(`Cargo.toml:27-28`) and `controller-runtime`'s default feature set includes
`embedded-frontend` (`Cargo.toml:11`), so it is in scope for every controller
image regardless of the `FEATURES` build-arg. If `cargo chef cook` runs that
`build.rs` (whether cargo-chef executes a workspace member's build script is
version-dependent), it runs against an absent `frontend/build/`. Declaring the
`ENV` **after** the frontend COPY and **before** the real `cargo build` makes
strict mode invisible to the cook stage in _either_ case — a stage-top `ENV`
would risk panicking cook on the (correct-for-cook) stub path. The after-COPY
placement is safe and correct regardless of cargo-chef's member-build behaviour.

Concretely, in `docker/Dockerfile` stage `builder`, between the existing
`COPY --from=frontend-builder /app/frontend/build /app/frontend/build`
(line ~72) and the `RUN … cargo build …` (line ~76):

```dockerfile
COPY --from=frontend-builder /app/frontend/build /app/frontend/build

# Strict frontend gate for all images. Declared AFTER cargo chef cook (which may
# run build.rs against an absent frontend/build/) and BEFORE the real cargo
# build. Every image built from this file has real assets (npm build or CI
# --build-context), so strict-always never fires spuriously; it only catches a
# genuinely empty/drifted frontend/build/. See docs/development/releases.md#strict-frontend-gate.
ENV UPTRAKIT_REQUIRE_FRONTEND=1

RUN if [ -n "${FEATURES}" ]; then \
      cargo build --release -p "${PACKAGE}" --no-default-features --features "${FEATURES}"; \
    ...
```

`build.rs` checks `CARGO_MANIFEST_DIR/build/index.html` — for `uptrakit-frontend`
that resolves to `/app/frontend/build/index.html`, exactly where the COPY lands
(verified against the current Dockerfile). No `docker.yml` change is required for
the gate — the flag lives entirely in the Dockerfile.

### Regression guard — CI-file invariant, scope stated honestly

`build.rs` is a build script: it is not compiled into any test target, so there
is no Rust unit/integration test for it (adding one is impossible, not merely
skipped). The protective logic is a two-line env-gated guard; the real
protection is the flag being present at the ship sites.

After Change 3, the **image** ship paths are guarded structurally: the flag is a
single unconditional `ENV` in the Dockerfile, inherited by every image job
including `build-swagger`, so there is no per-job wiring a future job can forget.
The only ship sites that carry a _forgettable_ per-step flag are the two
**release-plz binary** build sites (build-artifacts controller +
controller-standalone, and the backfill equivalents).

The guard is therefore a **repo-file assertion**: `ci/verify_require_frontend.sh`
— a short grep that fails if `UPTRAKIT_REQUIRE_FRONTEND` is not present at (a) the
release-plz controller/controller-standalone build steps (build-artifacts +
backfill) and (b) the Dockerfile `ENV` line — so a future edit that drops the
flag at either is caught in review/CI rather than at the next silent stub
release. It follows the existing `ci/verify_*.sh` convention verbatim: the five
current scripts all start `#!/usr/bin/env bash` + `set -euo pipefail`, resolve
`ROOT` via `cd "$(dirname "${BASH_SOURCE[0]}")/.."`, and exit 1 with a
`verify_require_frontend: <message>` prefix on failure (`verify_agents_md_budget.sh`
is the closest structural template — no allowlist). Keep it a pure grep — no
parsing framework.

**Guard mechanism considered and rejected: grep the produced binary for the stub
marker.** A stronger check would, after each release `cargo build`, grep the
built binary (or its embedded `index.html`) for the stub marker string and fail
if present — that directly tests "no stub shipped" and covers _future_ ship
paths for free. Rejected for this spec as heavier than the hazard warrants
(YAGNI): it adds a post-build step to every release/image job (the same per-job
maintenance the file-grep avoids), and after Change 3 the image paths are already
covered structurally while the binary paths number exactly two, low-churn jobs.
The file-grep is the proportionate floor; the binary-marker check is noted here
so a future maintainer who adds many more binary ship paths knows the upgrade.

**Wiring is a required deliverable, not just documentation.** Documenting the
command does not make CI run it. The script must be added as an explicit
`- run: bash ci/verify_require_frontend.sh` step in `.github/workflows/ci.yml`,
alongside the sibling `ci/verify_*.sh` steps (currently grouped ~lines 70-73);
without that step the guard is inert in CI and only fires for whoever runs it
locally — defeating its purpose. The same commit also adds it to the AGENTS.md
quick-start block and to `docs/development/quality-gates.md` (the canonical
source, which also decides whether it additionally runs at a husky hook tier —
this spec does not hard-code the tier, matching how the other `verify_*` gates
are governed).

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
  the `UPTRAKIT_REQUIRE_FRONTEND` contract (permissive predicate; unset/`0`/`false`
  = lenient), where it is set (step-level on the two release-plz binary jobs; a
  single unconditional `ENV` in the Dockerfile for all images), the Dockerfile
  placement constraint (after COPY / before real build), and why the lenient stub
  still exists for packaging worktrees. The inline release-plz + Dockerfile
  comments link here.
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
  placing it at stage-top could panic `cargo chef cook` if cook runs the member
  `build.rs` against an absent build dir. The after-COPY placement is safe
  regardless of whether cargo-chef executes the member build script. The
  load-bearing protection that the real build re-runs `build.rs` against real
  assets is `COPY . .` layer invalidation, not `rerun-if-env-changed` (which is
  kept as explicit-contract insurance).
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

# Strict Frontend Build Gate — release builds fail loud instead of shipping a stub UI

**Date:** 2026-07-12 (revised 2026-07-13 — `UPTRAKIT_REQUIRE_FRONTEND` env var dropped for a profile-based gate)
**Status:** Design
**Audit finding:** audit-2026-07-11 HIGH · stability · ci-tooling · effort S
**Hazard sites:** `frontend/build.rs:22-40`, `.github/workflows/release-plz.yml`,
`.github/workflows/docker.yml` (`build` + `build-swagger`), `docker/Dockerfile`
**Edited by fix:** `frontend/build.rs`, `docs/development/releases.md`, `AGENTS.md` (stale frontend note + quick-start
comment) — **nothing else**

## Problem

`frontend/build.rs` (crate `uptrakit-frontend`, pulled unconditionally into every controller build —
`uptrakit-controller`'s manifest hard-declares `uptrakit-controller-runtime = { features = ["embedded-frontend", …] }`)
embeds the real SvelteKit SPA when `frontend/build/index.html` exists, and otherwise writes a **stub** `index.html`
and emits only a `cargo::warning`. The same lenient script serves every real ship path:

- **release-plz.yml build-artifacts + backfill** download the `frontend-build` artifact to `frontend/build`, then
  `cargo build --release -p uptrakit-controller` / `-p uptrakit-controller-standalone` (lines ~374-424, ~658-707 —
  every ship build is `--release`).
- **docker.yml** (`build`, `build-swagger`) injects the artifact as the `frontend-builder` build-context; the
  Dockerfile's builder stage runs `cargo build --release` after
  `COPY --from=frontend-builder /app/frontend/build /app/frontend/build`.

If the artifact path, the SvelteKit adapter output dir, or `npm run build` output ever drifts (or the downloaded dir
is empty), release binaries and ghcr images silently ship a stub UI — no gate fails, the only signal is a build
warning nobody reads.

## Verified reality (premises the fix rests on — each traced, not assumed)

1. **release-plz never compiles this crate.** The stub's in-code justification ("release-plz git_only mode runs
   `cargo package --workspace` … the package must verify") is **dead**: `ci/release-plz/cargo-wrapper.sh` injects
   `--no-verify` into every release-plz `cargo package --workspace` invocation (wrapper body, "Inject --no-verify so
   cargo only emits the .crate tarball without building"), and `release-plz.toml` sets `semver_check = false`
   workspace-wide. No release-plz code path runs `build.rs` in a `frontend/build/`-less worktree. `cargo publish` /
   docs.rs are likewise no release-profile compile path: the workspace root sets `publish = ["uptrakit-private"]`
   (a named fake registry — cargo refuses crates.io; `release-plz.toml` separately sets `publish = false`), so no
   crate is ever published or built on docs.rs.
2. **`cargo chef cook` cannot fire the gate.** cargo-chef's skeleton replaces build scripts with **dummy** `build.rs`
   files (cargo-chef `src/skeleton/mod.rs`: "create dummy `lib.rs`, `main.rs` and `build.rs` files where needed";
   dummy build-script artifacts are removed after cook, `mod.rs` ~276, so the real script re-runs at the real build).
   The Dockerfile's real `cargo build --release` runs after both `COPY . .` and the frontend COPY.
3. **`PROFILE` is a documented build-script env var; probed empirically** (throwaway crate, cargo stable): `debug`
   for dev builds, `release` for `--release`, and `release` for a custom profile with `inherits = "release"`
   (matters: the workspace defines `[profile.release-fast]` inheriting release). Cargo derives the value from the
   profile's `inherits` chain, not its name — the probe confirms the documented behaviour, it is not a special case.
   **Known caveat, disclosed:** cargo's build-script env-var docs mark `PROFILE` "not recommended", pointing at
   `OPT_LEVEL` etc. for "a more correct view of the actual settings". That advice targets code that cares about
   optimization settings; ours cares about **shippability**, and for this workspace `OPT_LEVEL` is strictly worse:
   `[profile.release]` sets `opt-level = 3` while `[profile.release-fast]` overrides `opt-level = 2`
   (root `Cargo.toml` ~278-290), so any `OPT_LEVEL` predicate either misclassifies `release-fast` as non-shippable or
   degenerates into an allowlist of numbers. `DEBUG` separates nothing (reflects debug-info generation, not profile).
   `PROFILE`'s binary debug/release answer is exactly the question the gate asks.
4. **Every context that compiles `uptrakit-frontend` without real assets today is a debug build:** CI `backend-lint`
   (`cargo check`/`clippy`, both feature sets — no node step in the job), CI `test` (`cargo llvm-cov`, doctests),
   `.husky/pre-push` (workspace check/clippy/test + doctests, all before its npm section), and local workspace
   `cargo check/test`. None of them ships a binary.
5. **Every ship path is `--release`:** release-plz build-artifacts + backfill (all eight build invocations),
   `docker/Dockerfile` builder stage, cross builds. A source install (`cargo install --git …` /
   `cargo build --release`) is also release-profile — today it silently self-ships a stub; under this fix it fails
   loud.

## Root cause

One build script serves two audiences with opposite needs — compile-only debug contexts (CI lint/test, local dev)
must tolerate an absent build dir; shippable release builds must not — and it always picks the lenient branch. The
profile **is** the signal that separates the audiences; no external flag is needed.

## Chosen approach — profile-based gate, no environment variable

In the `else` branch (assets absent), fail hard unless the profile is `debug`:

```rust
// frontend/build/ is gitignored; compile-only debug contexts (CI lint/test
// jobs, pre-push, local workspace builds) may legitimately lack it — they
// embed a stub and warn. A release-profile build produces a shippable
// binary, where a stub would silently replace the real UI (the exact
// hazard: release-plz binaries, docker images, `cargo install`). Fail-closed:
// anything cargo ever reports other than "debug" is treated as shippable.
let debug_profile = std::env::var("PROFILE").as_deref() == Ok("debug");
if !debug_profile {
    panic!(
        "frontend/build/index.html missing in a release-profile build — \
         refusing to embed the stub UI. Run `npm run build` in frontend/ \
         (CI: ensure the frontend-build artifact was downloaded to \
         frontend/build/)."
    );
}
// … existing stub write + cargo::warning (debug only) …
```

Design points:

- **Fail-closed predicate.** Only the exact value `debug` is lenient. `release`, a future cargo value, or a missing
  var all resolve to strict — for a safety gate the dangerous direction is "guard silently does nothing", so
  ambiguity must fail loud, never stub.
- **No `rerun-if-env-changed=PROFILE`.** Profile selection changes `OUT_DIR`/fingerprint; cargo re-runs the script
  per profile already. The existing `cargo::rerun-if-changed=build` stays.
- **`panic!` is the sanctioned failure mechanism in this build script** — cargo wraps a build-script panic in
  `error: failed to run custom build command for uptrakit-frontend` and prints the panic message above the backtrace
  note, so the actionable text still reaches the operator; it also matches how this script already fails (every
  `.expect()` call). `eprintln!` + `std::process::exit(1)` would print marginally cleaner but diverge from the file's
  established failure idiom for no behavioural gain. Accepted cost: widening the `#[expect]` suppresses `clippy::panic`
  for the whole of `main()`, not just the gate — acceptable in a 60-line build script where panicking is the uniform
  error policy. `[workspace.lints.clippy]` sets `panic = "deny"` and the crate inherits workspace lints, so the
  existing attribute on `main()` **must be widened** (firm step, not conditional):

  ```rust
  #[expect(
      clippy::expect_used,
      clippy::panic,
      reason = "build script — panicking on missing environment variables, I/O \
                errors, or absent frontend assets in a release-profile build is \
                the correct behaviour"
  )]
  fn main() { … }
  ```

  `allow_attributes_without_reason = "deny"` makes the `reason` mandatory; do not substitute a bare `#[allow]`.
  This **replaces** the existing `#[expect(clippy::expect_used, …)]` attribute on `main()` — do not stack a second
  attribute.

- **Rewrite the stale stub comment.** The current comment justifies the stub with release-plz `cargo package`
  verification, which no longer compiles anything (Verified reality #1). The replacement text is the comment block in
  the snippet above — it names the real remaining audience (debug-only compile contexts) and the fail-closed rule;
  use it verbatim.
- **Nothing else changes.** No workflow edits, no Dockerfile edits, no `ci/verify_*` script: there is no per-site
  flag anyone can forget — the gate lives inside the build script every ship path already executes. The prior
  revision's `UPTRAKIT_REQUIRE_FRONTEND` + step-level `env:` wiring + Dockerfile `ENV` + `ci/verify_require_frontend.sh`
  grep guard are all superseded by this structural placement (the grep guard existed only to protect the forgettable
  wiring; with no wiring there is nothing to guard).

### What the gate does and does not guarantee (stated honestly)

- **Guarantees:** any release-profile compile of `uptrakit-frontend` either embeds a `frontend/build/` containing
  `index.html` or fails the build. This covers artifact-path drift, adapter-output drift, an empty/missing download
  dir, and source installs — on every current and future ship path, because they all build `--release`.
- **Does not guarantee:** asset _content_ or completeness — a `frontend/build/` with a valid `index.html` but stale
  or partial sibling assets passes (same signal `build.rs` keys on today; closing it means content-hashing the
  SvelteKit output — out of scope). A hypothetical ship path that builds a **debug** binary would also slip; none
  exists, and "ship builds are `--release`" is the documented convention.
- **Debug builds are behavior-unchanged** — stub + warning, exactly as today.
- **False-positive direction (accepted):** a release-profile compile of the controller graph for a non-ship purpose
  (a future `--release` CI smoke/coverage job, a local `release-fast` build) fails without assets. That failure is
  loud, self-explanatory, and bypassable via the documented break-glass placeholder — the opposite failure (silent
  stub in a shipped binary) is the one this spec exists to kill.

## Alternatives considered

- **Opt-in strict env var (`UPTRAKIT_REQUIRE_FRONTEND`) set at ship sites** — the previous revision of this spec.
  Rejected on review: it adds per-site wiring (two release-plz steps + a Dockerfile `ENV`), a CI grep script to
  protect that wiring, and an env-var contract to document — all to reconstruct a signal (`is this a shippable
build?`) that cargo already provides as `PROFILE`. It also kept the false in-code premise that release-plz package
  verification needs the stub (it doesn't; the wrapper injects `--no-verify`). Honest framing of the trade: the env
  flag was more **explicit** at each ship site; the profile gate is more **unforgettable** (no wiring to forget or
  rot). Both rest on the same convention ("ship builds are `--release`") — the profile gate wins because that
  convention holds everywhere today and a violation fails loud in CI rather than shipping a stub.
- **Unconditional fail (delete the stub).** Matches the finding's letter but taxes every debug context: CI
  `backend-lint` and `test` jobs would need node + `npm run build` (~1-2 min each), pre-push would need reordering,
  and every contributor would need frontend assets before any workspace-wide `cargo check/test` — including the
  minimal `--no-default-features --features db-sqlite` gates. Offered to the user; release-profile gate chosen.
- **Validate asset content (hash/manifest).** Scope creep; the audit's failure mode is a missing/empty build dir,
  which `index.html` presence detects. Named as a residual, not built.

## Documentation deliverables

- **`docs/development/releases.md`** — new `#strict-frontend-gate` subsection: the profile-based contract (debug =
  stub + warning; anything else = hard failure without `frontend/build/index.html`), why release-plz packaging is
  unaffected (`--no-verify` wrapper, `semver_check = false`), why docker cook is unaffected (cargo-chef dummy
  build scripts), and the residuals above. The subsection must also document two operational points surfaced by
  review:
  - **Break-glass path.** The gate deliberately couples release-profile controller builds to frontend build health.
    If an urgent backend-only release must ship while the frontend build is broken for unrelated reasons, an operator
    can supply a placeholder `frontend/build/index.html` by hand — the same degraded-UI outcome as today's silent
    stub, but as a **deliberate, visible act** instead of an invisible default. Document the command; do not weaken
    the gate for it. The placeholder is **transient and must never be committed** — `frontend/build/` stays
    gitignored; a committed placeholder (`git add -f`, or a future `.gitignore` edit) would satisfy the gate on every
    future build for everyone, silently reinstating the exact hazard this spec kills. Today that rule is documented
    discipline, not enforced; a one-line CI guard asserting `git ls-files frontend/build/` is empty would enforce it —
    named here as **deferred optional hardening** (see Optional cleanup), not a deliverable of this fix.
  - **Convention made explicit.** The gate encodes "release profile ⇒ shippable ⇒ real assets required". Anyone
    adding a future release-profile CI job that compiles the controller graph (e.g. `--release` coverage or a
    smoke-test job) must supply `frontend/build/` first, or the job fails at `uptrakit-frontend`'s build script. The
    failure direction is safe (loud CI breakage, never a shipped stub), but the note saves the next maintainer the
    surprise.
- **`frontend/build.rs`** — rewritten branch comment (the load-bearing doc for the next reader).
- **`AGENTS.md` quick-start note** — the existing "`--all-features` … requires `frontend/build/`" note is stale
  (debug `--all-features` works via the stub, today and after this change). Reword to: release-profile builds of the
  controller require `frontend/build/` (`npm run build` first); debug builds embed a stub UI. While editing, also fix
  the stale feature name `embed-frontend` → `embedded-frontend` (the flag is declared `embedded-frontend` in
  `crates/core/controller-runtime/Cargo.toml`, not the `controller` crate). The typo occurs **twice** in `AGENTS.md`
  — the prose Note and the `npm run build` comment in the Frontend quick-start block (locate by
  `grep -n embed-frontend AGENTS.md`, currently lines ~50 and ~60); fix both. The same stale name in `docker/Dockerfile`
  is **out of scope** for this finding. The reworded prose need not name the flag at all.
- **No ADR** — build-integrity mechanics, not an architectural decision. **No wire/OpenAPI/frontend-source change.**
  **No new dependency** (`std::env::var` only). **No `docs/development/quality-gates.md` change** — no gate command
  is added, removed, or altered (the AGENTS.md "same commit" rule for quality-gate edits is therefore not triggered).

## Optional cleanup (deferred, named for honesty)

- The release-pr job step `Build frontend (required by uptrakit-frontend cargo package verify)`
  (`release-plz.yml` ~157) is vestigial: under the `--no-verify` wrapper no compile happens in that job, and even a
  wrapper regression would package-verify in **debug** profile (lenient). Removing it saves ~1-2 min per release-PR
  run but touches release infrastructure beyond this finding — deferred.
- A CI guard asserting `git ls-files frontend/build/` returns nothing would turn the break-glass "never commit the
  placeholder" rule from documented discipline into an enforced invariant. Unlike the rejected
  `ci/verify_require_frontend.sh` (which protected forgettable per-site wiring), this guards a repo state no build
  exercises. Deferred: the hazard requires a deliberate `git add -f` or `.gitignore` edit, both visible in review.
- The stale feature name `embed-frontend` (actual: `embedded-frontend`) is **repo-wide doc drift**, not just the
  AGENTS.md note this spec fixes: `grep -rn embed-frontend` hits ~14 canonical docs (`ARCHITECTURE.md`,
  `frontend/AGENTS.md`, `docs/development/{quality-gates,setup,feature-flags,coding-standards,docker,…}.md`,
  `docs/end-user/deployment/*`) plus `docker/Dockerfile` comments. A rename sweep is a separate doc-cleanup change —
  deferred; this spec fixes only the two occurrences inside the file it must edit anyway.

## Quality gates

- `cargo fmt --all`; `cargo clippy --all-targets --all-features` (with `frontend/build/` present) **and**
  `cargo clippy --all-targets --no-default-features --features db-sqlite` **without** `frontend/build/` — the latter
  proves the debug stub path still compiles clean. Note: clippy always compiles in **debug** profile, so neither
  clippy run can reach the panic branch — clippy validates the copy path and the stub path respectively; only manual
  steps 1-2 below exercise the fail-closed gate.
- `markdownlint --config .markdownlint.json` on the edited docs.
- Manual verification at implementation (build.rs is not compiled into any test target; a unit test is impossible,
  not skipped):
  1. `rm -rf frontend/build && cargo build --release -p uptrakit-frontend` → **fails** with the gate message (fires
     even on a warm `target/`: the deleted `frontend/build` dir is itself the `cargo::rerun-if-changed=build` path,
     so its disappearance forces the script to re-run);
  2. same command with `--profile release-fast` → **fails** (inherited release, probed);
  3. `cargo check -p uptrakit-frontend` (debug, no build dir) → succeeds with the stub warning;
  4. after `npm run build`: `cargo build --release -p uptrakit-frontend` → succeeds, `$OUT_DIR/embed/` holds real
     assets;
  5. a local `docker build -f docker/Dockerfile …` → succeeds (frontend-builder stage supplies assets; cook stage
     runs dummy build scripts).

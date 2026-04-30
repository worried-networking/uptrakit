# Decouple Frontend CI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task.
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove `npm ci && npm run build` from four backend CI jobs so a broken SvelteKit build no longer blocks backend-only hotfixes.

**Architecture:** `frontend/build.rs` already embeds a stub HTML page when `frontend/build/` is
absent (written for release-plz git_only mode). The four target jobs do not test frontend asset
delivery, so the stub is sufficient. Only `.github/workflows/ci.yml` changes; no Rust code or
Dockerfiles are modified.

**Tech Stack:** GitHub Actions YAML, Rust (cargo), `Swatinem/rust-cache`

---

## Pre-Work (must complete before creating PR)

### Task 0: Enforce `frontend` as a required branch protection check

The `frontend` job in `ci.yml` becomes the sole gate preventing a broken SvelteKit build from
reaching `main` and triggering a release with no binary assets. This must be enforced before
the decoupling PR is opened, not concurrently.

**Files:** None (GitHub repository settings)

- [ ] **Step 1: Check current required status checks**

  Go to: GitHub → repository → Settings → Branches → Branch protection rules → `main` → Edit

  Look for "Require status checks to pass before merging" section. Check whether `frontend`
  appears in the list of required checks.

- [ ] **Step 2: Add `frontend` if missing**

  In the status check search box, type `frontend`. Select the job named exactly `frontend`
  (from `ci.yml`). Save the branch protection rule.

  If `frontend` is already required: no action needed, proceed to Task 1.

---

### Task 1: Confirm no test exercises asset content

The stub embeds one file: `index.html` with a plain-text paragraph. Any test calling
`Assets::get(...)` and asserting on file count, content-type, specific paths, or content will
compile against the stub but diverge from production.

**Files:** None modified

- [ ] **Step 1: Grep for Assets:: usage in test modules**

  ```bash
  grep -r "Assets::" crates/ --include="*.rs" -l
  ```

  Expected output: zero lines, or only `crates/ui/web-api/src/embedded_frontend.rs` (production
  code, not a test). Any file under a `tests/` directory or containing `#[cfg(test)]` is a
  problem.

- [ ] **Step 2: If test usage found — gate behind feature flag**

  If any test file appears in the output, open it and locate the `Assets::` call. Wrap the
  relevant test with:

  ```rust
  #[cfg(feature = "embedded-frontend")]
  #[test]
  fn test_name() { ... }
  ```

  Then add `embedded-frontend` as a dev-dependency feature in the crate's `Cargo.toml` if not
  already present. Commit before continuing.

  If no test file appears: proceed directly to Task 2.

---

## Implementation

### Task 2: Remove npm steps from `backend-lint`

**Files:**

- Modify: `.github/workflows/ci.yml`

- [ ] **Step 1: Remove the `actions/setup-node` block and npm run step from `backend-lint`**

  Locate the `backend-lint` job. It currently starts with:

  ```yaml
  backend-lint:
    runs-on:
      - ubuntu-latest
    steps:
      - uses: actions/checkout@v6
      - uses: actions/setup-node@v6
        with:
          node-version: lts/*
          cache: npm
          cache-dependency-path: frontend/package-lock.json
      - run: cd frontend && npm ci && npm run build
      - uses: dtolnay/rust-toolchain@stable
  ```

  Remove the `actions/setup-node` block and the `npm ci && npm run build` run step. Result:

  ```yaml
  backend-lint:
    runs-on:
      - ubuntu-latest
    steps:
      - uses: actions/checkout@v6
      - uses: dtolnay/rust-toolchain@stable
  ```

  The subsequent steps (`Swatinem/rust-cache`, `cargo check`, `cargo clippy`, script checks)
  are unchanged.

---

### Task 3: Remove npm steps from `backend-test`

**Files:**

- Modify: `.github/workflows/ci.yml`

- [ ] **Step 1: Remove the `actions/setup-node` block and npm run step from `backend-test`**

  Locate the `backend-test` job. It currently starts with:

  ```yaml
  backend-test:
    runs-on:
      - ubuntu-latest
    steps:
      - uses: actions/checkout@v6
      - uses: actions/setup-node@v6
        with:
          node-version: lts/*
          cache: npm
          cache-dependency-path: frontend/package-lock.json
      - run: cd frontend && npm ci && npm run build
      - uses: dtolnay/rust-toolchain@stable
  ```

  Remove the `actions/setup-node` block and the `npm ci && npm run build` run step. Result:

  ```yaml
  backend-test:
    runs-on:
      - ubuntu-latest
    steps:
      - uses: actions/checkout@v6
      - uses: dtolnay/rust-toolchain@stable
  ```

  The subsequent steps (`Swatinem/rust-cache`, `cargo test --all-features`, script checks)
  are unchanged.

---

### Task 4: Remove npm steps from `system-integration-tests`

**Files:**

- Modify: `.github/workflows/ci.yml`

- [ ] **Step 1: Remove the `actions/setup-node` block and npm run step from `system-integration-tests`**

  Locate the `system-integration-tests` job. It currently starts with:

  ```yaml
  system-integration-tests:
    runs-on:
      - ubuntu-latest
    steps:
      - uses: actions/checkout@v6
      - uses: actions/setup-node@v6
        with:
          node-version: lts/*
          cache: npm
          cache-dependency-path: frontend/package-lock.json
      - run: cd frontend && npm ci && npm run build
      - uses: dtolnay/rust-toolchain@stable
  ```

  Remove the `actions/setup-node` block and the `npm ci && npm run build` run step. Result:

  ```yaml
  system-integration-tests:
    runs-on:
      - ubuntu-latest
    steps:
      - uses: actions/checkout@v6
      - uses: dtolnay/rust-toolchain@stable
  ```

  The `Dockerfile.test` Docker build already has its own `frontend-builder` stage that runs
  `npm ci && npm run build` inside the container. No Docker changes are needed.

---

### Task 5: Remove npm steps from `database-integration-tests`

**Files:**

- Modify: `.github/workflows/ci.yml`

- [ ] **Step 1: Remove the `actions/setup-node` block and npm run step from `database-integration-tests`**

  Locate the `database-integration-tests` job. It currently starts with:

  ```yaml
  database-integration-tests:
    runs-on:
      - ubuntu-latest
    steps:
      - uses: actions/checkout@v6
      - uses: actions/setup-node@v6
        with:
          node-version: lts/*
          cache: npm
          cache-dependency-path: frontend/package-lock.json
      - run: cd frontend && npm ci && npm run build
      - uses: dtolnay/rust-toolchain@stable
  ```

  Remove the `actions/setup-node` block and the `npm ci && npm run build` run step. Result:

  ```yaml
  database-integration-tests:
    runs-on:
      - ubuntu-latest
    steps:
      - uses: actions/checkout@v6
      - uses: dtolnay/rust-toolchain@stable
  ```

  The `uptrakit-integration-tests` crate has no dependency on `uptrakit-frontend`; this job
  never needed npm.

---

### Task 6: Commit

**Files:**

- Modify: `.github/workflows/ci.yml`

- [ ] **Step 1: Verify the diff before committing**

  ```bash
  git diff .github/workflows/ci.yml
  ```

  Expected: four `actions/setup-node` blocks and four `npm ci && npm run build` lines removed,
  one per job. No other changes. Confirm that `frontend`, `reverse-proxy-tests`, and `markdown`
  jobs are untouched.

- [ ] **Step 2: Commit**

  ```bash
  git add .github/workflows/ci.yml
  git commit -m "ci: remove npm build from backend CI jobs

  Backend jobs don't test frontend asset delivery. frontend/build.rs
  embeds a stub when frontend/build/ is absent — the same path used by
  release-plz git_only mode. Dockerfile.test has its own frontend-builder
  stage. The frontend job remains the sole npm health gate in ci.yml."
  ```

---

## Verification (CI gate — do not merge until this passes)

### Task 7: Push to branch and verify CI

This change affects `cargo clippy --all-targets --all-features` and `cargo test --all-features`
with the stub active — a code path never previously exercised in CI. CI verification is required;
local toolchain versions can differ from `dtolnay/rust-toolchain@stable`.

**Files:** None modified

- [ ] **Step 1: Push to a branch (not main)**

  ```bash
  git push origin HEAD
  ```

  Open a draft PR or simply push to verify CI runs. Do not merge yet.

- [ ] **Step 2: Confirm `backend-lint` passes**

  In the CI run, check `backend-lint`. It must:
  - Complete without any `npm` step in its log
  - Pass `cargo check --no-default-features --features db-sqlite`
  - Pass `cargo check --all-features`
  - Pass `cargo clippy --all-targets --all-features -- -D warnings`
  - Pass `cargo clippy --all-targets --no-default-features --features db-sqlite -- -D warnings`
  - Pass `Check legacy error-match patterns`

  The build log will contain a cargo warning line like:
  `warning: frontend/build/ missing — embedded stub. Run npm run build in frontend/`.
  This is expected and is not a failure.

- [ ] **Step 3: Confirm `backend-test` passes**

  Check `backend-test`. It must:
  - Complete without any `npm` step in its log
  - Pass `cargo test --all-features`

- [ ] **Step 4: Confirm `system-integration-tests` passes**

  Check `system-integration-tests`. It must complete without a host-side npm step. The Docker
  build step will still run its own `frontend-builder` stage internally — that is expected.

- [ ] **Step 5: Confirm `database-integration-tests` passes**

  Check `database-integration-tests`. It must complete without a host-side npm step.

- [ ] **Step 6: Confirm `frontend` job still passes independently**

  The `frontend` job must still pass on its own — it is the npm health gate and must remain
  green for the branch protection rule to allow merge.

- [ ] **Step 7: Merge**

  Once all six checks above are green, merge the PR normally.

---

## Expected cache behaviour on first run

`backend-lint` is the sole cache writer (`save-if` defaults to true). On the first run after
merge, it will recompile `uptrakit-frontend` with the stub (real-asset cache is now stale) and
write the new stub-based cache. All other jobs read the cache with `save-if: "false"`. Expect
one slow `backend-lint` run; all subsequent runs hit the stub cache normally.

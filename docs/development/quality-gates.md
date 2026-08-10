# Quality gates

## Local enforcement (pre-commit hooks)

Quality gates are automatically enforced locally via git hooks managed by
[`husky-rs`](https://crates.io/crates/husky-rs). Hooks auto-install on the first
`cargo build` or `cargo test` — no manual setup required.

### Hook tiers

**`pre-commit`** (fast, ~30 s) — checks only staged files:

| Condition                   | Command                                                             |
| --------------------------- | ------------------------------------------------------------------- |
| Any `.rs` file staged       | `cargo fmt --all -- --check`                                        |
| Any `.md` file staged       | `markdownlint --config .markdownlint.json '**/*.md'`                |
| Any `frontend/` file staged | `npm run lint` + `npm run format:check` (if `node_modules` present) |

**`pre-push`** (thorough) — always runs on every push:

| Command                                                                       | Notes                                                                                        |
| ----------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------- |
| `cargo check --no-default-features --features db-sqlite`                      |                                                                                              |
| `cargo clippy --all-targets --no-default-features --features db-sqlite`       |                                                                                              |
| `cargo deny check`                                                            | Fast (~3 s)                                                                                  |
| `python3 ci/check_plugin_semantic_boundary.py`                                | Blocks production-code semantic leaks; `docs/**`, tests, examples, and migrations are exempt |
| `cargo test --no-default-features --features db-sqlite`                       |                                                                                              |
| `cargo test --workspace --all-features --doc --exclude uptrakit-mqtt-runtime` | Doctests at CI parity; `cargo test` (not nextest — nextest skips doctests)                   |
| `npm run check` + `npm run test` + `npm run build` (cwd: `frontend/`)         | Guarded by `node_modules`                                                                    |

### Bypass methods

- **CI**: set `NO_HUSKY_HOOKS=1` before `cargo build`/`cargo test` to skip hook installation.
- **Emergency**: `git commit --no-verify` or `git push --no-verify` skips hooks for that
  invocation.

See [Setup — Pre-commit hooks](setup.md#pre-commit-hooks) for installation details.

## Full quality gate suite (must pass before committing)

Run all relevant quality gates for the areas touched by your change.

## Backend (Rust)

```sh
cargo fmt --all                                                      # Format
cargo check --no-default-features --features db-sqlite               # Lint with minimal features-set
cargo check --all-features                                           # Lint
cargo clippy --all-targets --no-default-features --features db-sqlite # Lint with Clippy over minimal features-set
cargo clippy --all-targets --all-features                            # Lint with Clippy
cargo test --all-features                                            # Tests
cargo deny check                                                     # Validate new dependencies
python3 ci/check_plugin_semantic_boundary.py                         # Production code must not depend on plugin semantics directly
bash ci/verify_no_security_audit.sh                                  # No legacy security_audit or raw semantic action literals
bash ci/verify_typed_audit_actions.sh                                # Dynamic audit action parsing/building stays at explicit boundaries
bash ci/verify_handler_state_contract.sh                             # No handler mixes State<Arc<AppState>> with sub-state
python3 ci/verify_db_access_policy.py                                # db_access_policy.toml consistent with routes/
bash ci/verify_agents_md_budget.sh                                   # AGENTS.md files within size budgets
bash ci/verify_no_raw_body_extractors.sh                             # Request bodies go through Unvalidated<T>/Validated<T> (see coding-standards.md § Request Type Validation)
python3 ci/verify_no_orphan_modules.py                               # Every tracked .rs reachable via mod resolution (no orphan modules)
bash ci/verify_no_new_cfg_not_feature.sh                             # Additive-only feature flags: no new negated-feature cfg outside allowlist
python3 ci/verify_action_security_declarations.py                    # Operation oauth2 scope lists match handler action extractors
cargo xtask contribution-monotonicity-check                          # Plugin contributions survive feature unification (ADR-0032)
```

Workspace lints (`[workspace.lints]` in root `Cargo.toml`) enforce `warnings = "deny"` and
`clippy::all = "deny"` across all 28 crates. The `-- -D warnings` flag is no longer needed on
clippy commands — it is inherited automatically via `[lints] workspace = true` in each crate.

**Note:** `--all-features` includes `embed-frontend`, which requires `frontend/build/` to exist.
Build the frontend first (`cd frontend && npm ci && npm run build`) before running `--all-features` checks.

The semantic-boundary gate applies to production code paths, including non-plugin Rust under `crates/**`,
production frontend code under `frontend/src/**`, and in-scope manifest dependency tables. It intentionally exempts
`docs/**`, test-only code, examples, and migrations. Those exemptions do not apply to production files that merely
contain `test` in their name.

A route family's `action_extractor!` entries and its handler conversions should land in the same commit.
`ci/verify_action_security_declarations.py`'s R1 check attributes an action to a handler only when the extractor
name is actually imported from `middleware::action` in that file — the import-gating in its `_check_file` skips R1
for any file that has not imported `middleware::action`.

### OIDC feature toggle

The `oidc` feature (default-enabled on `uptrakit-web-api` and `uptrakit-controller`) gates
`openidconnect` and its transitive dependencies. It propagates from `uptrakit-web-api` to
`uptrakit-web-api-auth/oidc`. The workspace minimal checks already provide the baseline
verification with OIDC disabled. When working in OIDC-gated code, `AppState` fields, or
authentication routes, the crate-specific `--no-default-features` commands below provide targeted
verification and faster iteration:

```sh
cargo check -p uptrakit-web-api-auth --no-default-features                           # auth crate without OIDC
cargo check -p uptrakit-web-api --no-default-features                                # web-api without OIDC
cargo check -p uptrakit-controller --no-default-features --features db-sqlite        # controller without OIDC
cargo clippy -p uptrakit-web-api --no-default-features -- -D warnings                # clippy without OIDC
cargo clippy -p uptrakit-controller --no-default-features --features db-sqlite -- -D warnings
cargo test -p uptrakit-web-api --no-default-features                                 # tests without OIDC
```

The workspace-level `--no-default-features` commands already cover the baseline OIDC-disabled
state (the controller's default features include both `db-sqlite` and `oidc`).

### Embedded-service zero-embedded check

The shutdown-token fields in `uptrakit-controller-runtime` (`src/embedded/mod.rs`) are `#[cfg]`-gated
to exactly their readers across the `embedded-*` features (see `service_host/builtins.rs`). The
workspace-level minimal gate (`cargo check --no-default-features --features db-sqlite`) does **not**
verify the gating: Cargo unifies features across workspace members, which enables **all** `embedded-*`
features on controller-runtime, so every field is present and read. The release image build
(`cargo build -p uptrakit-controller`, no embedded readers) instead compiles controller-runtime in
**package isolation** with **zero** embedded features — the only state where a mis-gated field becomes
dead code. This CI-enforced (`backend-lint`) check reproduces that isolation:

```sh
cargo check -p uptrakit-controller-runtime --no-default-features --features db-sqlite
```

The `-p` package flag is load-bearing: without it, feature unification masks the zero-embedded path.

### Reverse proxy-sensitive changes (mandatory)

If the change can affect reverse proxy behavior, you must also run ignored reverse proxy integration tests:

```sh
cargo test -p uptrakit-integration-tests --test reverse_proxy -- --ignored
```

This includes (non-exhaustive):

- mTLS identity extraction and certificate forwarding
- authentication and authorization behavior behind proxies
- client IP detection (`ClientIp`), forwarded headers, and trusted proxy logic
- TLS termination and certificate validation behavior that proxies depend on
- reverse proxy middleware, settings, or related wire/auth flows

### System integration-sensitive changes (mandatory)

If the change can affect enrollment, wire protocol, service lifecycle, or inter-component
communication, you must also run the system integration tests:

```sh
docker build -f docker/Dockerfile.test -t uptrakit-test:latest .
cargo test -p uptrakit-integration-tests -- --ignored
```

This requires Docker and covers end-to-end enrollment and communication between all binaries
(controller, agent, agent-ssh, scheduler, mqtt).

### Reload-mechanism changes (mandatory)

If the change touches config reload, the `Reloadable` trait, the coordinator, or file-watch
debouncing, you must also run the reload integration tests:

```sh
docker build -f docker/Dockerfile.test -t uptrakit-test:latest . && cargo test -p uptrakit-integration-tests reexec -- --ignored
```

### REST API contract staleness gates

After any backend route change or REST-contract change, the OpenAPI spec and the generated TypeScript client must be
regenerated and committed. CI (`backend-test:` job) gates on both:

1. **Rust openapi\_ test** — `UPDATE_OPENAPI=1 cargo test -p uptrakit-web-api --all-features openapi_`
   dumps `crates/ui/web-api/openapi.json`; CI diffs and fails if stale.
2. **Generated TS client** — `npm run gen:api` (cwd: `frontend/`) regenerates `frontend/src/lib/api/generated/`; CI diffs and fails if stale.

Run both in one command from the repo root:

```sh
./scripts/regen-api.sh
```

Then commit `crates/ui/web-api/openapi.json` and `frontend/src/lib/api/generated/`.

### Wire protocol (AsyncAPI) staleness gate

After any wire-type change (`crates/shared/wire/`), `crates/shared/wire/asyncapi.yaml` must be
regenerated and committed. CI's `--all-features` run gates on staleness via the
`asyncapi_yaml_is_up_to_date` golden test.

```sh
./scripts/regen-asyncapi.sh
```

## Frontend (SvelteKit)

```sh
cd frontend && npm install                                   # Install dependencies
cd frontend && npm run lint                                  # ESLint
cd frontend && npm run format:check                          # Prettier format check
cd frontend && npm run check                                 # Svelte/TypeScript type check
cd frontend && npm run test                                  # Vitest unit and component tests
cd frontend && npm run build                                 # Production build
cd frontend && npm run test:e2e                              # Playwright end-to-end + UI parity tests
```

### End-to-end and UI parity tests

`npm run test:e2e` runs the Playwright suite in `frontend/tests/e2e/`. It covers auth, hosts,
services, public-entry flows, and the desktop + mobile UI parity fixtures
(`ui-parity.test.ts`, `ui-parity-responsive.test.ts`). Snapshot regeneration must run on
**macOS + Chromium** per the parity-suite guard in `frontend/playwright.config.ts`.

One-time setup installs the browser:

```sh
cd frontend && npx playwright install --with-deps chromium
```

The suite is mandatory for visual or DOM-contract changes: any edit touching theme tokens,
shared primitives, route-level markup, or parity fixtures must pass `npm run test:e2e` locally
before push. The pre-push hook does **not** run Playwright (it would add several minutes to
every push); contributors are responsible for running it when relevant.

## Documentation

Run markdownlint whenever any `.md` files are added or modified:

```sh
markdownlint --config .markdownlint.json '**/*.md'
```

All warnings and errors must be resolved; do not silence them by adding exceptions to
`.markdownlintignore` or `.markdownlint.json` unless explicitly approved.
The `.markdownlintignore` file excludes `node_modules/`, `target/`, `.claude/`, and `CODEREVIEW.md`.

CI runs markdownlint on every PR. A PR that fails any quality gate will not merge.

### ADR corpus

ADRs are managed and validated with the `adrs` CLI — see
[Architecture Decision Records](architecture-decision-records.md). Gates (all wired into the git hooks and the CI
`markdown` job):

```sh
bash ci/verify_adr_numbers.sh          # duplicate ADR numbers (pure shell, hard everywhere)
bash scripts/regen-adr-toc.sh --check  # docs/adr/README.md staleness + link validity
adrs doctor                            # format/link validation; warnings hard-fail (skipped locally if adrs absent; CI enforces)
```

Create ADRs with `adrs new "Title"` — never hand-allocate a number or hand-edit `docs/adr/README.md`.

## UI Visual Parity

The approved UI design language is enforced through shared token and shell
contracts, not ad hoc route styling. `frontend/src/theme/tokens.ts` is the
single source of truth for semantic token values (emitted as CSS custom
properties through the `themeTokensPlugin` virtual module) and must stay
aligned with the semantic token mappings described in
[UI design language](ui/README.md). Built-in plus surface-backed
desktop parity coverage is required for any visual change that touches those
contracts, and the Playwright parity suite (`npm run test:e2e`) must pass on
macOS + Chromium before merge.

Mobile parity coverage is deferred until the responsive shell leaves `Target`.
Until then, desktop parity fixtures are the required baseline, and mobile
fixture work should follow once the shell status changes.

Before merging frontend visual changes:

1. Update `frontend/src/theme/tokens.ts` if semantic token values changed.
   `cssForTheme()` + the golden-CSS tests pin the emitted output.
2. Add or update deterministic parity fixtures for changed built-in and
   surface-backed patterns.
3. Run `npm run test:e2e` on macOS + Chromium and update any intentional
   snapshot deltas with `npx playwright test --update-snapshots`.
4. Keep `frontend/tests/e2e/ui-parity-waivers.json` empty unless a temporary
   exception is explicitly needed.
5. Remove or renew expired waivers in the same change window.

The only accepted source of visual-parity waivers is
`frontend/tests/e2e/ui-parity-waivers.json`. Each waiver entry must include the
required schema fields `scope`, `owner`, `expiry_date`, `capture_region`,
`justification`, and `review_ref`. Expired waivers must be renewed or removed;
they do not remain valid by default.

## Architecture health

The previous third-party architecture tool was abandoned and has been removed (see ADR-0022).
Architecture is now governed by:

- **Plugin boundary** — `python3 ci/check_plugin_semantic_boundary.py` (blocking; `semantic-boundary:` CI
  job). Enforces the plugin/production boundary as a path/layer rule.
- **Licenses / advisories / bans** — `cargo deny check` (blocking; `backend-deny:` CI job).
- **Unused dependencies** — `cargo machete` runs **advisory** (non-blocking) in the `unused-deps:` CI job.
  It surfaces unused-dep signal but does not gate merges: this workspace is macro- and feature-heavy, so
  `cargo machete` produces false positives that would otherwise need an ongoing per-crate ignore list.
- **Behavioral health** — code-health grade, hotspots, change/temporal coupling — lives in the **CodeScene**
  dashboard (advisory, SaaS; see the architecture-health design spec).

> **Why no module-cycle gate?** Rust's resolver already forbids circular _crate_ dependencies at build time.
> `cargo modules --acyclic` was evaluated and rejected: it analyses the _item_ graph, so any idiomatic
> `Debug`/`Clone`/`Display`/`fn new() -> Self` impl reads as a `Type ↔ Type::method` cycle — it flagged 66 of
> 71 crates with zero genuine cycles and no flag suppresses it. There is no turnkey Rust module-cycle tool
> worth gating on.

### Coverage (advisory)

The `backend-test:` CI job produces coverage with `cargo llvm-cov --workspace --all-features --lcov`. It
replaces the plain `cargo test` step — `cargo-llvm-cov` runs the same tests under instrumentation — and a
separate `cargo test --workspace --all-features --doc` step keeps doctests running (llvm-cov does not
instrument doctests on stable). That step excludes `uptrakit-mqtt-runtime` (`--exclude`): it sets
`[lib] doctest = false`, and `cargo test --doc` would otherwise override that opt-out. The resulting
`lcov.info` is uploaded to **Codecov** (the coverage home — report, PR delta, and the README badge).
Upload does not block merges (`fail_ci_if_error: false`).

Codecov needs the `CODECOV_TOKEN` secret (already configured). Coverage is **not** uploaded to CodeScene:
its coverage import requires an API access token that the CodeScene open-source plan does not provide.

### CodeScene dashboard (advisory)

CodeScene (SaaS, free for open source) provides the behavioral health view — code-health grade, hotspots,
change/temporal coupling — that no cargo tool reproduces. It analyses source + git history only (no
coverage import on the open-source plan). An opt-in local MCP server
(`codescene-oss/codescene-mcp-server`) exposes it to Claude Code for developers who want it.

**Auto-analysis on push.** Connect the repo via the **CodeScene GitHub App** (not a plain Git URL): the App
installs a push webhook, so CodeScene re-analyses automatically on every push to an analysed branch. A plain
Git-URL connection only polls on a schedule. Verify the connection at `codescene.io` → the uptrakit project →
**Project Configuration → Integrations / VCS**, and confirm the webhook under the repo's GitHub **Settings →
Webhooks**. If your CodeScene tier only polls, analysis still refreshes — on a schedule, not per-push; say so
rather than relying on a toggle that does nothing. The per-PR delta status is optional and only worth
enabling if someone owns reviewing it.

# Quality gates

## Local enforcement (pre-commit hooks)

Quality gates are automatically enforced locally via git hooks managed by
[`husky-rs`](https://crates.io/crates/husky-rs). Hooks auto-install on the first
`cargo build` or `cargo test` — no manual setup required.

### Hook tiers

**`pre-commit`** (fast, ~30 s) — checks only staged files:

| Condition | Command |
| --- | --- |
| Any `.rs` file staged | `cargo fmt --all -- --check` |
| Any `.md` file staged | `markdownlint --config .markdownlint.json '**/*.md'` |
| Any `frontend/` file staged | `npm run lint` + `npm run format:check` (if `node_modules` present) |

**`pre-push`** (thorough) — always runs on every push:

| Command | Notes |
| --- | --- |
| `cargo check --no-default-features --features db-sqlite` | |
| `cargo clippy --all-targets --no-default-features --features db-sqlite` | |
| `cargo deny check` | Fast (~3 s) |
| `python3 ci/check_plugin_semantic_boundary.py` | Blocks production-code semantic leaks; `docs/**`, tests, examples, and migrations are exempt |
| `cargo test --no-default-features --features db-sqlite` | |
| `sentrux check .` | Skipped gracefully if `sentrux` is not installed |
| `npm run check` + `npm run test` + `npm run build` (cwd: `frontend/`) | Guarded by `node_modules` |

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
bash ci/verify_handler_state_contract.sh                             # No handler mixes State<Arc<AppState>> with sub-state
python3 ci/verify_db_access_policy.py                                # db_access_policy.toml consistent with routes/
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

### OIDC feature toggle

The `oidc` feature (default-enabled on `uptrakit-web-api` and `uptrakit-controller`) gates
`openidconnect` and its transitive dependencies. It propagates from `uptrakit-web-api` to
`uptrakit-web-api-auth/oidc`. Changes that touch OIDC-gated code, `AppState` fields, or
authentication routes should also be verified with the feature disabled:

```sh
cargo check -p uptrakit-web-api-auth --no-default-features                           # auth crate without OIDC
cargo check -p uptrakit-web-api --no-default-features                                # web-api without OIDC
cargo check -p uptrakit-controller --no-default-features --features db-sqlite        # controller without OIDC
cargo clippy -p uptrakit-web-api --no-default-features -- -D warnings                # clippy without OIDC
cargo clippy -p uptrakit-controller --no-default-features --features db-sqlite -- -D warnings
cargo test -p uptrakit-web-api --no-default-features                                 # tests without OIDC
```

The workspace-level `--no-default-features` commands already cover this (the controller's
default features include both `db-sqlite` and `oidc`).

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

## Frontend (SvelteKit)

```sh
cd frontend && npm install                                   # Install dependencies
cd frontend && npm run lint                                  # ESLint
cd frontend && npm run format:check                          # Prettier format check
cd frontend && npm run check                                 # Svelte/TypeScript type check
cd frontend && npm run test                                  # Vitest unit and component tests
cd frontend && npm run build                                 # Production build
```

## Documentation

Run markdownlint whenever any `.md` files are added or modified:

```sh
markdownlint --config .markdownlint.json '**/*.md'
```

All warnings and errors must be resolved; do not silence them by adding exceptions to
`.markdownlintignore` or `.markdownlint.json` unless explicitly approved.
The `.markdownlintignore` file excludes `node_modules/`, `target/`, `.claude/`, and `CODEREVIEW.md`.

CI runs markdownlint on every PR. A PR that fails any quality gate will not merge.

## UI Visual Parity

The approved UI design language is enforced through shared token and shell
contracts, not ad hoc route styling. `frontend/src/theme/adapter-manifest.json`
must stay aligned with the semantic token mappings described in
[UI design language](ui-design-language.md), and built-in plus surface-backed
desktop parity coverage is required for any visual change that touches those
contracts.

Mobile parity coverage is deferred until the responsive shell leaves `Target`.
Until then, desktop parity fixtures are the required baseline, and mobile
fixture work should follow once the shell status changes.

Before merging frontend visual changes:

1. Update the adapter manifest if semantic token mappings changed.
2. Add or update deterministic parity fixtures for changed built-in and
   surface-backed patterns.
3. Keep `docs/superpowers/ui-parity-waivers.json` empty unless a temporary
   exception is explicitly needed.
4. Remove or renew expired waivers in the same change window.

The only accepted source of visual-parity waivers is
`docs/superpowers/ui-parity-waivers.json`. Each waiver entry must include the
required schema fields `scope`, `owner`, `expiry_date`, `capture_region`,
`justification`, and `review_ref`. Expired waivers must be renewed or removed;
they do not remain valid by default.

## Architectural rules (sentrux)

Run `sentrux check .` to validate the architectural constraints defined in `.sentrux/rules.toml`.
This checks layer ordering, boundary rules, and structural thresholds (coupling grade, cyclomatic
complexity, file/function length, circular dependencies).

Install sentrux if not already present:

```sh
curl -fsSL https://raw.githubusercontent.com/sentrux/sentrux/main/install.sh | sh
```

```sh
sentrux check .
```

The pre-push hook runs this automatically when `sentrux` is on `PATH`. CI always runs it.

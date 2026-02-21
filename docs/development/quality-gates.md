# Quality gates (must pass before committing)

Run all relevant quality gates for the areas touched by your change.

## Backend (Rust)

```sh
cargo fmt --all                                                      # Format
cargo check --workspace --no-default-features --features db-sqlite   # Lint with minimal features-set
cargo check --workspace --all-features                               # Lint
cargo clippy --workspace --all-targets --no-default-features --features db-sqlite -- -D warnings # Lint with Clippy over minimal features-set
cargo clippy --workspace --all-targets --all-features -- -D warnings # Lint with Clippy
cargo test --all-features                                            # Tests
cargo deny check                                                     # Validate new dependencies
```

There shouldn't even be any warnings in the output of these commands.

**Note:** `--all-features` includes `embed-frontend`, which requires `frontend/build/` to exist.
Build the frontend first (`cd frontend && npm ci && npm run build`) before running `--all-features` checks.

### OIDC feature toggle

The `oidc` feature (default-enabled on `uptrakit-web-api` and `uptrakit-controller`) gates
`openidconnect` and its transitive dependencies. Changes that touch OIDC-gated code, `AppState`
fields, or authentication routes should also be verified with the feature disabled:

```sh
cargo check -p uptrakit-web-api --no-default-features                               # web-api without OIDC
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
cargo test -p uptrakit-controller reverse_proxy -- --ignored
```

This includes (non-exhaustive):

- mTLS identity extraction and certificate forwarding
- authentication and authorization behavior behind proxies
- client IP detection (`ClientIp`), forwarded headers, and trusted proxy logic
- TLS termination and certificate validation behavior that proxies depend on
- reverse proxy middleware, settings, or related wire/auth flows

## Frontend (SvelteKit)

```sh
cd frontend && npm install                                   # Install dependencies
cd frontend && npm run lint                                  # ESLint
cd frontend && npm run format:check                          # Prettier format check
cd frontend && npm run check                                 # Svelte/TypeScript type check
cd frontend && npm run build                                 # Production build
```

## Documentation

All Markdown files (`.md`) are linted with `markdownlint`. Ensure that `markdownlint` passes without errors.
**Critically**, you must address all warnings and errors; do not silence them by adding exceptions to
`.markdownlintignore` or `.markdownlint.json` unless explicitly approved.

```sh
markdownlint --config .markdownlint.json '**/*.md'
```

The `.markdownlintignore` file excludes `node_modules/`, `target/`, `.claude/`, and `CODEREVIEW.md`.

CI runs these same checks. A PR that fails any of them will not merge.

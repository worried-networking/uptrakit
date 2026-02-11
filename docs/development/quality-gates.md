# Quality gates (must pass before committing)

If your changes are confined to the frontend, you do not need to run the Rust `cargo` checks. If your changes are
confined to Rust/backend crates, you do not need to run the frontend `npm` checks. For mixed changes, run both sections.

## Backend (Rust)

```sh
cargo fmt --all                                                      # Format
cargo check --workspace --no-default-features --features db-sqlite   # Lint with minimal features-set
cargo check --workspace --all-features                               # Lint
cargo clippy --workspace --all-targets --no-default-features --features db-sqlite -- -D warnings # Lint with Clippy over minimal features-set
cargo clippy --workspace --all-targets --all-features -- -D warnings # Lint with Clippy
cargo test --all-features                                            # Tests
cargo deny check                                                     # Validate new dependencies
# Docker integration tests (requires Docker, not part of normal CI gate):
# cargo test -p uptrakit-controller reverse_proxy -- --ignored
```

There shouldn't even be any warnings in the output of these commands.

## Frontend (SvelteKit)

```sh
cd frontend && npm install                                   # Install dependencies
cd frontend && npm run check                                 # Svelte/TypeScript type check
cd frontend && npm run build                                 # Production build
```

## Documentation

All Markdown files (`.md`) are linted with `markdownlint`. Ensure that `markdownlint` passes without errors.
**Critically**, you must address all warnings and errors; do not silence them by adding exceptions to
`.markdownlintignore` or `.markdownlint.json` unless explicitly approved.

```sh
markdownlint --config .markdownlint.json docs/
```

CI runs these same checks. A PR that fails any of them will not merge.

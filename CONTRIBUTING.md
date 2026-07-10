# Contributing to Uptrakit

Thanks for helping improve Uptrakit. Focus on boring, testable, and well-reviewed changes.

Read [CONTEXT.md](CONTEXT.md) for the project's controlled vocabulary; use its terms in code, docs, and commit messages.

## Key links

- Development setup: [docs/development/setup.md](docs/development/setup.md)
- Testing expectations: [docs/development/testing.md](docs/development/testing.md)
- Coding standards: [docs/development/coding-standards.md](docs/development/coding-standards.md)
- Rust idioms: [docs/development/rust-idioms.md](docs/development/rust-idioms.md)
- Logging conventions: [docs/development/logging.md](docs/development/logging.md)
- Error handling: [docs/development/error-handling.md](docs/development/error-handling.md)
- PR process: [docs/development/pr-process.md](docs/development/pr-process.md)
- Plugin guidance: [docs/development/plugin-guidelines.md](docs/development/plugin-guidelines.md)
- Plugin system architecture: [docs/development/plugin-system.md](docs/development/plugin-system.md)
- Responsible AI use: [docs/development/ai-guidelines.md](docs/development/ai-guidelines.md)
- Security-aware development: [docs/security/secure-development.md](docs/security/secure-development.md)
- Vulnerability reporting: [SECURITY.md](SECURITY.md)

## Before you code

- Clone the repo, install Rust + Node.js, and provide your master key via `--master-key-file <path>`.
- Run the backend commands, linting, and frontend build listed in [docs/development/setup.md](docs/development/setup.md).
- Pre-commit and pre-push git hooks auto-install on the first `cargo build` or `cargo test` run
  (via [`husky-rs`](https://crates.io/crates/husky-rs)) — no manual setup required. See
  [Setup — Pre-commit hooks](docs/development/setup.md#pre-commit-hooks) for bypass options.
- Update documentation under `docs/` whenever behavior, config, or UI changes.

## Adding an API client endpoint

Use `crates/shared/openapi-client/src/hosts.rs` as the template: one method per operation, `&Uuid`
for IDs, internal helpers from `lib.rs`, and a unit test for any request body or query params.
Run `cargo xtask openapi-client-check` before committing — the check runs in CI (`backend-lint`)
and fails if the new method or its `paths.rs` constant is missing from both the client and the
reviewed ledger at `xtask/src/openapi_client_check/ledgers.rs`.

See [OpenAPI Client](docs/development/openapi-client.md) for the full guide.

## Testing and PRs

- Use the commands in [docs/development/testing.md](docs/development/testing.md) before opening a PR.
- Format, lint, and run `cargo deny check` as noted in the setup guide.
- After any backend route or REST-contract change, run `./scripts/regen-api.sh` and commit
  `crates/ui/web-api/openapi.json` + `frontend/src/lib/api/generated/`; CI gates on staleness.
- Describe what changed, how you tested it, and any rollout/migration risks in your PR body.
- Provide screenshots for UI changes and mention significant AI assistance if applicable.
- Follow Conventional Commits ([docs/development/pr-process.md](docs/development/pr-process.md)) for your PR title/message.

## Releases

Conventional commits on `main` automatically trigger [release-please](https://github.com/googleapis/release-please)
to open a release PR. Merging it creates a GitHub release with binary artifacts and Docker images.
See [docs/development/releases.md](docs/development/releases.md) for details.

## Responsible AI use

Read and follow [docs/development/ai-guidelines.md](docs/development/ai-guidelines.md) before incorporating AI-generated content. Your judgment and
ownership matter.

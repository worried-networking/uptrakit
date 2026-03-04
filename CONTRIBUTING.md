# Contributing to Uptrakit

Thanks for helping improve Uptrakit. Focus on boring, testable, and well-reviewed changes.

## Key links

- Development setup: [docs/development/setup.md](docs/development/setup.md)
- Testing expectations: [docs/development/testing.md](docs/development/testing.md)
- Coding standards: [docs/development/coding-standards.md](docs/development/coding-standards.md)
- Error handling: [docs/development/error-handling.md](docs/development/error-handling.md)
- PR process: [docs/development/pr-process.md](docs/development/pr-process.md)
- Plugin guidance: [docs/development/plugin-guidelines.md](docs/development/plugin-guidelines.md)
- Plugin system architecture: [docs/development/plugin-system.md](docs/development/plugin-system.md)
- Responsible AI use: [docs/development/ai-guidelines.md](docs/development/ai-guidelines.md)
- Security-aware development: [docs/security/secure-development.md](docs/security/secure-development.md)
- Vulnerability reporting: [SECURITY.md](SECURITY.md)

## Before you code

- Clone the repo, install Rust + Node.js, and export your master key (`UPTRAKIT_MASTER_KEY` or `--master-key-file`).
- Run the backend commands, linting, and frontend build listed in [docs/development/setup.md](docs/development/setup.md).
- Pre-commit and pre-push git hooks auto-install on the first `cargo build` or `cargo test` run
  (via [`husky-rs`](https://crates.io/crates/husky-rs)) — no manual setup required. See
  [Setup — Pre-commit hooks](docs/development/setup.md#pre-commit-hooks) for bypass options.
- Update documentation under `docs/` whenever behavior, config, or UI changes.

## Testing and PRs

- Use the commands in [docs/development/testing.md](docs/development/testing.md) before opening a PR.
- Format, lint, and run `cargo deny check` as noted in the setup guide.
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

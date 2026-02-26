# Pull Request Process

## Before Opening a PR

- Rebase onto the latest `main`.
- Run formatting and linting:
  - `cargo fmt --all`
  - `cargo clippy --all-targets --all-features -- -D warnings`
  - `cargo deny check`
  - `cd frontend && npm run check` (if touching the frontend)
- Run tests (`cargo test --all-features` or `cargo nextest run --all-features`).
- Build the frontend (`cd frontend && npm run build`).
- Update documentation for any behavioral/config/output changes using the new docs under `docs/`.

## PR Description Checklist

- Describe what changed and why.
- Detail how the change was tested (commands and manual steps).
- Note risks, rollout instructions, or migrations if applicable.
- Include screenshots for UI adjustments.

## Commit Messages

Use Conventional Commits to aid changelog generation.

```text
<type>(optional-scope): <description>

(optional body)
(optional footer)
```

Common types include `feat`, `fix`, `docs`, `refactor`, `perf`, `test`, `build`, `ci`, and `chore`. Use scopes that map to crates/components (e.g.,
`controller`, `agent`, `plugin-releases-github`). Breaking changes require `!` (e.g., `feat(api)!: ...`). Keep commits small and focused.

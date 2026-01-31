# Contributing to Uptrakit

Thanks for taking the time to contribute. Uptrakit is a security-adjacent system (it can run update commands on hosts), so we aim for changes that are boring, testable, and reviewable.

## Ways to contribute

- Bug reports (with logs + repro steps)
- Documentation improvements
- New providers (controller-side remote provider logic and/or agent-side local provider logic)
- Performance improvements (with evidence)
- CI / tooling improvements

If you're planning a larger change, open an issue first so we can agree on the shape before you write 2,000 lines of Rust.

For system design and architecture context, see [ARCHITECTURE.md](ARCHITECTURE.md). For the full documentation catalogue, see [docs/README.md](docs/README.md).

---

## Development setup

### Prerequisites

- Rust stable (preferably via `rustup`)
- A recent `cargo`
- Node.js (LTS) and npm (for the frontend)
- Optional but recommended: `cargo-nextest`, `cargo-deny`

Install `cargo-deny` for dependency checks:

```sh
cargo install cargo-deny
```

### Backend (Rust)

#### Build

```sh
cargo build
```

#### Format

```sh
cargo fmt --all
```

#### Lint

```sh
cargo clippy --all-targets --all-features -- -D warnings
```

#### Dependency checks

```sh
cargo deny check
```

This checks for security advisories, license compliance, and dependency issues.

### Frontend (SvelteKit)

The frontend is a SvelteKit SPA in the `frontend/` directory, using Skeleton UI and Tailwind CSS. The controller serves the built frontend at runtime.

#### Install dependencies

```sh
cd frontend && npm install
```

#### Type check

```sh
cd frontend && npm run check
```

#### Build

```sh
cd frontend && npm run build
```

The static build output goes to `frontend/build/`.

## Testing (required)

If your change can break, it should have a test. If it can’t reasonably be tested (e.g. some OS integration), explain why in the PR description and add at least one of:

- a unit test around the decision logic
- a contract test around parsing/serialisation
- a small integration test using a fixture or mock

### Run tests locally

```sh
cargo test --all-features
```

If you use `nextest`:

```sh
cargo nextest run --all-features
```

### What we test

- **Pure logic**: unit tests.
- **Provider behaviour**: parsing, version comparison, mapping upstream metadata → internal model.
- **API boundaries**: request/response types, (de)serialisation, backwards compatibility where relevant.
- **Error paths**: tests for expected failures and good error messages.

A PR that changes behaviour without adding/adjusting tests will usually be sent back for more work.

## Commit messages: Conventional Commits (required)

We use Conventional Commits so we can generate changelogs and reason about releases.

Format:

```text
<type>(optional-scope): <description>

(optional body)

(optional footer)
```

Common types:

- `feat`: new functionality
- `fix`: bug fix
- `docs`: documentation-only change
- `refactor`: behaviour-preserving code change
- `perf`: performance improvement
- `test`: tests only
- `build`: build system / dependencies
- `ci`: CI configuration
- `chore`: maintenance tasks

Examples:

- `feat(agent): add helper-scripts autodiscovery`
- `fix(controller): handle websocket reconnect backoff`
- `refactor(provider-github): simplify release tag normalisation`
- `docs: clarify sudoers requirements for agents`

Breaking changes:

- Add `!` after type/scope: `feat(api)!: change ws handshake payload`
- Or add `BREAKING CHANGE:` in the footer.

## Error handling

Use the `rootcause` crate for error construction and propagation.

Reference:

- <https://github.com/rootcause-rs/rootcause>

Guidelines:

- Add context at boundaries (host, provider, software item, operation).
- Avoid logging secrets or tokens.
- Prefer structured context over stringly-typed “something failed”.

## Pull requests

### Before opening a PR

- Rebase onto latest `main`
- Ensure clean formatting and lint:
  - `cargo fmt --all`
  - `cargo clippy --all-targets --all-features -- -D warnings`
  - `cargo deny check`
  - `cd frontend && npm run check` (if frontend changes)
- Run tests:
  - `cargo test --all-features` (or `cargo nextest run --all-features`)
- Build the frontend: `cd frontend && npm run build`
- Update docs if behaviour/config/output changes

### PR description should include

- What changed and why
- How it was tested (commands run + any manual steps)
- Risks / rollout notes / migration notes (if any)
- Screenshots for UI changes (if applicable)

## Design principles

- Keep boundaries crisp:
  - **Controller**: orchestration, scheduling, remote provider logic, MQTT integration, API/UI
  - **Agent**: local inspection + update execution, outbound-only WS connection
  - **Providers**: small, composable units with clear responsibilities
- Authorization uses a typed `Permission` enum, not raw role-name strings:
  - Route handlers check `user.has_permission(Permission::...)` — never compare role names directly.
  - JWT tokens carry resolved permissions, not roles. The frontend only sees permissions.
  - When adding a new protected route, pick the most specific existing permission or propose a new one. See the "Permissions model" section in [AGENTS.md](AGENTS.md) for the full list and instructions on adding a new permission.
- Security-sensitive code should be boring:
  - No shell injection footguns.
  - Validate inputs on any command execution path.
  - Any "custom script" feature must be explicit, documented, and treated as untrusted input.
- Logging:
  - Log high-level operational summaries.
  - Do not store full command output internally; rely on journald/stdout as designed.

## Adding or changing a provider

Please document in the same PR:

- How installed version is detected (agent side)
- How upstream/latest version is determined (controller side)
- Version comparison rules (semver, tag prefixes, build metadata handling)
- Update mechanism, required privileges, and failure modes
- Required config fields, with examples

Providers should be testable: keep parsing/version logic as pure functions where possible.

## PKI / CA rotation

The controller manages an internal CA for mTLS. Relevant rules:

- CA rotation, CRL generation, and server cert renewal are handled automatically by background tasks. See `AGENTS.md` § "PKI & CA rotation" for the full flow.
- All agent certificates record which CA signed them via the `ca_fingerprint` column. CRLs are partitioned by CA — never mix issuers.
- Runtime CA state flows through a `tokio::sync::watch` channel (`CaSnapshot`). Read from the receiver; never cache PEM strings in long-lived structs.
- When adding new code that needs the current CA material, accept a `watch::Receiver<CaSnapshot>` or read from `AppState.ca_snapshot`.
- External CA mode (`--ca-cert` / `--ca-key`) disables rotation. Ensure any new CA-related feature degrades gracefully when `managed` is `false`.

## Dependencies

- Avoid heavy dependencies without a strong reason.
- Prefer well-maintained crates with a clear track record.
- Anything affecting command execution, parsing untrusted input, crypto, or networking gets extra scrutiny.

## Security reports

For vulnerability reporting, security design details, and cryptographic information, see [SECURITY.md](SECURITY.md).

If you discover a security issue, please use GitHub's "Report a vulnerability" feature or follow the process described in the security policy. Do not post exploit details in public issues.

## Licence

By contributing, you agree that your contributions are licensed under MIT OR Apache-2.0.

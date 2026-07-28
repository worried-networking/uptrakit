# Dependency Policy

- Avoid heavy dependencies unless you can justify the need.
- Prefer well-maintained crates with a clear maintenance record.
- Give extra scrutiny to dependencies that touch command execution, untrusted input parsing, cryptography, or networking.
- Run `cargo deny check` to validate licenses, ensure no vulnerabilities from RustSec, and confirm there are no transitive policy violations.

## Single source of truth: `[workspace.dependencies]`

All dependencies — both external third-party crates and internal workspace crates — must be declared
in `[workspace.dependencies]` in the root `Cargo.toml`. Individual crate `Cargo.toml` files must
reference them via `workspace = true`. A crate may add extra features on top of the workspace
definition (e.g. `tokio = { workspace = true, features = ["fs"] }`), but must never redeclare a
version number or path locally.

```toml
# Root Cargo.toml
[workspace.dependencies]
serde = { version = "1", features = ["derive"] }
uptrakit-wire = { path = "crates/shared/wire" }

# crates/foo/Cargo.toml
[dependencies]
serde = { workspace = true }
uptrakit-wire = { workspace = true }
# adding extra features on top of the workspace definition is allowed:
tokio = { workspace = true, features = ["fs"] }
```

The only permitted exceptions are build scripts (`build.rs`): even then, the build-script
dependency itself must still be declared in `[workspace.dependencies]` and referenced via
`workspace = true` in `[build-dependencies]`.

**Never** add a bare version string (`dep = "1.2"`) or a local path (`dep = { path = "..." }`)
inside any crate's own `Cargo.toml`.

## Feature specification strategy

The workspace root pins each crate's version and sets a baseline of features (or `default-features
= false`). Individual crates declare only the _additional_ features they need. This applies
uniformly to all workspace dependencies, not just tokio.

For example, tokio:

```toml
# Root Cargo.toml — version only, no features
tokio = { version = "1" }

# crates/foo/Cargo.toml — the features this specific crate needs
tokio = { workspace = true, features = ["macros", "rt-multi-thread", "sync"] }
```

Rules:

1. Add only the specific features your crate actually uses.
2. If a dep is used only in tests, put it in `[dev-dependencies]` with the necessary features.
3. Binary crates that need `#[tokio::main]` require `macros` and `rt-multi-thread`.
4. Optional heavy deps must use `workspace = true, optional = true` and be gated with `dep:crate`
   in the feature definition.

### Self dev-dependencies (required interim mitigation)

A crate MUST declare a dev-dependency on itself (via `workspace = true`, enabling its own features) when both hold:

1. it carries `#[cfg(not(feature = "X"))]` fallback code whose shape depends on a shared dependency's feature state, and
2. any of its dev-dependencies can transitively force that foreign feature on.

Without it, dev-dep feature unification compiles the dependency's real signatures against the crate's fallback stubs,
and every bare `cargo test|clippy -p <crate>` fails (the 2026-07-20 proxmox incident). Worked example:
`crates/plugins/infrastructure/proxmox/Cargo.toml` `[dev-dependencies]` — see its inline comment.

Caveats:

- `cargo test --no-default-features -p <crate>` no longer means what it says: dev-dep features are additive and
  unsuppressable, so the crate's test targets always run in the enabled feature world. Feature-subset verification for
  such a crate is compile-only (lib `cargo check -p`, consumer binary builds).
- This rule is an **interim mitigation with an expiry**: retire it (and the proxmox self-dev-dep) when the infra-core
  feature-switched aliases become additive — the compliant long-term answer to feature desync is additive features, not
  more self-dev-deps.
- This rule covers only the type-switching/fallback shape. Feature-gated _test code_ with no fallback involved (the
  E0599 shape) is covered by the CI bare-crate clippy sweep over `crates/plugins/**` instead.
- Thin-binary constraint, stated precisely: `uptrakit-agent` is the binary that must stay free of `sea-orm-migration`
  (it has no registry/migration dependency at all). `uptrakit-agent-ssh` already carries `sea-orm-migration`
  unconditionally via `agent-ssh-runtime` — do not read this rule as implying otherwise.

## Feature-gated optional dependencies

Heavy dependencies that are only needed for specific functionality are gated behind Cargo
features so builds that do not need them avoid the compile-time and binary-size cost:

| Feature          | Crate                                     | Dependency gated                                                   | Default  |
| ---------------- | ----------------------------------------- | ------------------------------------------------------------------ | -------- |
| `oidc`           | `uptrakit-web-api`                        | `openidconnect` (+ transitive `oauth2`, RSA/EC crypto)             | enabled  |
| `oidc`           | `uptrakit-controller`                     | Propagates `uptrakit-web-api/oidc`                                 | enabled  |
| `embed-frontend` | `uptrakit-controller`                     | `rust-embed` + embedded SvelteKit build                            | disabled |
| `swagger-ui`     | `uptrakit-web-api`                        | `utoipa-swagger-ui`                                                | disabled |
| `email`          | `uptrakit-plugin-infrastructure-registry` | `mail-send` (SMTP client) via `uptrakit-notification-plugin-email` | disabled |
| `mock`           | `uptrakit-openapi-client`                 | `httpmock` (HTTP mocking)                                          | disabled |
| `daemon`         | `uptrakit-plugin-releases-docker`         | `bollard` (Docker daemon API)                                      | enabled  |

When adding a new optional dependency, gate it with `dep:crate_name` in the feature definition
and use `#[cfg(feature = "...")]` on all code paths that reference it (imports, struct fields,
route registrations, OpenAPI schemas, rate-limit entries, and test helpers).

## Re-export strategy

Where a shared crate already depends on a heavy crate, prefer re-exporting specific types rather
than adding direct dependencies in downstream crates:

- `uptrakit-openapi-client` re-exports `reqwest::Error` as `ReqwestError` so the CLI avoids a
  direct reqwest dependency.
- `uptrakit-plugin-infrastructure-core` re-exports `tokio::sync::mpsc` so plugin crates can use channels
  without depending on tokio directly (tokio only in dev-deps for `#[tokio::test]`).

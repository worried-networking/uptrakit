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
uptrakit-internal-wire = { path = "crates/shared/wire" }

# crates/foo/Cargo.toml
[dependencies]
serde = { workspace = true }
uptrakit-internal-wire = { workspace = true }
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
= false`). Individual crates declare only the *additional* features they need. This applies
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

## Feature-gated optional dependencies

Heavy dependencies that are only needed for specific functionality are gated behind Cargo
features so builds that do not need them avoid the compile-time and binary-size cost:

| Feature | Crate | Dependency gated | Default |
| --- | --- | --- | --- |
| `oidc` | `uptrakit-web-api` | `openidconnect` (+ transitive `oauth2`, RSA/EC crypto) | enabled |
| `oidc` | `uptrakit-controller` | Propagates `uptrakit-web-api/oidc` | enabled |
| `embed-frontend` | `uptrakit-controller` | `rust-embed` + embedded SvelteKit build | disabled |
| `swagger-ui` | `uptrakit-web-api` | `utoipa-swagger-ui` | disabled |
| `email` | `uptrakit-notification-plugin-registry` | `lettre` (SMTP client) via `uptrakit-notification-plugin-email` | disabled |
| `mock` | `uptrakit-openapi-client` | `httpmock` (HTTP mocking) | disabled |
| `daemon` | `uptrakit-plugin-releases-docker` | `bollard` (Docker daemon API) | enabled |

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

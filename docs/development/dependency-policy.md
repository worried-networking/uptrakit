# Dependency Policy

- Avoid heavy dependencies unless you can justify the need.
- Prefer well-maintained crates with a clear maintenance record.
- Give extra scrutiny to dependencies that touch command execution, untrusted input parsing, cryptography, or networking.
- Only list a crate in `[workspace.dependencies]` when two or more workspace members share it. Otherwise keep it in the crate-specific `Cargo.toml`.
- Run `cargo deny check` to validate licenses, ensure no vulnerabilities from RustSec, and confirm there are no transitive policy violations.

## Tokio feature strategy

The workspace root pins the tokio version only (`tokio = { version = "1" }`). Each crate declares
exactly the tokio features it uses. This keeps compile times lower and makes each crate's runtime
surface area explicit. When adding tokio usage to a crate:

1. Add the specific features you need (e.g. `sync`, `time`, `fs`) in that crate's `Cargo.toml`.
2. If tokio is only used in tests, put it in `[dev-dependencies]` with `macros` and `rt`.
3. Binary crates that need `#[tokio::main]` require `macros` and `rt-multi-thread`.

## Feature-gated optional dependencies

Heavy dependencies that are only needed for specific functionality are gated behind Cargo
features so builds that do not need them avoid the compile-time and binary-size cost:

| Feature | Crate | Dependency gated | Default |
| --- | --- | --- | --- |
| `oidc` | `uptrakit-web-api` | `openidconnect` (+ transitive `oauth2`, `reqwest` 0.12, RSA/EC crypto) | enabled |
| `oidc` | `uptrakit-controller` | Propagates `uptrakit-web-api/oidc` | enabled |
| `embed-frontend` | `uptrakit-controller` | `rust-embed` + embedded SvelteKit build | disabled |

When adding a new optional dependency, gate it with `dep:crate_name` in the feature definition
and use `#[cfg(feature = "...")]` on all code paths that reference it (imports, struct fields,
route registrations, OpenAPI schemas, rate-limit entries, and test helpers).

## Re-export strategy

Where a shared crate already depends on a heavy crate, prefer re-exporting specific types rather
than adding direct dependencies in downstream crates:

- `uptrakit-openapi-client` re-exports `reqwest::Error` as `ReqwestError` so the CLI avoids a
  direct reqwest dependency.
- `uptrakit-plugin-core` re-exports `tokio::sync::mpsc` so plugin crates can use channels
  without depending on tokio directly (tokio only in dev-deps for `#[tokio::test]`).

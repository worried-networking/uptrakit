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

## Re-export strategy

Where a shared crate already depends on a heavy crate, prefer re-exporting specific types rather
than adding direct dependencies in downstream crates:

- `uptrakit-openapi-client` re-exports `reqwest::Error` as `ReqwestError` so the CLI avoids a
  direct reqwest dependency.
- `uptrakit-provider-core` re-exports `tokio::sync::mpsc` so provider crates can use channels
  without depending on tokio directly (tokio only in dev-deps for `#[tokio::test]`).

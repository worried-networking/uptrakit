# Dependency Policy

- Avoid heavy dependencies unless you can justify the need.
- Prefer well-maintained crates with a clear maintenance record.
- Give extra scrutiny to dependencies that touch command execution, untrusted input parsing, cryptography, or networking.
- Only list a crate in `[workspace.dependencies]` when two or more workspace members share it. Otherwise keep it in the crate-specific `Cargo.toml`.
- Run `cargo deny check` to validate licenses, ensure no vulnerabilities from RustSec, and confirm there are no transitive policy violations.

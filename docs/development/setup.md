# Development Setup

## Ways to Contribute

- Bug reports (logs and repro steps)
- Documentation improvements
- New plugins (version detection, upstream checking, update execution)
- Performance improvements with evidence
- CI or tooling improvements
- Larger changes should start with an issue to align on scope

## Prerequisites

- Rust stable (managed with `rustup`)
- Recent `cargo` toolchain
- Node.js LTS and `npm` (for the frontend)
- Optional but recommended: `cargo-nextest`, `cargo-deny`

Install `cargo-deny` before running dependency checks:

```bash
cargo install cargo-deny
```

## Master Encryption Key

The controller requires a 256-bit master key (64 hex characters) for encrypting sensitive DB fields. Provide it via:

```bash
export UPTRAKIT_MASTER_KEY="0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
```

Alternatively use `--master-key-file <path>`. For development only, `--allow-plaintext-secrets` disables encryption at rest (and logs a warning), but
do not use the example key in production. Generate a production key with `openssl rand -hex 32`.

## Backend Commands

```bash
cargo build
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo deny check
```

## Build Speed Optimizations

The workspace includes several settings that speed up development builds
without affecting release builds or CI.

### Dev profile

The `[profile.dev]` section in the workspace root `Cargo.toml` applies
these optimizations automatically:

- **`debug = "line-tables-only"`** — reduces debug info to line tables.
  Backtraces still work, but variables are not inspectable in a debugger.
  Saves significant compile and link time.
- **`opt-level = 1` for dependencies** (`[profile.dev.package."*"]`) —
  compiles all third-party crates at optimization level 1. Minimal
  compile-time cost, but makes runtime (tests, local runs) noticeably
  faster. Workspace crates remain at `opt-level = 0` for fast incremental
  rebuilds.
- **`opt-level = 3` for `aws-lc-sys`** — the cryptography C library is
  always fully optimized to avoid extremely slow builds.

### macOS link-time optimization

`.cargo/config.toml` sets `split-debuginfo=unpacked` on macOS, which
skips the expensive `dsymutil` step during linking. Debug info is kept in
individual object files instead of being merged into a `.dSYM` bundle.

### `release-fast` profile

The default `release` profile uses `lto = "fat"` and `codegen-units = 1`
for maximum runtime performance, but is very slow to build. For iterative
release testing, use the `release-fast` profile:

```bash
cargo build --profile release-fast -p uptrakit-controller
```

This profile inherits from `release` but disables LTO and uses
`codegen-units = 16` for significantly faster builds at the cost of
slightly larger binaries and marginally lower runtime performance. It is
not intended for production artifacts.

## Frontend Workflow

```bash
cd frontend && npm install
cd frontend && npm run check
cd frontend && npm run build
```

The controller serves the static `frontend/build/` output at runtime.

### Embedded frontend (single-binary)

To build a self-contained controller binary with the frontend embedded:

```bash
cd frontend && npm ci && npm run build
cargo build -p uptrakit-controller --features embed-frontend
```

The `--static-dir` flag is not available when this feature is enabled.
See [Embedded Frontend](embedded-frontend.md) for details.

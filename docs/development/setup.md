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
- macOS only: `lld` linker (`brew install lld`) — see
  [Build Speed Optimizations](#build-speed-optimizations) below
- Optional but recommended: `cargo-nextest`, `cargo-deny`

Install `cargo-deny` before running dependency checks:

```bash
cargo install cargo-deny
```

## Pre-commit hooks

Git hooks are managed by [`husky-rs`](https://crates.io/crates/husky-rs) and auto-install on the
first `cargo build` or `cargo test` run. No manual setup is required.

The hooks are committed in `.husky/` and activated via `core.hooksPath = .husky` in the local git
config. This works for both regular clones and git worktrees.

### Hook tiers

| Hook | Trigger | What runs |
| --- | --- | --- |
| `pre-commit` | Every commit | `cargo fmt --check` (if `.rs` staged), `markdownlint` (if `.md` staged), `npm run lint` + `npm run format:check` (if `frontend/` staged) |
| `pre-push` | Every push | `cargo check`, `cargo clippy`, `cargo deny check`, `cargo test` (all with `db-sqlite`), frontend checks (if `node_modules` present) |

### Disabling hooks

- **CI / hermetic builds**: set `NO_HUSKY_HOOKS=1` before running `cargo build` or `cargo test`
  to prevent `core.hooksPath` from being set.
- **Emergency bypass**: `git commit --no-verify` or `git push --no-verify` skips all hooks for
  that invocation.

See [Quality Gates](quality-gates.md) for the full list of checks enforced at each tier.

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

### macOS linker and debug info

`.cargo/config.toml` applies two optimizations on macOS:

- **`split-debuginfo=unpacked`** — skips the expensive `dsymutil` step
  during linking. Debug info is kept in individual object files instead
  of being merged into a `.dSYM` bundle.
- **`lld` linker** — uses LLVM's `ld64.lld` instead of Apple's default
  `ld`, which is significantly faster for large binaries. Requires
  installing `lld` via Homebrew:

  ```bash
  brew install lld
  ```

  If `lld` is not installed, cargo will fail with a clear error
  pointing to this section. The linker path
  (`/opt/homebrew/bin/ld64.lld`) is hardcoded for Apple Silicon Macs.
  Intel Macs need to adjust the path in `.cargo/config.toml` to
  `/usr/local/bin/ld64.lld`.

### Dependency deduplication

The workspace pins `async-nats` and `rumqttc` with `default-features = false`
to avoid pulling in the `ring` cryptography library alongside `aws-lc-rs`.
All TLS is routed through `aws-lc-rs` via the workspace-level `rustls` and
`tokio-rustls` dependencies. Some upstream crates (`reqwest v0.12` via
`openidconnect`, `sqlx`) still force `ring` through feature unification —
this will resolve as upstreams migrate to provider-agnostic TLS.

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

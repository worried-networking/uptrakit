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

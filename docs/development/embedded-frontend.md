# Embedded Frontend

The `embedded-frontend` Cargo feature (declared on `uptrakit-controller-runtime`)
compiles the SvelteKit build output into the controller binary, producing a single
self-contained executable. This feature is **enabled by default** — and both
`uptrakit-controller` and `uptrakit-controller-standalone` enable it unconditionally
via their dependency spec, so it cannot be turned off from those crates.

## How it works

When enabled, the [`rust-embed`](https://crates.io/crates/rust-embed) crate
bakes the entire `frontend/build/` directory into the binary's read-only data
section at compile time. At runtime the controller serves these files directly
from memory with zero filesystem access.

### Build requirements

Whether a missing `frontend/build/index.html` is fatal depends on the build profile
(see [Strict frontend gate](releases.md#strict-frontend-gate)): release-profile builds
**fail at compile time**, while debug builds embed a placeholder stub and emit a
`cargo::warning`. Build the frontend first:

```bash
cd frontend && npm ci && npm run build
```

Then build the controller (the feature is enabled by default):

```bash
cargo build --release -p uptrakit-controller
```

### Behavior changes

| Aspect                  | Without feature                       | With `embedded-frontend`                              |
| ----------------------- | ------------------------------------- | ----------------------------------------------------- |
| Frontend auto-detection | Probes `frontend/build/`, `frontend/` | Debug builds only; skipped entirely in release builds |
| Frontend source         | Filesystem                            | Embedded in binary (debug: filesystem when detected)  |
| Binary size impact      | N/A                                   | ~400-500 KB larger (compressed SPA output)            |

There is **no runtime override**: a shipped (release-profile) controller always serves
the embedded copy. Debug builds probe `./frontend/build` then `./frontend` relative to
the process working directory, so frontend changes are picked up without recompiling —
that probe is what makes hot-reload development work, and it is compiled out of release
builds (`crates/core/controller-runtime/src/boot/init/validation.rs`).

### Caching

- `_app/immutable/*` (fingerprinted SvelteKit assets): `Cache-Control: public, max-age=31536000, immutable`
- `index.html`: `Cache-Control: no-cache`

### SPA routing

Non-API paths that do not match an embedded file are served `index.html`
(standard SPA fallback). API paths (`/api` and `/api/*`) return 404.

## CI considerations

CI's backend jobs run `--all-features` — which includes `embedded-frontend` — without
building the frontend first. They compile in the debug profile, so the stub is embedded
and the job proceeds; a separate `frontend` job builds the SPA. Release artifacts are
built by the release workflow, which downloads the frontend build artifact into
`frontend/build/` before compiling. See
[`.github/workflows/ci.yml`](../../.github/workflows/ci.yml).

## Related documentation

- [Development Setup](setup.md) -- general build prerequisites
- [Quality Gates](quality-gates.md) -- CI quality gates
- [Deployment](../end-user/deployment/README.md) -- deployment options including single-binary

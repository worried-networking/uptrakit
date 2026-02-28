# Embedded Frontend

The `embed-frontend` Cargo feature compiles the SvelteKit build output into the
controller binary, producing a single self-contained executable. This is opt-in
and does not affect default builds.

## How it works

When enabled, the [`rust-embed`](https://crates.io/crates/rust-embed) crate
bakes the entire `frontend/build/` directory into the binary's read-only data
section at compile time. At runtime the controller serves these files directly
from memory with zero filesystem access.

### Build requirements

The feature **hard-fails at compile time** if `frontend/build/index.html` does
not exist. Build the frontend first:

```bash
cd frontend && npm ci && npm run build
```

Then build the controller with the feature:

```bash
cargo build -p uptrakit-controller --features embed-frontend
```

### Behavior changes

| Aspect | Without feature | With `embed-frontend` |
| --- | --- | --- |
| `--static-dir` CLI arg | Available | Available; **overrides** embedded assets when set |
| Frontend auto-detection | Probes `frontend/build/`, `frontend/` | Skipped unless `--static-dir` is provided |
| Frontend source | Filesystem | Embedded in binary (or filesystem if `--static-dir` given) |
| Binary size impact | N/A | ~400-500 KB larger (compressed SPA output) |

The `--static-dir` argument is always compiled into the binary regardless of whether
`embed-frontend` is active. When both are present and `--static-dir` is specified, the
filesystem directory takes precedence over the embedded assets. This is useful for
hot-reload development workflows or when the embedded frontend needs to be overridden
without rebuilding the binary.

### Caching

- `_app/immutable/*` (fingerprinted SvelteKit assets): `Cache-Control: public, max-age=31536000, immutable`
- `index.html`: `Cache-Control: no-cache`

### SPA routing

Non-API paths that do not match an embedded file are served `index.html`
(standard SPA fallback). API paths (`/api` and `/api/*`) return 404.

## CI considerations

CI uses `--all-features`, which includes `embed-frontend`. The backend CI job
builds the frontend before running Cargo commands. See
[`.github/workflows/ci.yml`](../../.github/workflows/ci.yml).

## Related documentation

- [Development Setup](setup.md) -- general build prerequisites
- [Quality Gates](quality-gates.md) -- CI quality gates
- [Deployment](../end-user/deployment/README.md) -- deployment options including single-binary

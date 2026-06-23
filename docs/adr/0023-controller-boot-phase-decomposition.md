# 0023 — Controller Boot Phase Decomposition

**Date:** 2026-06-23 **Status:** Accepted

## Context

`crates/core/controller-runtime/src/lib.rs` had become the repo's worst CodeScene hotspot: Code Health
4.19/10, 234 revisions (most-changed file), a 916-line (logical LoC) `run_server` orchestrator (cyclomatic
complexity 31), and a 1695-line file total. Every new controller subsystem — NATS, plugins, OAuth, reload
coordinator, embedded services — bolted another inline phase into `run_server` with no seam. The function
accumulated cross-cutting locals that were threaded across dozens of lines with no structure enforced by the
compiler.

Three latent issues were also found during analysis and fixed as a pre-refactor commit:

1. `build_audit_logger` returned a dead `AuditFilter` that was discarded at the call site.
2. The reload-wiring block built a 7-tuple of always-`Some` `Option`s matched by an unreachable `_ => builder`
   arm.
3. `reload_audit_bridge` took 6 parameters (CodeScene Excess-Arguments smell; below Clippy's threshold of 7,
   so no lint fires).

See `docs/superpowers/specs/2026-06-22-controller-boot-decomposition-design.md` for the full problem analysis.

## Decision

Decompose `run_server` into a `boot/` submodule where each phase is a free async function returning an owned,
typed struct. The compiler enforces phase ordering through the dataflow: a later phase cannot call a function
that returns a struct before the struct's producer has been awaited.

### Component-struct dataflow pattern

Each phase function signature follows the same shape:

```rust
async fn <phase>(deps from prior phases…) -> Result<<PhaseOutput>>
```

The orchestrator (`boot/mod.rs`) is a flat sequence of ~12 phase calls (the function is ~178 lines including
the inline `ServeDeps` assembly block and comments). The pseudocode below is illustrative — exact signatures
(including optional `#[cfg(…)]` parameters) live in the source:

```rust
let cfg        = boot::config::load(args, &info).await?;
let crypto     = boot::crypto::init(&cfg)?;
let layout     = boot::directories::resolve().await?;
let db         = boot::persistence::open(&cfg, &layout).await?;
                 boot::crypto::verify_and_migrate(&db.db).await?;
let settings   = boot::settings::load_and_seed(&cfg, &db).await?;
let listeners  = boot::listeners::claim(&settings)?;          // ATOMIC — see below
let identity   = boot::identity::init(&cfg.booted.runtime, &db.db, …, &settings.reconciled).await?;
let components = boot::components::build(&cfg, &db, &settings, &crypto).await?;
// build ServeDeps from identity/components BEFORE assemble consumes them
let serve_deps = serve::ServeDeps { … };
let reload     = boot::reload::wire(booted, …, &identity, …).await?;
let state      = boot::app_state::assemble(settings, identity, components, reload, …).await?;
boot::recovery::run(&state).await?;
boot::serve::run(state, listeners, serve_deps, &info).await
```

The resulting binding clusters replace the single mutable context in `run_server` with typed outputs whose
field membership is stable and compiler-checked.

### AppState-first for post-assembly phases

Once `AppState` is assembled, all post-assembly phases (`recovery`, `serve`) take `Arc<AppState>` as their
primary argument plus only the values genuinely absent from `AppState` — the raw OS handles (`https_std`,
`pki_std_for_spawn`) and the `BackgroundTasks` accumulator. This prevents the phase structs from collectively
becoming a renamed god-bag of the values `AppState` already carries.

Actual signature:

```rust
pub(crate) async fn run(
    state: Arc<AppState>,
    listeners: Listeners,
    deps: ServeDeps,
    info: &BuildInfo,
) -> crate::Result<()>
```

`ServeDeps` is built inline in `run_server` before `assemble` consumes `identity` and `components` by move. It
carries the handles that the serve tail needs: `crl_manager`, `ca_tx`, `ca_managed`, `initial_ca_version`,
`has_external_tls_cert`, `service_connections`, `oauth_instance_for_shutdown` (moved via `Option::take`),
`nats_transport`, `shutdown_token`, `controller_id`, embedded-service fields (`builtin_host`,
`controller_installation_id`, `state_dir`), and server options (`https_addr`, `static_dir`, `pki_http_port`).

### Consecutive-FD atomicity constraint

The inherited-listener claim, HTTPS bind, PKI bind, and both `clear_cloexec` calls are kept in a single atomic
function: `boot::listeners::claim`. No fd-allocating call may execute between the HTTPS and PKI binds.
Splitting these across phase boundaries would break every-other-reexec (the process would re-inherit two
consecutive FDs only if no fd-allocator fired between them; see `coding-standards.md` "Reexec Hook Pattern").

### `large_futures` boxing constraint

The `large_futures = "deny"` Clippy lint is active workspace-wide. `async_main` wraps `run_server` in
`Box::pin(…)` today; this is preserved. Phase functions are written to avoid holding prior-phase locals across
`await` points within a phase — each completed phase struct is the only thing carried forward, keeping
per-phase futures small. If an individual phase future still trips the lint, it must be individually
`Box::pin`-ned, not suppressed.

### Indivisible reload+app_state boundary

`boot::reload::wire` and `boot::app_state::assemble` ship in one indivisible commit. The `AppState::builder()`
cannot finish without the coordinator handle, and removing the `Option` wrapper from the reload-wiring tuple
(latent issue #2) means reload fields must reach the builder unconditionally. Attempting to split them or
leaving a temporary `Option` shim produces a non-compiling intermediate state.

### Module layout

```text
crates/core/controller-runtime/src/boot/
  mod.rs           — run_server orchestrator + phase struct re-exports
  config.rs        — TOML load, bootstrap args parse, tracing init → BootConfig
  crypto.rs        — master key init/verify, key ring, ENC:v3 re-encrypt → Crypto
  directories.rs   — AppDirs::resolve + ensure_dirs + installation id → AppLayout
  persistence.rs   — DB init, default tenant → Persistence
  settings.rs      — Settings::load, reconcile, OIDC/enrollment seeding → SettingsBundle
  listeners.rs     — inherited-FD claim + HTTPS/PKI bind + cloexec (atomic fn) → Listeners
  identity/        — sub-module directory (oauth, pki, jwt sub-files) → Identity
  components.rs    — broadcasters, plugin catalog, dispatchers, surfaces → Components
  nats.rs          — all #[cfg(feature = "nats")] wiring (callers stay cfg-free)
  app_state.rs     — AppState::builder() assembly
  reload.rs        — coordinator + reconciler + audit-bridge wiring → ReloadWiring
  recovery.rs      — rollout cleanup + token denylist seeding
  serve.rs         — background tasks, embedded services, signal loop, shutdown
  init/            — former startup/ low-level helpers (master_key, database, jwt, …)
```

`boot/identity/` is started as a sub-module directory rather than a single file because its sub-concerns
(OAuth, PKI runtime including TLS setup and `cert_signer`, and JWT) already meet the split threshold. The
actual files are `{oauth, pki, jwt}.rs` plus `mod.rs`; there is no separate `tls.rs` — TLS setup and
`cert_signer` live inside `pki.rs`. Likewise all NATS feature-gating is concentrated in `boot/nats.rs` so
`components.rs` stays `cfg`-free.

The existing `startup/` module is unified into `boot/init/` — `startup` and `boot` are synonyms for "process
start", and one tree with a clear `init/` (helpers) vs `<phase>.rs` (orchestrators) split is less ambiguous
than two siblings.

`AppError`, `Result`, `ControllerReexecHook`, `async_main`, and `run` remain in `lib.rs`.

### Guard rails — avoiding a renamed god-struct

Several values fan out across many phases (`db_conn`, `shutdown_token`, `audit_emitter`). Guard rails prevent
the phase structs from collectively re-forming the god-bag:

- `AppState` is the convergence point: once assembled it already holds the high-fan-out values.
- `Arc<T>` and `CancellationToken` are cloned into phases that need them; they do not bloat struct field
  counts conceptually.
- Field-count gate: any phase struct exceeding ~7–8 fields is absorbing two concerns and must be split.

## Alternatives Considered

**Single mutable `BootContext` struct** — fewer call-site signatures, but reintroduces a `Option<T>`-field
god-bag that must be unwrapped later. Loses compiler-enforced ordering (any field can be accessed at any
point) and contradicts ADR 0001's deep-module/narrow-interface rule. Rejected.

**Builder pattern** (`BootBuilder::with_db()…`) — builders suit optional or unordered config. The boot path is
a fixed linear sequence where optional steps do not exist. Unneeded ceremony. Rejected.

**Private functions in `lib.rs` only** — leaves the 1695-line file and the CodeScene findings intact and does
not prevent `run_server` from re-monolithizing on the next subsystem addition. Rejected by the user in favour
of full-file scope.

## Consequences

- The compiler enforces boot phase ordering via the dataflow graph. A phase cannot run before the struct it
  needs exists; the orchestrator is self-documenting.
- `run_server` is a ~178-line orchestrator (~40 lines of phase calls plus the inline `ServeDeps` assembly and
  comments). New subsystems require a new `boot/<phase>.rs` file rather than an inline addition to an existing
  function (see "Boot phase pattern" in `coding-standards.md` for the mandatory convention).
- The consecutive-FD invariant is documented in one place (`boot/listeners::claim`) and guarded by the
  integration test suite (Docker gate: enrollment → reexec → re-enrollment).
- The `large_futures` constraint is satisfied structurally rather than by suppression. Per-phase futures are
  bounded by the phase's own locals, not by the full boot accumulation.
- The indivisible reload+app_state boundary is documented here and enforced by the commit protocol (the pair
  must compile and pass the SQLite gate as a unit).
- The `startup/` module rename to `boot/init/` is mechanical and has no semantic effect.
- CodeScene findings on `lib.rs` are eliminated; the file drops from 1695 lines to ~250 lines (`run_server`
  and all phase logic moved into `boot/`; `lib.rs` retains the error types, `ControllerReexecHook`,
  `async_main`/`run`, and the crate's module declarations).

## Cross-references

- Spec: `docs/superpowers/specs/2026-06-22-controller-boot-decomposition-design.md`
- ADR 0001: targeted per-concept extraction and the deep-module / narrow-interface rule
- ADR 0003: `AppState` sub-state pattern (`ServerState` / `PluginState`); `uptrakit-controller-core` boundary
- ADR 0008: graceful reload architecture; the irreversibly-bound key set that triggers reexec

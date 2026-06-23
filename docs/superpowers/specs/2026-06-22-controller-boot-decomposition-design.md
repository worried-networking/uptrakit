# Controller Boot Decomposition — Design Spec

**Date:** 2026-06-22
**Status:** Draft
**Target:** `crates/core/controller-runtime/src/lib.rs`
**Related ADRs:** 0001 (decomposition strategy), 0003 (controller-core boundary), 0008 (graceful reload)

## Problem

`controller-runtime/src/lib.rs` is the repo's worst CodeScene hotspot:

- **Code Health 4.19/10** (red-edge), **234 revisions** (most-changed file in the repo), rising friction.
- `run_server` (lines 206–1321): **916 LoC**, **cyclomatic complexity 31**, nesting depth 4, 5 bumps.
- `reload_audit_bridge` (1494–1660): 161 LoC, **6 arguments** (CodeScene Excess-Arguments smell,
  threshold 4). NB: this is _below_ Clippy's `too_many_arguments` default of 7 — no lint fires; it
  is complexity debt, not a gate failure.
- File: **1695 lines** → "Lines of Code in a Single File" / Brain Class risk.

Root cause: `run_server` is a single linear boot orchestrator. Every new subsystem (NATS,
plugins, OAuth, reload coordinator, embedded services) bolts another inline phase into this one
function. There is no seam, so each addition risks the whole boot path and the file churns
without bound. The ROI here is **comprehension and onboarding**, not defect count (boot runs
once; low runtime-defect surface).

## Goal

Decompose `run_server` into cohesive, independently-readable boot phases; eliminate every
CodeScene finding in the file; and fix the latent issues uncovered during analysis — without
changing externally observable boot behavior or ordering (one exception: dead-code removal,
which is behavior-neutral by definition).

## Chosen approach (primary recommendation)

**Cohesive component structs threaded through phase functions, in a new `boot/` submodule.**

Each phase function produces a cohesive, typed struct consumed by later phases; `AppState` is
assembled from those structs. The compiler enforces phase ordering through the dataflow (a phase
cannot run before the struct it needs exists). No mutable god-bag. This matches ADR 0003's
`AppState` sub-state pattern (`ServerState`/`PluginState`) and ADR 0001's deep-module rule
(narrow public interface, substantial hidden orchestration).

### Target orchestrator shape

```rust
// boot/mod.rs
pub(crate) async fn run_server(args: cli::Args, info: BuildInfo) -> Result<()> {
    // `BootConfig` carries the parsed RuntimeConfig, bootstrap args, config_path, AND the
    // un-spawned `ReloadCoordinator` + its watch channels (today's `startup::BootedConfig`).
    let cfg        = boot::config::load(args, &info).await?;          // Phase 0 + tracing + bootstrap args
    let crypto     = boot::crypto::init(&cfg).await?;                 // Phases 1, 4, 4b–4d
    let layout     = boot::directories::resolve(&cfg).await?;         // Phase 2
    let db         = boot::persistence::open(&cfg, &layout).await?;   // Phase 3
    let settings   = boot::settings::load_and_seed(&cfg, &db).await?; // Phases 5–7c (incl. validate → pki_http_port)
    let listeners  = boot::listeners::claim(&settings)?;             // Phase 8b (ATOMIC — see invariant); needs only settings
    let identity   = boot::identity::init(&cfg, &db, &layout, &settings).await?; // 7d OAuth, 9 PKI/TLS, 10 JWT, cert_signer
    let components = boot::components::build(&cfg, &db, &settings, &identity).await?; // web-api tail
    // Reload wiring extends the coordinator, sets the reexec hook (needs listener fds + oauth
    // instance), spawns coordinator + reconciler + audit-bridge, and yields the handles the
    // AppState builder consumes. MUST run before assembly (the handle is stored in AppState).
    let reload     = boot::reload::wire(&cfg, &components, &listeners, &identity).await?; // → ReloadWiring
    let state      = boot::app_state::assemble(cfg, db, settings, identity, components, reload)?;
    boot::recovery::run(&state).await?;                              // rollout cleanup + denylist seed
    boot::serve::run(state, listeners, &info).await                 // bg tasks, embedded, signals, serve, shutdown
}
```

Exact struct field membership is an implementation detail; the binding clusters in the current
`run_server` already map cleanly onto these groups (verified during analysis).

**Ordering note (verified against `lib.rs`):** listener binding (Phase 8b, lib.rs 431–516) runs
**before** PKI/TLS init (Phase 9, lib.rs 519) in the current code, so `listeners` precedes
`identity` in the orchestrator. `listeners::claim` depends only on `settings` (`reconciled.https_addr`,
`validated.pki_http_port`) — it consumes nothing from `identity`. Reload wiring must precede
`assemble` because the coordinator handle is a builder input. This preserves the exact current
order; no behavioral reordering is introduced.

### Module layout

New `crates/core/controller-runtime/src/boot/` submodule:

| File                  | Owns                                                                                                                                                                       |
| --------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `boot/mod.rs`         | `run_server` orchestrator (≈40 lines) + phase struct re-exports                                                                                                            |
| `boot/config.rs`      | TOML load, `OidcBootstrapArgs`/`EnrollmentBootstrapArgs` parse, tracing init → `BootConfig`                                                                                |
| `boot/crypto.rs`      | master key init/verify, AAD mappings, key ring, ENC:v3 reencrypt → `Crypto`                                                                                                |
| `boot/directories.rs` | `AppDirs::resolve` + `ensure_dirs` + installation id → `AppLayout`                                                                                                         |
| `boot/persistence.rs` | DB init, default tenant → `Persistence`                                                                                                                                    |
| `boot/settings.rs`    | `Settings::load`, reconcile, OIDC/enrollment/OAuth-defaults seeding → `SettingsBundle`                                                                                     |
| `boot/identity.rs`    | OAuth boot, PKI runtime, TLS, JWT, cert_signer → `Identity`                                                                                                                |
| `boot/listeners.rs`   | inherited-FD claim + HTTPS/PKI bind + cloexec (single atomic fn) → `Listeners`                                                                                             |
| `boot/components.rs`  | web-api component construction (broadcasters, plugin catalog, dispatchers, surfaces, …)                                                                                    |
| `boot/nats.rs`        | all `#[cfg(feature = "nats")]` wiring — transport connect + `with_nats` on notification/event/batch broadcasters + `credential_sources.nats_url` (callers stay `cfg`-free) |
| `boot/app_state.rs`   | `AppState::builder()` assembly                                                                                                                                             |
| `boot/reload.rs`      | reload coordinator + reconciler + audit bridge wiring → `ReloadWiring`                                                                                                     |
| `boot/recovery.rs`    | owner-aware rollout cleanup + token denylist seeding                                                                                                                       |
| `boot/serve.rs`       | background tasks, embedded service registration, signal handlers, server spawn, select loop, graceful shutdown                                                             |

`AppError`, `Result`, `ControllerReexecHook`, `async_main`, `run` stay in `lib.rs`. All extracted
items are `pub(crate)` or narrower (`unreachable_pub = "deny"`).

The existing `startup/` module (low-level init helpers: `master_key`, `database`, `encryption`,
`jwt`, `oauth`, `pki_init`, `settings`, `validation`, `bootstrap`, `installation_id`) is unified
into this tree as `boot/init/` — `startup` and `boot` are synonyms for "process start", and one
module with a clear `init/` (helpers) vs `<phase>.rs` (orchestrators) split is less ambiguous than
two siblings. The phase fns call `boot::init::*`. This relocation is mechanical (module rename +
path updates) and lands as the last structural step.

`boot/identity.rs` (five sub-concerns: OAuth, PKI runtime, TLS, JWT, cert_signer) and
`boot/components.rs` (broadcasters, plugin catalog, dispatchers, surfaces) are the highest-risk
files for re-acquiring the brain-class smell at phase granularity. The five identity sub-concerns
already meet the split threshold, so **start identity as a sub-module directory**
(`boot/identity/mod.rs` + `boot/identity/{oauth,pki,tls,jwt}.rs`) rather than consolidating into
one file and splitting later — the module names already exist; use them from the start. Likewise,
all NATS feature-gating is concentrated in `boot/nats.rs` (above) so `components.rs` does not
accumulate scattered `#[cfg]` blocks — the same one-level-down decomposition principle.

### Cross-cutting values — avoiding a renamed god-struct

A real risk (raised in review): several values fan out across many phases — `db_conn`
(~12 call sites), `shutdown_token`, `controller_id`, `audit_emitter`, `reconciled.*`. If every
phase struct re-carries these, the structs collectively become the `BootContext` god-bag this
design rejects. Guard rails, binding on the implementation:

- **`AppState` is the convergence point.** Once assembled, `Arc<AppState>` already holds
  `db_conn`, `shutdown_token`, `controller_id`, `audit_emitter`, broadcasters, etc. The
  post-assembly phases (`recovery`, `serve`) take `Arc<AppState>` **exclusively** plus only the
  values genuinely _not_ in `AppState` — the raw OS handles (`https_std`, `pki_std_for_spawn`) and
  the `BackgroundTasks` accumulator. Target `serve::run(state: Arc<AppState>, listeners: Listeners,
&info)` — narrow and cohesive.
- **Cheap shared handles are cloned, not re-owned.** `Arc<T>` and `CancellationToken` are cloned
  into the phases that need them; they do not bloat struct field counts conceptually.
- **Field-count gate.** Enumerate every struct's fields before writing code; if any phase struct
  exceeds ~7–8 fields it is absorbing two concerns — split it. This is a hard check, not advice.

### Alternatives considered

- **Single mutable `BootContext` struct** — fewer signatures, but reintroduces a god-struct of
  `Option<T>` fields unwrapped later; contradicts ADR 0001's deep-module/narrow-interface rule and
  loses compiler-enforced ordering. Rejected.
- **Builder pattern** (`BootBuilder::with_db()…`) — builders fit optional/unordered config; the
  boot path is a fixed linear sequence. Unneeded ceremony. Rejected.
- **`run_server`-only / private fns in `lib.rs`** — leaves the 1695-line file and
  `reload_audit_bridge` flagged. User chose full-file scope. Rejected.

## Latent issues to fix in this change

The user opted to fix latent issues alongside the refactor. Found during analysis:

1. **Dead `AuditFilter` return (confirmed).** `build_audit_logger` returns `(AuditFilter,
AuditLogDispatcher)`; the caller binds `_audit_filter` (lib.rs 753) and discards it. `AuditFilter`
   is constructed nowhere else and never consumed — runtime filtering flows through
   `AuditDispatcherReloadable`'s watch channel (`audit_log_filter_rx`), seeded from
   `runtime.audit`. **Fix:** change `build_audit_logger` to return only `AuditLogDispatcher`, and
   narrow the combined import at lib.rs 59 from `use uptrakit_audit_log::{AuditFilter,
   AuditLogDispatcher}` to `{AuditLogDispatcher}` (otherwise `warnings = "deny"` fires an
   unused-import error). Behavior-neutral.

2. **All-`Some` Option tuple + unreachable match (confirmed).** The reload-wiring block
   (lib.rs 835–979) builds a 7-tuple of `Option`s, always populated with `Some(...)`, then
   matches with an unreachable `_ => builder` arm. **Fix:** return a non-optional `ReloadWiring`
   struct from `boot::reload::wire` and apply its fields to the builder directly. Removes dead
   branch + cyclomatic load. **Note:** the `_reconciler` `JoinHandle` (lib.rs 919) is
   deliberately dropped to detach the task (Tokio drop does not cancel). `ReloadWiring` must NOT
   store it — preserve detach-on-drop; storing it would change task lifetime.

3. **`reload_audit_bridge` 6 args (CodeScene debt, not a Clippy gate).** At 6 params it is below
   Clippy's `too_many_arguments` default of 7, so no lint fires — but it trips CodeScene's
   Excess-Arguments smell (threshold 4). **Fix:** bundle the three status-watch senders +
   `config_path` into a `ReloadBridgeChannels` struct (semantic-role name per coding-standards
   "parameter-struct pattern"). Clears the CodeScene smell; good practice regardless of the lint.

4. **Silent unused-var suppression.** `spawn_background_tasks` ends with
   `let _ = &service_connections;` (lib.rs 1452) to quiet a `nats`-disabled warning — a silent
   suppression the snapshot bans. The `service_connections` param is read only inside the
   `#[cfg(feature = "nats")]` block, but its call-site argument (lib.rs 1174) is passed
   **unconditionally**, so gating only the parameter would break the non-nats build. **Fix:** gate
   **both** the parameter AND the call-site argument with `#[cfg(feature = "nats")]` — exactly as
   the adjacent `nats_transport` param/arg already are (lib.rs 1386 / 1176) — then delete the
   `let _ =` line. Purely additive (enabling `nats` only _adds_ the param+arg); no
   `cfg(not(feature))` and no suppression.

5. **`reload_audit_bridge` ring buffer `Vec::remove(0)`** (O(n) shift, cap 20; four sites,
   lib.rs 1556/1580/1596/1600). **Fix (minor, optional):** use `VecDeque` with `pop_front` when
   `len > 20`. Include only if it stays a clean drive-by; do not expand scope.

6. **~~`master_key_hex` as plain `String`~~ — WITHDRAWN (not an issue).** Verification against
   `startup/master_key.rs` shows `init_master_key` already returns
   `crate::Result<Option<uptrakit_wire::SecretString>>`, and `ServiceCredentialSources::new` already
   takes `Option<uptrakit_wire::SecretString>`. The master key is already a `SecretString` end to
   end — no plain `String` exists. The only requirement on the refactor: the new `MasterKey`/`Crypto`
   struct field preserves the `Option<uptrakit_wire::SecretString>` type (no re-wrap, no
   `.expose_secret()` roundtrip). No deferred follow-up needed.

## Hard constraints (must not regress)

- **`large_futures = "deny"`.** `async_main` currently `Box::pin(run_server(...))`s to satisfy
  this. The orchestrator future still aggregates all phase locals → **keep the `Box::pin`** (and
  box individual phase futures if any single one trips the lint). Proactively: write phase
  functions to avoid holding prior-phase locals across `await` points within a phase, and let
  each completed phase struct be the only thing carried forward, so per-phase futures stay small.
- **Consecutive-FD invariant.** The inherited-listener claim + HTTPS bind + PKI bind + both
  `clear_cloexec` calls must remain in **one function** (`boot::listeners::claim`). No
  fd-allocating call may execute between the HTTPS and PKI binds (documented at lib.rs 425–516).
  Splitting these across phase boundaries would break every-other-reexec.
- **Boot ordering and semantics unchanged** (except dead-code removal). Same phase order, same
  error mapping (`AppError::*`), same feature-gating (`embedded-*`, `nats`, `oidc`, `zeroconf`,
  `journald`, `test-utils`) preserved verbatim.
- **No new dependencies.** All work uses existing crates (`VecDeque` is std). No version pins
  required.
- **Lint hygiene:** no new `#[allow]`; any suppression uses `#[expect(..., reason = "...")]`
  (`allow_attributes`/`allow_attributes_without_reason = "deny"`). No `unwrap`/`expect`/indexing
  in production paths (existing `#[expect(clippy::expect_used, …)]` on `run`/journald stay).

## Verification

Boot path is enrollment/wire/service-lifecycle-sensitive → full gate set required:

```bash
cargo fmt --all --check
cargo check --no-default-features --features db-sqlite
cargo check --all-features
cargo clippy --all-targets --no-default-features --features db-sqlite
cargo clippy --all-targets --all-features
cargo test --all-features
cargo deny check
markdownlint --config .markdownlint.json '**/*.md'
# Boot-path integration (Docker):
docker build -f docker/Dockerfile.test -t uptrakit-test:latest . \
  && cargo test -p uptrakit-integration-tests -- --ignored
```

The Docker integration suite is the primary behavioral guard — it exercises real boot, enrollment,
reexec (the consecutive-FD invariant), and graceful shutdown. Reverse-proxy tests not required
(no proxy code touched). Per-phase unit tests added where a phase has pure logic worth pinning
(e.g. `boot::settings` reconcile seams); do not test upstream-crate behavior (testing.md).

### Implementation sequencing & commit boundaries

1. **Commit 1 — latent fixes, pre-refactor, no structural moves.** Land issues #1, #3, #4 (and #5
   if clean) as a single standalone commit _before_ any extraction, while the code is still one
   file. (Issue #2 is structurally part of the reload extraction; #6 is withdrawn.) They are
   self-contained and behavior-neutral today; isolating them means a later `git bisect` on a
   Docker-gated boot regression attributes failures unambiguously to the structural refactor, not
   to a semantic fix.
2. **Commits 2..N — pure structural extraction, one phase per commit.** Each compiles and passes
   the SQLite gate independently.
3. **Indivisible pair.** The reload-wiring extraction (`boot::reload::wire` returning the
   non-optional `ReloadWiring`) and the `boot::app_state::assemble` extraction **must ship in one
   commit** — the `AppState::builder()` cannot finish without the coordinator handle, and removing
   the `Option` wrapper (issue #2) means the reload fields must reach the builder unconditionally.
   Do not attempt to split them or leave the `Option` wrapper as a "temporary" shim. The
   per-commit gate applies to every phase except this pair.

Verify each commit independently compiles and passes the SQLite gate; behavior-preserving each
step (commit 1 excepted — it removes confirmed dead code).

## Documentation deliverables

- **New ADR** `docs/adr/0023-controller-boot-phase-decomposition.md` — records the boot-phase
  decomposition pattern (component-struct dataflow, the consecutive-FD atomicity constraint, the
  `large_futures` boxing constraint). Architectural/structural decision → ADR is non-optional per
  project convention. Cross-reference this spec.
- **`docs/development/coding-standards.md`** — append a "Boot phase pattern" note under the
  controller section (**mandatory deliverable**, not optional). Decomposition alone does not slow
  the 234-revision churn — without a documented convention, the next subsystem bolts inline into
  `boot/serve.rs` or `boot/components.rs` and the Brain Class re-forms under a new name. The note
  binds future contributors: new subsystems get a new phase fn producing a typed struct (or a
  named sub-module), never an inline addition to an existing phase.
- **No** README / API / OpenAPI / user-guide / runbook changes — refactor is internal with no
  externally observable behavior, surface, or config change. Stated explicitly to satisfy the
  doc-impact check.

## Out of scope / deferred

- Other controller-runtime hotspots beyond `lib.rs` (`tasks.rs`, `startup.rs`, `reload/*`) — user
  scoped this change to `lib.rs` only.
- Any change to reload semantics, reexec triage, or the irreversibly-bound key set (ADR 0008
  amendment territory).
- Pre-existing `file_digest` format inconsistency (`boot_config` writes `sha256:…` digests while
  `reload_audit_bridge`'s copy writes plain hex / empty-on-error) — surfaced during review.
  Out of scope to keep this refactor behavior-neutral; flag as a separate follow-up.

## Open questions

1. **Issue #5 (`VecDeque` ring buffer)** — include as drive-by, or leave the `Vec::remove(0)`?
   (Default: include only if trivially clean.)
2. ~~Coding-standards "Boot phase pattern" note~~ — **resolved:** mandatory deliverable (see
   Documentation deliverables). Decomposition without the convention just relocates the debt.

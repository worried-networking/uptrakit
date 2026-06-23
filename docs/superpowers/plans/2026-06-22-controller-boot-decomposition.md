# Controller Boot Decomposition Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for
> tracking.

**Goal:** Decompose the 916-LoC `run_server` boot orchestrator in `crates/core/controller-runtime/src/lib.rs`
into cohesive component-struct phases under a new `boot/` submodule, clearing every CodeScene finding and
fixing the confirmed latent issues — behavior-preserving.

**Architecture:** Each boot phase becomes a free `async fn` (or sync where no `.await`) that calls the
existing low-level init helpers plus the inline construction currently in `run_server`, and returns one owned,
typed component struct. Later phases consume earlier structs by move/borrow, so the compiler enforces
ordering. `AppState` is assembled from those structs; post-assembly phases take `Arc<AppState>` plus only the
raw OS handles. No mutable god-bag. **Final module shape (single tree):** `startup/` and `boot/` are unified
under one `boot/` module — `boot/init/` holds the relocated low-level init helpers (former `startup/*`:
`master_key`, `database`, `encryption`, `jwt`, `oauth`, `pki_init`, `settings`, `validation`, `bootstrap`,
`installation_id`), and `boot/<phase>.rs` holds the phase orchestrators that call them. This removes the
`startup` vs `boot` naming overlap (both mean "process start"). The relocation lands last among structural
work (Task 15) so earlier tasks reference `crate::startup::*` until a single rename.

**Tech Stack:** Rust, Tokio (multi-thread runtime), axum, sea-orm, `rootcause` errors,
`uptrakit-config-reload`.

**Spec:** `docs/superpowers/specs/2026-06-22-controller-boot-decomposition-design.md`

## Global Constraints

(Copied verbatim from spec — every task's requirements implicitly include these.)

- **Behavior-preserving.** Same boot order, same `AppError::*` mapping, same feature-gating (`embedded-*`,
  `nats`, `oidc`, `zeroconf`, `journald`, `test-utils`) preserved verbatim. Only exception: Task 1 removes
  confirmed dead code.
- **Consecutive-FD invariant.** The inherited-listener claim + HTTPS bind + PKI bind + both `clear_cloexec`
  calls MUST remain in one function. No fd-allocating call (file open, socket, dup) between the HTTPS and PKI
  binds (lib.rs 424–516).
- **`large_futures = "deny"`.** Keep `Box::pin(run_server(...))` in `async_main` (lib.rs 198). Write phase fns
  so prior-phase locals are not held across `.await` points within a phase; carry forward only the completed
  phase struct.
- **Additive features only.** Use `#[cfg(feature = "X")]`; never `#[cfg(not(feature = "X"))]` or
  `#[cfg_attr(not(feature = ...), ...)]` (snapshot: `coding-standards.md#feature-flags`).
- **Lint hygiene.** No new `#[allow]`; suppression uses `#[expect(..., reason = "...")]` (Cargo.toml:
  `allow_attributes`/`allow_attributes_without_reason = "deny"`). No `unwrap`/`expect`/indexing/slicing in
  production (Cargo.toml panic-prevention lints all `deny`).
- **Visibility.** All extracted items `pub(crate)` or narrower (`unreachable_pub = "deny"`).
- **Errors.** `rootcause::Report` via `report!()` / `.context()` / `.context_transform()`; `crate::Result<T>`
  alias; no error masking (`coding-standards.md#error-handling`).
- **Locks.** `parking_lot::Mutex` only in async code; guard dropped before `.await` (none introduced here, but
  preserve existing).
- **No new dependencies.** `VecDeque` is std.
- **Commit format.** Conventional Commits `<type>(scope): <desc>`, scope `controller` (`commit-messages.md`).
  End commit messages with the project trailer.

### Per-task verification gate (the "test cycle" for this refactor)

This is a behavior-preserving refactor: the existing test suite + the Docker integration suite are the
behavioral oracle; do not write new behavioral tests for moved code. Each structural task's gate is:

```bash
cargo fmt --all
cargo check --no-default-features --features db-sqlite
cargo check --all-features
cargo clippy --all-targets --no-default-features --features db-sqlite
cargo clippy --all-targets --all-features
cargo test -p uptrakit-controller-runtime --all-features
```

The full Docker boot-path integration suite runs once after the indivisible reload+assemble commit (Task 12)
and once at the end (Task 17) — it is the primary behavioral guard:

```bash
docker build -f docker/Dockerfile.test -t uptrakit-test:latest . \
  && cargo test -p uptrakit-integration-tests -- --ignored
```

Add per-phase **unit** tests only where a phase exposes genuinely pure logic worth pinning (called out in the
relevant task). Do not test upstream-crate behavior (`testing.md`).

---

## Phase 0 — Latent fixes (single pre-refactor commit)

> Lands while the code is still one file, so a later `git bisect` on a boot regression attributes failures to
> the structural refactor, not to these semantic fixes (spec §Implementation sequencing 1). **Issue #2
> (Option-tuple → `ReloadWiring`) is NOT here** — it is structurally entangled with the reload extraction and
> ships in Task 12.

### Task 1: Latent fixes bundle

**Files:**

- Modify: `crates/core/controller-runtime/src/lib.rs:59` (import), `:753` (`build_audit_logger` call site),
  `:1332-1368` (`build_audit_logger`), `:1377-1453` + `:1172-1177` (`spawn_background_tasks` def + call),
  `:1494-1660` (`reload_audit_bridge`), `:935-942` (bridge spawn call site), `:1554-1600` (ring-buffer sites)
- Test: none (behavior-neutral; existing suite is the oracle)

**Interfaces:**

- Produces:
  - `fn build_audit_logger(runtime, db_conn) -> Result<AuditLogDispatcher>` (was
    `Result<(AuditFilter, AuditLogDispatcher)>`)
  - `struct ReloadBridgeChannels { file_state_tx, last_reload_tx, recent_events_tx, config_path }` (all the
    existing param types) and `async fn reload_audit_bridge(rx, emitter, channels: ReloadBridgeChannels)`

- [ ] **Step 1: Fix #1 — drop dead `AuditFilter`.** In `build_audit_logger` (lib.rs 1332), change the return
      type to `Result<AuditLogDispatcher>` and return only the dispatcher (delete the `AuditFilter::new(...)`
      first tuple element in both the `FilterMode::None` early-return at 1347 and the final `Ok` at 1364). At
      the call site (lib.rs 753) change `let (_audit_filter, audit_dispatcher) = build_audit_logger(...)` to
      `let audit_dispatcher = build_audit_logger(...)`. Narrow the import at lib.rs 59 from
      `use uptrakit_audit_log::{AuditFilter, AuditLogDispatcher};` to
      `use uptrakit_audit_log::AuditLogDispatcher;`. Rationale: `AuditFilter` is constructed nowhere else;
      runtime filtering flows through `AuditDispatcherReloadable`'s `audit_log_filter_rx`. (snapshot:
      error-handling; `warnings = "deny"` requires the import narrow.)

- [ ] **Step 2: Fix #3 — `reload_audit_bridge` param struct.** Define near the fn:

```rust
/// Status-watch channels + config path consumed by `reload_audit_bridge`.
pub(crate) struct ReloadBridgeChannels {
    pub file_state_tx: tokio::sync::watch::Sender<uptrakit_config_reload::ConfigFileState>,
    pub last_reload_tx:
        tokio::sync::watch::Sender<Option<uptrakit_config_reload::LastReloadInfo>>,
    pub recent_events_tx: tokio::sync::watch::Sender<Vec<serde_json::Value>>,
    pub config_path: std::path::PathBuf,
}
```

Change the signature to
`async fn reload_audit_bridge(mut rx: …UnboundedReceiver<ReloadAuditEvent>, emitter: AuditEmitter, channels: ReloadBridgeChannels)`
and update the body to read `channels.file_state_tx` / `channels.last_reload_tx` / `channels.recent_events_tx`
/ `channels.config_path`. Update the spawn call site (lib.rs 935-942) to build `ReloadBridgeChannels { … }`.
(snapshot: `coding-standards.md#parameter-struct-pattern` — semantic-role name. NB CodeScene smell, not a
Clippy gate.)

- [ ] **Step 3: Fix #5 (optional, only if clean) — ring buffer.** In `reload_audit_bridge`, the recent-events
      list uses `v.push(json); if v.len() > 20 { v.remove(0); }` at four sites (lib.rs ~1554, 1579, 1595,
      1600). If the watch channel value can be changed to `VecDeque<serde_json::Value>` without rippling into
      the `AppState`/endpoint consumer types, switch to
      `v.push_back(json); if v.len() > 20 { v.pop_front(); }`. **If the consumer expects `Vec`, skip this
      step** (do not expand scope) and note it in the commit body.

- [ ] **Step 4: Fix #4 — additive nats gating (both sites).** In `spawn_background_tasks` (lib.rs 1377), the
      `service_connections` parameter is read only inside the `#[cfg(feature = "nats")]` block, BUT its
      call-site argument (lib.rs 1174, `&service_connections,`) is currently passed **unconditionally** — so
      gating only the parameter breaks the non-nats build. Gate **both**, mirroring the adjacent
      `nats_transport` param/arg (lib.rs 1386 / 1176): add `#[cfg(feature = "nats")]` to the parameter
      (`#[cfg(feature = "nats")] service_connections: &…::ServiceConnectionRegistry,`) AND to the call-site
      argument (`#[cfg(feature = "nats")] &service_connections,`). Delete the trailing
      `let _ = &service_connections;` (lib.rs 1452). Fully additive — no `cfg(not(feature))`, no suppression.
      Verify it compiles with and without `--features nats`. (snapshot: `coding-standards.md#feature-flags`.)

> Issue #6 (`master_key_hex` → `SecretString`) is **withdrawn**: `startup::init_master_key` already returns
> `crate::Result<Option<uptrakit_wire::SecretString>>` and `ServiceCredentialSources::new` already takes
> `Option<uptrakit_wire::SecretString>` — the master key is already a `SecretString` end to end. No change
> needed here; later phases (Task 4) just preserve that type. Do not add a `SecretString::from(...)` wrap or
> an `.expose_secret()` roundtrip.

- [ ] **Step 5: Run the per-task gate** (see Global Constraints) with AND without `--features nats`. Expected:
      all green, no clippy warnings, no unused-import error.

- [ ] **Step 6: Commit**

```bash
git add crates/core/controller-runtime/src/lib.rs
git commit -m "refactor(controller): pre-decomposition latent fixes

Drop dead AuditFilter return; ReloadBridgeChannels param struct; additive
#[cfg(feature=nats)] on spawn_background_tasks param+arg (both sites).
Behavior-neutral; lands before boot/ extraction for bisect isolation.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Phase 1 — `boot/` scaffold and leaf-phase extraction

> Pure structural moves. Each task moves a contiguous block out of `run_server` into a `boot/<phase>.rs` fn
> returning a typed struct, then rewrites that part of `run_server` to call it. Each task compiles and passes
> the SQLite + clippy gate independently. Field-count gate: if any phase struct would exceed ~7–8 fields, it
> is absorbing two concerns — split it (spec §Cross-cutting values).

### Task 2: Create the `boot/` module skeleton

**Files:**

- Create: `crates/core/controller-runtime/src/boot/mod.rs`
- Modify: `crates/core/controller-runtime/src/lib.rs` (add `mod boot;` near the other `mod` decls ~line 4;
  move `run_server` body into `boot::run_server` incrementally in later tasks)

**Interfaces:**

- Produces: `pub(crate) mod boot;` with
  `boot::run_server(args: cli::Args, info: BuildInfo) -> crate::Result<()>` as the entry the existing
  `async_main` calls.

- [ ] **Step 1: Add the module.** Insert `mod boot;` in lib.rs alphabetically among existing `mod`
      declarations (after `mod audit_enricher;` is fine). Create `boot/mod.rs` with a crate-invariant
      doc-comment and re-export the current `run_server` by **moving** the entire `async fn run_server`
      (lib.rs 206-1321) into `boot/mod.rs` as `pub(crate) async fn run_server(...)`. Keep `AppError`,
      `Result`, `ControllerReexecHook`, `async_main`, `run`, and all helper fns (`build_audit_logger`,
      `spawn_background_tasks`, `spawn_zeroconf`, `spawn_pki_http`, `reload_audit_bridge`, `file_digest`) in
      `lib.rs` for now — `boot::run_server` calls them via `crate::`.

- [ ] **Step 2: Fix paths.** Update `async_main` (lib.rs 198) to call
      `Box::pin(boot::run_server(args, info))`. Resolve all `crate::`-relative references inside the moved
      `run_server` (the helper fns, `AppError`, `reexec::*`, `startup::*`, `tasks::*`, `server::*`, etc. are
      all crate-internal — prefix with `crate::` as needed).

- [ ] **Step 3: Run the per-task gate.** Expected green. `Box::pin` preserved on the call.

- [ ] **Step 4: Commit** — `refactor(controller): introduce boot/ module, move run_server`

### Task 3: Extract `boot::config` (Phase 0 + tracing + bootstrap args)

**Files:**

- Create: `crates/core/controller-runtime/src/boot/config.rs`
- Modify: `crates/core/controller-runtime/src/boot/mod.rs` (lines corresponding to old lib.rs 207-323)

**Interfaces:**

- Consumes: `cli::Args`, `BuildInfo`
- Produces:

```rust
pub(crate) struct BootConfig {
    pub booted: crate::startup::BootedConfig, // carries runtime + un-spawned coordinator + all channels
    pub oidc_bootstrap: crate::cli::OidcBootstrapArgs,
    pub enrollment_bootstrap: crate::cli::EnrollmentBootstrapArgs,
    pub config_path: std::path::PathBuf,
    pub args: crate::cli::Args, // master_key_from, verbose still needed downstream
}
pub(crate) async fn load(args: cli::Args, info: &BuildInfo) -> crate::Result<BootConfig>;
```

- [ ] **Step 1: Move the block.** Move old lib.rs 207-323 (config-path resolve, `startup::boot_config`,
      OIDC/enrollment bootstrap arg parsing, `TracingBuilder` setup incl. the `#[cfg(feature = "journald")]`
      layer, `builder.init()`) into `boot::config::load`. Return `BootConfig`. The `runtime` borrow
      (`&booted.runtime`) is re-derived by callers from `cfg.booted.runtime`.

- [ ] **Step 2: Rewrite the orchestrator head.** In `boot::run_server`, replace the moved block with
      `let cfg = boot::config::load(args, &info).await?;` and thread `cfg.booted`, `cfg.oidc_bootstrap`,
      `cfg.enrollment_bootstrap`, `cfg.config_path`, `cfg.args` into the remaining (still-inline) body.

- [ ] **Step 3: Gate + Step 4: Commit** — `refactor(controller): extract boot::config phase`

### Task 4: Extract `boot::crypto` (Phases 1, 4, 4b–4d)

**Files:**

- Create: `crates/core/controller-runtime/src/boot/crypto.rs`
- Modify: `boot/mod.rs` (old lib.rs 325-378)

**Interfaces:**

- Consumes: `&BootConfig` (for `runtime.master_key`, `args.master_key_from`), `&Persistence` is NOT yet
  available — **note ordering**: master-key _init_ (Phase 1) needs only config; master-key _verify_ +
  key-ring + reencrypt (Phases 4–4d) need the DB. Split accordingly (see Step 1).
- Produces:

```rust
// init_master_key already returns Option<uptrakit_wire::SecretString>; carry that exact type, no re-wrap.
pub(crate) struct MasterKey { pub hex: Option<uptrakit_wire::SecretString> }
pub(crate) fn init(cfg: &BootConfig) -> crate::Result<MasterKey>;          // Phase 1 only (no DB)
pub(crate) async fn verify_and_migrate(db: &Persistence) -> crate::Result<()>; // Phases 4, 4b, 4c, 4d (needs DB)
```

- [ ] **Step 1: Move + split by dependency.** Phase 1 (lib.rs 325-336, `startup::init_master_key` — which
      already returns `crate::Result<Option<uptrakit_wire::SecretString>>`; store it in `MasterKey.hex`
      verbatim, NO `SecretString::from` wrap) → `boot::crypto::init`. Phases 4/4b/4c/4d (lib.rs 369-378:
      `verify_master_key`, `register_column_aad_mappings`, `init_data_key_ring`, `reencrypt_to_v3`) →
      `boot::crypto::verify_and_migrate(db)`. Carry `MasterKey` forward; `verify_and_migrate` runs after
      persistence (preserves current order: Phase 1 at 336 is before DB at 360, Phases 4\* at 369+ are after
      DB).

- [ ] **Step 2: Rewrite orchestrator.** `let crypto = boot::crypto::init(&cfg)?;` early;
      `boot::crypto::verify_and_migrate(&db).await?;` after `boot::persistence::open`.

- [ ] **Step 3: Gate + Step 4: Commit** — `refactor(controller): extract boot::crypto phase`

### Task 5: Extract `boot::directories` (Phase 2)

**Files:** Create `boot/directories.rs`; Modify `boot/mod.rs` (old lib.rs 338-358)

**Interfaces:**

- Consumes: nothing from prior phases (uses platform defaults)
- Produces:

```rust
pub(crate) struct AppLayout {
    pub app_dirs: uptrakit_directories::AppDirs,
    #[cfg(any(feature = "embedded-scheduler", feature = "embedded-agent",
              feature = "embedded-ssh-agent", feature = "embedded-mqtt"))]
    pub installation_id: /* type of startup::init_installation_id return */,
}
pub(crate) async fn resolve() -> crate::Result<AppLayout>;
```

- [ ] **Step 1:** Move `AppDirs::resolve` + `ensure_dirs` + the two `tracing::info!` lines + the
      `#[cfg(...embedded...)] init_installation_id` into `boot::directories::resolve`. Preserve the exact
      `#[cfg(any(...))]` predicate on the `installation_id` field and its init. (Confirm the return type of
      `startup::init_installation_id` by reading `startup/installation_id.rs`.)
- [ ] **Step 2:** `let layout = boot::directories::resolve().await?;`
- [ ] **Step 3: Gate + Step 4: Commit** — `refactor(controller): extract boot::directories phase`

### Task 6: Extract `boot::persistence` (Phase 3)

**Files:** Create `boot/persistence.rs`; Modify `boot/mod.rs` (old lib.rs 360-366)

**Interfaces:**

- Consumes: `&BootConfig` (`runtime.db.url`, `runtime.db.pool_size`), `&AppLayout` (`app_dirs.state_dir()`)
- Produces:

```rust
pub(crate) struct Persistence {
    pub db: sea_orm::DatabaseConnection,
    pub url: String,
    pub default_tenant_id: uuid::Uuid,
}
pub(crate) async fn open(cfg: &BootConfig, layout: &AppLayout) -> crate::Result<Persistence>;
```

- [ ] **Step 1:** Move `startup::init_database(&runtime.db.url, runtime.db.pool_size, app_dirs.state_dir())` +
      the destructure + the `tracing::info!(%default_tenant_id, ...)` into `boot::persistence::open`,
      returning `Persistence`.
- [ ] **Step 2:** `let db = boot::persistence::open(&cfg, &layout).await?;` then
      `boot::crypto::verify_and_migrate(&db).await?;` (from Task 4). Replace downstream `db_conn` references
      with `db.db`, `db_url` with `db.url`, `default_tenant_id` with `db.default_tenant_id`.
- [ ] **Step 3: Gate + Step 4: Commit** — `refactor(controller): extract boot::persistence phase`

### Task 7: Extract `boot::settings` (Phases 5, 6, 7, 7b, 7c, 8)

**Files:** Create `boot/settings.rs`; Modify `boot/mod.rs` (old lib.rs 380-417)

**Interfaces:**

- Consumes: `&BootConfig` (runtime, oidc_bootstrap, enrollment_bootstrap), `&Persistence`
- Produces:

```rust
pub(crate) struct SettingsBundle {
    pub settings: uptrakit_web_api::settings::Settings,
    pub reconciled: crate::startup::ReconciledSettings,
    pub validated: crate::startup::ValidatedConfig,
}
pub(crate) async fn load_and_seed(cfg: &BootConfig, db: &Persistence)
    -> crate::Result<SettingsBundle>;
```

- [ ] **Step 1:** Move into `boot::settings::load_and_seed`: `Settings::load` (5; keep the `reg_token`
      one-time-registration `eprintln!` block at 385-390), `reconcile_all_settings` (6), `bootstrap_oidc` (7),
      `bootstrap_enrollment_tokens` (7b), `seed_oauth_defaults` (7c), and `validate_configuration` (8, lib.rs
      416-417). Note: `validate_configuration` needs `reconciled` — so it belongs here, producing `validated`.
      The discarded `_tenant_raw` and `global_raw` (used only by `reconcile_all_settings`) stay internal to
      this fn.
- [ ] **Step 2:** `let settings = boot::settings::load_and_seed(&cfg, &db).await?;` Downstream: `reconciled` →
      `settings.reconciled`, `validated` → `settings.validated`, `settings` (the `Settings`) →
      `settings.settings`.
- [ ] **Step 3: Gate + Step 4: Commit** — `refactor(controller): extract boot::settings phase`

### Task 8: Extract `boot::listeners` (Phase 8b — ATOMIC)

**Files:** Create `boot/listeners.rs`; Modify `boot/mod.rs` (old lib.rs 419-516)

**Interfaces:**

- Consumes: `&SettingsBundle` only (`reconciled.https_addr`, `validated.pki_http_port`)
- Produces:

```rust
pub(crate) struct Listeners {
    pub https_std: std::net::TcpListener,
    pub pki_std_for_spawn: Option<std::net::TcpListener>,
    pub listener_count: usize,
    pub first_listener_fd: std::os::unix::io::RawFd,
}
pub(crate) fn claim(settings: &SettingsBundle) -> crate::Result<Listeners>;
```

- [ ] **Step 1: Move the ENTIRE block 419-516 verbatim** (inherited-listener claim, HTTPS bind,
      `clear_cloexec`, `first_listener_fd`, PKI bind, PKI `clear_cloexec`, `listener_count`) into
      `boot::listeners::claim` as ONE function body. **Do not reorder or split** — the consecutive-FD
      invariant requires no fd-allocating call between the HTTPS and PKI binds (preserve the existing
      `// CONSECUTIVE-FD INVARIANT` and `// ORDERING` comments verbatim). It is sync (`fn`, no `.await`).
- [ ] **Step 2: Place the call after settings, before identity.**
      `let listeners = boot::listeners::claim(&settings)?;` (matches current order: Phase 8b precedes Phase
      9). Downstream references use `listeners.https_std` / `.pki_std_for_spawn` / `.listener_count` /
      `.first_listener_fd`.
- [ ] **Step 3: Gate + Step 4: Commit** — `refactor(controller): extract boot::listeners phase (FD-atomic)`

### Task 9: Extract `boot::identity` as a sub-module (Phases 7d, 9, 10 + cert_signer)

**Files:**

- Create: `boot/identity/mod.rs`, `boot/identity/oauth.rs`, `boot/identity/pki.rs`, `boot/identity/jwt.rs`
- Modify: `boot/mod.rs` (old lib.rs 406-414 OAuth boot, 518-564 PKI/TLS + cert_signer, 522-523 JWT). NB lines
  396-405 are reconcile/bootstrap (Phases 6–7c) — already extracted in Task 7; do not re-touch them here.

> Start identity as a directory — five sub-concerns already meet the split threshold (spec §brain-class note).

**Interfaces:**

- Consumes: `&BootConfig` (runtime.tls), `&Persistence`, `&SettingsBundle` (reconciled), `&MasterKey` (not
  needed here), `&AppLayout` (state_dir for JWT)
- Produces:

```rust
pub(crate) struct Identity {
    // Destructure PkiRuntime inside init() and store its fields directly (see below) so that the
    // builder (Task 12) AND ServeDeps (Task 12.3) reach them by plain field access — NOT a nested
    // un-destructured PkiRuntime that both consumers must fight over after the move.
    pub pki: PkiFields,  // ca_managed, pki_path, ca_tx, ca_rx, ca_key_store, rustls_config,
                         // server_cert_resolver, revocation_notify, ca_rotation_trigger,
                         // crl_pem_cache, crl_manager, initial_ca_version, has_external_tls_cert
    pub jwt_manager: /* startup::init_jwt return type */,
    pub cert_signer: std::sync::Arc<dyn uptrakit_web_api::cert_signer::AgentCertSigner>,
    pub oauth_state: /* boot_oauth_state return type */,
    pub oauth_instance_for_shutdown: Option<(uuid::Uuid, sea_orm::DatabaseConnection)>,
}
pub(crate) async fn init(cfg: &BootConfig, db: &Persistence, layout: &AppLayout,
                         settings: &SettingsBundle) -> crate::Result<Identity>;
```

> **Before coding, build a field-ownership map** for the 13 `PkiRuntime` fields (read `startup::PkiRuntime` at
> startup/mod.rs:213): type, is-`Clone`, and consumer (AppState builder / ServeDeps+`spawn_background_tasks` /
> embedded registration). `ca_key_store`, `rustls_config`, `server_cert_resolver`, `crl_manager`,
> `revocation_notify`, `ca_rotation_trigger` are `Arc`-wrapped; `ca_tx`/`ca_rx` are `watch::Sender`/`Receiver`
> (Sender `Clone` requires Tokio ≥ 1.27 — verify the workspace pin). This map prevents a retroactive
> `Identity` redefinition at Task 12.3.

- [ ] **Step 1: `identity/oauth.rs`.** Move OAuth boot (lib.rs 406-414: `boot_oauth_state`, the
      `oauth_instance_for_shutdown` capture) into a
      `pub(super) async fn boot(db) -> Result<(OAuthState, Option<(Uuid, DatabaseConnection)>)>`.
- [ ] **Step 2: `identity/pki.rs`.** Move PKI/TLS init (lib.rs 518-520 `init_pki_runtime`) + the `cert_signer`
      construction (lib.rs 542-564: `issuer_source`, `effective_trust_domain`, `RcgenAgentCertSigner`) into
      `pub(super) fn build_cert_signer(pki: &PkiRuntime, runtime) -> Arc<dyn AgentCertSigner>` + a thin PKI
      init wrapper.
- [ ] **Step 3: `identity/jwt.rs`.** Move `startup::init_jwt` (lib.rs 522-523) into
      `pub(super) async fn init(db, state_dir) -> Result<JwtManager>`.
- [ ] **Step 4: `identity/mod.rs`.** `init` orchestrates the three sub-modules in the current order (OAuth at
      7d, PKI at 9, JWT at 10). **Destructure `PkiRuntime` here** (the existing destructure lives at lib.rs
      525-540) and store its fields in `Identity.pki: PkiFields` so both the builder (Task 12) and ServeDeps
      (Task 12.3) read individual fields directly — no post-move fight over a nested struct.
- [ ] **Step 5:** `let identity = boot::identity::init(&cfg, &db, &layout, &settings).await?;`
- [ ] **Step 6: Gate + Step 7: Commit** — `refactor(controller): extract boot::identity sub-module`

### Task 10: Extract `boot::components` + `boot::nats` (the web-api construction tail)

**Files:**

- Create: `boot/components.rs`, `boot/nats.rs`
- Modify: `boot/mod.rs` (old lib.rs 566-789)

**Interfaces:**

- Consumes: `&BootConfig`, `&Persistence`, `&SettingsBundle`, `&Identity`, `&MasterKey`
- Produces: a `Components` struct holding every value the `AppState` builder consumes that is built in 566-789
  — OIDC stores (`#[cfg(feature = "oidc")]`), device-flow/rate-limit stores, `service_connections`,
  `controller_id`, `workload_claim_registry`, `notification_service`, broadcasters, `token_denylist`,
  `global_providers`, `shutdown_token`, plugin catalog/`plugin_ops`, `notification_dispatcher`,
  `credential_sources`, `audit_dispatcher`, `audit_emitter`, `surface_registry`, `surface_proxy`,
  `embedded_host`, instance-plugin-snapshot handle. **Field-count gate applies** — group into these explicit
  sub-bundles (defined upfront so Task 12's `assemble` destructure is stable; see code block below).

```rust
struct AuditBits { dispatcher, emitter }
struct NotificationBits { service, dispatcher, event_broadcaster, batch_progress_broadcaster }
struct PluginBits { plugin_ops, instance_snapshot_handle, surface_registry, surface_proxy, embedded_host }
struct AuthStores { device_flow, rate_limit, token_denylist, global_providers /* , #[cfg(oidc)] oidc_* */ }
pub(crate) struct Components {   // 9 top-level fields
    controller_id, workload_claim_registry, shutdown_token, credential_sources, service_connections,
    audit: AuditBits, notification: NotificationBits, plugins: PluginBits, auth: AuthStores,
}
pub(crate) async fn build(cfg: &BootConfig, db: &Persistence, settings: &SettingsBundle,
                          identity: &Identity, crypto: &MasterKey) -> crate::Result<Components>;
```

- [ ] **Step 1: `boot/nats.rs` — concentrate all `#[cfg(feature = "nats")]` wiring.** Define
      `pub(crate) fn wire(/* reconciled.nats_url, controller_id, notification_service, event_broadcaster, batch_progress_broadcaster */) -> NatsBits`
      (and the connect at lib.rs 602-624). Move every NATS `#[cfg]` block from the current 602-650, 745-749
      into this one module so `components.rs` callers stay `#[cfg]`-free. `NatsBits` carries
      `Option<NatsTransport>` + the `with_nats`-augmented broadcasters/service. Keep the connect error mapping
      (`context_transform` → `AppError::Config`) verbatim.
- [ ] **Step 2: `boot/components.rs`.** Move lib.rs 566-789 (all the non-NATS construction) into
      `components::build`, calling `boot::nats::wire` for the NATS augmentation. Preserve every
      `#[cfg(feature = "oidc")]` on the OIDC stores. Group fields into cohesive sub-bundles per the
      field-count gate. **Master key:** `ServiceCredentialSources::new(Some(db_url.clone()), None, …)` takes
      `Option<uptrakit_wire::SecretString>` as its 3rd arg — pass `crypto.hex.clone()` directly (no
      `.expose_secret()`, no `String` conversion). **`audit_emitter`:** it is `Clone` (the current code
      already `.clone()`s it at lib.rs 775/821/899). Store it in `Components` by value; later phases
      (`reload::wire`, builder) clone it. The current move-into-`reload_audit_bridge` at lib.rs 937 becomes a
      `.clone()` in Task 11.
- [ ] **Step 3:** `let components = boot::components::build(&cfg, &db, &settings, &identity, &crypto).await?;`
- [ ] **Step 4: Gate** — additionally confirm `clippy::large_futures` does not fire on `components::build` (it
      `.await`s several constructions while holding ~20 locals). If it does, `Box::pin` the offending internal
      future or the fn itself (allowed; mirrors the `async_main` `Box::pin`). **Step 5: Commit** —
      `refactor(controller): extract boot::components + boot::nats`

---

## Phase 2 — Indivisible reload + AppState assembly

> **Tasks 11 and 12 ship as ONE commit.** `AppState::builder()` cannot finish without the coordinator handle,
> and removing the all-`Some` Option wrapper (latent issue #2) means the reload fields must reach the builder
> unconditionally — which only works once `boot::reload::wire` returns a concrete `ReloadWiring`. Do not
> split; do not leave the `Option` wrapper as a "temporary" shim (spec §Implementation sequencing 3).

### Task 11: Extract `boot::reload` (issue #2 — `ReloadWiring`, no Option tuple)

**Files:** Create `boot/reload.rs`; Modify `boot/mod.rs` (old lib.rs 830-979); move `reload_audit_bridge` +
dedupe `file_digest`

**Interfaces:**

- Consumes: `BootConfig` (takes ownership of `booted` — coordinator, channels, `settings_version_cache`,
  `receivers`, `audit_rx`, the six watch tx/rx, `config_path`), `&Components` (clones `audit_emitter` — it is
  `Clone` — plus `audit_dispatcher`, `shutdown_token`), `&Listeners` (listener_count, first_listener_fd),
  `&Identity` (oauth_instance_for_shutdown), `&BootConfig.args` (master_key_from),
  `#[cfg(feature = "nats")] &NatsBits`
- Produces:

```rust
pub(crate) struct ReloadWiring {                 // all fields non-optional (replaces 7-tuple of Option)
    pub coordinator_handle: uptrakit_config_reload::ReloadCoordinatorHandle,
    pub settings_version_cache: uptrakit_config_reload::SettingsVersionCache,
    pub receivers: uptrakit_config_reload::RuntimeConfigReceivers,
    pub reload_file_state_rx: tokio::sync::watch::Receiver<uptrakit_config_reload::ConfigFileState>,
    pub reload_last_reload_rx:
        tokio::sync::watch::Receiver<Option<uptrakit_config_reload::LastReloadInfo>>,
    pub reload_recent_events_rx: tokio::sync::watch::Receiver<Vec<serde_json::Value>>,
    pub audit_log_filter_rx: /* AuditDispatcherReloadable filter rx type */,
}
pub(crate) async fn wire(/* see Consumes */) -> crate::Result<ReloadWiring>;
```

- [ ] **Step 1: Move the coordinator block (lib.rs 835-953) into `boot::reload::wire`,** returning
      `ReloadWiring` directly — **delete the `Option` wrapping and the trailing 7-`Some(...)` tuple**. Build
      all Reloadables (DB→TLS→Listeners→NATS→Audit→Zeroconf→Plugins→Embedded) exactly as today, including the
      `#[cfg(feature = "nats")]` `NatsReloadable` push. Set alert writer, `current_exe`, config path, current
      config, and the `ControllerReexecHook` (consuming `listeners.first_listener_fd`/`.listener_count`,
      `identity.oauth_instance_for_shutdown.clone()`, `cfg.args.master_key_from`). Spawn
      `spawn_config_reconciler` and `coordinator.run()`.
- [ ] **Step 2: Move `reload_audit_bridge` into `boot/reload.rs`** and spawn it here (cloning `audit_emitter`
      from `Components`, since the current code moves it at lib.rs 937), building the `ReloadBridgeChannels`
      (from Task 1). **Move the lib.rs `file_digest` (1479-1487) into `boot/reload.rs` as a private fn —
      preserve it verbatim. Do NOT dedupe to `startup::file_digest`: the two are NOT identical** — the lib.rs
      copy returns plain `pki::sha256_hex` and `String::new()` on error, while `startup::file_digest` returns
      `"sha256:…"` / `"size:N"`. Swapping would silently change the digest format + error fallback written by
      `reload_audit_bridge` (not behavior-neutral). (A pre-existing inconsistency — `boot_config` writes
      `sha256:` digests while the bridge writes plain hex — is noted as a separate out-of-scope follow-up in
      the spec; do not fix it here.)
- [ ] **Step 3: Preserve `_reconciler` detach semantics.** The reconciler `JoinHandle` is dropped to detach
      (Tokio drop ≠ cancel). `ReloadWiring` must **not** store it — keep the
      `let _ = spawn_config_reconciler(...)` drop inside `wire` (spec §latent issue #2 note).
- [ ] (No standalone gate/commit — combined with Task 12.)

### Task 12: Extract `boot::app_state` (assembly) — combined commit with Task 11

**Files:** Create `boot/app_state.rs`; Modify `boot/mod.rs` (old lib.rs 781-1031: builder + the post-build
`#[cfg(feature = "test-utils")]` reexec block)

**Interfaces:**

- Consumes (by move): `SettingsBundle`, `Identity`, `Components`, `ReloadWiring`, plus
  `controller_id`/`default_tenant_id` (from `Components`/`Persistence`), and the
  `#[cfg(feature = "test-utils")]` reexec plan inputs (`config_path`, `master_key_from`,
  `listeners.{listener_count, first_listener_fd}`)
- Produces:

```rust
pub(crate) async fn assemble(/* see Consumes */) -> crate::Result<std::sync::Arc<AppState>>;
```

- [ ] **Step 1:** Move the `AppState::builder()` chain (lib.rs 790-828) + the embedded-host pre-build
      (781-788) into `boot::app_state::assemble`. Apply `ReloadWiring` fields to the builder **directly and
      unconditionally** (replace the old `match (Some(...), …)` at 955-979 with plain
      `.coordinator_handle(reload.coordinator_handle).settings_version_cache(...)…`). Preserve the
      `#[cfg(feature = "oidc")]` builder block (981-986). Keep `.reject_dangerous_commands(true)`.
- [ ] **Step 2:** Move the `#[cfg(feature = "test-utils")]` force-reexec block (994-1031) into `assemble` (or
      a `#[cfg(feature = "test-utils")]` helper it calls), preserving the 50 ms flush sleep and the
      `match infallible {}` handling verbatim.
- [ ] **Step 3: Separate serve-needed handles BEFORE the move.** `assemble` consumes `identity` and
      `components` by move, but `boot::serve::run` (Task 14) still needs values that live inside them. Build
      `ServeDeps` first. Clone the `Arc`/`Copy` handles from `identity.pki` — `crl_manager: Arc<CrlManager>`,
      `ca_tx: watch::Sender<CaSnapshot>` (clone — verify Tokio ≥ 1.27), `ca_managed: bool`,
      `initial_ca_version: i64`, `has_external_tls_cert: bool`; from `components` — `service_connections`
      (clone — **unconditional**, see Task 14), and `#[cfg(feature = "nats")]     nats_transport` (clone —
      **verify `NatsTransport: Clone` is `Arc`-cheap, not a deep connection clone**; if not, carry only
      `nats.nats_client()`). **`oauth_instance_for_shutdown` — MOVE, do not clone:** take it out of `identity`
      by move into `ServeDeps` (the SIGTERM/SIGINT deregister path owns it, exactly as today's single binding
      at lib.rs 1316). `reload::wire`'s `ControllerReexecHook` already makes its own `.clone()` (lib.rs 914) —
      so wire runs first (borrowing `&identity`), then the move into `ServeDeps`. Do **not** create a third
      holder. From `layout` (under the embedded-feature cfg) — `controller_installation_id` and
      `state_dir: PathBuf`; from `components` (same embedded cfg) — `builtin_host`. Then:
      `let reload = boot::reload::wire(cfg, &components, &listeners, &identity).await?;` → build `ServeDeps`
      (moving `oauth_instance_for_shutdown` out of `identity`) →
      `let state = boot::app_state::assemble(settings, identity, components, reload, /* ids */).await?;`
- [ ] **Step 4: Run the per-task gate AND the full Docker integration suite** (this is the highest-risk commit
      — boot + reexec + reload all exercised; run the gate with AND without `--features nats`):

```bash
docker build -f docker/Dockerfile.test -t uptrakit-test:latest . \
  && cargo test -p uptrakit-integration-tests -- --ignored
```

Expected: PASS (boot, enrollment, two-generation reexec, graceful shutdown).

- [ ] **Step 5: Commit (Tasks 11 + 12 together)**

```bash
git add crates/core/controller-runtime/src/boot/ crates/core/controller-runtime/src/lib.rs
git commit -m "refactor(controller): extract boot::reload + boot::app_state

Replace all-Some Option tuple + unreachable match with non-optional
ReloadWiring; assemble AppState from phase structs. Reconciler JoinHandle
kept detached. Indivisible: builder needs the coordinator handle.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Phase 3 — Tail extraction (recovery + serve)

### Task 13: Extract `boot::recovery` (rollout cleanup + denylist seed)

**Files:** Create `boot/recovery.rs`; Modify `boot/mod.rs` (old lib.rs 1033-1162)

**Interfaces:**

- Consumes: `&Arc<AppState>`
- Produces: `pub(crate) async fn run(state: &std::sync::Arc<AppState>) -> crate::Result<()>;`

- [ ] **Step 1:** Move into `boot::recovery::run`: the GitHub global-provider diagnostic (1033-1037),
      `mark_all_in_progress_as_failed_for_rollout` + the per-record `finalize_post_update_hook` /
      `finalize_post_update_with_timeout` / `dispatch_next_in_batch` / `dispatch_next_queued_for_host` loop
      (1039-1149, preserving every `#[cfg(feature = "plugin-ops")]`), and the token-denylist `load_from_db`
      seed (1151-1162). All reachable via `state` accessors (`state.db()`, `state.notification.*`,
      `state.auth.token_denylist`, etc.) — no extra params.
- [ ] **Step 2:** `boot::recovery::run(&state).await?;`
- [ ] **Step 3: Gate + Step 4: Commit** — `refactor(controller): extract boot::recovery phase`

### Task 14: Extract `boot::serve` (bg tasks, embedded, signals, server, shutdown)

**Files:** Create `boot/serve.rs`; Modify `boot/mod.rs` (old lib.rs 1164-1320); move `spawn_background_tasks`,
`spawn_zeroconf`, `spawn_pki_http` into `boot/serve.rs`

**Interfaces:**

- Consumes (by move): `Arc<AppState>`, `Listeners`, `&BuildInfo`; pulls
  `crl_manager`/`ca_*`/`service_connections`/`nats_transport` from `Components` (thread the needed handles via
  a small `ServeDeps` if they are not all in `AppState`)
- Produces:
  `pub(crate) async fn run(state: Arc<AppState>, listeners: Listeners, deps: ServeDeps, info: &BuildInfo) -> crate::Result<()>;`

> Field-count gate: prefer reaching values through `Arc<AppState>`; `ServeDeps` carries only what is genuinely
> not in `AppState`. Full field list (built in Task 12 Step 3, before `assemble` consumes `identity`/
> `components`), with exact conditionality:
>
> - **Unconditional:** `crl_manager: Arc<CrlManager>`, `ca_managed: bool`, `ca_tx: watch::Sender<CaSnapshot>`,
>   `initial_ca_version: i64`, `has_external_tls_cert: bool`,
>   `oauth_instance_for_shutdown: Option<(Uuid, DatabaseConnection)>` (moved in, not cloned),
>   `service_connections: ServiceConnectionRegistry`, `bg: BackgroundTasks`.
> - **`#[cfg(feature = "nats")]`:** `nats_transport: NatsTransport`.
> - **`#[cfg(any(feature = "embedded-*"))]`** (same 4-feature predicate as lib.rs 782/788):
>   `builtin_host: BuiltinServiceHost`, `controller_installation_id`, `state_dir: PathBuf`.
>
> **`service_connections` is UNCONDITIONAL here** — even though Task 1 Step 4 gates the
> `spawn_background_tasks` param/arg behind `#[cfg(feature = "nats")]`, the final
> `bg.shutdown(server_handle, service_connections, …)` at lib.rs 1313 takes it by value on every build. Do NOT
> cfg-gate the `ServeDeps` field, or the non-nats build fails to compile.

- [ ] **Step 1: Move the helper fns** `spawn_background_tasks` (lib.rs 1377-1453), `spawn_zeroconf`
      (1456-1474, keep `#[cfg(feature = "zeroconf")]`), `spawn_pki_http` (1666-1683) into `boot/serve.rs` as
      `pub(super)`/private fns. **Idiom fix (snapshot `coding-standards.md#parameter-struct-pattern`):**
      `spawn_background_tasks` has 8 args and currently carries `#[expect(clippy::too_many_arguments, …)]` — a
      suppression the snapshot bans. Replace the suppression with a semantic-role parameter struct holding
      **owned** `Arc`/`Copy` fields (no lifetime parameter — `Arc::clone` is the idiomatic copy; a `<'a>`
      struct just to hold `&Arc<_>` adds lifetime noise for no benefit): `CaTaskDeps` with
      `crl_manager: Arc<CrlManager>`, `ca_managed: bool`, `ca_tx: tokio::sync::watch::Sender<CaSnapshot>`,
      `initial_ca_version: i64`, `has_external_tls_cert: bool`. Keep `nats_transport` as its own
      `#[cfg(feature = "nats")]` param and `service_connections` gated `#[cfg(feature = "nats")]` (per Task 1
      Step 4). Signature: `spawn_background_tasks(bg, app_state, ca: CaTaskDeps, …)` taking `CaTaskDeps` by
      value. Delete the `#[expect(too_many_arguments)]`; verify `cargo clippy --all-features` reports no
      `too_many_arguments`.
- [ ] **Step 2: Move the serve body** (lib.rs 1164-1320) into `boot::serve::run`: `BackgroundTasks::new`,
      `spawn_background_tasks`, the four `#[cfg(feature = "embedded-*")]` `register_*` calls (1182-1244, keep
      predicates + fatal error mapping verbatim), signal handlers, `server::run` spawn (consuming
      `listeners.https_std`), `spawn_zeroconf`, `spawn_pki_http` (consuming `listeners.pki_std_for_spawn`),
      `sd_notify::signal_ready`, the `tokio::select!` shutdown loop (1283-1308), graceful `bg.shutdown`
      (1313), and the OAuth deregister (1316-1318). Preserve the server-error
      `return Err(e).context(AppError::Server)?` path.
- [ ] **Step 3:** Orchestrator final line: `boot::serve::run(state, listeners, serve_deps, &info).await`.
- [ ] **Step 4: Gate + Step 5: Commit** —
      `refactor(controller): extract boot::serve phase; run_server now ~40 lines`

---

## Phase 4 — Module consolidation, documentation, final verification

### Task 15: Consolidate `startup/` into `boot/init/`

> Unifies the two start-time modules into one tree (per design decision). `startup/` becomes `boot/init/`; the
> phase orchestrators (which call its helpers) and `boot/` now live under a single module. Mechanical rename +
> path updates — no logic change.

**Files:**

- Move: `crates/core/controller-runtime/src/startup/*.rs` →
  `crates/core/controller-runtime/src/boot/init/*.rs` (10 files: `mod.rs`, `bootstrap.rs`, `database.rs`,
  `encryption.rs`, `installation_id.rs`, `jwt.rs`, `master_key.rs`, `oauth.rs`, `pki_init.rs`, `settings.rs`,
  `validation.rs`)
- Modify: `crates/core/controller-runtime/src/lib.rs` (remove `mod startup;`),
  `crates/core/controller-runtime/src/boot/mod.rs` (add `pub(crate) mod init;`), and all references
  `crate::startup::` → `crate::boot::init::` (and `super::`-relative paths inside the moved files)

> All of Steps 1–3 land in ONE commit — there is no intermediate `cargo check` between them, so the broken
> half-renamed state never appears in history. `git mv` on the directory relocates each file as a tracked
> rename (it creates the `boot/init/` parent). Verify with `git status` that files show as renames, not
> delete+add, so `git log --follow` keeps working.

- [ ] **Step 1: Relocate + rewire modules (same step).**
      `git mv crates/core/controller-runtime/src/startup crates/core/controller-runtime/src/boot/init`. Remove
      `mod startup;` from lib.rs; add `pub(crate) mod init;` to `boot/mod.rs`.
- [ ] **Step 2: Fix paths.** Update every `crate::startup::X` to `crate::boot::init::X` across the crate
      (`grep -rn "crate::startup\|startup::" crates/core/controller-runtime/src`). The intermediate types
      (`BootedConfig`, `ReconciledSettings`, `ValidatedConfig`, `PkiRuntime`, and `startup`'s own internal
      `file_digest`) move with the files — update the phase-struct field types in
      `boot/{config,settings,identity,reload}.rs` accordingly (`crate::startup::BootedConfig` →
      `crate::boot::init::BootedConfig`, etc.). NB the private `file_digest` in `boot/reload.rs` (moved there
      in Task 11) is a **separate** fn and is unaffected by this sweep — the grep targets only `startup::`
      refs.
- [ ] **Step 3: Fix internal `super::` paths** inside the moved files if any reach back to former-`startup`
      siblings (they now resolve within `boot::init`) or to `crate::` items (unchanged).
- [ ] **Step 4: Run the per-task gate** (full set). Expected green — pure relocation, zero behavior change.
- [ ] **Step 5: Commit** — `refactor(controller): unify startup/ into boot/init/`

### Task 16: Documentation deliverables

**Files:**

- Create: `docs/adr/0023-controller-boot-phase-decomposition.md`
- Modify: `docs/development/coding-standards.md`

- [ ] **Step 1: Write ADR 0023.** Follow the existing ADR format (Date, Status: Accepted, Context, Decision,
      Alternatives Considered, Consequences). Record: the component-struct dataflow pattern; the
      consecutive-FD atomicity constraint; the `large_futures` boxing constraint; the indivisible
      reload+assemble boundary; the unified `boot/` module layout with `boot/init/` helper layer (former
      `startup/`); `AppState`-first for post-assembly phases. Cross-reference the spec and ADR 0001/0003/0008.
      (snapshot: ADR is non-optional for structural decisions.)
- [ ] **Step 2: Add the "Boot phase pattern" note (mandatory) to `coding-standards.md`** under the controller
      section: new controller subsystems get a new `boot/<phase>.rs` fn producing a typed struct (or a named
      sub-module), never an inline addition to an existing phase fn; keeps `run_server` from re-monolithizing.
      Cite ADR 0023.
- [ ] **Step 3: Lint docs** —
      `npx prettier --write docs/adr/0023-*.md docs/development/coding-standards.md && markdownlint --config .markdownlint.json docs/adr/0023-*.md docs/development/coding-standards.md`.
      Expected clean (snapshot: MD013 150-char, MD024 siblings_only).
- [ ] **Step 4: Commit** — `docs(controller): ADR 0023 boot decomposition + coding-standards note`

### Task 17: Full quality-gate sweep + CodeScene confirmation

- [ ] **Step 1: Run the complete project gate** (snapshot `quality-gates.md`):

```bash
cargo fmt --all --check
cargo check --no-default-features --features db-sqlite
cargo check --all-features
cargo clippy --all-targets --no-default-features --features db-sqlite
cargo clippy --all-targets --all-features
cargo test --all-features
cargo test --workspace --all-features --doc --exclude uptrakit-mqtt-runtime  # doctests: module paths moved (startup→boot::init)
cargo deny check
# CI guard scripts (snapshot quality-gates.md):
python3 ci/check_plugin_semantic_boundary.py
python3 ci/verify_db_access_policy.py
bash ci/verify_no_security_audit.sh
bash ci/verify_typed_audit_actions.sh
bash ci/verify_handler_state_contract.sh
markdownlint --config .markdownlint.json '**/*.md'
```

- [ ] **Step 2: Docker boot-path integration suite** (final behavioral confirmation):

```bash
docker build -f docker/Dockerfile.test -t uptrakit-test:latest . \
  && cargo test -p uptrakit-integration-tests -- --ignored
```

Expected: PASS.

- [ ] **Step 3: Confirm CodeScene findings cleared.** Re-run a health check on the file; `run_server` should
      be ≈40 lines, `lib.rs` well under the file-size smell, no Complex Method / Large Method / Excess
      Arguments findings remaining on the boot path. (Manual: review `boot/*.rs` function sizes; each should
      be under the 70-LoC Rust threshold or close.)
- [ ] **Step 4: No commit** (verification only) — or a trivial `chore` if any fmt drift surfaced.

---

## Self-Review

**Spec coverage:**

- Decompose `run_server` → Tasks 2-14. ✓
- Cohesive component structs threaded through phases → struct defs in Tasks 3-12. ✓
- `boot/` module layout incl. `boot/nats.rs` + `identity/` sub-module → Tasks 9, 10. ✓
- Latent #1 (dead AuditFilter) → Task 1.1; #2 (Option tuple) → Task 11.1; #3 (param struct) → Task 1.2; #4
  (additive cfg, both sites) → Task 1.4; #5 (VecDeque, optional) → Task 1.3; #6 (SecretString) → **withdrawn**
  (already `SecretString` end-to-end; Task 4 preserves the type). Plus `spawn_background_tasks` 8-arg →
  `CaTaskDeps` (Task 14.1). ✓
- Consecutive-FD invariant → Task 8 (atomic). ✓
- `large_futures` Box::pin → Task 2.2 + Global Constraints. ✓
- Indivisible reload+assemble → Tasks 11+12 (one commit). ✓
- Pre-refactor latent commit for bisect isolation → Task 1. ✓
- Cross-cutting god-bag guard (AppState-first, field-count gate) → Tasks 10, 14 + Global Constraints. ✓
- Unify `startup/` + `boot/` into one module (`boot/init/`) → Task 15. ✓
- ADR 0023 + coding-standards note → Task 16. ✓
- Verification gates → per-task + Tasks 12, 17. ✓
- Deferred: VecDeque if it ripples (Task 1.3 guard); pre-existing `file_digest` format inconsistency — carried
  as out-of-scope.

**Placeholder scan:** struct field types that depend on reading a current return type (`init_jwt`,
`init_installation_id`, `boot_oauth_state`, audit filter rx) are marked with an inline `/* type … */`
instruction to read the source — these are deliberate "read the exact type here" markers for a refactor, not
invented types. Implementer resolves them from the named source file before writing the struct.

**Type consistency:** `MasterKey.hex: Option<uptrakit_wire::SecretString>` (Task 4) matches
`init_master_key`'s actual return and `ServiceCredentialSources::new`'s 3rd param — no wrap/unwrap;
`Persistence.db` used as `db.db` downstream (Task 6); `ReloadWiring` field names match the builder calls they
replace (Task 12); `ReloadBridgeChannels` fields consistent between Task 1.2 and Task 11.2; `CaTaskDeps`
owned-`Arc` fields (Task 14.1) and `ServeDeps` field list (Task 14) match the Task 12.3 separation step.

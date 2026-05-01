# Extract Surface Proxy into `uptrakit-surface-proxy` Crate — Design

## Goal

Move `surface_proxy.rs`, `surface_proxy/`, and `surface_registry.rs` out of
`uptrakit-web-api` into a new `uptrakit-surface-proxy` crate at
`crates/ui/surface-proxy/`. Third step in ADR-0001's targeted per-concept
extraction plan, following MCP and notification-delivery.

**Scope of benefit:** Organizational improvement. `uptrakit-surface-proxy` pulls
`sea-orm` and `uptrakit-plugin-infrastructure-registry` transitively, so the
incremental build win for changes touching only surface proxy code is small
(measure before/after per the ADR's build-time gate). The wins are: the ~10 k line
surface subsystem has a clear home outside `web-api`, and controller-runtime imports
types directly from the authoritative crate rather than via `web-api` re-exports.

---

## Motivation

- `surface_proxy.rs` (5 869 lines) + `surface_registry.rs` (2 167 lines) + submodules
  total ~10 k lines. They pass all three ADR-0001 criteria: coherent concept, clear
  seam, self-contained test surface.
- `controller-runtime` already imports `SurfaceProxy`, `SurfaceRegistry`, and
  `PluginSurfaceLocalExecutor` from `uptrakit_web_api`. After extraction these
  types live in their authoritative crate.

---

## Commit Sequence

Six self-contained commits, each leaving the codebase green. Commits 4 and 5 must
be consecutive with no other commits between them — between them the surface proxy
types exist in both locations simultaneously, which must never land on `main` in
isolation.

---

### Commit 1 — Extract `ServiceConnectionRegistry` to `crates/ui/service-connections/`

**Goal:** give `uptrakit-surface-proxy` a dep-graph-safe import path for
`ServiceConnectionRegistry`.

`ServiceConnectionRegistry` is used by surface_proxy but also by
`update_orchestrator`, `notification_service`, `event_delivery`,
`config_test_proxy`, and `nats_transport` inside `web-api`. It cannot move into
the surface-proxy crate without dragging unrelated subsystems along. It has no
DB deps — its full import set is `uptrakit-wire`, `parking_lot`, `rand`, `time`,
`tokio`, `tokio-util`, `uuid` — making it safe to extract as a standalone
micro-crate.

**New crate `crates/ui/service-connections/`:**

```toml
[package]
name = "uptrakit-service-connections"
version = "0.0.1"
# workspace fields as usual

[dependencies]
parking_lot  = { workspace = true }
rand         = { workspace = true }
time         = { workspace = true }
tokio        = { workspace = true }
tokio-util   = { workspace = true }
tracing      = { workspace = true }
uptrakit-wire = { workspace = true }
uuid         = { workspace = true }
```

Move `crates/ui/web-api/src/service_connections.rs` → `crates/ui/service-connections/src/lib.rs`.
No logic changes.

Add to root `Cargo.toml` `[workspace.dependencies]`:

```toml
uptrakit-service-connections = { path = "crates/ui/service-connections", version = "0.0.1" }
```

Add `uptrakit-service-connections = { workspace = true }` to `web-api/Cargo.toml`.

Replace `web-api/src/service_connections.rs` with a one-line re-export shim so
all existing `crate::service_connections::ServiceConnectionRegistry` callers
inside `web-api` compile unchanged:

```rust
pub use uptrakit_service_connections::ServiceConnectionRegistry;
```

No callers outside the shim need editing in this commit.

---

### Commit 2 — Fix `uptrakit_web_api_auth` coupling in `controller_local/settings_store.rs`

**Goal:** remove `uptrakit-web-api-auth` as a required import for the future
surface-proxy crate.

`surface_proxy/controller_local/settings_store.rs` calls five functions from
`uptrakit_web_api_auth::settings_store`:

| Auth call                              | Replacement                                                                               |
| -------------------------------------- | ----------------------------------------------------------------------------------------- |
| `load_typed_settings_by_prefix`        | `raw_settings::load_settings_by_prefix` + `raw_settings::decode_prefixed_settings`        |
| `load_typed_global_settings_by_prefix` | `raw_settings::load_global_settings_by_prefix` + `raw_settings::decode_prefixed_settings` |
| `load_global_settings_by_prefix`       | `raw_settings::load_global_settings_by_prefix`                                            |
| `upsert_setting_raw`                   | `raw_settings::upsert_setting_raw`                                                        |
| `upsert_global_setting_raw`            | `raw_settings::upsert_global_setting_raw`                                                 |

All five auth functions are thin wrappers that delegate to
`uptrakit_shared_db::raw_settings` and re-wrap errors into `AuthError::Internal`.
Replace each call site in `settings_store.rs` with the direct `raw_settings` call.
Update error handling: `RawSettingsError` must be converted to the trait's expected
error type at each call site. The trait implementations in `settings_store.rs`
return `PluginError` (from `uptrakit-plugin-infrastructure-registry`); map with
`PluginError::Internal(e.to_string())` or the pattern already used by that trait.
The `AuthError` wrapper disappears entirely.

This is the same pattern established by the notification-delivery pre-condition
(commit 1 of that extraction).

No behaviour change. All existing tests pass.

---

### Commit 3 — Move `build_settings_bag` to `uptrakit-web-api-queries`

**Goal:** give `uptrakit-surface-proxy` an import path for `build_settings_bag`
that doesn't reach back into `web-api`.

`controller_local/notifications.rs` (currently dead code, `#[allow(dead_code)]`)
calls `crate::notifications::dispatcher::build_settings_bag`. After extraction
that path is broken — the new crate cannot call back into `web-api`.

`uptrakit-web-api-queries` already depends on both `uptrakit_shared_db` and
`uptrakit_plugin_infrastructure_registry` (which provides `EmailSmtpSettings`),
so the move requires no new deps for that crate.

**New module `crates/ui/web-api-queries/src/notification_settings.rs`:**

Move from `dispatcher.rs` into this module:

- `pub async fn build_settings_bag(db: &DatabaseConnection, tenant_id: Uuid) -> serde_json::Value`
- `typed_smtp_settings_or_empty` (private)
- `normalize_smtp_settings` (private)
- `normalize_non_empty_string` (private)
- `decode_smtp_password` (private)
- `smtp_settings_to_prefixed_map` (private)
- All associated constants (`SMTP_PREFIX`, `GLOBAL_SMTP_PREFIX`, `GLOBAL_TELEGRAM_PREFIX`,
  `SMTP_PASSWORD_AAD`, `GLOBAL_SMTP_PASSWORD_AAD`)

Add `pub mod notification_settings;` to `web-api-queries/src/lib.rs`.

Update `dispatcher.rs`: replace the `build_settings_bag` definition with an import:

```rust
pub(crate) use uptrakit_web_api_queries::notification_settings::build_settings_bag;
```

The `build_settings_bag` call in `dispatch_loop` is unchanged.

Update `controller_local/notifications.rs` (dead code): replace
`crate::notifications::dispatcher::build_settings_bag` with
`uptrakit_web_api_queries::notification_settings::build_settings_bag`.
The dead code now compiles after extraction.

No behaviour change. All existing tests pass.

---

### Commit 4 — Create `uptrakit-surface-proxy` crate scaffold

**Goal:** new crate at `crates/ui/surface-proxy/` with all files moved in;
compiles standalone; `web-api` not yet updated.

**New crate `crates/ui/surface-proxy/`:**

```toml
[package]
name = "uptrakit-surface-proxy"
version = "0.0.1"

[features]
default = []
db-all      = ["db-sqlite", "db-postgres"]
db-sqlite   = ["sea-orm/sqlx-sqlite", "uptrakit-web-api-queries/db-sqlite"]
db-postgres = ["sea-orm/sqlx-postgres", "uptrakit-web-api-queries/db-postgres"]

[dependencies]
async-trait           = { workspace = true }
parking_lot           = { workspace = true }
rand                  = { workspace = true }
rootcause             = { workspace = true }
sea-orm               = { workspace = true }
serde_json            = { workspace = true }
time                  = { workspace = true }
tokio                 = { workspace = true }
tracing               = { workspace = true }
uuid                  = { workspace = true }
uptrakit-audit-log                     = { workspace = true, features = ["db"] }
uptrakit-notification-delivery         = { workspace = true }
uptrakit-plugin-infrastructure-registry = { workspace = true }
uptrakit-service-connections           = { workspace = true }
uptrakit-shared-db                     = { workspace = true }
uptrakit-shared-types                  = { workspace = true }
uptrakit-web-api-queries               = { workspace = true }
uptrakit-web-api-types                 = { workspace = true }
uptrakit-wire                          = { workspace = true }
uptrakit-crypto                        = { workspace = true }
```

`uptrakit-notification-delivery` is included now (per ADR-0001 sequencing note)
so that when `controller_local/notifications.rs` is wired up it can use the
delivery abstraction without a new dep addition.

**Files moving:**

| Current path in `web-api/src/`                         | New path in `surface-proxy/src/`               |
| ------------------------------------------------------ | ---------------------------------------------- |
| `surface_proxy.rs`                                     | `proxy.rs`                                     |
| `surface_proxy/bookkeeping.rs`                         | `proxy/bookkeeping.rs`                         |
| `surface_proxy/controller_local.rs`                    | `proxy/controller_local.rs`                    |
| `surface_proxy/controller_local/notifications.rs`      | `proxy/controller_local/notifications.rs`      |
| `surface_proxy/controller_local/params.rs`             | `proxy/controller_local/params.rs`             |
| `surface_proxy/controller_local/proxmox_add_config.rs` | `proxy/controller_local/proxmox_add_config.rs` |
| `surface_proxy/controller_local/settings_store.rs`     | `proxy/controller_local/settings_store.rs`     |
| `surface_proxy/dispatch.rs`                            | `proxy/dispatch.rs`                            |
| `surface_proxy/entity_enrichment.rs`                   | `proxy/entity_enrichment.rs`                   |
| `surface_proxy/idempotency.rs`                         | `proxy/idempotency.rs`                         |
| `surface_proxy/local_executor.rs`                      | `proxy/local_executor.rs`                      |
| `surface_proxy/prepared.rs`                            | `proxy/prepared.rs`                            |
| `surface_proxy/tests.rs`                               | `proxy/tests.rs`                               |
| `surface_proxy/tests/`                                 | `proxy/tests/`                                 |
| `surface_proxy/validation.rs`                          | `proxy/validation.rs`                          |
| `surface_registry.rs`                                  | `registry.rs`                                  |

**`src/lib.rs` public surface:**

```rust
mod proxy;
mod registry;

pub use proxy::{
    AppStateSurfaceActionController,
    PluginOpsSurfaceActionInvoker,
    PluginSurfaceActionInvoker,
    PluginSurfaceLocalExecutor,
    SurfaceCallerOrigin,
    SurfaceInvokeRequest,
    SurfaceLocalActionExecutor,
    SurfaceProxy,
    SurfaceProxyError,
};
pub use proxy::entity_enrichment;
pub use registry::{
    ResolvedSurfaceAction,
    SurfaceCatalogItem,
    SurfaceRegistry,
    SurfaceRegistryConfig,
    SurfaceRegistryError,
    SurfaceRegistryLookupError,
};
```

**Import path updates inside moved files:**

- `crate::surface_registry::*` → `crate::registry::*`
- `crate::service_connections::ServiceConnectionRegistry` → `uptrakit_service_connections::ServiceConnectionRegistry`
- `crate::queries::notifications::*` → `uptrakit_web_api_queries::queries::notifications::*`
- `crate::notifications::dispatcher::build_settings_bag` → already fixed in commit 3 → `uptrakit_web_api_queries::notification_settings::build_settings_bag`

Add to root `Cargo.toml` `[workspace.dependencies]`:

```toml
uptrakit-surface-proxy = { path = "crates/ui/surface-proxy", version = "0.0.1", default-features = false }
```

At this point `cargo test -p uptrakit-surface-proxy --all-features` passes.
`web-api` still compiles against its own copies.

---

### Commit 5 — Update `web-api` and `controller-runtime` to use new crate

**Goal:** remove the original files, wire up imports, introduce `SurfaceProxyDeps`.

**In `web-api/Cargo.toml`:** add

```toml
uptrakit-surface-proxy     = { workspace = true }
uptrakit-service-connections = { workspace = true }  # already added in commit 1
```

**Delete from `web-api/src/`:**

- `surface_proxy.rs`
- `surface_proxy/` directory (all submodules)
- `surface_registry.rs`

**Update `web-api/src/lib.rs`:**
Remove `pub mod surface_proxy;` and `pub mod surface_registry;`. Add re-exports that
preserve the existing public path for any external consumer that may import via
`uptrakit_web_api::surface_proxy`:

```rust
pub use uptrakit_surface_proxy as surface_proxy_crate;
pub mod surface_proxy {
    pub use uptrakit_surface_proxy::{
        AppStateSurfaceActionController, PluginOpsSurfaceActionInvoker,
        PluginSurfaceActionInvoker, PluginSurfaceLocalExecutor,
        SurfaceCallerOrigin, SurfaceInvokeRequest, SurfaceLocalActionExecutor,
        SurfaceProxy, SurfaceProxyError,
    };
    pub mod entity_enrichment {
        pub use uptrakit_surface_proxy::entity_enrichment::*;
    }
}
pub mod surface_registry {
    pub use uptrakit_surface_proxy::{
        ResolvedSurfaceAction, SurfaceCatalogItem, SurfaceRegistry,
        SurfaceRegistryConfig, SurfaceRegistryError, SurfaceRegistryLookupError,
    };
}
```

This keeps `controller-runtime` import paths working unchanged and avoids a
simultaneous multi-crate churn.

**Introduce `SurfaceProxyDeps` in `web-api/src/app_state.rs`:**

```rust
pub struct SurfaceProxyDeps {
    pub registry: Arc<SurfaceRegistry>,
    pub proxy: Arc<SurfaceProxy>,
}
```

Replace the two separate `AppState` fields:

```rust
// before
pub surface_registry: Arc<SurfaceRegistry>,
pub surface_proxy:    Arc<SurfaceProxy>,

// after
pub surface_proxy_deps: SurfaceProxyDeps,
```

AppStateBuilder staging fields stay separate
(`surface_registry: Option<Arc<SurfaceRegistry>>` and
`surface_proxy: Option<Arc<SurfaceProxy>>`); `build()` assembles them into
`SurfaceProxyDeps`. Builder methods `.surface_registry(v)` and `.surface_proxy(v)`
are unchanged, so `controller-runtime` needs no changes to its AppStateBuilder calls.

Update all `state.surface_proxy.*` and `state.surface_registry.*` access sites in
`web-api` to go through `state.surface_proxy_deps.proxy.*` and
`state.surface_proxy_deps.registry.*`:

- `routes/surfaces.rs`
- `app_state.rs` (the `build()` method and builder field accessors)
- Test harness setup code in `middleware/resolve_ip.rs`, `middleware/require_auth.rs`,
  `routes/settings_nats.rs`, `test_harness/mod.rs`
- `lib.rs` default construction code

**Full test suite green:** `cargo test -p uptrakit-web-api --all-features` and
`cargo test -p uptrakit-surface-proxy --all-features`.

---

### Commit 6 — Update ADR-0001

**Goal:** mark surface_proxy extraction complete.

In `docs/adr/0001-web-api-decomposition-strategy.md`:

- Update the candidates table: change `surface_proxy/` row status to "Completed".
- Add a spec reference: `docs/superpowers/specs/2026-05-01-extract-surface-proxy-crate-design.md`.
- Update the Consequences section to note the three pre-condition steps (commits 1–3)
  that resolved the `ServiceConnectionRegistry`, `settings_store`, and
  `build_settings_bag` couplings.

---

## Architecture After

```text
uptrakit-service-connections          (new)
  src/lib.rs  ← ServiceConnectionRegistry

uptrakit-web-api-queries              (updated)
  src/notification_settings.rs  ← build_settings_bag + SMTP helpers

uptrakit-surface-proxy                (new)
  src/proxy.rs                  ← SurfaceProxy, SurfaceProxyError, traits
  src/proxy/bookkeeping.rs
  src/proxy/controller_local.rs ← AppStateSurfaceActionController (live)
  src/proxy/controller_local/notifications.rs  (dead, compiles)
  src/proxy/controller_local/settings_store.rs ← raw_settings only
  src/proxy/dispatch.rs
  src/proxy/entity_enrichment.rs
  src/proxy/idempotency.rs
  src/proxy/local_executor.rs   (dead, compiles)
  src/proxy/prepared.rs
  src/proxy/validation.rs
  src/proxy/tests/
  src/registry.rs               ← SurfaceRegistry

uptrakit-web-api                      (updated)
  service_connections.rs  ← re-export shim (pub use uptrakit_service_connections::*)
  surface_proxy/  ← deleted
  surface_registry.rs ← deleted
  lib.rs          ← re-export shims for surface_proxy + surface_registry modules
  app_state.rs    ← SurfaceProxyDeps { proxy, registry }
```

**Dependency graph delta:**

```text
uptrakit-surface-proxy
  ├── uptrakit-service-connections
  ├── uptrakit-web-api-queries
  ├── uptrakit-notification-delivery  (planned; enables future local_executor wiring)
  ├── uptrakit-plugin-infrastructure-registry
  ├── uptrakit-shared-db
  ├── uptrakit-shared-types
  ├── uptrakit-web-api-types
  ├── uptrakit-audit-log (features = ["db"])
  ├── uptrakit-wire
  ├── uptrakit-crypto
  └── sea-orm, parking_lot, async-trait, uuid, serde_json, rootcause, time, rand, tokio, tracing

uptrakit-web-api  (adds: uptrakit-surface-proxy, uptrakit-service-connections)
controller-runtime (no Cargo.toml change needed; imports via web-api re-exports)
```

---

## Testing

### `uptrakit-surface-proxy` (after commit 4)

All existing tests inside `surface_proxy/tests/` move with the files:

- `proxy/tests/controller_local.rs` — in-memory DB tests for `AppStateSurfaceActionController`
- `proxy/tests/provider_proxied/` — proxy dispatch tests (mock WebSocket transport)

Run: `cargo test -p uptrakit-surface-proxy --all-features`

No new tests are required for a mechanical extraction. The existing suite covers
the surface proxy logic end-to-end.

### `uptrakit-web-api` (after commit 5)

All web-api integration tests that exercise surfaces routes remain in `web-api` and
continue to pass — they use the re-export path transparently.

Run: `cargo test -p uptrakit-web-api --all-features`

---

## Build-Time Gate (per ADR-0001)

Before starting commit 4: touch `surface_proxy/controller_local.rs` and measure
`cargo build -p uptrakit-web-api` incremental time.

After commit 5: touch `crates/ui/surface-proxy/src/proxy/controller_local.rs` and
measure `cargo build -p uptrakit-surface-proxy` incremental time.

Record both deltas. `sea-orm` and `uptrakit-plugin-infrastructure-registry` are
still transitive deps of the new crate, so the saving may be modest. Document the
result in the ADR update commit.

---

## Out of Scope

- Wiring up `local_executor.rs` — the dead code stays dead; a future spec will
  handle the `PluginSurfaceLocalExecutor` refactor and deduplication.
- Removing the `web-api` re-export shims (`pub mod surface_proxy { ... }`) — left
  for a follow-on cleanup once all known consumers have updated their import paths.
- Any behaviour changes to surface proxy or surface registry logic.

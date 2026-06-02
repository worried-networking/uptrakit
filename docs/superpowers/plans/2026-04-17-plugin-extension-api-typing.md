# Plugin Extension API Typing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to
> implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.
>
> **Plan status (2026-06-02):** Core typed boundary and the first migration wave have **already landed** on `main`. This plan has been re-synced to
> the current codebase. Most tasks now execute as **verification passes** (run command → expect PASS), confirming the live state still matches the
> spec acceptance criteria. The one genuinely new task left is the missing architecture decision record (Task 5).

**Goal:** Confirm that `dyn Any` and `Result<_, String>` at the controller-facing plugin boundary have been replaced with typed capability contexts
and typed reusable error contracts, that the first controller-side plugin wave compiles against that seam, and capture the design rationale in a
durable ADR so the boundary cannot drift back.

**Architecture (as landed):** The typed boundary lives in `uptrakit-plugin-infrastructure-core` and is composed from three narrow, workflow-scoped
controller traits — `SurfaceActionController`, `UpdateProtectionController`, and `UpdateHookController` — each exposing tenant/user identity plus a
single tenant-scoped persistence accessor (`tenant_db()`). Reusable error contracts are `PluginConfigValidationError`, `SurfaceActionError`, and
`PluginOpsError` (all `#[non_exhaustive]` + `thiserror`). User-facing string rendering happens at the web edge via
`uptrakit_surface_proxy::proxy::controller_local::map_surface_action_error`. The first migration wave (Proxmox infrastructure, email/telegram/webhook
notifications, Docker releases) consumes the typed seam directly. The web-edge surface proxy was extracted from
`crates/ui/web-api/src/surface_proxy.rs` into its own crate at `crates/ui/surface-proxy/` in commit `c50be1d01`; commit `ce0dba5d3`
(`refactor(controller-boundaries): type plugin and web surface contracts`) is the primary landing commit for the typed boundary itself.

**Tech Stack:** Rust workspace crates, `async_trait`, `thiserror`, `rootcause::Report`, `serde_json`, SeaORM controller code, plugin infrastructure
core/registry, `uptrakit-surface-proxy`, `uptrakit-tenant-db`, package-level `cargo check`, `cargo clippy`, `cargo test`, `cargo deny`.

---

## File Structure

### Core boundary types (landed)

- Modify:
  [`crates/plugins/infrastructure/core/src/descriptor.rs`](/Users/andreyyantsen/Development/uptrakit/crates/plugins/infrastructure/core/src/descriptor.rs)
  Responsibility: typed `SurfaceActionContext` (`controller: &'a dyn SurfaceActionController`) and typed `SurfaceActionError`
  (`#[non_exhaustive]`, `thiserror`) replace the previous `dyn Any` / stringly function-pointer signatures. Status: **landed**.
- Modify:
  [`crates/plugins/infrastructure/core/src/roles.rs`](/Users/andreyyantsen/Development/uptrakit/crates/plugins/infrastructure/core/src/roles.rs)
  Responsibility: narrow controller capability traits `SurfaceActionController`, `UpdateProtectionController`, `UpdateHookController`, plus
  `ControllerProtectionContext` / `ControllerProtectionDecision` / `ControllerPostUpdateContext` / `PostUpdateOutcome` / `UpdateHookPreContext` /
  `UpdateHookPostContext`. Status: **landed**.
- Modify:
  [`crates/plugins/infrastructure/core/src/plugin_config.rs`](/Users/andreyyantsen/Development/uptrakit/crates/plugins/infrastructure/core/src/plugin_config.rs)
  Responsibility: `PluginConfigValidationError` (`#[non_exhaustive]` + `thiserror`) with `InvalidField` / `InvalidIdentifier` / `Contract` variants
  replaces `Result<(), String>` validation surfaces. Status: **landed**.
- Modify:
  [`crates/plugins/infrastructure/core/src/plugin_ops.rs`](/Users/andreyyantsen/Development/uptrakit/crates/plugins/infrastructure/core/src/plugin_ops.rs)
  Responsibility: `PluginOpsError` (`#[non_exhaustive]` + `thiserror`) wraps `PluginConfigValidationError`; reusable ops traits return typed
  failures. Status: **landed**.
- Modify: [`crates/plugins/infrastructure/core/src/lib.rs`](/Users/andreyyantsen/Development/uptrakit/crates/plugins/infrastructure/core/src/lib.rs)
  Responsibility: re-exports for the typed boundary items. Status: **landed**.
- Modify:
  [`crates/plugins/infrastructure/registry/src/lib.rs`](/Users/andreyyantsen/Development/uptrakit/crates/plugins/infrastructure/registry/src/lib.rs)
  Responsibility: registry re-exports aligned with the typed core boundary. Status: **landed**.

### First migration wave (landed)

- Modify:
  [`crates/plugins/infrastructure/proxmox/src/surfaces.rs`](/Users/andreyyantsen/Development/uptrakit/crates/plugins/infrastructure/proxmox/src/surfaces.rs)
- Modify:
  [`crates/plugins/infrastructure/proxmox/src/update_protection.rs`](/Users/andreyyantsen/Development/uptrakit/crates/plugins/infrastructure/proxmox/src/update_protection.rs)
- Modify:
  [`crates/plugins/notifications/email/src/surfaces.rs`](/Users/andreyyantsen/Development/uptrakit/crates/plugins/notifications/email/src/surfaces.rs)
- Modify: `crates/plugins/notifications/telegram/src/surfaces.rs` and supporting files
- Modify: `crates/plugins/notifications/webhook/src/surfaces.rs` and supporting files
- Modify: `crates/plugins/releases/docker/src/surfaces.rs` and supporting files. Responsibility: compile against the typed controller boundary
  without downcasts or stringly reusable errors. Status: **landed**.

### Controller/web edge (landed — moved to dedicated crate)

The surface proxy was extracted out of `uptrakit-web-api` into the standalone `uptrakit-surface-proxy` crate
([`crates/ui/surface-proxy/`](/Users/andreyyantsen/Development/uptrakit/crates/ui/surface-proxy/)).

- Modify:
  [`crates/ui/surface-proxy/src/proxy/controller_local.rs`](/Users/andreyyantsen/Development/uptrakit/crates/ui/surface-proxy/src/proxy/controller_local.rs)
  Responsibility: hosts `map_surface_action_error` (the typed → web error mapper) and `AppStateSurfaceActionController` (the live
  `SurfaceActionController` adapter built from `sea_orm::DatabaseConnection`). Status: **landed**.
- Modify:
  [`crates/ui/surface-proxy/src/proxy/local_executor.rs`](/Users/andreyyantsen/Development/uptrakit/crates/ui/surface-proxy/src/proxy/local_executor.rs)
  Responsibility: invokes plugin handlers through the typed `SurfaceActionContext` and routes typed failures through `map_surface_action_error`.
  Status: **landed**.
- Any remaining controller-side caller surfaced by:
  `rg -n "handle_surface_action|validate_plugin_config|controller_update_protection" crates/core crates/ui crates/plugins/infrastructure`
  Responsibility: keep string conversion at the outer edge.

### Documentation (NEW — outstanding)

- Add: `docs/adr/0018-plugin-extension-typed-boundary.md` Responsibility: capture the typed controller-side plugin boundary decision, including
  the rejection of per-plugin capability stores in favour of `tenant_db()` on workflow-scoped traits, and the deliberate collapse of
  `ControllerIntegration` / `PluginInternal` into `SurfaceProxyError::SendFailed` at the web edge.
- Modify: `CONTEXT.md` Responsibility: extend the glossary with the typed boundary terminology (Surface Action Controller, Controller Protection
  Context, Surface Action Error) so future agents share the same vocabulary.

### Verification commands

Default verification (per-crate, default features):

- `cargo fmt --all`
- `cargo test -p uptrakit-plugin-infrastructure-core`
- `cargo check -p uptrakit-plugin-infrastructure-core`
- `cargo check -p uptrakit-plugin-infrastructure-proxmox`
- `cargo check -p uptrakit-notification-plugin-email`
- `cargo check -p uptrakit-notification-plugin-telegram`
- `cargo check -p uptrakit-notification-plugin-webhook`
- `cargo check -p uptrakit-plugin-releases-docker`
- `cargo check -p uptrakit-surface-proxy`
- `cargo check -p uptrakit-web-api`
- `cargo clippy -p uptrakit-plugin-infrastructure-core --all-targets`
- `cargo clippy -p uptrakit-surface-proxy --all-targets`
- `cargo clippy -p uptrakit-web-api --all-targets`

Pre-push parity (workspace, both feature sets — required by `docs/development/quality-gates.md` before merging any change touching this
boundary):

- `cargo check --no-default-features --features db-sqlite`
- `cargo check --all-features`
- `cargo clippy --all-targets --no-default-features --features db-sqlite`
- `cargo clippy --all-targets --all-features`
- `cargo test --all-features`
- `cargo deny check`
- `markdownlint --config .markdownlint.json '**/*.md'`

Tightened drift grep (run from repo root) — narrow enough to skip legitimate `Result<String>` / `Result<Vec<String>, E>` shapes:

```bash
rg -n 'Result\s*<\s*[^,>]+,\s*String\s*>' \
  crates/plugins/infrastructure/core \
  crates/plugins/infrastructure/proxmox \
  crates/plugins/notifications/email \
  crates/plugins/notifications/telegram \
  crates/plugins/notifications/webhook \
  crates/plugins/releases/docker
rg -n '\bdyn\s+Any\b' \
  crates/plugins/infrastructure/core/src \
  crates/plugins/infrastructure/proxmox/src \
  crates/plugins/notifications/email/src \
  crates/plugins/notifications/telegram/src \
  crates/plugins/notifications/webhook/src \
  crates/plugins/releases/docker/src
```

Expected results: no hits in the public reusable boundary. Legitimate matches still present and scoped out by spec § Acceptance Criteria
("agent-only role internals and plugin-private helpers are excluded unless they surface through those boundaries") include plugin-private parsing
helpers in `proxmox/src/surfaces.rs` and Docker client/registry helpers (`docker/src/{auth.rs,registry.rs,docker_client.rs,image_ref.rs}`), plus
historical doc comments in `descriptor.rs` and the per-plugin `plugin.rs` headers. Record any new hits outside that allow-list as a regression.

### Task 1: Verify Typed Boundary And Error Contracts In Core

**Status:** typed contracts landed in commits `ce0dba5d3`, `4d8071798`, `f83f2b370`. Re-run the checks below; this task is purely a regression
guard.

**Files:**

- Verify:
  [`crates/plugins/infrastructure/core/src/descriptor.rs`](/Users/andreyyantsen/Development/uptrakit/crates/plugins/infrastructure/core/src/descriptor.rs)
- Verify:
  [`crates/plugins/infrastructure/core/src/roles.rs`](/Users/andreyyantsen/Development/uptrakit/crates/plugins/infrastructure/core/src/roles.rs)
- Verify:
  [`crates/plugins/infrastructure/core/src/plugin_config.rs`](/Users/andreyyantsen/Development/uptrakit/crates/plugins/infrastructure/core/src/plugin_config.rs)
- Verify:
  [`crates/plugins/infrastructure/core/src/plugin_ops.rs`](/Users/andreyyantsen/Development/uptrakit/crates/plugins/infrastructure/core/src/plugin_ops.rs)
- Verify: [`crates/plugins/infrastructure/core/src/lib.rs`](/Users/andreyyantsen/Development/uptrakit/crates/plugins/infrastructure/core/src/lib.rs)
- Verify:
  [`crates/plugins/infrastructure/registry/src/lib.rs`](/Users/andreyyantsen/Development/uptrakit/crates/plugins/infrastructure/registry/src/lib.rs)

- [ ] **Step 1: Confirm the typed validation-error surface**

The reusable validation error is already defined at
[`crates/plugins/infrastructure/core/src/plugin_config.rs:19`](/Users/andreyyantsen/Development/uptrakit/crates/plugins/infrastructure/core/src/plugin_config.rs):

```rust
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum PluginConfigValidationError {
    #[error("{field}: {message}")]
    InvalidField { field: &'static str, message: String },
    #[error("invalid identifier: {0}")]
    InvalidIdentifier(String),
    #[error("{0}")]
    Contract(String),
}
```

Run the focused regression test:

```bash
cargo test -p uptrakit-plugin-infrastructure-core plugin_config_validation_error -- --nocapture
```

Expected: PASS.

Then, while in this file, **delete the `err.to_string()` assertion** still present in
`plugin_config.rs::tests::plugin_config_validation_error_formats_for_display` (currently
`assert_eq!(err.to_string(), "url: must be https");`). It violates
[`docs/development/testing.md`](/Users/andreyyantsen/Development/uptrakit/docs/development/testing.md): *thiserror Display format string tests
forbidden — do not test `#[error("...")]` output.* Replace the body with discriminant-only assertions and rename the test to reflect what it
now checks:

```rust
#[test]
fn plugin_config_validation_error_carries_invalid_field_metadata() {
    let err = PluginConfigValidationError::invalid_field("url", "must be https");
    assert!(matches!(
        err,
        PluginConfigValidationError::InvalidField { field: "url", .. }
    ));
    assert_eq!(err.field(), Some("url"));
}
```

Rerun:

```bash
cargo test -p uptrakit-plugin-infrastructure-core plugin_config_validation_error -- --nocapture
```

Expected: PASS.

- [ ] **Step 2: Confirm the typed surface-action error surface**

The typed surface error is at
[`crates/plugins/infrastructure/core/src/descriptor.rs:199`](/Users/andreyyantsen/Development/uptrakit/crates/plugins/infrastructure/core/src/descriptor.rs):

```rust
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum SurfaceActionError {
    #[error("{0}")] InvalidInput(String),
    #[error("{0}")] ControllerIntegration(String),
    #[error("{0}")] PluginInternal(String),
}
```

`SurfaceActionContext` is composed against the narrow `SurfaceActionController` trait (descriptor.rs:173–194):

```rust
pub struct SurfaceActionContext<'a> {
    pub controller: &'a dyn roles::SurfaceActionController,
}
```

There is **no** `ctx.db: &'a dyn Any` field anywhere on the boundary. Confirm by grepping:

```bash
rg -n '\bdyn\s+Any\b' crates/plugins/infrastructure/core/src
```

Expected: only the two doc-comment matches (`descriptor.rs:172` — historical description; `descriptor.rs:315` — the migrations
placeholder `Vec<Box<dyn Any>>`, gated off when `migrations` feature is disabled). Any other match is a regression.

- [ ] **Step 3: Confirm the narrow workflow capability traits**

The boundary uses three narrow controller traits, each exposing tenant/user identity plus a single tenant-scoped persistence accessor.
This satisfies spec § Acceptance Criteria ("No single catch-all controller trait replaces `dyn Any`; the exported boundary is composed from
narrow capability traits or typed adapters"). The traits at
[`crates/plugins/infrastructure/core/src/roles.rs:287–306, 457–460`](/Users/andreyyantsen/Development/uptrakit/crates/plugins/infrastructure/core/src/roles.rs):

```rust
pub trait SurfaceActionController: Send + Sync {
    fn tenant_id(&self) -> Uuid;
    fn user_id(&self) -> Option<Uuid>;
    #[cfg(feature = "plugin-ops")]
    fn tenant_db(&self) -> &uptrakit_tenant_db::TenantDb;
}

pub trait UpdateProtectionController: Send + Sync {
    #[cfg(feature = "plugin-ops")]
    fn tenant_db(&self) -> &uptrakit_tenant_db::TenantDb;
}

#[cfg(feature = "plugin-ops")]
pub trait UpdateHookController: Send + Sync {
    fn tenant_db(&self) -> &uptrakit_tenant_db::TenantDb;
}
```

Per-plugin capability stores (e.g. a hypothetical `NotificationChannelStore`, `EmailSmtpSettingsStore`, `ProxmoxProtectionStore`) were
considered during implementation and **deliberately rejected** in favour of the shared `tenant_db()` accessor. The rationale lives in Task 5's
ADR. Do not reintroduce the per-plugin store layer as part of a sync pass.

- [ ] **Step 4: Run the core regression checks**

```bash
cargo fmt --all
cargo test -p uptrakit-plugin-infrastructure-core
cargo check -p uptrakit-plugin-infrastructure-core
cargo clippy -p uptrakit-plugin-infrastructure-core --all-targets
```

Expected: PASS. No commit follows this task — the source is already on `main`.

### Task 2: Verify Controller/Web Error Mapping At The Surface-Proxy Edge

**Status:** the mapper landed in commit `ce0dba5d3`, then migrated with the rest of the surface proxy into the dedicated crate in commit
`c50be1d01`. Confirm the mapping rule is still the one captured below; any change requires updating the ADR (Task 5).

**Files:**

- Verify:
  [`crates/ui/surface-proxy/src/proxy/controller_local.rs`](/Users/andreyyantsen/Development/uptrakit/crates/ui/surface-proxy/src/proxy/controller_local.rs)
  Responsibility: hosts both `map_surface_action_error` and `AppStateSurfaceActionController`.
- Verify:
  [`crates/ui/surface-proxy/src/proxy/local_executor.rs`](/Users/andreyyantsen/Development/uptrakit/crates/ui/surface-proxy/src/proxy/local_executor.rs)
  Responsibility: invokes plugin handlers via the typed context and pipes typed failures through `map_surface_action_error`.

- [ ] **Step 1: Confirm the boundary-only mapping layer**

The live mapping is **not** the one originally drafted in the plan — `ControllerIntegration` and `PluginInternal` deliberately collapse
into `SurfaceProxyError::SendFailed` with a `tracing::error!` for observability, while only `InvalidInput` becomes
`SchemaValidationFailed`. This is a deliberate short-term compromise (the web edge has no `ControllerIntegration` variant yet) — record it
in Task 5's ADR rather than papering over it. Live code:

```rust
pub fn map_surface_action_error(err: SurfaceActionError) -> SurfaceProxyError {
    match err {
        SurfaceActionError::InvalidInput(message) => {
            SurfaceProxyError::SchemaValidationFailed(message)
        }
        SurfaceActionError::ControllerIntegration(message)
        | SurfaceActionError::PluginInternal(message) => {
            tracing::error!(error = %message, "controller-local surface action failed");
            SurfaceProxyError::SendFailed
        }
        other => {
            tracing::error!(error = ?other, "unexpected controller-local surface action failure");
            SurfaceProxyError::SendFailed
        }
    }
}
```

Plugin-config validation errors still flow through the existing web-edge plumbing — there is no dedicated `map_plugin_config_validation`
helper in `uptrakit-surface-proxy`; the mapping happens where `PluginConfigValidationError` surfaces in `uptrakit-web-api` route handlers.
Do not introduce a duplicate helper as part of this sync.

- [ ] **Step 2: Lock in the `tracing::error!` side effect with a test assertion**

The `ControllerIntegration → SendFailed` and `PluginInternal → SendFailed` collapse is **only** acceptable because the `tracing::error!`
call preserves the controller-side failure detail in logs. That side effect is currently undocumented in tests; a future refactor could
downgrade the log level (or drop the structured `error` field) and operators would silently lose the only signal distinguishing controller
failures from generic send failures. Add a `tracing_test`-style assertion in
[`crates/ui/surface-proxy/src/proxy/tests/controller_local.rs`](/Users/andreyyantsen/Development/uptrakit/crates/ui/surface-proxy/src/proxy/tests/controller_local.rs)
that captures an `error!` event with `error = <message>` when `map_surface_action_error` is fed a `ControllerIntegration` variant (and a
second case for `PluginInternal`). Use whatever subscriber-capture helper the workspace already provides; do not introduce a new dependency
just for this assertion. If no such helper exists, document the gap in the ADR (Task 5) and leave a `// TODO(adr-0018)` next to the
mapper — but prefer the assertion if a helper exists.

- [ ] **Step 3: Run the surface-proxy and web-api regression checks**

```bash
cargo test -p uptrakit-surface-proxy map_surface_action_error -- --nocapture
cargo check -p uptrakit-surface-proxy
cargo check -p uptrakit-web-api
cargo clippy -p uptrakit-surface-proxy --all-targets
cargo clippy -p uptrakit-web-api --all-targets
```

Expected: PASS. The existing test
([`crates/ui/surface-proxy/src/proxy/tests/controller_local.rs`](/Users/andreyyantsen/Development/uptrakit/crates/ui/surface-proxy/src/proxy/tests/controller_local.rs))
asserts the variant-to-variant mapping; Step 2 above adds the `tracing::error!` observability assertion.

### Task 3: Verify The First Controller-Side Plugin Wave

**Status:** the first wave (Proxmox, email, telegram, webhook, Docker) compiles against the typed boundary. Confirm and record any private
helper modules still using `Result<_, String>` so that future plans know they are spec-excluded.

**Files:**

- Verify:
  [`crates/plugins/infrastructure/proxmox/src/surfaces.rs`](/Users/andreyyantsen/Development/uptrakit/crates/plugins/infrastructure/proxmox/src/surfaces.rs)
- Verify:
  [`crates/plugins/infrastructure/proxmox/src/update_protection.rs`](/Users/andreyyantsen/Development/uptrakit/crates/plugins/infrastructure/proxmox/src/update_protection.rs)
- Verify:
  [`crates/plugins/notifications/email/src/surfaces.rs`](/Users/andreyyantsen/Development/uptrakit/crates/plugins/notifications/email/src/surfaces.rs)
- Verify: `crates/plugins/notifications/telegram/src/surfaces.rs` (+ siblings)
- Verify: `crates/plugins/notifications/webhook/src/surfaces.rs` (+ siblings)
- Verify: `crates/plugins/releases/docker/src/surfaces.rs` (+ siblings)

- [ ] **Step 1: Confirm wave-one boundary adoption**

Every wave-one plugin imports `SurfaceActionContext` / `SurfaceActionError` from `uptrakit_plugin_infrastructure_core` and returns
`std::result::Result<_, SurfaceActionError>` from its surface action functions. Confirm with:

```bash
rg -n 'SurfaceActionContext|SurfaceActionError' \
  crates/plugins/infrastructure/proxmox/src \
  crates/plugins/notifications/email/src/surfaces.rs \
  crates/plugins/notifications/telegram/src/surfaces.rs \
  crates/plugins/notifications/webhook/src/surfaces.rs \
  crates/plugins/releases/docker/src
```

Expected: dense hits in every listed file; no surface-action signature still uses `Result<_, String>`. Pre-update protection consumes the
typed `ControllerProtectionContext` exclusively (no leftover `ctx.db` escape hatches in Proxmox).

- [ ] **Step 2: Confirm the *exported* registry-facing boundary signatures stay typed**

Spec § Acceptance Criteria mandates: "Exported plugin-facing controller boundary signatures in the named core crates describe required
capabilities through typed contexts or narrow traits rather than erased controller objects." Verify with a targeted grep on the registry
re-export surface:

```bash
rg -n '\bpub\b.*\bdyn\s+Any\b' crates/plugins/infrastructure/core/src/lib.rs crates/plugins/infrastructure/registry/src/lib.rs
rg -n '\bpub\b.*Result\s*<\s*[^,>]+,\s*String\s*>' crates/plugins/infrastructure/core/src/lib.rs crates/plugins/infrastructure/registry/src/lib.rs
```

Expected: no hits. If a new `pub` re-export with a `dyn Any` argument or a stringly result type slips in, treat it as a regression.

- [ ] **Step 3: Run the migration-wave package checks**

```bash
cargo check -p uptrakit-plugin-infrastructure-proxmox
cargo check -p uptrakit-notification-plugin-email
cargo check -p uptrakit-notification-plugin-telegram
cargo check -p uptrakit-notification-plugin-webhook
cargo check -p uptrakit-plugin-releases-docker
```

Expected: PASS. The remaining `Result<_, String>` hits inside Proxmox `surfaces.rs` (~50 occurrences in plugin-private parsing helpers) and
Docker client/registry/auth modules are plugin-internal and explicitly excluded by spec § Acceptance Criteria — do not file them as
regressions.

### Task 4: Final Boundary Verification

**Files:**

- Verify any remaining callers surfaced by verification.

- [ ] **Step 1: Run the tightened drift grep**

```bash
rg -n 'Result\s*<\s*[^,>]+,\s*String\s*>' \
  crates/plugins/infrastructure/core \
  crates/plugins/infrastructure/proxmox \
  crates/plugins/notifications/email \
  crates/plugins/notifications/telegram \
  crates/plugins/notifications/webhook \
  crates/plugins/releases/docker
rg -n '\bdyn\s+Any\b' \
  crates/plugins/infrastructure/core/src \
  crates/plugins/infrastructure/proxmox/src \
  crates/plugins/notifications/email/src \
  crates/plugins/notifications/telegram/src \
  crates/plugins/notifications/webhook/src \
  crates/plugins/releases/docker/src
```

Expected: only spec-excluded plugin-private helpers (Proxmox `surfaces.rs` parsing helpers, Docker client/registry/auth, `image_ref.rs`,
`version_detect.rs`) plus the two `descriptor.rs` doc comments and the `migrations`-feature placeholder. Any new hits outside that
allow-list are regressions.

- [ ] **Step 2: Run the full typed-boundary verification set (per-crate)**

```bash
cargo fmt --all
cargo test -p uptrakit-plugin-infrastructure-core
cargo check -p uptrakit-plugin-infrastructure-core
cargo check -p uptrakit-plugin-infrastructure-proxmox
cargo check -p uptrakit-notification-plugin-email
cargo check -p uptrakit-notification-plugin-telegram
cargo check -p uptrakit-notification-plugin-webhook
cargo check -p uptrakit-plugin-releases-docker
cargo check -p uptrakit-surface-proxy
cargo check -p uptrakit-web-api
cargo clippy -p uptrakit-plugin-infrastructure-core --all-targets
cargo clippy -p uptrakit-surface-proxy --all-targets
cargo clippy -p uptrakit-web-api --all-targets
```

Expected: PASS.

- [ ] **Step 3: Run the pre-push quality gate (both feature sets + workspace)**

`docs/development/quality-gates.md` requires both feature legs and the workspace-wide checks before any change touching this boundary
ships. Run them once at the end of the sync pass:

```bash
cargo check --no-default-features --features db-sqlite
cargo check --all-features
cargo clippy --all-targets --no-default-features --features db-sqlite
cargo clippy --all-targets --all-features
cargo test --all-features
cargo deny check
markdownlint --config .markdownlint.json '**/*.md'
```

Expected: PASS. If only the doc changes from Task 5 are in the working tree, `cargo deny` and `markdownlint` are the load-bearing checks.

### Task 5: Capture The Typed Plugin Boundary Decision (NEW)

**Status:** outstanding. The typed-boundary track shipped without a durable architectural record. Spec § Goals introduces a typed plugin
contract for the controller-side extension layer; that decision needs an ADR so future agents do not re-litigate `tenant_db()` vs per-plugin
stores, the `ControllerIntegration → SendFailed` mapping, or the rejection of a catch-all controller trait. Pre-push parity also requires
this plan-level change to pass markdownlint and `cargo deny check`.

The ADR is **retroactive** — it ratifies decisions already on `main` rather than deliberating prospectively. Frame it accordingly:
status line should read *"Accepted (retroactive, ratified by implementation in commits `ce0dba5d3`, `c50be1d01`)"* so future readers
know the ADR is a record of settled state, not a fresh design debate. If, while drafting, the author concludes that `tenant_db()` over
per-plugin stores is the *wrong* outcome, do not bury the disagreement in this ADR — open a follow-up design memo and pause this task
until the question is resolved.

**Files:**

- Add: `docs/adr/0018-plugin-extension-typed-boundary.md`
- Modify: `CONTEXT.md` (glossary additions only)

- [ ] **Step 1: Draft `docs/adr/0018-plugin-extension-typed-boundary.md`**

Follow the format used by neighbouring ADRs (`docs/adr/0017-etag-route-layer-middleware.md` is the most recent template). Sections to
cover:

- **Context:** the previous `dyn Any` + `Result<_, String>` plugin boundary, the spec
  (`docs/superpowers/specs/2026-04-17-plugin-extension-api-typing-design.md`), and the first migration wave (Proxmox, email, telegram,
  webhook, Docker).
- **Decision:** three narrow workflow-scoped controller traits (`SurfaceActionController`, `UpdateProtectionController`,
  `UpdateHookController`), each exposing `tenant_db()` plus identity; typed reusable error contracts
  (`PluginConfigValidationError`, `SurfaceActionError`, `PluginOpsError`); web-edge mapping via
  `uptrakit_surface_proxy::proxy::controller_local::map_surface_action_error`.
- **Alternatives considered:** per-plugin capability stores (e.g. `NotificationChannelStore`, `EmailSmtpSettingsStore`,
  `ProxmoxProtectionStore`) — rejected because the resulting trait surface scaled per plugin without buying additional safety over a
  shared tenant-scoped accessor with FK-enforced isolation. Document the trade-off plainly.
- **Consequences:** `&TenantDb` exposes broader read/write surface than per-plugin stores would, so reviewers must continue gating new
  plugin queries on tenant-isolation rules from `docs/development/coding-standards.md` (`TenantDb::find_via_tenant_join`,
  BEGIN IMMEDIATE for read-then-write). Document the deliberate `ControllerIntegration → SendFailed` collapse at the web edge and note
  that adding a `ControllerIntegration` variant to `SurfaceProxyError` is a future option requiring a follow-up ADR.
- **Status:** Accepted (retroactive, ratified by implementation in commits `ce0dba5d3` and `c50be1d01`).

- [ ] **Step 2: Update `CONTEXT.md` glossary**

Add glossary entries for the controller-side typed boundary terms — at minimum: *Surface Action Controller*, *Update Protection
Controller*, *Surface Action Context*, *Surface Action Error*, *Plugin Config Validation Error*. Keep each entry one line, consistent with
the existing glossary style. Do not duplicate ADR content here.

- [ ] **Step 3: Run documentation gates**

```bash
markdownlint --config .markdownlint.json 'docs/adr/0018-plugin-extension-typed-boundary.md' 'CONTEXT.md'
cargo deny check
```

Expected: PASS.

- [ ] **Step 4: Commit the documentation update**

Conventional Commits with explicit scope:

```bash
git add docs/adr/0018-plugin-extension-typed-boundary.md CONTEXT.md docs/superpowers/plans/2026-04-17-plugin-extension-api-typing.md
git commit -m "docs(plugin-extension): record typed boundary ADR and sync plan"
```

If only the plan itself changed (no ADR or glossary delta because they already exist on `main` when this task runs), drop those files
from the `git add` list and adjust the commit message accordingly.

## Self-Review

- Spec coverage: Task 1 verifies the core typed boundary and error enums; Task 2 verifies the outer-edge conversion at the surface-proxy crate
  (including the deliberate `ControllerIntegration → SendFailed` collapse); Task 3 verifies the named first migration wave; Task 4 closes the
  explicit no-`dyn Any` / no-stringly-reusable-contract checks and the pre-push quality gate; Task 5 captures the architectural decisions in a
  durable ADR plus the matching CONTEXT.md glossary entries.
- File-path freshness: every `crates/ui/web-api/src/surface_proxy.rs` reference has been retargeted at the dedicated
  `crates/ui/surface-proxy/src/proxy/{controller_local.rs,local_executor.rs}` files; verification commands include
  `cargo check -p uptrakit-surface-proxy` and `cargo clippy -p uptrakit-surface-proxy --all-targets`.
- Testing-rule conformance: validation-error verification uses discriminant/`field()` assertions, not `#[error("...")]` Display string
  comparisons, per `docs/development/testing.md`.
- Pre-push parity: Task 4 Step 3 runs both feature legs, `cargo deny check`, and `markdownlint`, matching `docs/development/quality-gates.md`.
- Type consistency: the plan uses `PluginConfigValidationError`, `SurfaceActionError`, `PluginOpsError`, `SurfaceActionController`,
  `UpdateProtectionController`, `UpdateHookController`, `ControllerProtectionContext`, and `SurfaceActionContext` consistently across all
  tasks, matching the live source.
- Spec-document scope: the spec at `docs/superpowers/specs/2026-04-17-plugin-extension-api-typing-design.md` still references the
  pre-extraction path `crates/ui/web-api/src/surface_proxy.rs` in its File Map (lines 115, 120, 123). Specs are frozen records of intent
  at authoring time; this plan deliberately does not edit the spec, but the ADR in Task 5 must call out the path migration explicitly so
  the historical spec text remains traceable to the current code.
- Placeholder scan: no unfinished-plan markers remain.

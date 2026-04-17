# Plugin Extension API Typing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> `superpowers:subagent-driven-development` (recommended) or
> `superpowers:executing-plans` to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace `dyn Any` and `Result<_, String>` at the controller-facing
plugin boundary with typed capability contexts and typed reusable error
contracts, then migrate the first controller-side plugin wave onto that seam.

**Architecture:** Land the new typed boundary in
`uptrakit-plugin-infrastructure-core` first, then adapt the controller/catalog
call paths, then migrate a first wave of controller-side plugins
(`proxmox`, `email`, `telegram`, `webhook`, `docker`). Keep user-facing string
rendering at the web/controller edge instead of inside reusable plugin traits.
This plan lands before both the SMTP/settings cleanup in
`crates/plugins/notifications/email/src/surfaces.rs` and the later structural
split of `crates/ui/web-api/src/surface_proxy.rs`; those later tracks build on
this boundary but do not redefine it.

**Tech Stack:** Rust workspace crates, `async_trait`, `serde_json`, SeaORM
controller code, plugin infrastructure core/registry, package-level `cargo
check` and `cargo test`

---

## File Structure

### Core boundary types

- Modify:
  [`crates/plugins/infrastructure/core/src/descriptor.rs`](/Users/andreyyantsen/Development/uptrakit/crates/plugins/infrastructure/core/src/descriptor.rs)
  Responsibility: replace `dyn Any`/stringly function-pointer signatures with
  typed controller-side boundary types and typed action error results.
- Modify:
  [`crates/plugins/infrastructure/core/src/roles.rs`](/Users/andreyyantsen/Development/uptrakit/crates/plugins/infrastructure/core/src/roles.rs)
  Responsibility: define narrow controller capability traits and typed
  controller protection context/error contracts.
- Modify:
  [`crates/plugins/infrastructure/core/src/plugin_config.rs`](/Users/andreyyantsen/Development/uptrakit/crates/plugins/infrastructure/core/src/plugin_config.rs)
  Responsibility: replace `Result<(), String>` validation surfaces with typed
  validation error enums and preserve presentation conversion at the edge.
- Modify:
  [`crates/plugins/infrastructure/core/src/plugin_ops.rs`](/Users/andreyyantsen/Development/uptrakit/crates/plugins/infrastructure/core/src/plugin_ops.rs)
  Responsibility: thread typed action/protection error contracts through the
  reusable ops traits.
- Modify:
  [`crates/plugins/infrastructure/core/src/lib.rs`](/Users/andreyyantsen/Development/uptrakit/crates/plugins/infrastructure/core/src/lib.rs)
  Responsibility: re-export the new typed boundary items from one public
  surface.
- Modify:
  [`crates/plugins/infrastructure/registry/src/lib.rs`](/Users/andreyyantsen/Development/uptrakit/crates/plugins/infrastructure/registry/src/lib.rs)
  Responsibility: keep downstream registry re-exports aligned with the new
  typed core boundary.

### First migration wave

- Modify:
  [`crates/plugins/infrastructure/proxmox/src/surfaces.rs`](/Users/andreyyantsen/Development/uptrakit/crates/plugins/infrastructure/proxmox/src/surfaces.rs)
- Modify:
  [`crates/plugins/infrastructure/proxmox/src/update_protection.rs`](/Users/andreyyantsen/Development/uptrakit/crates/plugins/infrastructure/proxmox/src/update_protection.rs)
- Modify:
  [`crates/plugins/notifications/email/src/surfaces.rs`](/Users/andreyyantsen/Development/uptrakit/crates/plugins/notifications/email/src/surfaces.rs)
- Modify:
  `crates/plugins/notifications/telegram/src/**/*.rs`
- Modify:
  `crates/plugins/notifications/webhook/src/**/*.rs`
- Modify:
  `crates/plugins/releases/docker/src/**/*.rs`
  Responsibility: compile against the typed controller boundary without
  downcasts or stringly reusable errors.

### Controller/web edge

- Modify:
  [`crates/ui/web-api/src/surface_proxy.rs`](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api/src/surface_proxy.rs)
  Responsibility: adapt only boundary wiring and error mapping; do not perform
  the later structural refactor here.
- Modify any controller-side catalog call sites surfaced by:
  `rg -n "handle_surface_action|controller_update_protection|validate_plugin_config"`
  `crates/core crates/ui crates/plugins/infrastructure`
  Responsibility: keep string conversion at the outer edge.

### Verification commands

- `cargo fmt --all`
- `cargo test -p uptrakit-plugin-infrastructure-core`
- `cargo check -p uptrakit-plugin-infrastructure-core`
- `cargo check -p uptrakit-plugin-infrastructure-proxmox`
- `cargo check -p uptrakit-notification-plugin-email`
- `cargo check -p uptrakit-notification-plugin-telegram`
- `cargo check -p uptrakit-notification-plugin-webhook`
- `cargo check -p uptrakit-plugin-releases-docker`
- `cargo check -p uptrakit-web-api`
- `cargo clippy -p uptrakit-plugin-infrastructure-core --all-targets`
- `cargo clippy -p uptrakit-web-api --all-targets`
- `rg -n "dyn Any|Result<.*String>" crates/plugins/infrastructure/core`

### Task 1: Introduce Typed Boundary And Error Contracts In Core

**Files:**

- Modify:
  [`crates/plugins/infrastructure/core/src/descriptor.rs`](/Users/andreyyantsen/Development/uptrakit/crates/plugins/infrastructure/core/src/descriptor.rs)
- Modify:
  [`crates/plugins/infrastructure/core/src/roles.rs`](/Users/andreyyantsen/Development/uptrakit/crates/plugins/infrastructure/core/src/roles.rs)
- Modify:
  [`crates/plugins/infrastructure/core/src/plugin_config.rs`](/Users/andreyyantsen/Development/uptrakit/crates/plugins/infrastructure/core/src/plugin_config.rs)
- Modify:
  [`crates/plugins/infrastructure/core/src/plugin_ops.rs`](/Users/andreyyantsen/Development/uptrakit/crates/plugins/infrastructure/core/src/plugin_ops.rs)
- Modify:
  [`crates/plugins/infrastructure/core/src/lib.rs`](/Users/andreyyantsen/Development/uptrakit/crates/plugins/infrastructure/core/src/lib.rs)
- Modify:
  [`crates/plugins/infrastructure/registry/src/lib.rs`](/Users/andreyyantsen/Development/uptrakit/crates/plugins/infrastructure/registry/src/lib.rs)
- Test:
  `crates/plugins/infrastructure/core/src/plugin_config.rs`

- [ ] **Step 1: Add a focused failing validation-error test**

Add a unit test alongside the existing `PluginConfig` test module:

```rust
#[test]
fn plugin_config_validation_error_formats_for_display() {
    let err = PluginConfigValidationError::invalid_field("url", "must be https");
    assert_eq!(err.field(), Some("url"));
    assert_eq!(err.to_string(), "url: must be https");
}
```

- [ ] **Step 2: Run the targeted test to prove the typed error does not exist yet**

Run:

```bash
cargo test -p uptrakit-plugin-infrastructure-core plugin_config_validation_error_formats_for_display -- --exact
```

Expected: FAIL with an unresolved `PluginConfigValidationError`.

- [ ] **Step 3: Replace stringly reusable contracts with typed ones**

Add typed reusable error shapes like:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginConfigValidationError {
    InvalidField { field: &'static str, message: String },
    InvalidIdentifier(String),
    Contract(String),
}

impl PluginConfigValidationError {
    pub fn invalid_field(field: &'static str, message: impl Into<String>) -> Self {
        Self::InvalidField {
            field,
            message: message.into(),
        }
    }

    pub fn field(&self) -> Option<&'static str> {
        match self {
            Self::InvalidField { field, .. } => Some(*field),
            Self::InvalidIdentifier(_) | Self::Contract(_) => None,
        }
    }
}
```

```rust
pub trait PluginConfig: Sized + Send + Sync + 'static {
    fn validate(&self) -> Result<(), PluginConfigValidationError> {
        Ok(())
    }

    fn validate_identifier(_value: &str) -> Result<(), PluginConfigValidationError> {
        Ok(())
    }
}
```

Add matching typed surface/protection errors in `descriptor.rs` and `plugin_ops.rs`:
Thread the typed surface-action error through the existing async handler
signatures in `descriptor.rs`, `plugin_ops.rs`, and the registry re-exports
while keeping the project’s current `async_trait`-style dispatch shape.

```rust
pub enum SurfaceActionError {
    InvalidInput(String),
    ControllerIntegration(String),
    PluginInternal(String),
}
```

Replace `ctx.db: &'a dyn Any` with narrow controller capability traits instead
of a raw database escape hatch:

```rust
pub trait SurfaceActionController: Send + Sync {
    fn tenant_id(&self) -> Uuid;
    fn user_id(&self) -> Option<Uuid>;
}

pub struct SurfaceActionContext<'a> {
    pub controller: &'a dyn SurfaceActionController,
    // existing fields stay typed
}
```

Add first-wave capability traits next to that base boundary, for example
`NotificationChannelStore`, `EmailSmtpSettingsStore`, and
`ProxmoxProtectionStore`, so the wave-one plugins depend on explicit operations
rather than a generic SeaORM handle.

- [ ] **Step 4: Re-run the core crate tests and check**

Run:

```bash
cargo test -p uptrakit-plugin-infrastructure-core
cargo check -p uptrakit-plugin-infrastructure-core
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/plugins/infrastructure/core/src/descriptor.rs crates/plugins/infrastructure/core/src/roles.rs crates/plugins/infrastructure/core/src/plugin_config.rs crates/plugins/infrastructure/core/src/plugin_ops.rs crates/plugins/infrastructure/core/src/lib.rs crates/plugins/infrastructure/registry/src/lib.rs
git commit -m "refactor: type controller plugin extension boundary"
```

### Task 2: Adapt Controller/Web Error Mapping To The Typed Boundary

**Files:**

- Modify:
  [`crates/ui/web-api/src/surface_proxy.rs`](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api/src/surface_proxy.rs)
  Responsibility: map typed reusable errors to the existing web edge contract
  and wire the AppState-backed controller adapter that satisfies the new
  capability traits when `SurfaceActionContext` is constructed.
- Modify any controller-side caller located by:
  `rg -n "handle_surface_action|validate\\(|controller_update_protection" crates/core crates/ui`

- [ ] **Step 1: Add a failing boundary-mapping test in `surface_proxy.rs`**

Add a targeted test near the existing proxy tests:

```rust
#[tokio::test]
async fn invoke_maps_typed_plugin_action_error_to_schema_failure() {
    let err = map_surface_action_error(SurfaceActionError::InvalidInput(
        "missing config".to_string(),
    ));
    assert!(matches!(err, SurfaceProxyError::SchemaValidationFailed(_)));
}
```

- [ ] **Step 2: Run the targeted test**

Run:

```bash
cargo test -p uptrakit-web-api invoke_maps_typed_plugin_action_error_to_schema_failure -- --exact
```

Expected: FAIL because the typed mapper does not exist yet.

- [ ] **Step 3: Add the boundary-only mapping layer**

Keep the conversion at the outer edge:

```rust
fn map_surface_action_error(err: SurfaceActionError) -> SurfaceProxyError {
    match err {
        SurfaceActionError::InvalidInput(message) => {
            SurfaceProxyError::SchemaValidationFailed(message)
        }
        SurfaceActionError::ControllerIntegration(message)
        | SurfaceActionError::PluginInternal(message) => {
            SurfaceProxyError::SchemaValidationFailed(message)
        }
    }
}
```

Keep the boundary grounded in the current `surface_proxy.rs` contract. Do not
invent a new `SurfaceProxyError` variant in this plan slice; if later work wants
to distinguish controller failures more precisely, that belongs in the separate
runtime-decomposition track.
That means `ControllerIntegration` currently maps to the same web error bucket
as schema failures as a deliberate short-term compromise in this track, not
because the two cases are semantically identical.

Use the existing `ApiError::new(...)` constructor rather than inventing a new
helper:

```rust
fn map_plugin_config_validation(err: PluginConfigValidationError) -> ApiError {
    ApiError::new(
        StatusCode::BAD_REQUEST,
        err.to_string(),
        "plugin_config_validation",
        None,
    )
}
```

- [ ] **Step 4: Run the package check and targeted tests**

Run:

```bash
cargo test -p uptrakit-web-api invoke_maps_typed_plugin_action_error_to_schema_failure -- --exact
cargo check -p uptrakit-web-api
cargo clippy -p uptrakit-web-api --all-targets
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/ui/web-api/src/surface_proxy.rs
git commit -m "refactor: map typed plugin boundary errors at web edge"
```

### Task 3: Migrate The First Controller-Side Plugin Wave

**Files:**

- Modify:
  [`crates/plugins/infrastructure/proxmox/src/surfaces.rs`](/Users/andreyyantsen/Development/uptrakit/crates/plugins/infrastructure/proxmox/src/surfaces.rs)
- Modify:
  [`crates/plugins/infrastructure/proxmox/src/update_protection.rs`](/Users/andreyyantsen/Development/uptrakit/crates/plugins/infrastructure/proxmox/src/update_protection.rs)
- Modify:
  [`crates/plugins/notifications/email/src/surfaces.rs`](/Users/andreyyantsen/Development/uptrakit/crates/plugins/notifications/email/src/surfaces.rs)
- Modify:
  `crates/plugins/notifications/telegram/src/**/*.rs`
- Modify:
  `crates/plugins/notifications/webhook/src/**/*.rs`
- Modify:
  `crates/plugins/releases/docker/src/**/*.rs`

- [ ] **Step 1: Add one representative failing compile target from the migration wave**

Run:

```bash
cargo check -p uptrakit-plugin-infrastructure-proxmox
```

Expected: FAIL on `ctx.db` downcasts and/or `Result<_, String>` handler signatures.

- [ ] **Step 2: Migrate the Proxmox surface and protection boundary first**

Replace downcasts and string results with the new typed contracts:

```rust
pub async fn handle_surface_action(
    ctx: &SurfaceActionContext<'_>,
    action: &str,
    params: serde_json::Value,
) -> Result<serde_json::Value, SurfaceActionError> {
    let protection_store = ctx.controller.proxmox_protection_store();
    // existing logic stays intact
}
```

```rust
async fn prepare_pre_update_protection(
    &self,
    ctx: &ControllerProtectionContext<'_>,
) -> uptrakit_plugin_infrastructure_core::Result<ControllerProtectionDecision> {
    let protection_store = ctx.controller.proxmox_protection_store();
    // existing logic stays intact
}
```

Keep this migration consistent with Task 1: migrated handlers should resolve
their controller dependencies through the new capability traits exposed by
`ctx.controller`, not through a leftover `ctx.db` escape hatch.

- [ ] **Step 3: Migrate the notification and Docker controllers onto the same seam**

For notification surface files:

```rust
pub async fn handle_surface_action(
    ctx: &SurfaceActionContext<'_>,
    action: &str,
    params: serde_json::Value,
) -> Result<serde_json::Value, SurfaceActionError> { /* ... */ }
```

Keep `crates/plugins/notifications/email/src/surfaces.rs` scoped in this task
to typed controller capabilities and typed reusable errors only. The later
typed-config plan changes its SMTP settings parsing after this boundary task
lands.

For config validation:

```rust
fn validate(&self) -> Result<(), PluginConfigValidationError> {
    if self.url.is_empty() {
        return Err(PluginConfigValidationError::invalid_field("url", "must not be empty"));
    }
    Ok(())
}
```

- [ ] **Step 4: Run the migration-wave package checks**

Run:

```bash
cargo check -p uptrakit-plugin-infrastructure-proxmox
cargo check -p uptrakit-notification-plugin-email
cargo check -p uptrakit-notification-plugin-telegram
cargo check -p uptrakit-notification-plugin-webhook
cargo check -p uptrakit-plugin-releases-docker
cargo clippy -p uptrakit-plugin-infrastructure-core --all-targets
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/plugins/infrastructure/proxmox/src/surfaces.rs crates/plugins/infrastructure/proxmox/src/update_protection.rs crates/plugins/notifications/email/src/surfaces.rs crates/plugins/notifications/telegram/src crates/plugins/notifications/webhook/src crates/plugins/releases/docker/src crates/ui/web-api/src/surface_proxy.rs
git commit -m "refactor: migrate first plugin wave to typed controller boundary"
```

### Task 4: Final Boundary Verification And Cleanup

**Files:**

- Modify any remaining callers surfaced by verification.

- [ ] **Step 1: Search for stale `dyn Any` and stringly reusable contracts**

Run:

```bash
rg -n "dyn Any|Result<.*String>" crates/plugins/infrastructure/core crates/plugins/infrastructure/proxmox crates/plugins/notifications/email crates/plugins/notifications/telegram crates/plugins/notifications/webhook crates/plugins/releases/docker
```

Expected: only intentional plugin-private/local uses remain, or no matches.

- [ ] **Step 2: Run the full typed-boundary verification set**

Run:

```bash
cargo fmt --all
cargo test -p uptrakit-plugin-infrastructure-core
cargo check -p uptrakit-plugin-infrastructure-core
cargo check -p uptrakit-plugin-infrastructure-proxmox
cargo check -p uptrakit-notification-plugin-email
cargo check -p uptrakit-notification-plugin-telegram
cargo check -p uptrakit-notification-plugin-webhook
cargo check -p uptrakit-plugin-releases-docker
cargo check -p uptrakit-web-api
cargo clippy -p uptrakit-plugin-infrastructure-core --all-targets
cargo clippy -p uptrakit-web-api --all-targets
```

Expected: PASS.

- [ ] **Step 3: Commit the verification cleanup if needed**

```bash
git add crates/plugins/infrastructure/core crates/plugins/infrastructure/proxmox crates/plugins/notifications/email crates/plugins/notifications/telegram crates/plugins/notifications/webhook crates/plugins/releases/docker crates/ui/web-api/src/surface_proxy.rs
git commit -m "chore: finish plugin extension typing track verification"
```

## Self-Review

- Spec coverage: Task 1 covers the core typed boundary and error enums. Task 2
  covers outer-edge conversion. Task 3 covers the named first migration wave.
  Task 4 closes the explicit no-`dyn Any`/no-stringly-reusable-contract checks.
- Placeholder scan: no unfinished-plan markers remain.
- Type consistency: the plan uses `PluginConfigValidationError`,
  `SurfaceActionError`, and typed context traits consistently across all tasks.

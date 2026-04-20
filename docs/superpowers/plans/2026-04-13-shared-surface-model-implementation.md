# Shared Surface Model Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement
> this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the current extension framework with the new shared surface model, unify built-in and extension rendering, and enforce strict
controller-side capability gating while keeping the new runtime path inert behind a dedicated rollout flag until the Phase 0 cutover contract is
satisfied.

**Architecture:** Introduce a new shared surface-contract crate, add rollout gating first, replace the controller extension registry/proxy/protocol
with surface-aware equivalents, migrate the shared REST/client seam plus frontend to a shared surface renderer and slot registry, port built-in routes
and providers onto the new model behind the rollout flag, then remove the old extension framework and extension-specific renderer path once the
cutover guard can safely activate the new runtime.

**Tech Stack:** Rust workspace crates, Axum/websocket transport, Svelte/SvelteKit frontend, existing permission/auth model, existing service
connection infrastructure, existing SSE infrastructure where applicable.

---

## Task 0: Add The Rollout Flag And Cutover Guard

**Files:**

- Modify: `crates/core/controller/src/main.rs`
- Modify: `crates/ui/web-api/src/app_state.rs`
- Modify: controller config/bootstrap code that owns runtime feature flags

- [ ] **Step 1: Add a dedicated surface-runtime rollout flag**

Introduce a controller-owned rollout flag for the new surface runtime. The default must keep shared-surface endpoints fail-closed and inert until the
Phase 0 activation guard is satisfied.

Run: `cargo check -p uptrakit-controller` Expected: controller startup compiles with an explicit surface-runtime flag path.

- [ ] **Step 2: Add the Phase 0 activation guard**

Implement the startup/runtime guard from the spec:

- refuse activation of the new runtime path unless all required first-party providers report a compatible framework generation and capability set;
- keep phases 1 through 6 production-inert when the guard is not satisfied, with `GET /api/v1/surfaces` returning `[]`, read/invoke returning
  `surface_runtime_inactive`, and provider listing behaving as absence;
- make activation state observable in logs and tests.

Run: `cargo test -p uptrakit-web-api surface_rollout` Expected: tests cover flag-off behavior, guard rejection, and successful activation when the
cutover condition is satisfied.

- [ ] **Step 3: Commit**

```bash
git add crates/core/controller/src/main.rs crates/ui/web-api/src/app_state.rs
git commit -m "feat: add surface runtime rollout guard"
```

---

### Task 1: Create The Shared Surface Contract Crate

**Files:**

- Create: `crates/shared/surfaces/Cargo.toml`
- Create: `crates/shared/surfaces/src/lib.rs`
- Create: `crates/shared/surfaces/src/ids.rs`
- Create: `crates/shared/surfaces/src/slot.rs`
- Create: `crates/shared/surfaces/src/surface.rs`
- Create: `crates/shared/surfaces/src/data.rs`
- Create: `crates/shared/surfaces/src/interaction.rs`
- Create: `crates/shared/surfaces/src/protocol.rs`
- Modify: `Cargo.toml`

- [ ] **Step 1: Add the new crate to the workspace**

Create the crate manifest and workspace entry for `crates/shared/surfaces`.

Run: `cargo check -p uptrakit-surfaces` Expected: the new crate is discovered by Cargo, even if the source files are still stubbed.

- [ ] **Step 2: Define identifier and slot primitives**

Implement `SurfaceId`, `InteractionId`, `DataSourceId`, slot constants, lexical validation helpers, and slot metadata (`single_entry`, `multi_entry`,
provider priority band).

Include:

```rust
pub struct SurfaceId(String);
pub struct InteractionId(String);
pub struct DataSourceId(String);

pub struct SurfaceSlotDef {
    pub id: &'static str,
    pub multi_entry: bool,
    pub provider_priority_min: i32,
    pub provider_priority_max: i32,
}
```

Run: `cargo test -p uptrakit-surfaces ids` Expected: identifier grammar tests and slot registry tests pass.

- [ ] **Step 3: Define surface, data-source, and interaction contracts**

Implement:

- `SurfaceDescriptor`
- `Targeting`
- `Scope`
- `ProviderKind`
- `FrameworkGeneration`
- `CapabilitySet`
- `SurfaceNode`
- `DataSourceDescriptor`
- `RefreshPolicy`
- `InteractionDescriptor`
- `InteractionKind`

Follow the approved spec exactly for:

- targeted vs universal semantics
- constrained JSON Schema placeholders/types
- controller-query restrictions
- `direct built-in API` allowlist binding and provider-side prohibition
- controller-owned SSE topic identifiers
- minimum refresh interval rules for provider-polled sources
- workflow/modal semantics
- surface-local uniqueness for `interaction_id` and `data_source_id`

Run: `cargo test -p uptrakit-surfaces` Expected: serde round-trip tests pass for the new contract types, including explicit generation/capability
negotiation values.

- [ ] **Step 4: Define protocol payloads**

Implement:

- `SurfaceRegistration`
- `SurfaceActionRequest`
- `SurfaceActionCancel`
- `SurfaceActionResponse`
- structured error code enum

`SurfaceRegistration` must include:

- provider identity
- negotiated `framework_generation`
- capability set
- effective tenant binding
- surfaces / interactions / data sources
- optional encryption metadata for provider-proxied sensitive fields

`SurfaceActionRequest` must include:

- `request_id`
- `tenant_id`
- `idempotency_key`
- `target_provider_id`
- controller-derived `caller_origin`
- regular params plus optional encrypted sensitive params

Do not model `caller_origin` as frontend-supplied request JSON. It is controller-populated routing metadata and must only enter the wire payload on
controller-originated dispatch.

Run: `cargo test -p uptrakit-surfaces protocol` Expected: protocol payload serde tests pass, including unsupported-generation and missing-capability
rejection fixtures.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml crates/shared/surfaces
git commit -m "feat: add shared surface contract crate"
```

### Task 2: Replace Wire Re-Exports And Service SDK Surface Types

**Files:**

- Create: `crates/shared/wire/src/surfaces.rs`
- Modify: `crates/shared/wire/src/lib.rs`
- Modify: `crates/shared/wire/src/messages.rs`
- Modify: `crates/shared/wire/src/wire_validate_impls.rs`
- Modify: `crates/shared/wire/src/tests.rs`
- Modify: `crates/shared/service-sdk/src/lib.rs`
- Create: `crates/shared/service-sdk/src/surface_proxy.rs`
- Modify: `crates/shared/service-sdk/Cargo.toml`
- Modify: `crates/shared/wire/Cargo.toml`

- [ ] **Step 1: Re-export the new surface contract through the wire crate**

Replace the old extension-framework barrel usage with a new `surfaces` barrel.

Run: `cargo check -p uptrakit-shared-wire` Expected: the wire crate builds against `uptrakit-surfaces`.

- [ ] **Step 2: Add new websocket message variants**

Add `SurfaceRegistration`, `SurfaceActionRequest`, `SurfaceActionCancel`, and `SurfaceActionResponse` to the shared message enums.

Preserve the existing extension messages temporarily behind migration shims only so the old runtime path continues to operate while the rollout flag
is off. Do not activate mixed old/new runtime handling in production; the old path remains the sole active runtime until Task 0's guard allows the
cutover.

Run: `cargo test -p uptrakit-shared-wire messages` Expected: message serialization tests cover the new variants.

- [ ] **Step 3: Introduce the service-side surface proxy**

Create a `ServiceSurfaceProxy` equivalent of the current `ServiceExtensionProxy` so services can issue provider-initiated surface actions through the
controller.

Run: `cargo check -p uptrakit-service-sdk` Expected: service SDK compiles with the new proxy and message types.

- [ ] **Step 4: Commit**

```bash
git add crates/shared/wire crates/shared/service-sdk
git commit -m "feat: add surface wire protocol and service proxy"
```

### Task 3: Replace Controller Extension Registry And Proxy With Surface Runtime

**Files:**

- Create: `crates/ui/web-api/src/surface_registry.rs`
- Create: `crates/ui/web-api/src/surface_proxy.rs`
- Create: `crates/ui/web-api/src/routes/surfaces.rs`
- Create: `crates/shared/web-api-types/src/surfaces.rs`
- Modify: `crates/shared/web-api-types/src/lib.rs`
- Create: `crates/shared/openapi-client/src/surfaces.rs`
- Modify: `crates/shared/openapi-client/src/lib.rs`
- Modify: `crates/shared/openapi-client/src/paths.rs`
- Modify: `crates/ui/web-api/src/lib.rs`
- Modify: `crates/ui/web-api/src/app_state.rs`
- Modify: `crates/ui/web-api/src/router.rs`
- Modify: `crates/ui/web-api/src/routes/service_ws/handler/mod.rs`
- Modify: `crates/ui/web-api/src/routes/service_ws/handler/messages.rs`
- Modify: `crates/core/controller/src/main.rs`
- Modify: `crates/ui/web-api/Cargo.toml`

- [ ] **Step 1: Build the new controller registry**

Implement `SurfaceRegistry` with:

- tenant-aware indexing
- built-in surface bootstrap
- provider registration admission
- all-or-nothing batch registration semantics; reject the full batch on any invalid surface or interaction
- generation and capability validation
- targeted canonical contract checks
- slot validation
- controller-query transport restrictions
- `direct built-in API` allowlist validation and provider-side prohibition
- allowed SSE topic validation against a controller-owned topic registry
- refresh-policy admission checks, including the minimum 10-second interval for provider-polled sources
- sensitive-field registration requirements, including provider encryption metadata
- permission validation
- surface-local interaction/data-source uniqueness checks
- resource-limit validation using the spec defaults (64 surfaces/batch, 256 interactions/batch, depth 16, 512 KiB registration payload)
- structured provider rejection reasons for unsupported generation, missing capability, invalid slot, invalid transport, and schema/limit failures

Run: `cargo test -p uptrakit-web-api surface_registry` Expected: registration, conflict, tenant-partition, batch-atomicity, and rejection-reason tests
pass.

- [ ] **Step 2: Build the new controller action proxy**

Implement `SurfaceProxy` to replace `ExtensionProxy`, including:

- request/response correlation
- controller-side `caller_origin` injection
- target-provider validation
- runtime input-schema validation before dispatch
- rejection of any cleartext value supplied for a declared `sensitive_field`
- validation that declared `sensitive_fields` are present only in the encrypted envelope before dispatch
- runtime result-schema validation before forwarding responses
- sensitive-field envelope enforcement and opaque ciphertext pass-through
- provider-initiated authorization gating
- provider-initiated routing
- idempotency-key storage with explicit retention policy, deterministic duplicate handling, and a default retention window of at least 15 minutes
- runtime budget enforcement using the spec defaults (32 in-flight/provider, 128 in-flight/tenant, 1 MiB response payload, 200-row page cap, 1-300
  second timeout bounds, repeated-failure rate limiting)
- cancellation emission on timeout
- late-response ignore behavior

Run: `cargo test -p uptrakit-web-api surface_proxy` Expected: timeout, cancellation, disconnect, and duplicate-request tests pass.

- [ ] **Step 3: Add surface REST endpoints and shared DTO/client plumbing**

Expose endpoints for:

- listing surfaces by slot or page
- listing targeted providers for a surface
- invoking surface interactions

Add the shared DTOs and typed client support for those endpoints in:

- `uptrakit-web-api-types`
- `uptrakit-openapi-client`

Require targeted-provider discovery responses to include:

- provider ID
- display label
- tenant-compatible availability state
- encryption metadata when the surface requires sensitive-field encryption

These define the canonical `/surfaces` shape for the new runtime path and give the CLI/frontend a typed migration target.

Run: `cargo check -p uptrakit-web-api -p uptrakit-web-api-types -p uptrakit-openapi-client` Expected: router, shared web API types, and typed client
compile with the new runtime components.

- [ ] **Step 4: Register built-in surfaces at controller startup**

Bootstrap built-in surfaces into `SurfaceRegistry` in `crates/core/controller/src/main.rs` during app initialization.

Run: `cargo test -p uptrakit-web-api` Expected: tests verify built-in surfaces share the same normalized registry path as provider surfaces.

- [ ] **Step 5: Commit**

```bash
git add crates/ui/web-api crates/core/controller/src/main.rs crates/shared/web-api-types crates/shared/openapi-client
git commit -m "feat: add controller surface runtime"
```

### Task 4: Build The Frontend Surface Store And Shared Renderer

**Files:**

- Create: `frontend/src/lib/surfaces/contract.ts`
- Create: `frontend/src/lib/surfaces/registry.svelte.ts`
- Create: `frontend/src/lib/components/surfaces/SurfaceRenderer.svelte`
- Create: `frontend/src/lib/components/surfaces/SurfaceTable.svelte`
- Create: `frontend/src/lib/components/surfaces/SurfaceForm.svelte`
- Create: `frontend/src/lib/components/surfaces/SurfaceKeyValue.svelte`
- Create: `frontend/src/lib/components/surfaces/SurfaceActionBar.svelte`
- Create: `frontend/src/lib/components/surfaces/SurfaceWorkflow.svelte`
- Create: `frontend/src/lib/components/surfaces/SurfaceModal.svelte`
- Create: `frontend/src/lib/components/surfaces/SurfaceSlot.svelte`
- Modify: `frontend/src/lib/api.ts`
- Modify: `frontend/src/lib/types.ts`
- Modify: `frontend/src/routes/+layout.svelte`

- [ ] **Step 1: Add the TypeScript surface contract**

Mirror the new Rust contract in `contract.ts`, replacing the old extension DTOs as the new frontend source of truth.

Run: `cd frontend && npm run check` Expected: TS contract files compile cleanly.

- [ ] **Step 2: Add the runtime surface registry store**

Implement a surface store that:

- loads surfaces from controller endpoints
- indexes by slot
- indexes targeted providers
- supports built-in and provider surfaces uniformly

Run: `cd frontend && npm test -- surfaces` Expected: surface store tests cover slot indexing and deterministic ordering.

- [ ] **Step 3: Implement the shared renderer primitives**

Create the `components/surfaces/` renderer set and keep any compatibility wrappers thin and temporary.

Run: `cd frontend && npm run check` Expected: new surface primitives compile and existing routes are unchanged.

- [ ] **Step 4: Wire the global app shell to the new surface registry**

Replace the old extension nav/store loading with the new surface registry in `+layout.svelte`. Read the rollout activation state from a
controller-owned runtime signal and keep provider-backed shared-surface navigation absent until the controller reports that the new surface runtime is
enabled.

Run: `cd frontend && npm run build` Expected: app shell builds and surface-backed nav items render from the new store.

- [ ] **Step 5: Commit**

```bash
git add frontend/src/lib frontend/src/routes/+layout.svelte
git commit -m "feat: add frontend surface store and renderer"
```

### Task 5: Migrate Built-In Routes To Slot Rendering

**Files:**

- Modify: `frontend/src/routes/settings/+page.svelte`
- Modify: `frontend/src/routes/settings/GlobalSettingsTab.svelte`
- Modify: `frontend/src/routes/software/+page.svelte`
- Modify: `frontend/src/routes/software/[id]/+page.svelte`
- Modify: `frontend/src/routes/surfaces/[id]/+page.svelte`
- Modify: `frontend/src/routes/extensions/[id]/+page.ts`
- Delete: `frontend/src/lib/extensions.svelte.ts` (final step within this task or Task 8)
- Modify: `frontend/src/lib/components/extensions/*` (temporary wrappers only)

- [ ] **Step 1: Migrate `settings` route tabs and below-panels**

Replace `getGroupedTabExtensions`, `getBelowExtensions`, and `ExtensionTabContent` usage with slot-based rendering backed by the new registry.

Preserve route-owned `?tab=` state. Keep the new route path behind the Task 0 rollout flag until provider-backed `settings` surfaces are ported in
Tasks 6 and 7.

Run: `cd frontend && npm run check` Expected: `/settings` tab persistence still works through refresh.

- [ ] **Step 2: Migrate `software` route tab surfaces**

Replace `getTabExtensions('software')` with slot-driven rendering.

Preserve route-owned `?tab=` state and existing built-in tabs.

Run: `cd frontend && npm run check` Expected: `/software` built-in and provider tabs share the same renderer path.

- [ ] **Step 3: Migrate `software/[id]` embedded surface usage**

Replace direct `SchemaForm` usage for provider-backed operations with `SurfaceRenderer` and interaction execution.

Run: `cd frontend && npm run build` Expected: software detail page compiles using the new primitives only.

- [ ] **Step 4: Migrate generic surface-owned page route**

Make `/surfaces/[id]` the canonical generic surface-owned page container backed by the surface registry. Keep `/extensions/[id]` only as a
compatibility redirect to the canonical route.

Run: `cd frontend && npm run build` Expected: surface-owned pages use the same renderer primitives as built-in slot surfaces, and the legacy extension
URL only redirects.

- [ ] **Step 5: Commit**

```bash
git add frontend/src/routes frontend/src/lib/components/extensions
git commit -m "feat: migrate built-in and extension pages to surface slots"
```

### Task 6: Port Plugin-Backed Surfaces To The New Contract

**Files:**

- Modify: `crates/plugins/infrastructure/core/src/descriptor.rs`
- Modify: `crates/plugins/infrastructure/core/src/plugin_ops.rs`
- Modify: `crates/plugins/infrastructure/core/src/catalog.rs`
- Modify: `crates/plugins/infrastructure/proxmox/src/extensions.rs`
- Modify: `crates/plugins/infrastructure/proxmox/src/agent/extension_actions.rs`
- Modify: `crates/plugins/infrastructure/proxmox/src/agent/plugin.rs`
- Modify: `crates/plugins/releases/docker/src/extensions.rs`
- Modify: `crates/plugins/notifications/*/src/plugin.rs`
- Modify: plugin crates that currently import `uptrakit_extension_framework`

- [ ] **Step 1: Replace plugin extension descriptors with surface descriptors**

Update plugin extension ops so compiled-in plugins emit `SurfaceDescriptor`, `InteractionDescriptor`, and `DataSourceDescriptor`.

Run: `cargo check --all-features` Expected: plugin crates compile against `uptrakit-surfaces`.

- [ ] **Step 2: Port representative plugins first**

Start with:

- Proxmox infrastructure surfaces
- Docker release surfaces
- notification plugins: email, telegram, and webhook
- any remaining plugin crates found by grepping for `uptrakit_extension_framework`

Then apply the pattern across remaining plugin-backed surfaces.

Run: `cargo test --all-features` Expected: controller-local plugin-backed surfaces register through the new surface registry.

- [ ] **Step 3: Commit**

```bash
git add crates/plugins
git commit -m "feat: port plugin-backed surfaces to new surface contract"
```

### Task 7: Port Service-Backed Providers And CLI

**Files:**

- Modify: `crates/core/agent-ssh/src/extension.rs`
- Modify: `crates/core/agent-ssh/src/main.rs`
- Modify: `crates/core/mqtt/src/extension.rs`
- Modify: `crates/core/controller/src/ssh_agent/mod.rs`
- Modify: `crates/ui/cli/src/commands/extensions.rs`
- Create: `crates/ui/cli/src/commands/surfaces.rs`
- Modify: service SDK consumers to use `ServiceSurfaceProxy`

- [ ] **Step 1: Port `agent-ssh` surface registration and action handling**

Replace extension registration payloads and handlers with surface registration and action handling.

Run: `cargo check -p uptrakit-agent-ssh` Expected: `agent-ssh` compiles using the new protocol.

- [ ] **Step 2: Port `mqtt` service-backed settings surface**

Replace MQTT extension registration/action code with the new surface contract and targeted-provider flow.

Run: `cargo check -p uptrakit-mqtt` Expected: MQTT compiles using the new protocol.

- [ ] **Step 3: Replace CLI dynamic extension logic**

Introduce a new CLI command surface that consumes controller-vetted surfaces/interactions instead of old `ExtensionUi`/`ActionUi`.

Run: `cargo check -p uptrakit-cli` Expected: CLI builds without dependency on the old extension framework enums.

- [ ] **Step 4: Commit**

```bash
git add crates/core/agent-ssh crates/core/mqtt crates/ui/cli crates/core/controller/src/ssh_agent
git commit -m "feat: port service providers and cli to surface runtime"
```

### Task 8: Remove The Old Extension Framework And Finalize Verification

**Files:**

- Delete: `crates/shared/extension-framework/`
- Modify: all crates still depending on `uptrakit-extension-framework`
- Delete or replace: `crates/ui/web-api/src/extension_registry.rs`
- Delete or replace: `crates/ui/web-api/src/extension_proxy.rs`
- Delete or replace: `crates/ui/web-api/src/routes/extensions.rs`
- Delete or replace: `frontend/src/lib/components/extensions/*`
- Modify: docs that reference old extension framework paths

- [ ] **Step 1: Remove the old crate and dead references**

Delete the old framework crate and replace remaining imports/usages across the workspace only after the Task 0 rollout guard passes with all required
first-party providers migrated to the new generation.

Run: `cargo check --all-features` Expected: no crate still depends on `uptrakit-extension-framework`.

- [ ] **Step 2: Remove the old frontend extension-only path**

Delete compatibility wrappers and old extension store/helpers once all routes use the surface runtime.

Run: `cd frontend && npm run check && npm run build` Expected: frontend builds without `lib/extensions.svelte.ts` or `components/extensions/*`.

- [ ] **Step 3: Run backend and frontend verification**

Run:

```bash
cargo fmt --all
cargo check --no-default-features --features db-sqlite
cd frontend && npm run build
cd ..
cargo check --all-features
cargo clippy --all-targets --no-default-features --features db-sqlite
cargo clippy --all-targets --all-features
cargo test --all-features
cargo test -p uptrakit-web-api surface_rollout
bash ci/verify_handler_state_contract.sh
python3 ci/verify_db_access_policy.py
```

Expected:

- all checks pass
- no old extension-framework compile references remain
- new surface runtime passes unit/integration coverage
- rollout guard passes with the migrated provider set, so deleting the old runtime path cannot strand the deployment on an inactive new path

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "refactor: replace extension framework with shared surface model"
```

### Task 9: Documentation And Follow-Through

**Files:**

- Modify: `ARCHITECTURE.md`
- Modify: `docs/development/plugin-system.md`
- Modify: `docs/development/frontend-components.md` if present
- Modify: any docs that currently reference `extension-framework`, `ExtensionManifest`, or extension-only renderer paths

- [ ] **Step 1: Update architecture and developer docs**

Document:

- new shared surface contract
- controller capability gating
- slot registry ownership
- provider registration protocol
- frontend shared renderer path

Run: `markdownlint --config .markdownlint.json 'docs/**/*.md'` Expected: updated docs pass markdown linting.

- [ ] **Step 2: Commit**

```bash
git add ARCHITECTURE.md docs
git commit -m "docs: describe shared surface runtime"
```

## File Structure Summary

### New Core Units

- `crates/shared/surfaces/`: canonical surface, data, interaction, slot, and protocol contracts
- `crates/ui/web-api/src/surface_registry.rs`: normalized controller runtime registry
- `crates/ui/web-api/src/surface_proxy.rs`: action dispatch/correlation/cancellation
- `frontend/src/lib/surfaces/`: TS contract, store, slot indexing
- `frontend/src/lib/components/surfaces/`: shared renderer primitives

### Primary Migration Targets

- Controller runtime: `crates/ui/web-api/src/*`, `crates/core/controller/src/main.rs`
- Built-in routes: `frontend/src/routes/settings/*`, `frontend/src/routes/software/*`, `frontend/src/routes/surfaces/*`,
  `frontend/src/routes/extensions/*`
- Providers: `crates/core/agent-ssh/src/*`, `crates/core/mqtt/src/*`, `crates/plugins/**/*`
- CLI: `crates/ui/cli/src/commands/*`

## Verification Notes

- Do not enable the new runtime path in production until Phase 0 cutover conditions are met.
- Keep shared-surface endpoints fail-closed while Tasks 1 through 7 land; all new-route and new-runtime behavior stays behind the rollout flag until
  the cutover guard can pass.
- Treat `settings`, `software`, and `software/[id]` as the proving routes for shared rendering before porting the rest.
- Do not remove compatibility redirects or dormant surface entry points until the provider-backed surfaces for that route have been ported and the
  rollout guard can still keep the new path inert.

## Suggested Commit Sequence

1. `feat: add surface runtime rollout guard`
2. `feat: add shared surface contract crate`
3. `feat: add surface wire protocol and service proxy`
4. `feat: add controller surface runtime`
5. `feat: add frontend surface store and renderer`
6. `feat: migrate built-in and extension pages to surface slots`
7. `feat: port plugin-backed surfaces to new surface contract`
8. `feat: port service providers and cli to surface runtime`
9. `refactor: replace extension framework with shared surface model`
10. `docs: describe shared surface runtime`

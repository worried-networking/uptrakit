# Extract MCP Server into `uptrakit-mcp` Crate — Design

## Goal

Move the MCP server out of `uptrakit-web-api` into a dedicated crate `uptrakit-mcp`
(`crates/ui/mcp/`). First step in a long-term effort to strip `web-api` down to pure
API concerns (routing, middleware, validation), with business logic extracted into
focused satellite crates.

**Scope of benefit:** This extraction is an _organizational_ improvement, not a
compile-time one. `uptrakit-mcp` depends on `uptrakit-web-api` unconditionally, so
the full `web-api` graph still compiles when building `mcp`. The wins are: MCP code has
a clear home, the OAuth 2.1 auth layer can be rewritten without touching HTTP routes,
and the pattern is established for further `web-api` decomposition.

## Motivation

- `web-api` is growing too large; MCP is a coherent subsystem that can stand alone.
- The next planned MCP feature (OAuth 2.1 authorization flow) requires owning the auth
  layer in a place where it can be rewritten without touching HTTP route code.
- Establishes the pattern for future `web-api` decomposition.

---

## Dependency Graph

```text
controller-runtime
  ├── uptrakit-web-api        (HTTP routes, AppState, auth)
  └── uptrakit-mcp            (MCP transport, tools, auth layer)
        └── uptrakit-web-api  (AppState, bridge fns, McpRequestContext)
```

Direction: `uptrakit-mcp` → `uptrakit-web-api`. One-way. `web-api` gains no new
dependency on the MCP crate.

**Constraint:** `uptrakit-mcp` imports only from `web-api`'s public API. No `pub(crate)`
items in `web-api` are promoted just to feed the MCP crate. Where `pub(crate)` internals
are needed, `web-api` exposes a single clean bridge function that wraps them.

---

## What Moves to `crates/ui/mcp/`

| Current path (in `web-api`) | New path (in `uptrakit-mcp`) | Notes                                                          |
| --------------------------- | ---------------------------- | -------------------------------------------------------------- |
| `mcp/mod.rs`                | `src/lib.rs`                 | `build_mcp_router`, config helpers                             |
| `mcp/auth.rs`               | `src/auth.rs`                | `McpAuthLayer`, `McpAuthService` — rewritten to call bridge fn |
| `mcp/terminal.rs`           | `src/terminal.rs`            | No changes needed                                              |
| `mcp/tools/mod.rs`          | `src/tools/mod.rs`           | `McpHandler`, `mcp_error`                                      |
| `mcp/tools/history.rs`      | `src/tools/history.rs`       | Import path updates (see below)                                |
| `mcp/tools/user.rs`         | `src/tools/user.rs`          | Import path updates (see below)                                |
| `mcp/tools/update.rs`       | `src/tools/update.rs`        | Rewritten to call `mcp_trigger_update` bridge fn               |

`McpRequestContext` is the exception: it stays in `web-api` (moved to
`src/mcp_compat.rs`, promoted to unconditional `pub`). The new crate imports it from
`uptrakit_web_api::McpRequestContext`. This avoids a circular dependency while keeping
the type accessible to both the bridge functions and the MCP crate.

### Import path changes for moved files

Files where `crate::` references must be rewritten when moving to `uptrakit-mcp`:

**`history.rs`:**

- `crate::mcp::auth::McpRequestContext` → `uptrakit_web_api::McpRequestContext`
- `crate::auth::permissions::Permission` → `uptrakit_web_api::auth::permissions::Permission`
- `crate::mcp::terminal::render_terminal_output` → `crate::terminal::render_terminal_output`
- `crate::mcp::tools::{McpHandler, mcp_error}` → `crate::tools::{McpHandler, mcp_error}`
- `crate::queries` → `uptrakit_web_api::queries`

**`user.rs`:**

- `crate::mcp::auth::McpRequestContext` → `uptrakit_web_api::McpRequestContext`
- `crate::mcp::tools::{McpHandler, mcp_error}` → `crate::tools::{McpHandler, mcp_error}`
- `crate::auth::permissions::Permission` → `uptrakit_web_api::auth::permissions::Permission`
  _(test module only)_

**`auth.rs`** (McpAuthLayer — after Phase 1 bridge migration):

- `crate::AppState` → `uptrakit_web_api::AppState`
- `crate::auth::permissions::Permission` → `uptrakit_web_api::auth::permissions::Permission`
- `crate::middleware::require_auth::{AuthFailure, authenticate_api_token, emit_api_token_auth_audit}`
  → replaced entirely by `uptrakit_web_api::{validate_api_token_for_mcp, McpAuthError}`
  (Phase 1 migration removes direct use of these `pub(crate)` items)

---

## New Public Surface in `web-api`

Two bridge functions added to `crates/ui/web-api/src/mcp_compat.rs`. Both are
unconditional (no feature gate). Neither references any type from `uptrakit-mcp`.

### `McpRequestContext`

```rust
/// Per-request auth context injected into MCP request extensions.
/// Imported by uptrakit-mcp; defined here to avoid a circular dependency.
///
/// `#[non_exhaustive]` because OAuth 2.1 will add fields (scope claims, sub, etc.).
/// External code must use `McpRequestContext::new(...)`.
#[non_exhaustive]
pub struct McpRequestContext {
    pub user_id: Uuid,
    pub token_id: Uuid,
    pub tenant_id: Uuid,
    pub permissions: Vec<Permission>,
}
```

### Auth bridge

```rust
#[non_exhaustive]
pub enum McpAuthError {
    Unauthorized,
    Forbidden,
    Internal,
}

/// Validate a bearer token for an MCP request.
///
/// Wraps the current API-token auth path. Marked for replacement when the
/// OAuth 2.1 authorization flow lands — at that point `McpAuthLayer` in
/// `uptrakit-mcp` drops this import and owns its own validation.
// TODO: replace with OAuth 2.1 validation
pub async fn validate_api_token_for_mcp(
    state: &AppState,
    token: &str,
) -> Result<McpRequestContext, McpAuthError>
```

Internally calls `authenticate_api_token` + `emit_api_token_auth_audit` and maps
`AuthFailure` → `McpAuthError`. These remain `pub(crate)` in `require_auth.rs`.

### Update bridge

```rust
/// Error type for the update bridge. Variants finalized during implementation;
/// wraps permission failures, not-found cases, and internal dispatch errors
/// sourced from the existing `trigger_update` action.
#[non_exhaustive]
pub enum McpTriggerError {
    PermissionDenied,
    HostNotFound,
    SoftwareItemNotFound,
    /// Host exists but has no assignment for this software item, or no execute
    /// plugin is configured for the pair (covers HostNotAssigned,
    /// NoExecuteUpdatePlugin, PluginConfigNotFound, UnknownPluginType).
    NotConfigured,
    /// Host has no linked agent, or agent is not in Approved status
    /// (covers NoAgent, AgentNotApproved).
    AgentUnavailable,
    AlreadyInProgress,
    Internal,
}

pub async fn mcp_trigger_update(
    state: Arc<AppState>,
    ctx: &McpRequestContext,
    host_id: Uuid,
    software_item_id: Uuid,
    to_version: String,
) -> Result<(Uuid, TriggerUpdateStatus), rootcause::Report<McpTriggerError>>
```

`TriggerUpdateStatus` is from `uptrakit-web-api-types` (already a shared dep — no new
dep introduced). Internally calls `trigger_update`, `spawn_protection_and_dispatch`, and
`emit_software_update_audit` — all of which remain `pub(crate)`.

`TriggerUpdateInput` / `TriggerUpdateResult` (the MCP-specific DTOs) stay in
`uptrakit-mcp` — the bridge fn takes/returns primitives only.

---

## What `web-api` Loses

- `#[cfg(feature = "mcp")] pub mod mcp` — deleted
- `rmcp`, `vt100`, `schemars` — removed from `[dependencies]`
- `mcp` feature flag — removed entirely from `Cargo.toml`

---

## `controller-runtime` Changes

```toml
# Before
mcp = ["uptrakit-web-api/mcp"]

# After
mcp = ["dep:uptrakit-mcp"]

[dependencies]
uptrakit-mcp = { workspace = true, optional = true }
```

`server.rs`:

```rust
// Before
#[cfg(feature = "mcp")]
router = router.merge(uptrakit_web_api::build_mcp_router(Arc::clone(&state)));

// After
#[cfg(feature = "mcp")]
router = router.merge(uptrakit_mcp::build_mcp_router(Arc::clone(&state)));
```

---

## New Crate: `crates/ui/mcp/Cargo.toml`

Key dependencies:

```toml
[dependencies]
uptrakit-web-api     = { workspace = true }
rmcp                   = { workspace = true }
vt100                  = { workspace = true }
schemars               = { workspace = true }
sea-orm                = { workspace = true }
axum                 = { workspace = true }
tower                = { workspace = true }
uuid                 = { workspace = true }
serde                = { workspace = true }
uptrakit-web-api-types = { workspace = true }
uptrakit-shared-db   = { workspace = true }
```

No feature flags needed — the crate is unconditionally MCP.

**Note:** `rmcp`, `vt100`, and `schemars` are currently bare-version deps in
`web-api/Cargo.toml` only — not in `[workspace.dependencies]`. Phase 2 step 1 must
promote them to the workspace before the new crate's `Cargo.toml` can reference them
with `{ workspace = true }`.

`uptrakit-audit-log` is **not** a dependency of `uptrakit-mcp`. All audit calls
(auth audit, update audit) move into the bridge functions in `web-api`, which already
depends on `uptrakit-audit-log`. The new crate never calls audit functions directly.

---

## Implementation Phases

### Phase 1 — Prepare `web-api` (in-place, no structural change)

All changes compile and pass tests before Phase 2 begins.

1. Add `src/mcp_compat.rs` to `web-api`:
   - Move `McpRequestContext` from `mcp/auth.rs` (keep `pub` re-export in `mcp/auth.rs`
     during transition)
   - Add `pub use mcp_compat::McpRequestContext` in `lib.rs` (unconditional)
2. Add `validate_api_token_for_mcp` + `McpAuthError` to `mcp_compat.rs`
3. Add `mcp_trigger_update` + `McpTriggerError` to `mcp_compat.rs`
4. Migrate `mcp/auth.rs`: replace inline `authenticate_api_token` /
   `emit_api_token_auth_audit` / `AuthFailure` usage with call to
   `validate_api_token_for_mcp`
5. Migrate `mcp/tools/update.rs`: replace `trigger_update` /
   `spawn_protection_and_dispatch` / `emit_software_update_audit` calls with
   `mcp_trigger_update`
6. **Gate: `cargo check --all-features && cargo test --all-features` — green**

### Phase 2 — Create crate and move files

1. Promote `rmcp`, `vt100`, `schemars` to `[workspace.dependencies]` in root `Cargo.toml`
   (move from bare version strings in `web-api/Cargo.toml`)
2. Create `crates/ui/mcp/Cargo.toml`; add `crates/ui/mcp` to `[workspace.members]` and
   `uptrakit-mcp` to `[workspace.dependencies]` in root `Cargo.toml`
3. Move `mcp/` files into new crate; rewrite `crate::` imports per the import-path
   table above
4. Remove `mcp/` module from `web-api`; drop `rmcp` / `vt100` / `schemars` optional
   dep entries; remove `mcp` feature from `web-api/Cargo.toml`
5. **Sub-gate:** `cargo check --all-features` — green before touching `controller-runtime`
   (catches broken imports in new crate and web-api independently before wiring)
6. Update `controller-runtime` `Cargo.toml` and `server.rs`
7. **Gate:**
   - `cargo check --all-features && cargo test --all-features` — green
   - `cargo test -p uptrakit-mcp` — green (new crate tests pass in isolation)
   - `cargo test -p uptrakit-web-api` — green (web-api tests pass without mcp module)

---

## OAuth 2.1 Forward Compatibility

`McpAuthLayer` in the new crate will be rewritten for OAuth 2.1 as a follow-on task.
At that point:

- `validate_api_token_for_mcp` is deleted from `web-api`
- `McpAuthLayer` owns its own AS/RS logic entirely within `uptrakit-mcp`
- `McpAuthError` and `McpRequestContext` may be revised or extended

The bridge function is **intentionally temporary scaffolding** — the `// TODO: replace
with OAuth 2.1 validation` comment signals this to future implementors.

---

## Out of Scope

- OAuth 2.1 implementation (separate spec)
- Further `web-api` decomposition beyond this extraction
- Adding new MCP tools

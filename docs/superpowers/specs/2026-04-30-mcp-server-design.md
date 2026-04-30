# MCP Server — Design

## Problem

There is no machine-accessible interface for AI agents (Claude Desktop, Cursor, etc.) to query
controller state or trigger actions. The REST API serves human-driven UIs and scripts, not
AI-agent tool-call patterns. An MCP server closes this gap.

## Approach

Embed an MCP server as a module inside `uptrakit-web-api` (`src/mcp/`). Mount it as a
Streamable HTTP (SSE) endpoint at `/mcp` on the existing Axum HTTPS server. Use the `rmcp`
1.x crate for protocol handling. Gate the entire feature behind a `mcp` Cargo feature
(default-on; disable with `--no-default-features`).

The module is intentionally kept inside `web-api` for MVP because `AppState` is defined there
and the compile-time isolation benefit of a separate crate is not achievable until `AppState` is
extracted into a lighter `web-api-core` crate. Promotion to `crates/ui/mcp/` is a mechanical
refactor deferred to after that extraction.

## Architecture

```text
Axum HTTPS server (existing, port 9443)
├── /api/...          existing REST API
├── /mcp              new — rmcp Streamable HTTP (SSE)
└── /                 SPA fallback
```

`build_mcp_router()` returns a `Router<Arc<AppState>>`. Merged in
`controller-runtime/src/server.rs` after `build_router` returns, before middleware layers are
applied — this is the guaranteed merge order:

```rust
let router = uptrakit_web_api::build_router(app_state.clone())
    .merge(uptrakit_web_api::mcp::build_mcp_router(app_state));
// middleware layers applied here, covering both routers
```

## Module Structure

```text
crates/ui/web-api/src/mcp/
├── mod.rs          build_mcp_router() → Router<Arc<AppState>>
├── auth.rs         extract AuthenticatedUser, enforce Permission::AccessMcp
├── terminal.rs     vt100 rendering (width=220) + plain-text output
└── tools/
    ├── mod.rs
    ├── user.rs     get_current_user
    ├── history.rs  list_update_history, get_update_history_detail
    └── update.rs   trigger_update
```

## Authentication

The `/mcp` endpoint uses the same `Authorization: Bearer` header as the REST API. Token
dispatch logic in the existing `require_auth` middleware handles both token types:

- `upk_...` prefix → opaque API token → DB lookup via `ApiTokenService::verify_token()` →
  resolves `(user_id, token_id)`
- No `upk_` prefix → JWT access token → stateless validation

Both produce an `AuthenticatedUser` struct injected into request extensions. `mcp/auth.rs`
extracts this and verifies `Permission::AccessMcp` is present. Missing token → 401. Missing
permission → 403.

**User setup:** create an API token in the UI, paste it into Claude Desktop / Cursor config as
`Authorization: Bearer upk_...`.

## Permissions

A new `Permission::AccessMcp` variant is added to the `Permission` enum. All existing roles
receive it by default via a DB migration.

### Permission enum prerequisite

`Permission` is stored in the database and deserialized at runtime. The current exhaustive
`FromStr` is a serialization hazard during rolling deployments: an old binary encountering a
new unknown variant returns `Err` and silently drops the permission, producing a `403`.

Before adding `AccessMcp`, `Permission` is migrated to the `Other(String)` wire-safe catch-all
pattern (canonical implementation: `crates/shared/wire/src/lib.rs`):

- Add `Other(String)` variant with `#[strum(disabled)]` — excluded from `EnumIter`, so
  `Permission::iter()` still yields only known variants
- Replace exhaustive `FromStr` with custom infallible `Deserialize`: unknown strings →
  `Other(s)` instead of `Err`
- Remove `Copy` (unavoidable — `String` is not `Copy`); update call sites to `.clone()` or
  `&Permission` as needed
- `EnumIter` is preserved via `#[strum(disabled)]`

## Tools

All tools are tenant-scoped via the resolved API token. The token's owning user is the tenant
scope — same isolation as the REST API.

### `get_current_user`

Returns identity information about the token owner.

- **Inputs:** none
- **Returns:** `user_id`, `username`, `email`, `permissions[]`
- **Source:** `AuthenticatedUser` already resolved by auth; one DB lookup for user profile

### `list_update_history`

Returns a paginated list of update records, without output text.

- **Inputs:** `host_id?` (UUID), `software_item_id?` (UUID), `status?`, `page?`, `per_page?`
- **Returns:** paginated — `id`, `host_name`, `software_item_name`, `from_version`,
  `to_version`, `status`, `actor_name`, `started_at`, `completed_at`, `interactive`,
  `update_category`
- **Output field:** excluded (can be 50 MB; useless in AI context)
- **Source:** `web-api-queries::update_history::list_update_history()` directly

### `get_update_history_detail`

Returns full detail for a single update record including rendered terminal output.

- **Inputs:** `update_history_id` (UUID)
- **Returns:** all fields above + `output` (plain text, terminal-rendered)
- **Output processing:** raw bytes fed through `vt100` crate at width=220, extracting the
  final screen state as plain text. This correctly collapses `\r` rewrites, cursor-up/down
  progress bar tricks, and strips all ANSI escape sequences. Width 220 avoids wrapping on
  typical package manager output.
- **Source:** `web-api-queries::update_history::get_update_history()` directly

### `trigger_update`

Triggers a software update for a specific host.

- **Inputs:** `host_id` (UUID), `software_item_id` (UUID), `to_version` (String)
- **Returns:** `update_history_id`, `status`
- **Hardcoded:** `interactive = false` (AI agent cannot interact with a PTY)
- **Omitted:** `release_info` (MVP scope)
- **Source:** `item_actions::trigger_update()` directly — same code path as the REST handler,
  no loopback HTTP
- **Actor recording:** `actor_type = "user"`, `actor_id = user_id` — the user who owns the
  token, consistent with REST-triggered update accountability

## Dependencies

Added to `web-api/Cargo.toml` behind `[features] mcp`:

| Crate   | Purpose                                                           |
|---------|-------------------------------------------------------------------|
| `rmcp`  | MCP protocol, Streamable HTTP transport, tool registration macros |
| `vt100` | VT100 terminal emulator for output rendering                      |

`controller-runtime/Cargo.toml` threads `mcp` → `uptrakit-web-api/mcp`.

## Testing

- **Unit — `terminal.rs`:** `\r` line collapsing, cursor-up progress bar collapse, ANSI
  stripping, multi-byte UTF-8 boundary safety
- **Unit — each tool:** mock `AppState` / `TenantDb`, assert correct query arguments and
  response shape
- **Manual end-to-end:** connect Claude Desktop to a local controller instance, exercise all
  four tools, verify tenant isolation holds

## Commit Sequence

1. `Permission`: add `Other(String)` with `#[strum(disabled)]` + custom infallible
   `Deserialize`, remove `Copy`, update call sites
2. `Permission`: add `AccessMcp` variant + DB migration seeding it into all existing roles
3. MCP scaffold: `web-api/src/mcp/mod.rs`, `mcp` feature flag, thread through
   `controller-runtime`, wire `/mcp` route in `server.rs`, add `rmcp` dep
4. MCP auth: `mcp/auth.rs` — extract `AuthenticatedUser`, enforce `AccessMcp`
5. MCP terminal: `mcp/terminal.rs` — `vt100` rendering, add `vt100` dep
6. MCP tool: `get_current_user`
7. MCP tools: `list_update_history` + `get_update_history_detail`
8. MCP tool: `trigger_update`

## Future Work

- **OAuth 2.1 flow** — browser-based "Connect to uptrakit" UX for Claude Desktop. Requires
  `AppState` extraction into `web-api-core` first (authorization server state must not couple
  back into `web-api`). Track as a follow-up after `AppState` decoupling.
- **Promote to `crates/ui/mcp/`** — after `AppState` extraction, promoting this module to a
  separate crate is a mechanical refactor that delivers real compile-time isolation from PKI
  and plugin infrastructure.
- **Additional tools** — host listing, software item inventory, scheduled task management.

# 0003 — Introduce `uptrakit-controller-core` as Business-Logic Boundary

Date: 2026-05-07

## Status

Accepted

## Context

`uptrakit-mcp` depended on `uptrakit-web-api` to access shared state types and
helper functions (`authenticate_api_token`, `mcp_trigger_update`, etc.). This
created a cross-concern dependency that: (a) prevented the MCP server from being
deployed separately from the HTTP server, and (b) would force OAuth 2.1 MCP
authorisation machinery to land in the wrong crate.

The project is preparing to add OAuth 2.1 MCP authorisation (future spec). That
work must land in `uptrakit-mcp`, not in `uptrakit-web-api`. Adding it to mcp
while mcp depends on web-api for core state types would create a tangled
dependency graph that is hard to untangle later.

## Decision

Introduce `uptrakit-controller-core` as a pure business-logic crate with zero
knowledge of `uptrakit-web-api` or `uptrakit-mcp`. Both `web-api` and `mcp`
depend on `controller-core`; neither depends on the other.

The crate boundary is enforced by the absence of `uptrakit-web-api` and
`uptrakit-mcp` path deps in `controller-core/Cargo.toml`. Verified post-Phase 4
by `cargo tree -p uptrakit-mcp | grep uptrakit-web-api` producing no output.

## `crates/ui/` Placement

`controller-core` is pure domain logic yet lives in `crates/ui/` alongside HTTP
and CLI crates. This was chosen over `crates/core/controller-core` for two
reasons:

1. **Co-location with consumers.** Both `web-api` and `mcp` (the primary
   consumers) live in `crates/ui/`. Placing `controller-core` there reduces
   cross-directory import paths and keeps related crates adjacent.

2. **`crates/core/` already has a different signal.** `crates/core/` contains
   the agent runtime, controller runtime, MQTT, and scheduler — background
   services. `controller-core` is not a service; it is shared state and logic.
   Placing it in `crates/core/` would mislead contributors into thinking it
   runs as a service.

Contributors should NOT use the `crates/ui/` directory location as a signal
that `controller-core` has any UI, HTTP, or Axum concerns. The `lib.rs`
invariant doc-comment makes this explicit.

## Alternatives Considered

1. **Keep `mcp → web-api` dep.** Rejected: would force OAuth 2.1 auth machinery
   into `web-api`, bloating it with auth concerns that do not belong there and
   making future MCP standalone deployment harder.

2. **God-struct bundle — expose all types from `web-api` as a flat dep.**
   Rejected: amplifies the coupling problem rather than resolving it.

3. **Place in `crates/core/controller-core`.** Rejected: misleads contributors
   (see placement rationale above). Kept in `crates/ui/` by majority preference.

## Consequences

- `uptrakit-mcp` has zero `uptrakit-web-api` imports (verified by CI).
- OAuth 2.1 MCP auth work lands in `uptrakit-mcp` from day one.
- `AppState` is smaller: grouped into `ServerState` and `PluginState` sub-structs.
- `ControllerUpdateDispatcher` is the single testable production impl of
  `UpdateDispatcher`; tests inject `NoopUpdateDispatcher`.
- All consumers of `authenticate_api_token` pass explicit `db`/`default_tenant_id`
  instead of threading `&AppState` — consistent with `AgentCertSigner` pattern.

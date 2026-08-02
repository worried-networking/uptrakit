# 0017 — ETag Route-Layer Middleware over Per-Handler Extractors

Date: 2026-05-24

## Status

Accepted

## Context

ETag support for settings endpoints was partial and error-prone: only 2 of 11 GET handlers
returned `ETag` headers, only 2 of 9 PUT handlers returned `ETag` in the response, and 4
global-settings handlers used the wrong ETag scope (`SettingsVersion` instead of
`GlobalSettingsVersion`). Each handler that did have ETag support contained 4–6 lines of
boilerplate that was easy to copy with the wrong scope.

## Decision

Use an `axum::middleware::from_fn_with_state`-based route-level middleware typed over
`EtagSource` (`etag_middleware::<S>`), opted-in via `.route_layer()` on per-scope
`OpenApiRouter` sub-routers.

## Alternatives Considered

### 1. Per-handler extractor (current partial state)

Explicit but requires boilerplate in every handler. Error-prone: wrong scope type compiles
silently; missing return header is invisible until a client notices. Rejected.

### 2. Route-level middleware (chosen)

Zero handler boilerplate. The scope (`SettingsVersion` vs `GlobalSettingsVersion`) is declared
exactly once at the router. Extensible to non-settings resources by adding `EtagSource` impls.
Validates `If-Match` for PUT/PATCH, injects `ETag` for GET and mutations, and does a post-write
DB re-read for mutations to return the committed version.

### 3. Global tower layer

No opt-in required. Cannot be scoped per resource type without per-route metadata or a separate
registry. Would inject ETags on non-settings routes unintentionally. Rejected.

## Consequences

- All settings GET endpoints return `ETag` without handler code.
- All settings PUT/PATCH endpoints enforce `If-Match` and return the new `ETag` without handler
  code.
- Successful mutations incur one additional `SELECT` (`refresh_etag`) to read the committed
  version. This is correct regardless of how many fields the write bumped internally.
- `IfMatch<S>` extractor is retained because `plugin_configs.rs` handlers still use it directly.
  It is a candidate for removal in a future spec that migrates those handlers to the layer
  pattern.
- `POST /api/v1/global-settings/ca/rotate` is intentionally outside the `GlobalSettingsVersion`
  sub-router. The handler only signals a background CA-rotation task via `notify_one()` and
  returns immediately; no `settings_version` bump occurs during the HTTP transaction, so
  `refresh_etag` would return the pre-rotation version. Routes whose side-effects are
  asynchronous and do not bump the version counter must be kept outside the ETag sub-router.
- New resources outside settings may adopt the same pattern by implementing `EtagSource` and
  adding the middleware via
  `axum_mw::from_fn_with_state(state, crate::middleware::etag::etag_middleware::<NewResourceVersion>)`
  in the router.
- **Correctness gap (pre-existing):** `upsert_*_raw` calls `bump_*_settings_version` inside
  the same transaction but treats bump failure as non-fatal (logs `warn`, continues). If the
  bump fails, the write succeeds but the version counter is not incremented. `refresh_etag`
  cannot detect this — it returns the pre-write version, so the ETag on the mutation response
  encodes stale state. A subsequent client using the returned ETag would pass `If-Match`
  validation even though the counter did not advance. This is pre-existing behaviour this ADR
  does not change; a future improvement would make bump failures transaction-fatal.

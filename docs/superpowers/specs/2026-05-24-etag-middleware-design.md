# ETag Middleware for Settings Endpoints

**Date:** 2026-05-24
**Status:** Approved

## Problem

ETag support across settings endpoints is inconsistent and requires per-handler boilerplate:

1. **Only 2 of 11 settings GET handlers** return `ETag` headers (`access`, `oauth`). The rest
   expose no ETag on read, so clients cannot do a proper GET → ETag → PUT round-trip.
2. **Only 2 of 9 settings PUT handlers** return `ETag` headers in the response. The others
   require `If-Match` but give no ETag back, forcing clients to re-issue a GET.
3. **4 global-settings handlers use the wrong ETag scope.** `zeroconf`, `nats`, `network`, and
   `providers/github` all live under `/api/v1/global-settings/` but use `IfMatch<SettingsVersion>`
   (tenant scope) instead of `IfMatch<GlobalSettingsVersion>` (global scope). This means
   a write to one global-settings endpoint does not invalidate the version seen by the others.
4. **Boilerplate in handlers.** `settings_access.rs` and `settings_oauth.rs` each contain manual
   `settings_version_cache.get()` lookups, `format!("W/\"...\"")` strings, cache-bump logic,
   and `ETag` header construction that belong in a shared layer.

## Goals

- All settings GET endpoints return an `ETag` header.
- All settings PUT endpoints require `If-Match`, validate it, and return the new `ETag`.
- `POST /api/v1/global-settings/ca/rotate` returns the new `ETag` (it writes state).
- No ETag code in handler bodies.
- Opt-in declared at the router, right next to the route definition.
- Extensible: the same mechanism works for non-settings resources in the future.

## Out of Scope

- `If-None-Match` / 304 Not Modified (settings are low-traffic admin UI; caching optimisation
  is not worth the added complexity).
- Non-settings resources (the infrastructure is ready for them; the wiring is not in scope here).
- `plugin_configs.rs` handlers (`POST/PUT /api/v1/plugin-configs/…`) — these already use
  `IfMatch<SettingsVersion>` as handler-level extractors. Migrating them to the layer pattern
  is deferred to a follow-up. The `IfMatch<S>` extractor is kept (not removed) specifically
  because `plugin_configs.rs` still depends on it.

## Architecture

### `EtagSource` Trait (modified)

Remove the unused `&Parts` parameter (incompatible with middleware context; no existing
implementation references it). Add `refresh_etag` for post-write use, which re-reads the
version from the database, syncs the in-memory cache, and returns the fresh ETag string.

Drop `#[async_trait]` from `EtagSource` — the workspace is edition 2024, both implementations
are monomorphised via `S: EtagSource` bounds (never used as `dyn EtagSource`), so native async
fn in traits is correct and avoids a heap allocation per call. Also remove the now-dead
`use axum::http::request::Parts;` import from `etag_source.rs`.

```rust
// crates/ui/web-api/src/extractors/etag_source.rs
pub trait EtagSource: Sized + Send + Sync + 'static {
    /// Returns the current ETag from the in-memory cache. Fast; used for GET responses.
    async fn current_etag(state: &AppState) -> Result<String, Report>;

    /// Re-reads the version from the DB, syncs the cache, and returns the new ETag.
    /// Used after a successful mutation so the response carries the committed version.
    ///
    /// For GET-only resources this method is never called by `EtagLayer`. Implementors
    /// covering read-only resources may return `Err(report!("refresh not supported"))`.
    async fn refresh_etag(state: &AppState) -> Result<String, Report>;
}
```

**`SettingsVersion` impl** — `current_etag` reads `Scope::Tenant(default_tenant_id)` from cache
(unchanged). `refresh_etag` calls `get_settings_versions(db, tenant_id)`, updates the cache, and
returns `W/"settings-v{n}"`. Both methods use `default_tenant_id` — a single-tenant assumption
that must be revisited if multi-tenancy is ever added. Mark both call sites with
`// SINGLE-TENANT ASSUMPTION` at implementation time.

**`GlobalSettingsVersion` impl** — `current_etag` reads `Scope::Global` from cache (unchanged).
`refresh_etag` calls `get_settings_versions(db, tenant_id)`, updates the `Scope::Global` cache
entry, and returns `W/"global-settings-v{n}"`. This is exactly what `update_oauth_settings`
currently does manually; it moves into the middleware.

The `IfMatch<S>` extractor struct is **kept** — `plugin_configs.rs` still uses it (out of
scope for this spec). `IfMatch::for_test()` is also kept for the same reason. The extractor
and its test helper are only candidates for removal in a future spec that migrates
`plugin_configs.rs` to the layer pattern.

### `EtagLayer<S>` Middleware

New file: `crates/ui/web-api/src/middleware/etag.rs`

Implemented via `axum::middleware::from_fn_with_state`. A public factory function
`etag_layer::<S>(state: Arc<AppState>)` returns the configured layer. The middleware
function takes `Request<Body>` and inspects `req.method()` to choose its pre-handler
branch — only `PUT`/`PATCH` trigger the `If-Match` check before calling `next.run(req)`;
`GET` and `POST` call the handler immediately and only decorate the response after.

**Behaviour by HTTP method:**

| Method      | Before handler                                | After handler (2xx only)                        |
| ----------- | --------------------------------------------- | ----------------------------------------------- |
| GET         | —                                             | `S::current_etag(state)` → inject `ETag` header |
| PUT / PATCH | Check `If-Match`: 428 if absent, 409 if stale | `S::refresh_etag(state)` → inject `ETag` header |
| POST        | —                                             | `S::refresh_etag(state)` → inject `ETag` header |

Non-2xx responses pass through unmodified. The middleware never touches error bodies.

For GET responses the middleware reads from the in-memory cache (`current_etag`) — no DB
query. For successful mutations it re-reads from the DB (`refresh_etag`) — one extra `SELECT`
per successful write. This is correct regardless of how many fields `upsert_global_setting_raw`
wrote (and therefore how many times it bumped the DB version counter internally). The
`upsert_*_raw` functions' existing auto-bump behaviour is **unchanged**; the transactional
atomicity of write + version bump is preserved.

**ETag response contract:** The ETag returned in a mutation response reflects the DB version
at response time, which is guaranteed to be ≥ the version of the write just committed. In
the interval between `tx.commit()` and the `refresh_etag` SELECT, a concurrent writer may
have committed a further bump; in that case the returned ETag encodes the later write's
version rather than the caller's own. This is safe for optimistic locking — the value is
always valid as an `If-Match` header for the next mutation — but the ETag should not be
interpreted as a unique identifier for the caller's specific write.

**Bump failure behaviour:** `upsert_*_raw` calls `bump_*_settings_version` inside the same
transaction and treats failure as non-fatal (logs `warn`, continues). If the bump fails, the
data is written but the version counter is not incremented. `refresh_etag` would then return
the pre-write version — it cannot detect this case on its own because it has no knowledge of
the pre-write version value. This is pre-existing behaviour that this spec does not change.
It is acknowledged here so it is not silently promoted. A future improvement would make
bump failures fatal (rolling back the transaction); that is out of scope for this spec.

**`If-Match` validation logic** (mirrors existing `IfMatch<S>` extractor):

- Missing header → `428 Precondition Required`, code `"if_match.required"`
- Stale ETag → `409 Conflict`, code `"if_match.stale"`
- Both weak (`W/"..."`) and strong (`"..."`) ETags accepted (strip `W/` before comparison)

### Router Opt-In

```rust
.route(
    "/api/v1/settings/access",
    get(get_access_settings)
        .put(update_access_settings)
        .route_layer(etag_layer::<SettingsVersion>(state.clone()))
)
```

The type parameter on `etag_layer` is the only place the developer declares which scope
governs a route group. Handlers contain no ETag code.

## Complete Route Matrix

| Endpoint                                       | ETag scope              | Notes                                |
| ---------------------------------------------- | ----------------------- | ------------------------------------ |
| `GET /api/v1/settings`                         | `SettingsVersion`       | combined tenant view, GET only       |
| `GET /api/v1/settings/access`                  | `SettingsVersion`       |                                      |
| `PUT /api/v1/settings/access`                  | `SettingsVersion`       |                                      |
| `GET /api/v1/settings/agent-certificates`      | `SettingsVersion`       |                                      |
| `PUT /api/v1/settings/agent-certificates`      | `SettingsVersion`       |                                      |
| `GET /api/v1/global-settings`                  | `GlobalSettingsVersion` | combined global view, GET only       |
| `GET /api/v1/global-settings/oauth`            | `GlobalSettingsVersion` |                                      |
| `PUT /api/v1/global-settings/oauth`            | `GlobalSettingsVersion` |                                      |
| `GET /api/v1/global-settings/network`          | `GlobalSettingsVersion` | **fixes bug**: was `SettingsVersion` |
| `PUT /api/v1/global-settings/network`          | `GlobalSettingsVersion` | **fixes bug**: was `SettingsVersion` |
| `GET /api/v1/global-settings/nats`             | `GlobalSettingsVersion` | **fixes bug**: was `SettingsVersion` |
| `PUT /api/v1/global-settings/nats`             | `GlobalSettingsVersion` | **fixes bug**: was `SettingsVersion` |
| `GET /api/v1/global-settings/providers/github` | `GlobalSettingsVersion` | **fixes bug**: was `SettingsVersion` |
| `PUT /api/v1/global-settings/providers/github` | `GlobalSettingsVersion` | **fixes bug**: was `SettingsVersion` |
| `GET /api/v1/global-settings/zeroconf`         | `GlobalSettingsVersion` | **fixes bug**: was `SettingsVersion` |
| `PUT /api/v1/global-settings/zeroconf`         | `GlobalSettingsVersion` | **fixes bug**: was `SettingsVersion` |
| `POST /api/v1/global-settings/ca/rotate`       | `GlobalSettingsVersion` | writes state; returns new ETag       |
| `POST /api/v1/settings/reset-data`             | none                    | destructive teardown; no ETag        |

## Handler Migration

The following are **removed** from handler bodies as part of this change:

**From `get_access_settings`:**

- Manual `settings_version_cache.get()` lookup
- `format!("W/\"settings-v{version}\"")` string
- `[(axum::http::header::ETAG, etag)]` tuple in response

**From `update_access_settings`:**

- `_if_match: IfMatch<SettingsVersion>` parameter
- Cache bump block (`settings_version_cache.update(scope, next)`)
- `format!("W/\"settings-v{next}\"")` string and ETag header in response

**From `get_oauth_settings`:**

- Manual version lookup and ETag construction

**From `update_oauth_settings`:**

- `_if_match: IfMatch<GlobalSettingsVersion>` parameter
- `get_settings_versions(db)` re-read and `settings_version_cache.update()` call
- ETag header construction in response

**From all other PUT settings handlers** (`agent_certs`, `network`, `nats`, `github`, `zeroconf`):

- `_if_match: IfMatch<SettingsVersion>` parameter (and corrected to `GlobalSettingsVersion`
  where applicable — these handlers had the wrong scope)

**EtagSource trait call sites:**

- All `current_etag(parts, state)` calls updated to `current_etag(state)` (remove `parts`
  argument).

## File Layout

```text
crates/ui/web-api/src/
  middleware/
    mod.rs              ← add `pub mod etag;`
    etag.rs             ← NEW: EtagLayer<S>, etag_layer() factory
  extractors/
    etag_source.rs      ← MODIFY: remove &Parts, add refresh_etag()
    if_match.rs         ← MODIFY: update current_etag() call sites (remove parts arg);
                                  drop #[async_trait] from SettingsVersion/GlobalSettingsVersion impls;
                                  IfMatch<S> and IfMatch::for_test() are KEPT (plugin_configs still uses them)
```

## Tests

`integration_tests/if_match.rs` tests go through the full axum test router (via `TestApp`).
They continue to work after this change because the middleware now owns the logic the
extractor previously owned. Test coverage additions:

- All settings GET endpoints return `ETag` header (not just `access` and `oauth`)
- All settings PUT endpoints return `ETag` header on success (not just `access` and `oauth`)
- Round-trip for previously untested endpoints: GET → ETag → PUT → new ETag
- Global-settings endpoints correctly reject a tenant-scoped ETag (regression test for the
  scope bug)

Handler-direct unit tests in `settings_nats.rs` (~7), `settings_zeroconf.rs` (~1),
`settings_network.rs` (~2), and `settings_agent_certs.rs` (~2) currently call handler
functions directly passing `IfMatch::for_test()`. Once the `_if_match` parameter is removed
from those handler signatures, these ~12 test functions break. They are **deleted** and their
coverage is replaced by the new integration tests listed above (round-trip GET → ETag → PUT
via `TestApp`). This is an explicit migration deliverable, not a side-effect. `plugin_configs.rs`
unit tests (~34 call sites) are unaffected — that handler keeps `IfMatch<S>` throughout.

## ADR

A new ADR (`docs/adr/0017-etag-route-layer-middleware.md`) documents the decision to use
route-level middleware over per-handler extractors. The three alternatives considered:

1. **Per-handler extractor** (current partial state) — explicit but requires boilerplate in
   every handler and is error-prone (wrong scope, missing return header).
2. **Route-level middleware** (chosen) — zero handler boilerplate; scope declared once at
   the router; extensible to non-settings resources via `EtagSource` implementations.
3. **Global tower layer** — no opt-in needed; cannot be scoped per resource type without
   per-route metadata.

## Documentation Deliverables

- `docs/adr/0017-etag-route-layer-middleware.md` — new ADR (required; architectural decision)
- `docs/development/coding-standards.md` — add section describing `etag_layer` usage pattern,
  the rule that settings PUT handlers must be covered by the layer, and the explicit guidance
  that POST endpoints (especially destructive ones like reset-data) must be evaluated before
  being included in an etag-layer route group
- No changes to `CONTEXT.md` — ETag mechanics are implementation detail, not domain language

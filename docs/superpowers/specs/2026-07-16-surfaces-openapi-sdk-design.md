# Surfaces API: OpenAPI Registration, Generated SDK, Resource-Shaped Read Model

**Date:** 2026-07-16
**Status:** Approved (user interview + contrarian pass; split out of the combined surfaces-REST spec on user request)
**Scope:** `crates/ui/web-api` (routes, OpenAPI), `crates/shared/web-api-types`, `crates/shared/openapi-client`,
`crates/ui/cli`, `frontend/`, docs.
**Sequencing:** Lands **first**. The follow-up spec
[`2026-07-16-surfaces-dataload-get-typing-design.md`](2026-07-16-surfaces-dataload-get-typing-design.md) builds on
the utoipa/SDK rails this spec establishes.

## Problem

1. **Outside OpenAPI.** The four surface routes are registered as raw axum routes (`crates/ui/web-api/src/router.rs`,
   `auth_routes` block) with zero `#[utoipa::path]` registrations; `openapi.json` has no `/api/v1/surfaces` paths.
   Consequence: a hand-written frontend client (`frontend/src/lib/api/surfaces.ts`) and hand-written path constants
   in `crates/shared/openapi-client/src/{surfaces.rs,paths.rs}` instead of the generated-SDK pipeline every other
   endpoint uses.
2. **Verb path segment.** The full read model lives at `GET /surfaces/{surface_id}/read` instead of the resource
   itself.
3. **Undocumented authorization asymmetry.** `list_surfaces`/`list_surface_providers` perform no permission check
   (auth + visibility filtering only) while `get_surface_read`/`invoke_surface_interaction` enforce dynamic
   descriptor/interaction permissions in-handler via `enforce_required_permission` — none of it documented against
   the platform's typed-permission-extractor convention.
4. **No caching contract.** Surface GET responses carry no cache directives despite being per-tenant,
   per-permission data.

## Decisions (settled — do not reopen)

| #   | Decision       | Resolution                                                                                                                                                                                                                                   |
| --- | -------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| A1  | Read model     | `GET /surfaces/{surface_id}` replaces `GET /surfaces/{surface_id}/read`. Old path removed (404).                                                                                                                                             |
| A2  | OpenAPI        | All surface endpoints registered via utoipa; frontend switches to the generated SDK; hand-written `surfaces.ts` retired for the endpoints this spec covers.                                                                                  |
| A3  | Idempotency    | `idempotency_key` stays a body field on POST invoke, now documented in the OpenAPI schema. `Idempotency-Key` header move rejected (zero precedent in repo, zero current users — CLI sends `None`, frontend omits it, server auto-generates). |
| A4  | Error envelope | No change — surfaces already return the platform `ErrorResponse { error, code }` via `error_response_with_code`. Invoke success stays the raw provider JSON value.                                                                           |
| A5  | Compatibility  | Atomic breaking change for the `/read` path rename. Frontend, CLI, and `openapi-client` are all in-repo and update in the same change. No deprecation window.                                                                                |

Interaction invocation stays `POST` for **all** kinds in this spec; the method/kind split is the follow-up spec's
subject.

## Design

### 1. Routes (after this spec)

| Method | Path                                                          | Handler                      | Change                              |
| ------ | ------------------------------------------------------------- | ---------------------------- | ----------------------------------- |
| GET    | `/api/v1/surfaces`                                            | `list_surfaces`              | now in OpenAPI                      |
| GET    | `/api/v1/surfaces/{surface_id}`                               | `get_surface_read`           | **moved** from `/{surface_id}/read` |
| GET    | `/api/v1/surfaces/{surface_id}/providers`                     | `list_surface_providers`     | now in OpenAPI                      |
| POST   | `/api/v1/surfaces/{surface_id}/interactions/{interaction_id}` | `invoke_surface_interaction` | now in OpenAPI; behavior unchanged  |

**Caching:** every surface GET response (read model, providers, list) sets `Cache-Control: private, no-store`.
Results are per-tenant and per-permission; shared caches and bfcache must never serve them across users.

### 2. OpenAPI registration

- All four operations get `#[utoipa::path]` registrations via `.routes(routes!(...))` before `split_for_parts()`.
- Typed schemas: `SurfaceResponse`, `SurfaceProviderInfo`, `SurfaceReadResponse`,
  `InvokeSurfaceInteractionRequest` gain `ToSchema`; `ListSurfacesQuery` gains `IntoParams` (referenced as
  `params(<Struct>)`, satisfying `ci/verify_no_inline_query_params.sh` / ADR-0025). The invoke success response is
  documented as a free-form JSON value (provider-defined).
- `x-required-permission` extensions: for read/invoke the human-readable dynamic form — precedent: the
  multi-permission string form in `crates/ui/web-api/src/routes/system_services.rs` — e.g.
  `json!("dynamic: declared by the surface descriptor / interaction")`. `list`/`providers` document
  authenticated-only + visibility-filtered access.

### 3. SDK migration (atomic with the rename)

- Frontend: `./scripts/regen-api.sh`; generated SDK replaces `frontend/src/lib/api/surfaces.ts` for all four
  operations. Consumers to migrate: `frontend/src/lib/surfaces/registry.svelte.ts`, `SurfaceReadPanel.svelte`,
  `SurfaceForm.svelte`, and the surface interaction components — call-shape change only; invocation semantics
  unchanged.
- `crates/shared/openapi-client`: `read_surface` path updated to `/surfaces/{id}`; path constants in `paths.rs`
  updated. Satisfies "Keep the openapi-client in sync with web-api endpoints"; gated by
  `cargo xtask openapi-client-check`.
- CLI (`crates/ui/cli/src/commands/surfaces.rs`): uses the openapi-client methods — the `read` path change flows
  through; no CLI-side contract change.

### 4. Authorization (documented, not restructured)

- `list_surfaces` / `list_surface_providers`: authenticated-only; results filtered by descriptor visibility. No
  static permission exists that fits (no `Surfaces*` variant in `crates/shared/types/src/permissions.rs`), and
  inventing one is out of scope. Documented in `docs/security/surfaces.md`.
- Read model + invoke: dynamic in-handler checks via `enforce_required_permission` (descriptor first, then
  interaction) — the only viable model, since permissions are runtime descriptor data no static extractor can carry.
  The typed-permission-extractor platform rule ("Use typed permission extractors for route authorization") gets its
  documented exception here via the human-readable dynamic `x-required-permission` extension.

## Error semantics (delta only)

| Case             | Status | Code           |
| ---------------- | ------ | -------------- |
| Old `/read` path | 404    | — (route gone) |

Everything else keeps the existing mapping in `routes/surfaces.rs` (`action_error_code` table).

## Testing

New endpoint tests use the shared `TestApp` harness (`crates/ui/web-api/src/test_harness/`) per the
"New API endpoint tests must use the shared `TestApp` harness" rule.

1. Old `GET /surfaces/{id}/read` → 404; new `GET /surfaces/{id}` returns the read model.
2. `Cache-Control: private, no-store` present on each surface GET response class (read model, providers, list) —
   one assertion per route, not a representative.
3. OpenAPI: `/api/v1/surfaces` paths present in regenerated `openapi.json`; `cargo xtask openapi-client-check`
   green; `bash ci/verify_no_inline_query_params.sh` green.
4. Frontend unit tests updated for the generated-SDK call shapes (registry, read panel, form, interaction
   components).
5. Existing invoke/permission tests keep passing unchanged (no behavior change on POST invoke).

Verification commands and their scope: `cargo test --all-features` (full workspace; requires `frontend/build/` —
build the frontend first), scoped `cargo clippy --all-targets --no-default-features --features db-sqlite`,
`./scripts/regen-api.sh` then a clean-diff check on `crates/ui/web-api/openapi.json` +
`frontend/src/lib/api/generated/`, `cargo xtask openapi-client-check`,
`python3 ci/verify_db_access_policy.py`.

## Deliverables

**Code** — route rename + cache headers (`routes/surfaces.rs`, `router.rs`); utoipa registrations +
`IntoParams`/`ToSchema` derives (`web-api`, `web-api-types`); openapi-client path updates; frontend SDK migration.

**Docs (non-optional):**

- `docs/api/surfaces.md` — endpoint contract update (read-model path, OpenAPI availability, cache headers, authz
  model, idempotency-body decision).
- `docs/development/surfaces.md` — endpoint list update.
- `docs/security/surfaces.md` — authz model for all four operations, cache-control rationale.
- `crates/ui/web-api/db_access_policy.toml` — entries for any renamed/new handler names
  (`[routes."surfaces.rs"]` section exists).
- Regenerated `crates/ui/web-api/openapi.json` + `frontend/src/lib/api/generated/` committed with the originating
  change (repo convention: fold regen artifacts into the crate-scoped commit).
- `CONTEXT.md`: no change — no new domain vocabulary. No new ADR — this spec adopts existing conventions
  (ADR-0025 pipeline, platform envelope); the architectural decisions live in the follow-up spec's ADR.

## Deferred

Everything method/typing-related is the follow-up spec
([`2026-07-16-surfaces-dataload-get-typing-design.md`](2026-07-16-surfaces-dataload-get-typing-design.md)):
DataLoad GET migration, query typing, `ParamFieldDescriptor`, admission rules, CI guard, CLI/frontend GET dispatch.

## Open questions

None.

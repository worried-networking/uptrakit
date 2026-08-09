# 0039 — Replace Enum RBAC With Action String Grants And A Central Access Engine

Date: 2026-08-09

## Status

Accepted

## Context

Until milestone M1 the authorization model was a closed Rust enum (`Permission`, 36 variants at
deletion) mirrored into two database tables (`permissions`, `role_permissions`), embedded into every
JWT as a `permissions` claim, and enforced per-route by a `permission_extractor!` macro that declared
an `x-required-permission` vendor extension in OpenAPI. Preset endpoints
(`GET /api/v1/access-presets`, `POST /users/{id}/apply-preset`) applied fixed permission bundles.

This model had structural failure modes:

- **Stale authority.** Permissions lived in the token, so a grant change took effect only at the
  next refresh; revocation could lag by the full token lifetime.
- **Enum/table drift.** The enum and its mirror tables had to be kept in sync by seed migrations;
  every new permission was a code change plus a data migration plus a frontend-type regeneration.
- **Closed vocabulary.** Plugins, surfaces, and services could not introduce their own gates without
  editing the core enum, which conflicted with the runtime-registered surface model.
- **No resource dimension.** A permission was global per tenant; scoping authority to specific hosts
  or software was unrepresentable.
- **Nonstandard wire contract.** `x-required-permission` was a vendor extension invisible to
  standard OAuth tooling; scopes could not be expressed in the token's own grammar.

## Decision

Replace the enum model with an action-string grant model enforced by a single decision point:

- **Actions** are `resource:verb` strings with a closed verb set and an open, catalog-validated
  resource set. The catalog macro (`crates/shared/types/src/access/catalog.rs`) is the single source
  of truth for resources, verb validity, descriptions, and selector support. Dynamic namespaces
  (`plugin.<type>`, `surface.<id>`) admit runtime-registered actions. There is no `Other` catch-all:
  an unparseable action string is a parse error, and a parse error is a deny.
- **Grants are data.** The `access_grants` table stores pattern grants (`ActionPattern` with
  wildcard support; `system.`-prefixed resources are excluded from `*`) with a selector column
  (restricted to `All` until milestone M2 ships resource selectors). Roles are data too:
  `roles.tenant_id` is nullable (NULL = global built-in), and seed roles are frozen literal grant
  patterns guarded by a catalog-drift test.
- **One decision point.** `AccessEngine` (`crates/ui/controller-core/src/access/`) resolves every
  authorization question with the normative check order dynamic-action registry → grant match →
  token scope → target/selector. It is fail-closed (engine unavailable = HTTP 500, never a silent
  permit), cached with a 60 s TTL backstop, and invalidated by `AccessInvalidated` events, so grant
  changes take effect on the next request.
- **Native security declarations.** Handlers declare requirements via `action_extractor!` types;
  OpenAPI carries native `security(…)` requirements with a catalog-generated scope dictionary
  (`oauth2` scheme) plus a `developer_token` bearer scheme. `x-required-permission` is deleted and
  its absence is CI-enforced (`ci/verify_action_security_declarations.py`).
- **Runtime-valued actions.** Surface descriptors carry `required_action` as a string on the wire,
  parsed once to a typed `Action` at registration admission; an unparseable value rejects the whole
  registration. MCP tools declare per-tool actions the same way.
- **Presets retired.** The preset endpoints and their audit site are deleted; tier application is
  standard role assignment, and the catalog endpoint (`GET /api/v1/access/catalog`) serves role
  bundles and scope presets as introspectable metadata.
- **Tokens carry no authorization data.** The JWT `permissions` claim is removed; `me` returns the
  expanded action list plus an `authority: "ok" | "unavailable"` field.

## Alternatives considered

- **Extend the closed enum.** Rejected: keeps the drift and closed-vocabulary problems; every
  plugin/surface gate would still be a core code change, and the resource dimension would still be
  unrepresentable.
- **Keep JWT-embedded permissions with shorter token lifetimes.** Rejected: shrinks but never
  removes the staleness window, multiplies refresh traffic, and still requires the enum. The engine
  cache bounds staleness at 60 s without touching token lifetime.
- **Per-route inline checks without a central engine.** Rejected: no single place to enforce
  fail-closed semantics, caching, invalidation, or deny-event policy; audit coverage becomes a
  per-handler discipline instead of a structural property.
- **Per-request database resolution without a cache.** Rejected: every request would pay the grant
  query; the bounded cache plus event invalidation gives the same freshness for a fraction of the
  cost, with the 60 s TTL as the lost-event backstop.
- **Keep preset endpoints alongside role assignment.** Rejected: two parallel assignment surfaces
  with diverging audit semantics; role bundles in the catalog carry the same information as data.
  Reopen condition: none — a richer bundling UI would consume the same catalog metadata.

## Consequences

- Authority changes are immediate (next request), bounded only by the 60 s cache backstop after a
  lost invalidation event.
- The vocabulary is open: an unknown action denies instead of failing schema validation, so newer
  controllers can introduce actions without breaking older clients.
- Docs and tooling shift from enum inventories to catalog introspection; the OpenAPI scope
  dictionary and the grant UI/consent screens are catalog-driven.
- Milestone M2 amends this decision additively with selector variants beyond `All` (tag/host/item
  scoping and visibility filtering); milestone M3 makes scopes wire-visible by enforcing
  OAuth-presented token scopes through the same engine path. Both extend this ADR; neither
  supersedes it.
- The one-time migration cost was borne inside milestone M1: every enforcement surface moved to the
  engine, principals re-authenticate, and the `permissions`/`role_permissions` tables were dropped.

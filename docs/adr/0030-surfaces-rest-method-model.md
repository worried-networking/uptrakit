# 0030 — Surfaces REST Method Model

Date: 2026-07-20

## Status

Accepted

## Context

Every shared-surface interaction — read, create, update, delete, form submit, workflow step — was dispatched over a
single `POST /api/v1/surfaces/{surface_id}/interactions/{interaction_id}` route. This collapsed all HTTP method
semantics into one verb: `DataLoad` interactions (pure reads, including form preloads and option lists) were `POST`
requests with no cacheable shape and no query-string addressability; a "create" and a "delete" on the same
interaction id looked identical at the HTTP layer; there was no way to address a specific item in a
collection-shaped interaction without inventing an ad-hoc body field per provider.

`docs/superpowers/specs/2026-07-16-surfaces-dataload-get-typing-design.md` (2026-07-16) first proposed moving
`DataLoad` onto `GET` with typed query parameters. `docs/superpowers/specs/2026-07-20-surfaces-rest-method-model-design.md`
(2026-07-20) supersedes and subsumes that spec, generalizing it into a full REST method model covering every
interaction kind, plus item addressing via a path segment. This ADR records the design's binding decisions (B1–B8)
and the rejected alternatives; see the design spec for the full rationale, the three-tier GET query contract, and
the fifteen-case testing enumeration.

## Decision

### B1 — Method is a function of interaction kind

- `DataLoad` = `GET` only.
- `Workflow` = `POST` only.
- `FormSubmit` / `MutationAction` / `ConfirmableAction` = a declared method (`post` default, or `put`/`delete`).

### B2 — Query typing via reserved keys + opt-in declarations, no inference

GET query parameters resolve in three fixed tiers: envelope keys (`target_provider_id`, `timeout_seconds`) that
never reach the provider; framework-reserved typed keys (`page`, `per_page`, and the item-addressing `id`) coerced
unconditionally; and everything else, parsed per an opt-in `ParamFieldDescriptor` (`key`, `schema`, `required`) if
declared, or passed through as an untyped JSON string otherwise. There is no attempt to guess a query value's JSON
type from its shape — an ambiguous inferred type is a silent-corruption risk, not a convenience.

### B3 — Atomic breaking change, no deprecation window

The old `POST`-only interaction contract and the old `GET /api/v1/surfaces/{surface_id}/read` path (an extra
trailing segment on the surface-read endpoint) are removed outright, in the same change that introduces the new
route family. Every in-repo caller (frontend SDK, CLI, openapi-client, test harness) is cut over atomically. This
mirrors the project's general posture on internal wire/API contracts: a coordinated cutover is preferred to a
compatibility shim that would double the interaction contract's surface area indefinitely.

### B4 — Registration key extends to `(surface_id, interaction_id, http_method)`

ADR-0028 established `(surface_id, interaction_id)` exact-ID dispatch as the single source of truth for
interaction registration. This design extends that key with `http_method`: an interaction id may register under
several methods within the same surface (e.g. a `GET` for read and a `PUT` for update sharing the same conceptual
resource). `CONTROLLER_LOCAL_EXECUTOR_TABLE` and its bidirectional guard test are re-keyed to match. The
**cost** is that every registration/resolution site that assumed a 1:1 `(surface_id, interaction_id)` mapping had
to be re-audited for the extra dimension (registry lookup, admission uniqueness checks, content-node reference
resolution); the **payoff** is that a single conceptual resource — e.g. a settings row — can expose `GET` (read),
`PUT` (replace), and `DELETE` under one memorable interaction id instead of three synthetically distinct ids.

### B5 — Item addressing via optional trailing path segment

A collection-shaped interaction may be targeted at a specific item via `…/interactions/{interaction_id}/{item_id}`
rather than inventing a per-provider `id`-like body/query field. The path segment populates the framework-reserved
`id` params key, overwriting any `id` already supplied via query or body — the path segment is authoritative.
`POST` has no item-path route (create always targets the collection).

### B6 — No PATCH; PUT is full-replace only

Mutating, non-delete interactions declare `put`, never `patch`. Partial-update semantics, if ever needed, are a
provider-level body-shape concern (e.g. "send only the fields you want to change" as a documented convention), not
a distinct HTTP method this contract models.

### B7 — `DirectBuiltInApi` transport variant deleted entirely

ADR-0028 deferred this variant as additive future work with zero plugin users. This design confirms zero users
still exist and deletes the variant outright rather than carrying dead surface area forward into the new method
model.

### B8 — GET carries no idempotency key

`idempotency_key` remains a required body field on `InvokeSurfaceInteractionRequest` for mutating methods
(`POST`/`PUT`/`DELETE`), consistent with the pre-existing decision to keep it a body field rather than an
`Idempotency-Key` header (no header precedent in this API). `GET` requests have no body and no idempotency concept
— reads need no dedup — so the wire-level `SurfaceActionRequest.idempotency_key` (still a required `String` on the
wire) is synthesized by the controller via `Uuid::now_v7()` for GET/`DataLoad` dispatch, exactly as it already does
today when a mutating REST body omits the field.

## Rejected alternatives

| Alternative                                                           | Why rejected                                                                                                                                                               |
| --------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Data-source subresources instead of a method-mapped interaction id    | Cannot cover `DataLoad` interactions with no backing data source (form preloads, ad-hoc option lists) — those have no subresource to hang off of.                          |
| Unique interaction IDs per method, method recorded only on descriptor | Blocks the "one conceptual resource, several methods" noun-collapse this model exists to enable (see B4 payoff).                                                           |
| Item ID via query parameter instead of a path segment                 | Less RESTful than a path segment for identifying a specific resource; loses the ability to route/cache/log by URL shape.                                                   |
| `SchemaContract` gains a nested `Object { fields }` variant           | Breaks deserialization for older peers that don't know the new variant — the flat scalar-schema model was kept for wire compatibility.                                     |
| Generate `params` schemas from `schemars`                             | `schemars` output is a superset of what this contract needs; a lossy subsetting layer would be required regardless, so hand-authored `ParamFieldDescriptor`s were simpler. |
| Infer query-value JSON types from their string shape                  | Silent corruption risk (e.g. a string that looks like a number, a `"true"`/`"false"` string meant literally) — explicitly rejected by B2.                                  |

## Consequences

### Descriptor and wire additions

- `InteractionDescriptor.http_method: InteractionHttpMethod` — wire-safe enum (`Get | Post | Put | Delete |
Other(String)`), wire default `Post` (an older peer's omitted field deserializes identically to an explicit
  `post`).
- `InteractionDescriptor.params: Vec<ParamFieldDescriptor>`, `#[non_exhaustive]` — opt-in per-field declarations
  used for GET query typing and mutating-method body validation.
- `SurfaceActionRequest.method: InteractionHttpMethod` — controller-stamped, provider-bound metadata describing the
  effective method the resolved interaction was registered under; also defaults to `Post` for the same
  older-peer-compatibility reason. It is never read as an inbound resolution key.

### Plan-2 serde resolution: DataLoad + `post` is indistinguishable from omitted

Because the wire default for `http_method` is `post`, a `DataLoad` interaction that explicitly declares `post` is
byte-for-byte indistinguishable, after deserialization, from one that omits the field entirely — both normalize
silently to `get` at admission. Only an explicit `put` or `delete` declaration on a `DataLoad` is observably wrong
and rejected outright. This is an accepted asymmetry: it preserves wire compatibility with providers built before
this field existed (which always omit it) while still catching the methods that are unambiguously incompatible
with a read-only interaction kind.

### Provider-origin (wire-initiated) resolution constraint

Wire-initiated interaction calls (`ServiceMessage::SurfaceActionRequest`, used by provider-origin invocations) do
not carry method-selection intent from the caller — they always resolve via `resolve_surface_action_for_method`
with `method: None`, which succeeds only when the target interaction id has exactly one registered method. An
interaction id registered under multiple methods (a direct consequence of B4) is therefore **not
provider-origin-dispatchable today**: resolution fails with `MethodNotAllowed`, mapped on the wire to
`SurfaceActionErrorCode::InvalidRequest` — the REST-equivalent `invalid_request`, not `method_not_allowed` (that
REST code is reserved for the HTTP-level 405 path, which provider-origin calls never traverse). This is a known,
accepted gap for this change: designing wire-method trust so provider-origin calls can select among
method-siblings is out of scope here and is explicitly deferred to
`docs/superpowers/specs/2026-07-20-surfaces-id-naming-convention-design.md` (the companion "rename spec"), which
owns the broader interaction-id-naming and wire-trust work this gap belongs to.

### The honest trade

This change buys correct HTTP method semantics, a real `405` with a meaningful `Allow` header, and RESTful item
addressing. It deliberately does **not** buy:

- **HTTP caching.** Every surface GET response — success or error — still sets `Cache-Control: private, no-store`.
  Moving `DataLoad` onto `GET` makes the response cacheable in shape, not in policy: results are per-tenant,
  per-permission data that must never be served from a shared cache or bfcache.
- **Per-interaction typed SDK signatures.** Dynamic per-interaction `params` cannot be represented in static
  OpenAPI (`ci/verify_no_inline_query_params.sh`, ADR-0025), so the generated SDK still exposes one generic
  invoke/read shape per method, not a distinct typed function per interaction id.

## Cross-references

- Spec: `docs/superpowers/specs/2026-07-20-surfaces-rest-method-model-design.md`
- Superseded spec: `docs/superpowers/specs/2026-07-16-surfaces-dataload-get-typing-design.md`
- Companion spec (provider-origin wire-method trust, deferred): `docs/superpowers/specs/2026-07-20-surfaces-id-naming-convention-design.md`
- Extends: [ADR-0028 — Single-Source Plugin Interaction Registration](0028-single-source-plugin-interaction-registration.md)
- Route implementation: `crates/ui/web-api/src/routes/surfaces.rs`
- Registry resolution: `crates/ui/surface-proxy/src/registry.rs` (`resolve_surface_action_for_method`)
- Admission validation: `crates/shared/surfaces/src/protocol.rs`
- Param descriptors: `crates/shared/surfaces/src/params.rs`
- Wire mapping for provider-origin calls: `crates/ui/web-api/src/routes/service_ws/handler/surface_wire.rs`
- API documentation: [Shared Surface API](../api/surfaces.md)
- Authoring documentation: [Shared Surface Runtime Development](../development/surfaces.md)
- Security documentation: [Shared Surface Security](../security/surfaces.md)

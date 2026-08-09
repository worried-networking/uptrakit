# Shared Surface API

This document describes the runtime API used for built-in and provider-backed shared surfaces.

## Endpoints

### `GET /api/v1/surfaces`

Lists registered surfaces visible to the authenticated tenant.

Query params:

- `slot` (optional): return only surfaces in this slot
- `page` (optional): page alias filter (`settings`, `software`, `hosts`, `surfaces`)

Response: `SurfaceResponse[]`

```json
[
  {
    "surface_id": "ssh-agent.hosts",
    "label": "SSH Hosts",
    "priority": 500,
    "slot": "surface.page",
    "scope": "tenant",
    "targeting": "targeted",
    "required_action": "hosts:update",
    "provider_kind": "service",
    "required_capabilities": [
      "table_node",
      "targeted_targeting",
      "mutation_action"
    ],
    "root_node": {
      "kind": "text_block",
      "text": "..."
    },
    "provider_count": 1
  }
]
```

### `GET /api/v1/surfaces/{surface_id}/providers`

Lists providers for targeted surfaces.

Response: `SurfaceProviderInfo[]`

- `availability`: `available`, `disconnected`, or `incompatible_tenant`
- `encryption_metadata`: present when provider supports encrypted sensitive params

### `GET /api/v1/surfaces/{surface_id}`

Returns read model for a surface:

- descriptor
- interactions
- data sources

Response: `SurfaceReadResponse`

The previous path, with a trailing `/read` segment, was removed in this change (404; no deprecation window — all
in-repo clients updated atomically).

## Interaction route family (REST method model)

Every interaction is dispatched over one of four HTTP methods, plus an optional trailing item segment:

```text
GET|POST|PUT|DELETE /api/v1/surfaces/{surface_id}/interactions/{interaction_id}
GET|PUT|DELETE      /api/v1/surfaces/{surface_id}/interactions/{interaction_id}/{item_id}
```

Which methods are valid for a given `interaction_id` is runtime registry data (the interaction's declared
`http_method`, see [Method model (contract fields)](#method-model-contract-fields) below) — the route table itself
accepts all four verbs on every path, and the method/kind gate happens after interaction resolution. `HEAD` is
auto-derived from the `GET` registration by Axum but never reaches the provider: the handler inspects the extracted
method and short-circuits after the permission check, returning headers only.

Operation IDs: `read_surface_interaction` (GET), `invoke_surface_interaction` (POST, kept from the pre-method-model
API), `update_surface_interaction` (PUT), `delete_surface_interaction` (DELETE), and the item-path variants
`read_surface_interaction_item`, `update_surface_interaction_item`, `delete_surface_interaction_item`. `POST` has no
item-path counterpart — see [Item segment](#item-segment-and-the-id-reserved-key-contract) below.

Request body (POST/PUT/DELETE): `InvokeSurfaceInteractionRequest`

```json
{
  "params": {
    "host_id": "019585f4-1234-7000-8000-000000000001"
  },
  "target_provider_id": "service:ssh-agent:019585f4-1234-7000-8000-000000000001",
  "idempotency_key": "f286b304-7198-4e1f-9fcf-56f659560b38",
  "timeout_seconds": 30
}
```

`idempotency_key` is deliberately a body field, not an `Idempotency-Key` header (settled decision A3 of the
2026-07-16 spec — no header precedent in this API; server generates one when omitted). GET requests carry no
`idempotency_key` at all — see [B8: GET carries no idempotency key](#b8-get-carries-no-idempotency-key).

Sensitive values are sent via `encrypted_sensitive_params`, not plaintext `params`.

### GET query contract

GET requests have no body; the same envelope/params split is expressed through the query string with a
deterministic three-tier rule (no type inference, ever):

1. **Envelope keys** (never reach the provider as `params`): `target_provider_id: String`,
   `timeout_seconds: u16` — same semantics as the equivalent `InvokeSurfaceInteractionRequest` body fields. An
   empty `target_provider_id` value (`?target_provider_id=`) normalizes to omitted (implicit provider resolution),
   not a lookup for the empty-string provider id. A non-numeric `timeout_seconds` is a 422
   `schema_validation_failed`.
2. **Framework-reserved typed keys** (coerced unconditionally, forwarded inside `params`): `page: u64`,
   `per_page: u64`. An unparsable value is a 422 `schema_validation_failed`.
3. **Everything else:** if the key is declared in the interaction's `params` descriptor list, it is parsed
   strictly per its declared `SchemaContract` (failure is a 422 `schema_validation_failed`); if undeclared, it
   passes through as a JSON string, untyped.

Duplicate query keys: last one wins. Reserved/envelope keys are documented in one static
`#[derive(Deserialize, utoipa::IntoParams)]` struct referenced as `params(<Struct>)` (`ReadSurfaceInteractionQuery`
for the base path, sharing the same envelope/reserved-key shape on the item path), satisfying
`ci/verify_no_inline_query_params.sh` (ADR-0025). Dynamic per-interaction params cannot appear in static OpenAPI —
this is a structural limit, not an omission; per-interaction typed SDK signatures are explicitly not part of this
contract (see [Honest trade](../adr/0030-surfaces-rest-method-model.md) in the ADR).

Query strings appear in access logs and browser history. This is acceptable for DataLoad params because they never
carry secrets — enforced by the admission rule described in [Shared Surface Security](../security/surfaces.md).

### Item segment and the `id` reserved-key contract

The optional trailing `/{item_id}` path segment (B5) targets a specific item within a collection-shaped
interaction. When present, the framework populates the framework-reserved **string** params key `id` from the path
segment. The same key `id` may equally arrive as a GET query parameter (string passthrough — used by form preloads
whose row context spreads `id` into request params today) or a POST body field (item-targeted domain operations
like the notifications `test` interaction). **Providers read `params["id"]` uniformly and never care which route it
took.**

**Path wins over query/body:** when an `id` value arrives from the path segment, it overwrites any `id` already
present from the query string or body — the path segment is authoritative. Whether an interaction actually
_requires_ `id` is the provider's own contract; the framework injects/forwards the value and does not model
item-scoped-ness itself.

`id` joins the framework-reserved/envelope key list for declared `params` — a provider's `params` descriptor list
must not declare a field named `id` (nor `page`, `per_page`, `target_provider_id`, `timeout_seconds`); doing so is
an admission-time rejection.

`POST` accepts no item segment — create targets the collection, never a specific item.

### Resolution order and 405 semantics

Every method-mapped route resolves in the same normative order:

1. **404** — unknown surface ID or unknown interaction ID (`surface_not_found` / `interaction_not_found`).
2. **403/500** — the descriptor's `required_action`, then the interaction's `required_action`, run through
   `AccessEngine`: a deny decision is `403` (`forbidden`); an `Unavailable` authorization authority is `500`
   (fail-closed) and is checked before the `405` sweep completes.
3. **405** — the request's HTTP method does not match any registration for that interaction ID
   (`method_not_allowed`), with an `Allow` header listing every method actually registered for it.

The action check is run **before** the method-mismatch check, deliberately: an unauthorized caller must not be able
to probe an interaction's registered method set (or kind) by comparing 403/500 vs. 405 responses across methods.
This holds even when the interaction ID resolves to multiple method registrations with different required
actions — every candidate registration's action is checked before any 405 is returned.

405 responses use `StatusCode::METHOD_NOT_ALLOWED` with the platform `ErrorResponse` envelope and the new
`method_not_allowed` error code (see [Error codes](#error-codes)).

**Two distinct `Allow` shapes exist for the same URL family, and a consumer may legitimately see both:**

- **The POST item-path stub** (`POST .../{item_id}`) is **route-template-static**: POST never has a valid
  item-addressed registration (create always targets the collection), so this route always returns
  `Allow: GET, PUT, DELETE` regardless of what is actually registered for the interaction ID.
- **Every other 405** (base path or item path, GET/PUT/DELETE mismatches) derives `Allow`
  **dynamically from the methods actually registered** for that interaction ID at the registry — e.g. an
  interaction registered only under `POST` returns `Allow: POST` on a GET/PUT/DELETE attempt; one registered under
  both `GET` and `PUT` returns `Allow: GET, PUT`.

Do not assume the item-path POST stub's `Allow` header reflects live registry state — it does not; it is a fixed
constant describing which methods are legal on an item-addressed URL in general.

## Error codes

Lookup/routing errors:

- `surface_not_found`
- `interaction_not_found`
- `target_provider_required`
- `invalid_provider`
- `no_provider`
- `method_not_allowed` — the interaction ID exists, but not under the requested HTTP method; `Allow` header lists
  the registered methods (see [Resolution order and 405 semantics](#resolution-order-and-405-semantics))

Invocation failures:

- `forbidden`
- `invalid_request`
- `schema_validation_failed`
- `provider_unavailable`
- `duplicate_request`
- `rate_limited`
- `timeout`

## Caching

Every surface GET response — including GET-method interaction reads — carries `Cache-Control: private, no-store`.
Results are per-tenant and per-permission, so shared caches and bfcache must never serve them across users. This
does not change with the method model: moving DataLoads onto `GET` buys correct HTTP semantics and cacheable
_shape_, not actual caching — `no-store` remains the policy (see the ADR's honest-trade note).

## Authorization

`list`/`providers` are authenticated-only, with results filtered by descriptor visibility; `read`/every
method-mapped interaction route enforce the dynamic `required_action` declared by the surface descriptor/interaction
through `AccessEngine`. The wrapper operations carry `x-action-dynamic: true` with an authenticated-only security
declaration in OpenAPI — the runtime-valued requirement itself lives in the registered descriptor/interaction, not
in the spec. See [Shared Surface Security](../security/surfaces.md) for the full model, and [Authentication and
Authorization](../security/auth-and-authorization.md#runtime-valued-actions) for the
extractor-exception class this pattern belongs to.

### Provider-origin (wire-initiated) resolution

The route family above is the REST entry point used by browser/CLI callers. Services can also invoke
controller-side interactions directly over the wire via `ServiceMessage::SurfaceActionRequest` (provider-origin
calls — see [Shared Surface Runtime Development](../development/surfaces.md)). That path resolves
**method-agnostically**: it always calls the registry's method resolution with `method: None`, which succeeds only
when the interaction ID registers under exactly one method; an ID registered under multiple methods (B4) is
rejected with wire error code `invalid_request`, not dispatched to an arbitrary sibling.

`SurfaceActionRequest.method` (see [Descriptor and wire field additions](#descriptor-and-wire-field-additions)
below) is **controller-stamped, provider-bound metadata** — it tells the receiving provider which method the
_resolved_ interaction was ultimately registered under, for providers that want to branch on it. It is never
trusted as an inbound resolution key: an inbound wire request cannot select a sibling registration by setting
`method`, because provider-origin resolution never reads it. Wire-method trust for provider-origin dispatch of
multi-method interaction IDs is out of scope of this contract; it is deferred to the ID-naming-convention spec
(see the [ADR](../adr/0030-surfaces-rest-method-model.md)).

## OpenAPI

All ten endpoints (`list_surfaces`, `get_surface_read`, `list_surface_providers`, and the eight method-mapped
interaction routes — `read`/`invoke`/`update`/`delete_surface_interaction`, each with a base and `_item` path
variant) are registered in `crates/ui/web-api/openapi.json`. Descriptor-bearing fields are documented as free-form
JSON, with the surface contract model in `crates/shared/surfaces/` remaining the canonical shape.

## Types

Canonical type ownership:

- REST request/response envelopes: `crates/shared/web-api-types/src/surfaces.rs`
- Contract model: `crates/shared/surfaces/`
- Wire barrel re-export: `crates/shared/wire/src/surfaces.rs`

## Method model (contract fields)

Each interaction descriptor carries an `http_method` field (`get`, `post`, `put`, or `delete`) declaring the REST
method the interaction is dispatched with. The wire default is `post` — providers registered before this field
existed omit it, and it deserializes the same as an explicit `"post"`. `DataLoad` interactions are the one
exception: regardless of the declared value, they are normalized to `get` at admission (a declared `put`/`delete`
on a `DataLoad` is rejected outright; a declared `post` is silently equivalent to omission and normalizes to
`get`), so they are always stored and served as `get`. Every other interaction kind keeps its declared method, with
`workflow` interactions required to declare `post` and all non-`DataLoad` kinds forbidden from declaring `get`.

Descriptors may also opt in to `params`: a list of per-field declarations (`key`, `schema`, `required`) used for
strict typed parsing of GET query strings (see [GET query contract](#get-query-contract)) and per-field body
validation on mutating methods. Fields not listed in `params` still pass through untyped. Declared keys must not
collide with the framework-reserved envelope keys (`id`, `page`, `per_page`, `target_provider_id`,
`timeout_seconds`).

An interaction ID may register under several `http_method` values within the same surface — uniqueness is keyed
`(surface_id, interaction_id, http_method)`, an extension of ADR-0028's exact-ID dispatch (see
[ADR-0030](../adr/0030-surfaces-rest-method-model.md)). This is what makes the [resolution
order](#resolution-order-and-405-semantics)'s `Allow` header meaningful: a single `interaction_id` can legitimately
answer to more than one method.

### `SurfaceActionRequest.method`

On the wire side, `SurfaceActionRequest` (the controller↔service message that actually dispatches an interaction)
carries its own `method` field, independent of the descriptor's `http_method`. It defaults to `post` the same way:
older peers that predate this field simply omit it, and it parses as `post` on receipt. This wire field is what the
proxy stamps with the interaction's effective (resolved) method before delivery to a provider — it is descriptive
provider-bound metadata, not an inbound routing key (see [Provider-origin (wire-initiated)
resolution](#provider-origin-wire-initiated-resolution)).

### B8: GET carries no idempotency key

GET requests never accept or synthesize a client-visible `idempotency_key` in the REST body — there is no body.
Reads need no dedup, so the concept simply does not apply to the GET route family. The wire `idempotency_key`
field on `SurfaceActionRequest` stays a required `String`: for GET/DataLoad dispatch the controller synthesizes one
internally exactly as it already does when a mutating REST body omits the field.

## Notes

- `/api/v1/surfaces/*` is the active runtime path for shared surfaces.
- Frontend and backend use the same `/api/v1/surfaces/*` API family.

# Surfaces REST Method Model: HTTP-Method-Mapped Interactions

**Date:** 2026-07-20
**Status:** Approved (user interview 2026-07-20; supersedes and absorbs
[`2026-07-16-surfaces-dataload-get-typing-design.md`](2026-07-16-surfaces-dataload-get-typing-design.md) per user
decision — that spec's settled decisions are restated here, not reopened)
**Scope:** `crates/ui/web-api` (routes, OpenAPI), `crates/shared/surfaces`, `crates/shared/web-api-types`,
`crates/ui/surface-proxy`, `crates/shared/wire`, `crates/shared/openapi-client`, `crates/ui/cli`, `frontend/`,
`ci/`, docs.
**Sequencing:** No pending-spec dependencies (the surfaces OpenAPI+SDK spec and the interaction-system unification
spec are both delivered). The companion spec
[`2026-07-20-surfaces-id-naming-convention-design.md`](2026-07-20-surfaces-id-naming-convention-design.md) depends
on this one and must land second. Intent coupling: the pending agent-side surface-task-timeout spec edits
`crates/core/agent-ssh-runtime/src/surface_runtime.rs` too — coordinate landing order to avoid rebase churn.

## Problem

1. **Every interaction is a POST.** All interaction kinds — reads (`InteractionKind::DataLoad`), mutations,
   workflows — go through one generic `POST /api/v1/surfaces/{surface_id}/interactions/{interaction_id}` (handler
   `invoke_surface_interaction`, `crates/ui/web-api/src/routes/surfaces.rs`). Reads are indistinguishable from
   mutations at the HTTP layer; CRUD mutations (create/update/delete) cannot express their semantics through the
   method, so providers encode verbs into interaction IDs instead.
2. **Untyped interaction params.** Params are a free-form `serde_json::Map<String, Value>`; the only
   per-interaction typing is `input_schema: Option<SchemaContract>` (top-level type tag, checked shallowly by
   `schema_matches` in `crates/ui/surface-proxy/src/proxy/validation.rs`). Moving reads to GET forces the
   query-string typing question: query values arrive as strings, and blind coercion silently corrupts reads.
3. **Dead transport variant.** `InteractionTransport::DirectBuiltInApi`
   (`crates/shared/surfaces/src/interaction.rs`) survived the interaction-system unification with no frontend
   dispatcher, no operation allowlist, and an admission rule that forbids every real provider from declaring it.

## Decisions (settled — do not reopen)

| #   | Decision          | Resolution                                                                                                                                              |
| --- | ----------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------- |
| B1  | Method/kind split | `DataLoad`: GET only. `Workflow`: POST only. `FormSubmit`/`MutationAction`/`ConfirmableAction`: declared method — POST (default), PUT, or DELETE.       |
| B2  | Query typing      | Framework-reserved typed keys + opt-in per-field `ParamFieldDescriptor` declarations + string passthrough for undeclared keys. No type inference, ever. |
| B3  | Compatibility     | Atomic breaking change for in-repo consumers (frontend, CLI, `openapi-client` update in the same change). No deprecation window.                        |
| B4  | Uniqueness key    | Interactions are keyed `(surface_id, interaction_id, http_method)` — same ID may register under several methods. Extends ADR-0028 exact-ID dispatch.    |
| B5  | Item addressing   | Optional trailing path segment `/{item_id}`, injected into provider `params` as reserved key `id`. Plural-collection + `/{id}` REST convention.         |
| B6  | No PATCH          | Surface form saves submit full state — PUT (full replacement) only. PATCH is additive later via the wire-safe method enum if ever needed.               |
| B7  | DirectBuiltInApi  | Deleted (variant, `BuiltInApiOperationId`, validation arm, frontend type, wire/registry/proxy match arms). No production provider can declare it.       |
| B8  | Idempotency       | `idempotency_key` stays a body field on mutating methods (settled 2026-07-16). GET carries none — reads need no dedup.                                  |

**Alternatives rejected** (carried from the absorbed spec, plus new):

- **Data-source subresources** (`GET /surfaces/{id}/data/{source_id}`) — cannot cover form preload/option-load
  DataLoads that have no data source (`SurfaceForm.svelte` invokes DataLoad-kind interactions directly).
- **Unique interaction IDs with method declared on the descriptor only** — nouns cannot collapse; forces
  singular/plural hacks (`client` vs `clients`) or residual verb prefixes, defeating the REST shape. Rejected in
  favor of B4.
- **Item ID via query param** — less RESTful than the path segment; rejected in favor of B5.
- **Extending `SchemaContract` with a nested `Object { fields }` variant** — `#[serde(tag = "type")]` with no
  catch-all hard-fails deserialization of the whole registration payload on older peers; additive struct field is
  strictly safer.
- **schemars-generated schemas** — strictly richer than `SchemaContract`, forcing a lossy hand-maintained
  subsetting layer; repo OpenAPI is utoipa end-to-end.
- **JSON-guessing query values** (`"2"` → number) — silent wrong-read corruption.

## Design

### 1. Route family and method/kind gate

```text
GET|POST|PUT|DELETE /api/v1/surfaces/{surface_id}/interactions/{interaction_id}
GET|PUT|DELETE      /api/v1/surfaces/{surface_id}/interactions/{interaction_id}/{item_id}
```

- All method variants route to the same resolution logic; the method/kind/declared-method gate happens after
  interaction resolution (kind and declared method are runtime registry data — a static route split is
  impossible).
- **Resolution order (normative):** unknown surface or interaction ID → 404; then permission checks (descriptor,
  then interaction) → 403; then method mismatch → 405 with an `Allow` header listing every method registered for
  that interaction ID. Permission before 405 so an unauthorized caller cannot probe an interaction's kind or
  method set. 405 uses `StatusCode::METHOD_NOT_ALLOWED` with the platform `ErrorResponse` envelope.
- **Item segment (B5):** when `{item_id}` is present it is injected into the provider-bound `params` map as
  reserved key `id` (JSON string; the provider parses, exactly as row-context params work today). `POST` does not
  accept an item segment (create targets the collection). Whether an interaction _requires_ the item segment is
  the provider's contract with itself — the framework injects when present and does not model item-scoped-ness.
- **HEAD:** must not invoke the provider. Axum derives HEAD from GET handlers; the GET handler short-circuits
  HEAD after the permission check (headers only, no provider dispatch).
- **Caching:** GET invoke responses set `Cache-Control: private, no-store` (established convention for all
  surface GETs).

### 2. Descriptor and wire changes

- New method enum in `crates/shared/surfaces` via the repo's `wire_safe_enum!` convention:
  `InteractionHttpMethod { Get, Post, Put, Delete }` (serde lowercase, catch-all `Other(String)` per the
  wire-safe pattern; `Other` rejected at admission).
- `InteractionDescriptor` gains `http_method: InteractionHttpMethod` with `#[serde(default)]` = `Post` — additive;
  older peers drop the unknown field (no `deny_unknown_fields` anywhere in `crates/shared/surfaces` or
  `crates/shared/wire`; same additive pattern as `sensitive_fields`).
- `SurfaceActionRequest` (`crates/shared/surfaces/src/protocol.rs`) gains `method: InteractionHttpMethod`,
  `#[serde(default)]` = `Post` — additive. Item IDs ride inside `params`; no other wire change.
- Registry (`crates/ui/surface-proxy/src/registry.rs`): interaction storage/dispatch keyed
  `(surface_id, interaction_id, http_method)`; admission rejects a duplicate triple. Kind/method matrix (B1)
  enforced at the single admission choke point `validate_registration_admission_locked` (shared by
  `register_service` and `bootstrap_plugin`/`bootstrap_builtin` — the plan must verify this call linkage by
  grep). A `DataLoad` declaring a non-GET method (or any interaction declaring `Other`) is rejected; a `DataLoad`
  omitting the field is normalized to `Get`.
- **Compatibility of the default:** an old service binary registers descriptors without `http_method`. Non-DataLoad
  kinds default to POST (today's behavior); DataLoad kinds normalize to GET, and the frontend — which is
  data-driven and ships with the controller — dispatches GET. The wire request the old provider receives is
  unchanged in shape (`method` field dropped by its serde), and its dispatch matches on `interaction_id` alone,
  so old service binaries keep working without modification.
- `WireValidate` (`crates/shared/wire/src/wire_validate_impls.rs`): existing surface bounds extended to the new
  fields (method is a bounded enum; no new limits needed beyond the `params` bounds in §4).

### 3. GET query contract (carried from the absorbed spec)

Deterministic three-tier rule — **no type inference**:

1. **Envelope keys** (never reach the provider as params): `target_provider_id: String`,
   `timeout_seconds: u16` — same semantics as the existing `InvokeSurfaceInteractionRequest` body fields
   (`crates/shared/web-api-types/src/surfaces.rs`).
2. **Framework-reserved typed keys** (coerced unconditionally, forwarded inside `params`): `page: u64`,
   `per_page: u64`. Grep basis at spec time: the only numeric params any DataLoad handler reads
   (`handle_list_hosts` in `crates/core/agent-ssh-runtime/src/surface_runtime.rs`, proxmox `handle_list` in
   `crates/plugins/infrastructure/proxmox/src/surfaces.rs`, shared
   `crates/plugins/notifications/core/src/list_channels.rs`). Unparsable value → 422
   `schema_validation_failed`. **The plan must re-verify this inventory with a fresh untruncated grep.**
3. **Everything else:** if the key is declared in the interaction's `params` descriptor list, parse strictly per
   its declared `SchemaContract` (failure → 422); if undeclared, pass through as a JSON string. String
   passthrough is safe for the current DataLoad population (every non-reserved param is a `Uuid` — which serde
   deserializes from strings — or a string; re-verify at plan time).

Reserved/envelope keys are documented in one static `#[derive(Deserialize, utoipa::IntoParams)]` struct
referenced as `params(<Struct>)`, satisfying `ci/verify_no_inline_query_params.sh` (ADR-0025). Dynamic
per-interaction params cannot appear in static OpenAPI (structural limit, documented in the operation
description). **Explicitly not deliverable:** per-interaction typed SDK signatures.

Query strings appear in access logs and browser history — acceptable for DataLoad params (never secrets, enforced
by the admission rule in §4); stated in `docs/security/surfaces.md`.

### 4. Per-field param declarations (opt-in, carried from the absorbed spec)

New `crates/shared/surfaces/src/params.rs`:

```rust
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParamFieldDescriptor {
    pub key: String,
    pub schema: SchemaContract, // reuse the existing enum
    #[serde(default)]
    pub required: bool,
}
```

New additive field on `InteractionDescriptor`:
`#[serde(default, skip_serializing_if = "Vec::is_empty")] pub params: Vec<ParamFieldDescriptor>`.

Consumers: GET coercion (§3 tier 3); POST/PUT/DELETE body validation (`validate_input_schema` walks `params`
per-field after `schema_matches`; undeclared body keys still pass); CLI dynamic clap tree builds typed args from
declarations when `form_ui` is `None`; frontend pre-dispatch validation is a non-deliverable enhancement.

**Migration cost: zero declarations required** — every shipped DataLoad keeps working with reserved-key coercion +
string passthrough.

Admission rules (all enforced at the §2 choke point):

1. `params[].key` must not collide with a reserved/envelope key — `page`, `per_page`, `target_provider_id`,
   `timeout_seconds`, and (new, B5) `id`.
2. On `DataLoad`, `params[].schema` must be scalar (`String`/`Integer`/`Number`/`Boolean`).
3. `DataLoad` must not declare `sensitive_fields` (closes the "secret in a GET query string" class).

Wire bounds: `params.len()` bounded by the existing `MAX_SURFACE_FIELDS`, each `key` by `MAX_SHORT_STRING_LEN`
(`crates/shared/wire/src/limits.rs`).

Advisory telemetry: a `DataLoad` admitted with empty `params` emits a structured `tracing::warn!` (provider id +
interaction id as fields, message matching the target file's sibling message shape) and increments
`surface_registration_dataload_missing_params_total`. Non-rejecting.

CI guard for in-repo providers: new `ci/verify_dataload_declares_params.sh` + allowlist file, modeled on
`ci/verify_typed_audit_actions.sh`; genuinely param-less DataLoads go on the allowlist.

### 5. DirectBuiltInApi deletion (B7)

Remove `InteractionTransport::DirectBuiltInApi`, `BuiltInApiOperationId` (`crates/shared/surfaces/src/ids.rs`),
`InteractionValidationError::DirectBuiltInApiForbiddenForProvider` + its validation arm
(`crates/shared/surfaces/src/interaction.rs`), the match arms in `crates/ui/surface-proxy/src/proxy.rs` and
`crates/ui/surface-proxy/src/registry.rs`, the wire-validate arm in
`crates/shared/wire/src/wire_validate_impls.rs`, and the frontend union member in
`frontend/src/lib/surfaces/contract.ts`. Wire-safe: admission already rejects the variant for every non-builtin
provider and no builtin production surfaces exist (`bootstrap_builtin` production callers: none — test fixtures
only; re-verify by grep at plan time). The plan must sweep `#[cfg(test)]` code workspace-wide for references
before deletion (deletions break tests at compile, not runtime).

### 6. Consumer migration (atomic, B3)

- **Frontend:** `SurfaceTable.svelte` data loads and `SurfaceForm.svelte`/`SurfaceReadPanel.svelte`
  preload/option-load switch to GET; form submit and row actions dispatch by the interaction's declared method
  (PUT/DELETE row actions carry the row `id` into the item path segment). `SurfaceWorkflow.svelte` steps stay
  POST. A thin hand-written helper appends dynamic DataLoad query params to the generated GET call; everything
  else stays on the generated SDK.
- **`crates/shared/openapi-client`:** new GET/PUT/DELETE methods (with and without item segment) alongside the
  POST; path constants updated. Gated by `cargo xtask openapi-client-check`.
- **CLI:** `dynamic` dispatch (`crates/ui/cli/src/commands/surfaces.rs`) selects the method from the descriptor.
- **e2e:** Playwright mock route-matcher regexes in `frontend/tests/e2e/` swept for the interactions path (they
  key on paths, not the SDK — a method/path change silently falls through to catch-all mocks otherwise).
- **Audit:** invoke-path audit emissions gain the method as a field; if handler registration moves between
  `router.rs` and `routes!` forms, re-key `audit-catalog.toml` accordingly and run
  `cargo xtask audit-coverage-check` as a task gate (not only at pre-push).
- `crates/ui/web-api/db_access_policy.toml`: entries for new/renamed handlers.

## Error semantics (delta)

| Case                                         | Status                                                      | Code                            |
| -------------------------------------------- | ----------------------------------------------------------- | ------------------------------- |
| Method not registered for the interaction ID | 405 + `Allow`                                               | `method_not_allowed` (new code) |
| POST with an item segment                    | 405 (router-level: item path registers GET/PUT/DELETE only) | —                               |
| Reserved or declared GET param fails parse   | 422                                                         | `schema_validation_failed`      |
| Duplicate `(id, method)` at registration     | admission rejection (named code, existing rejection enum)   | —                               |

Everything else keeps the existing mapping in `routes/surfaces.rs`.

## Testing

New endpoint tests use the shared `TestApp` harness (`crates/ui/web-api/src/test_harness/`). Enumerated members —
one test each, not a representative (the harness needs a registered stub provider; verify harness wiring against
production boot — executor, registry bootstrap, migrations — before relying on it):

1. GET DataLoad happy path: query params land in provider `params` (reserved keys as numbers, undeclared as
   strings).
2. POST on a DataLoad → 405 with `Allow: GET`; GET on a POST-registered mutation → 405 with `Allow: POST`.
3. Same ID registered under two methods dispatches to distinct handlers (B4 disambiguation, both directions).
4. Item segment: `PUT .../{item_id}` delivers `params["id"]` = the segment string; GET without segment omits it.
5. Unknown interaction ID → 404 (not 405) on every method.
6. Caller lacking the interaction permission → 403 on GET **before** any kind/method disclosure.
7. Declared param strict-parse failure → 422; success coerces to the declared type; `page=abc` → 422; `page=2` →
   provider sees `Number(2)`.
8. HEAD on a DataLoad does not reach the provider (recording stub).
9. `Cache-Control: private, no-store` on GET responses.
10. Admission rejections, each its own unit test + one registration-path test through the choke point:
    reserved-key collision (including new `id`); `Array` schema on DataLoad; `sensitive_fields` on DataLoad;
    non-GET method on DataLoad; `Other` method; duplicate `(id, method)` triple.
11. Wire: `params` vec over `MAX_SURFACE_FIELDS` rejected; `SurfaceActionRequest` without `method` deserializes
    to `Post` (old-peer shape); descriptor without `http_method` normalizes per kind.
12. POST per-field validation: `required` declared field absent → 422; undeclared body key still passes.
13. Advisory path: DataLoad admitted with empty `params` registers and warns/increments (assert non-rejection at
    minimum).
14. At least one invocation test drives the production caller shape `target_provider_id: None` (implicit provider
    resolution), not only the explicit-provider shape.
15. Frontend: `SurfaceTable`/`SurfaceForm`/`SurfaceReadPanel` unit tests for method dispatch; CLI test for
    DataLoad → GET and declared-method dispatch.

Verification commands and their scope: `cargo test --all-features` (full workspace; requires `frontend/build/`),
`cargo clippy --all-targets --no-default-features --features db-sqlite` (canonical feature set — not an
abbreviated `-p` list), `./scripts/regen-api.sh` + clean-diff check on `crates/ui/web-api/openapi.json` and
`frontend/src/lib/api/generated/`, `cargo xtask openapi-client-check`, `cargo xtask audit-coverage-check`,
`bash ci/verify_no_inline_query_params.sh`, `python3 ci/verify_db_access_policy.py`, new
`bash ci/verify_dataload_declares_params.sh`.

## Deliverables

**Code:** route family + gates + resolution order + HEAD short-circuit + cache header (`routes/surfaces.rs`,
`router.rs`); `InteractionHttpMethod` + `http_method` + `ParamFieldDescriptor` + `params` + admission rules
(`crates/shared/surfaces`, `crates/ui/surface-proxy`); `(id, method)` registry keying; `SurfaceActionRequest.method`;
GET coercion layer; body per-field validation; wire bounds; reserved-keys `IntoParams` struct + OpenAPI
registration; openapi-client methods; CLI dispatch; frontend migration; e2e mock sweep; advisory warn+metric; CI
guard script + allowlist; DirectBuiltInApi deletion.

**Docs (non-optional):**

- `docs/api/surfaces.md` — method model, item segment, query tiers, resolution order, 405 semantics, params
  declarations.
- `docs/development/surfaces.md` — provider authoring guidance (declare `http_method` + `params`; DataLoad params
  are strings unless declared/reserved).
- `docs/security/surfaces.md` — GET query-string exposure statement; DataLoad `sensitive_fields` admission rule.
- New ADR `docs/adr/0030-surfaces-rest-method-model.md` (0029 is claimed by the asyncapi-codegen spec; re-verify
  next-free number at implementation time). Records B1–B8 + rejected alternatives; notes the ADR-0028 dispatch-key
  extension.
- Mark `docs/superpowers/specs/2026-07-16-surfaces-dataload-get-typing-design.md` Status: Superseded (done in the
  same change that registers this spec).
- Regenerated `crates/ui/web-api/openapi.json` + `frontend/src/lib/api/generated/` committed with the originating
  change.
- `CONTEXT.md`: no change — no new domain vocabulary.
- `crates/shared/wire/asyncapi.yaml`: surface payloads are not modeled there today; modeling them is owned by the
  asyncapi-codegen spec, not this one.

## Deferred (named follow-ups, out of scope)

1. **ID naming convention + rename sweep** — the companion spec
   [`2026-07-20-surfaces-id-naming-convention-design.md`](2026-07-20-surfaces-id-naming-convention-design.md).
2. **Phase 2 `#[derive(SurfaceParams)]`** proc-macro generating `declared_params()` from the same structs
   `parse_action_params::<T>()` deserializes (carried from the absorbed spec).
3. **Typed sort/filter reserved keys** driven by `DataSourceSorting`/`DataSourceFiltering` (carried).
4. **Pagination clamp unification** with `DataSourceDescriptor.pagination` (carried).
5. **Hard-block policy** for remote DataLoads registering without `params` declarations (carried; product
   decision, needs a deprecation window).
6. **PATCH support** (B6) — additive via the method enum if a partial-update use case appears.

## Open questions

None.

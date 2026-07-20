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

| #   | Decision          | Resolution                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| --- | ----------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| B1  | Method/kind split | `DataLoad`: GET only. `Workflow`: POST only. `FormSubmit`/`MutationAction`/`ConfirmableAction`: declared method — POST (default), PUT, or DELETE.                                                                                                                                                                                                                                                                                                                                          |
| B2  | Query typing      | Framework-reserved typed keys + opt-in per-field `ParamFieldDescriptor` declarations + string passthrough for undeclared keys. No type inference, ever.                                                                                                                                                                                                                                                                                                                                    |
| B3  | Compatibility     | Atomic breaking change for in-repo consumers (frontend, CLI, `openapi-client` update in the same change). No deprecation window.                                                                                                                                                                                                                                                                                                                                                           |
| B4  | Uniqueness key    | Interactions are keyed `(surface_id, interaction_id, http_method)` — same ID may register under several methods. Extends ADR-0028 exact-ID dispatch. Cost acknowledged: reference-node disambiguation, frontend lookup re-keying, and the new-service→old-controller registration rejection (see §2); the noun-collapse payoff lands in the companion rename spec. User-affirmed 2026-07-20 with these costs on the table; the ADR records the trade vs the cheaper unique-ID alternative. |
| B5  | Item addressing   | Optional trailing path segment `/{item_id}`, injected into provider `params` as reserved key `id`. Plural-collection + `/{id}` REST convention.                                                                                                                                                                                                                                                                                                                                            |
| B6  | No PATCH          | Surface form saves submit full state — PUT (full replacement) only. PATCH is additive later via the wire-safe method enum if ever needed.                                                                                                                                                                                                                                                                                                                                                  |
| B7  | DirectBuiltInApi  | Deleted (variant, `BuiltInApiOperationId`, validation arm, frontend type, wire/registry/proxy match arms). No production provider can declare it.                                                                                                                                                                                                                                                                                                                                          |
| B8  | Idempotency       | `idempotency_key` stays a body field on mutating methods (settled 2026-07-16). GET carries none — reads need no dedup.                                                                                                                                                                                                                                                                                                                                                                     |

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
- **OpenAPI shape:** the repo's `.routes(routes!(...))` + single-method `#[utoipa::path]` convention means one
  annotated wrapper handler per (method × path variant), each delegating to the shared resolution fn. Operation
  IDs: `invoke_surface_interaction` (POST, kept), `read_surface_interaction` (GET),
  `update_surface_interaction` (PUT), `delete_surface_interaction` (DELETE), and item-path variants
  `read_surface_interaction_item` / `update_surface_interaction_item` / `delete_surface_interaction_item`. All
  registered via `routes!` before `split_for_parts()` (unregistered handlers are silently absent from
  openapi.json).
- **Permission convention:** these handlers enforce runtime descriptor/interaction permissions in the body via
  the existing `enforce_required_permission()` helper (the pattern of the two shipped surfaces handlers) —
  the permission value is runtime registry data, which a static `permission_extractor!` cannot express. Each
  wrapper carries the existing extension literal
  `x-required-permission: "dynamic: declared by the surface descriptor / interaction"` (exact string from
  `routes/surfaces.rs`). This is NOT the documented `// APPROVED: custom auth path` exception (that one covers
  custom token extraction, e.g. WebSocket handlers) — it is a second, currently-undocumented exception class
  (runtime-valued permissions); recording it in `docs/security/auth-and-authorization.md` is a doc deliverable
  of this spec.
- **Resolution order (normative):** unknown surface or interaction ID → 404; then permission checks (descriptor,
  then interaction) → 403; then method mismatch → 405 with an `Allow` header listing every method registered for
  that interaction ID. Permission before 405 so an unauthorized caller cannot probe an interaction's kind or
  method set. 405 uses `StatusCode::METHOD_NOT_ALLOWED` with the platform `ErrorResponse` envelope.
- **Item segment (B5):** when `{item_id}` is present it is injected into the provider-bound `params` map as
  reserved key `id` (JSON string; the provider parses, exactly as row-context params work today). `POST` does not
  accept an item segment (create targets the collection). Whether an interaction _requires_ the item segment is
  the provider's contract with itself — the framework injects when present and does not model item-scoped-ness.
- **HEAD:** must not invoke the provider. Axum's auto-derived HEAD runs the full GET handler and only strips the
  response body — it skips nothing by itself. The GET handler therefore branches on the extracted `Method`
  explicitly and short-circuits HEAD after the permission check (headers only, no provider dispatch).
- **Uniform 405 envelope:** the item path also registers an explicit POST wrapper returning the platform
  `ErrorResponse`-enveloped 405 (`Allow: GET, PUT, DELETE`) so clients never see Axum's bare router-level 405
  alongside the enveloped one. The plan prototypes the shared-path `routes!` merge early (sibling precedent:
  `get_host`/`update_host`/`deactivate_host` share one path via separate `.routes()` calls in `router.rs`).
- **Caching:** GET invoke responses set `Cache-Control: private, no-store` (established convention for all
  surface GETs). **Stated honestly:** this spec buys method semantics, `405`/`Allow` discipline, and item
  addressing — not HTTP caching (`no-store` stays) and not per-interaction typed SDK signatures (§3). The ADR
  records this trade explicitly.

### 2. Descriptor and wire changes

- New method enum in `crates/shared/surfaces` via the repo's `wire_safe_enum!` convention:
  `InteractionHttpMethod { Get, Post, Put, Delete }` (serde lowercase, catch-all `Other(String)` per the
  wire-safe pattern; `Other` rejected at admission). `wire_safe_enum!` emits no `Default` impl
  (`crates/shared/macros/src/lib.rs`), so add a manual `impl Default for InteractionHttpMethod` returning `Post`
  — plain `#[serde(default)]` requires it.
- `InteractionDescriptor` gains `http_method: InteractionHttpMethod` with `#[serde(default)]` = `Post` — additive;
  older peers drop the unknown field (no `deny_unknown_fields` anywhere in `crates/shared/surfaces` or
  `crates/shared/wire`; same additive pattern as `sensitive_fields`).
- `SurfaceActionRequest` (`crates/shared/surfaces/src/protocol.rs`) gains `method: InteractionHttpMethod`,
  `#[serde(default)]` = `Post` — additive. Item IDs ride inside `params`; no other wire change. The wire
  `idempotency_key` stays a required `String`: for GET/DataLoad requests the controller synthesizes it exactly as
  it does today when the REST body omits the key (the REST GET simply never accepts one, per B8).
- **Reference-node disambiguation (B4 corollary):** content nodes reference interactions by bare `InteractionId`
  today (`SurfaceNode::Form.interaction_id`, `ActionBar.action_ids`, `SurfaceTableRowAction.interaction_id`,
  `ModalTrigger`/`WorkflowTrigger` — `crates/shared/surfaces/src/surface.rs`), and
  `validate_unique_interaction_id` (`crates/shared/surfaces/src/protocol.rs`) currently makes bare-ID lookup
  unambiguous. Relaxing uniqueness to `(id, method)` requires:
  - (a) Reference nodes whose target kind has a method choice (`SurfaceNode::Form`, `SurfaceTableRowAction`,
    `ModalTrigger`, `ActionBar` entries) gain method disambiguation. `WorkflowTrigger` needs none
    (`Workflow` is POST-fixed by B1). Scalar fields gain an additive `http_method: Option<InteractionHttpMethod>`
    — **no `Post` default on references**: an omitted method resolves only when the target ID registers exactly
    one method (resolve to it); a bare reference to a multi-method ID is rejected at admission. A `Post` default
    would make a delete-intent row action silently resolve to a registered `(id, Post)` create — a
    wrong-but-registered pair no validator could catch. `ActionBar.action_ids: Vec<InteractionId>` becomes
    `Vec<ActionRef>` with a tolerant `#[serde(untagged)]` reader accepting the legacy bare-string form (→ method
    omitted, same resolution rule) and an object form `{ interaction_id, http_method }`. Legacy registrations
    stay valid: old providers never register multi-method IDs, so their bare references always single-resolve.
    Acknowledged: untagged defers (not escapes) the forward-compat cliff — a third `ActionRef` form later would
    hard-fail two-form readers, and untagged mismatch errors are opaque; acceptable for a two-form reader.
  - (b) DataLoad-shaped references (`pre_load_interaction_id`, form option-loads) resolve `(id, Get)`
    implicitly — no field. `DataSourceKind::ProviderQuery.operation_id` is a plain `String` today with **no
    reference validation at all** (`crates/shared/surfaces/src/data.rs`; `protocol.rs` never checks it) — this
    spec adds its validation as a new admission rule: it must resolve to a registered `(id, Get)` interaction.
  - (c) `validate_unique_interaction_id` extends to the `(id, method)` pair; `ensure_known_reference` resolves
    by the pair — rejecting a reference whose explicit pair is unregistered, and rejecting a method-omitted
    reference to an ID with multiple registered methods (per (a)).
  - (d) Frontend descriptor lookups key on the pair too: `contract.ts` gains `http_method` on
    `InteractionDescriptor` and the reference nodes; every component that resolves descriptors by bare
    `interaction_id` today changes keying — `SurfaceRenderer`, `SurfaceActionBar`, `SurfaceTable`,
    `SurfaceForm`, `SurfaceInteractionButton`, `SurfaceReadPanel`, `SurfaceWorkflow` (all under
    `frontend/src/lib/components/surfaces/`; `.find()`/`Map` on bare ID silently mis-resolves under duplicate
    IDs otherwise).
- Registry (`crates/ui/surface-proxy/src/registry.rs`): interaction storage/dispatch keyed
  `(surface_id, interaction_id, http_method)`; admission rejects a duplicate triple. Kind/method matrix (B1)
  enforced at the single admission choke point `validate_registration_admission_locked` (shared by
  `register_service` and `bootstrap_plugin`/`bootstrap_builtin` — the plan must verify this call linkage by
  grep). A `DataLoad` declaring a non-GET method (or any interaction declaring `Other`) is rejected; a `DataLoad`
  omitting the field is normalized to `Get`.
- **Compatibility, old service → new controller:** an old service binary registers descriptors without
  `http_method`. Non-DataLoad kinds default to POST (today's behavior); DataLoad kinds normalize to GET, and the
  frontend — which is data-driven and ships with the controller — dispatches GET. The wire request the old
  provider receives is unchanged in shape (`method` field dropped by its serde), and its dispatch matches on
  `interaction_id` alone, so old service binaries keep working without modification.
- **Compatibility, new service → OLD controller (stated explicitly — B3's "atomic" covers in-repo _source_, not
  deployment lockstep; service binaries version independently):** two registration shapes hard-fail on an old
  controller: (a) a surface registering one ID under several methods trips the old id-only
  `validate_unique_interaction_id` → the whole surface registration is rejected; (b) the `ActionRef` _object_
  form deserializes as "expected string" in the old controller → whole registration payload rejected. Both are
  loud (registration failure), not silent. Consequence: **controller upgrades first**; service binaries may only
  ship multi-method/collapsed registrations in the companion rename spec's change, whose release notes must
  state the minimum controller version. Accepted per the no-alias skew decision — do not re-justify with a
  lockstep claim.
- **REST-consumer completeness (verified):** the Telegram webhook callback dispatches directly into
  `NotificationTransport::handle_callback` (not the interactions route), provider-origin invocations are
  wire-only (`SurfaceActionRequest` via the service WS, never REST), and `crates/ui/mcp` has zero surface
  references — frontend, CLI, and `openapi-client` are the complete REST consumer set.
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
3. `DataLoad` must not declare `sensitive_fields` (closes the "secret in a GET query string" class). This rule
   reaches provider runtimes outside this spec's code scope: `agent-ssh-runtime` attaches `sensitive_fields` via
   `sensitive_fields_for_action` (empty for list-style DataLoads today — a data-dependent pass). The plan must
   grep every in-repo DataLoad in `agent-ssh-runtime`/`mqtt-runtime` for non-empty `sensitive_fields`, and
   `docs/security/surfaces.md` must state the failure mode for older/out-of-repo providers: registration
   rejected at admission, surface absent (no runtime error).

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
  POST. Dynamic DataLoad query params ride a thin hand-written wrapper that funnels through the generated SDK
  operation — the generated per-operation `query` type is closed (only the reserved keys; no index signature),
  so the wrapper locally widens the query object with one explicit, commented type assertion; never a direct
  `fetch`. The wrapper is documented in `frontend/AGENTS.md` as the sanctioned escape hatch, replacing that
  file's stale `surfaces.ts` carve-out text. Everything else stays on the generated SDK.
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

| Case                                         | Status                                                             | Code                            |
| -------------------------------------------- | ------------------------------------------------------------------ | ------------------------------- |
| Method not registered for the interaction ID | 405 + `Allow`                                                      | `method_not_allowed` (new code) |
| POST with an item segment                    | 405 via the explicit enveloped wrapper (`Allow: GET, PUT, DELETE`) | `method_not_allowed`            |
| Reserved or declared GET param fails parse   | 422                                                                | `schema_validation_failed`      |
| Duplicate `(id, method)` at registration     | admission rejection (named code, existing rejection enum)          | —                               |

Everything else keeps the existing mapping in `routes/surfaces.rs`.

## Testing

**Harness prerequisite (named deliverable, not an assumption):** the shared `TestApp` harness cannot currently
exercise any of this spec's router-level guarantees — `TestApp::new()` installs a Noop local executor and an
empty `SurfaceRegistry`, existing invoke tests call handler functions directly (never the router, so 405/`Allow`,
HEAD, and `Cache-Control` are unobservable), and the only real-provider path is proxmox-backed with tests that
early-return when proxmox surfaces are absent — which is exactly the `--all-features` world (agent-infra
unification empties them), i.e. green-by-skipping under the mandated gate. This spec therefore delivers a
synthetic in-process stub-provider registration path for the harness: feature-independent, registers
configurable `(id, method)` interactions with a recording executor, driven through `TestClient` over the real
router. Tests below must not gate on proxmox presence.

Enumerated members — one test each, not a representative:

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
    non-GET method on DataLoad; `Other` method; duplicate `(id, method)` triple; content-node reference to an
    `(id, method)` pair that is not registered (extended `ensure_known_reference`); duplicate-pair rejection via
    the extended `validate_unique_interaction_id`.
11. Wire: `params` vec over `MAX_SURFACE_FIELDS` rejected; `SurfaceActionRequest` without `method` deserializes
    to `Post` (old-peer shape); descriptor without `http_method` normalizes per kind.
12. POST per-field validation: `required` declared field absent → 422; undeclared body key still passes.
13. Advisory path: DataLoad admitted with empty `params` registers and warns/increments (assert non-rejection at
    minimum).
14. At least one invocation test drives the production caller shape `target_provider_id: None` (implicit provider
    resolution), not only the explicit-provider shape.
15. Frontend: `SurfaceTable`/`SurfaceForm`/`SurfaceReadPanel` unit tests for method dispatch; a
    descriptor-lookup disambiguation test with two same-ID interactions (different methods) proving each
    component resolves the right descriptor (the leaked-value assertion, not a count); CLI test for
    DataLoad → GET and declared-method dispatch.

Verification commands and their scope: both canonical feature-set halves — `cargo check` + `cargo clippy
--all-targets` under `--no-default-features --features db-sqlite` **and** `--all-features` (never an abbreviated
`-p` list; every `--all-features` invocation — check, clippy, test — requires `frontend/build/` first);
`cargo test --all-features` (full workspace); `./scripts/regen-api.sh` +
clean-diff check on `crates/ui/web-api/openapi.json` and `frontend/src/lib/api/generated/`;
`cargo xtask openapi-client-check`; `cargo xtask audit-coverage-check`;
`bash ci/verify_no_inline_query_params.sh`; `python3 ci/verify_db_access_policy.py`; new
`bash ci/verify_dataload_declares_params.sh`. The remaining canonical gates (`cargo fmt --all`,
`cargo deny check`, markdownlint) apply as always per `docs/development/quality-gates.md`.

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
- `frontend/AGENTS.md` — replace the stale `surfaces.ts` carve-out ("non-spec surface endpoints, 0 utoipa
  paths"; file retired, all surface endpoints utoipa-registered) with the DataLoad query-wrapper escape-hatch
  description (owned here, not by the companion rename spec).
- `docs/security/auth-and-authorization.md` — record the runtime-valued-permission exception class for the
  surfaces handlers (distinct from the `// APPROVED: custom auth path` token-extraction exception).
- Wire-payload docs: `crates/shared/wire/asyncapi.yaml` carries no surface payloads (re-verified 2026-07-20) —
  the plan greps for where `SurfaceActionRequest`/`InteractionDescriptor` payloads are documented
  (`docs/api/surfaces.md` and/or `docs/api/wire-protocol.md`) and adds the new `method`/`http_method`/`params`
  fields there; the asyncapi-codegen spec inherits the new fields when it models surfaces.
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

## Implementation decomposition (advisory)

Deliverables are independently landable with different blast radii; the plan phase should split into ordered,
green-at-each-step plans rather than one:

1. **DirectBuiltInApi deletion** (§5) — pure removal, low risk, workspace-wide `#[cfg(test)]` sweep.
2. **Contract core** — `InteractionHttpMethod`, descriptor/wire additive fields, `(id, method)` registry keying,
   reference-node disambiguation + `ActionRef` reader, admission rules, wire bounds, harness stub-provider path.
3. **Route family + consumer cutover** — wrapper handlers, gates, GET query contract, frontend/CLI/openapi-client
   migration, e2e sweep, CI guard, docs + ADR.

The `Vec<ActionRef>` untagged-serde migration warrants its own verification pass within plan 2.

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

# Surfaces API: DataLoad Interactions via GET + Typed Query Params

**Date:** 2026-07-16
**Status:** Approved (user interview + contrarian pass + 3-round adversarial generator/critic loop; split out of the
combined surfaces-REST spec on user request)
**Scope:** `crates/ui/web-api` (routes, OpenAPI), `crates/shared/surfaces`, `crates/shared/web-api-types`,
`crates/ui/surface-proxy`, `crates/shared/wire`, `crates/shared/openapi-client`, `crates/ui/cli`, `frontend/`, `ci/`,
docs.
**Sequencing:** Depends on
[`2026-07-16-surfaces-openapi-sdk-design.md`](2026-07-16-surfaces-openapi-sdk-design.md) (utoipa registration,
generated SDK, read-model rename) landing first.

## Problem

1. **Read-only interactions are POSTs.** Every interaction kind — reads (`InteractionKind::DataLoad`, e.g.
   `list-hosts`), mutations, workflows — goes through one generic
   `POST /api/v1/surfaces/{surface_id}/interactions/{interaction_id}` (handler `invoke_surface_interaction`,
   `crates/ui/web-api/src/routes/surfaces.rs`). Reads get no GET semantics and are indistinguishable from mutations
   at the HTTP layer.
2. **Untyped interaction params.** Params are a free-form `serde_json::Map<String, Value>`; the only per-interaction
   typing is `input_schema: Option<SchemaContract>` — a top-level type tag (`crates/shared/surfaces/src/data.rs`,
   `SchemaContract`), checked shallowly by `schema_matches` (`crates/ui/surface-proxy/src/proxy/validation.rs`).
   Moving reads to GET forces the query-string typing question: query values arrive as strings, and blind coercion
   silently corrupts reads (a numeric-looking string filter becomes a number; the provider's `as_str()` returns
   `None`).

## Decisions (settled — do not reopen)

| #   | Decision      | Resolution                                                                                                                                              |
| --- | ------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------- |
| B1  | Method split  | `DataLoad` interactions: GET only (POST → 405 + `Allow: GET`). All other kinds: POST only (GET → 405 + `Allow: POST`).                                  |
| B2  | Query typing  | Framework-reserved typed keys + opt-in per-field `ParamFieldDescriptor` declarations + string passthrough for undeclared keys. No type inference, ever. |
| B3  | Compatibility | Atomic breaking change. Frontend, CLI, and `openapi-client` are all in-repo and update in the same change. No deprecation window.                       |

**Alternatives rejected:**

- **Data-source subresources** (`GET /surfaces/{id}/data/{source_id}`) — cannot cover form preload/option-load
  DataLoads that have no data source (`SurfaceForm.svelte` invokes DataLoad-kind interactions directly).
- **Extending `SchemaContract` with a nested `Object { fields }` variant** — `#[serde(tag = "type")]` with no
  `Other` catch-all means a new variant tag hard-fails deserialization of the whole registration payload on older
  peers, and dozens of bare `SchemaContract::Object` literals across all interaction kinds would churn; a new
  additive struct field is strictly safer.
- **schemars-generated schemas** — output is strictly richer than `SchemaContract`, forcing a lossy hand-maintained
  subsetting layer; repo OpenAPI is utoipa end-to-end (schemars is confined to `crates/ui/mcp`).
- **JSON-guessing query values** (`"2"` → number) — silent wrong-read corruption; rejected.
- **`Idempotency-Key` header** — rejected in the predecessor spec; GET DataLoads carry no idempotency (reads need no
  dedup — verified no caller uses it).

## Design

### 1. Route + method/kind gate

New route: `GET /api/v1/surfaces/{surface_id}/interactions/{interaction_id}` (DataLoad only). The existing POST
route keeps serving all non-DataLoad kinds. Both methods route to the same resolution logic; the method/kind gate
happens after interaction resolution (kind is runtime registry data — a static route split is impossible).

**Resolution order (normative):** unknown surface or interaction → 404; then permission checks (descriptor, then
interaction) → 403; then method/kind mismatch → 405 with the correct `Allow` header. Permission before 405 so an
unauthorized caller cannot probe an interaction's kind. 405 responses use `StatusCode::METHOD_NOT_ALLOWED` with the
platform `ErrorResponse` envelope and set `Allow`.

**HEAD:** must not invoke the provider. Axum derives HEAD from GET handlers; the invoke-GET handler short-circuits
HEAD after the permission check (headers only, no provider dispatch). A provider round-trip on HEAD is both wasteful
and unsafe if a provider mislabels a mutation as DataLoad.

**Caching:** the GET invoke response sets `Cache-Control: private, no-store` (convention established by the
predecessor spec for all surface GETs).

### 2. GET query contract

Deterministic three-tier rule — **no type inference**:

1. **Envelope keys** (never reach the provider as params): `target_provider_id: String`,
   `timeout_seconds: u16` — same semantics as the existing `InvokeSurfaceInteractionRequest` body fields
   (`crates/shared/web-api-types/src/surfaces.rs`).
2. **Framework-reserved typed keys** (coerced unconditionally, forwarded inside `params`): `page: u64`,
   `per_page: u64`. These are the only numeric params any DataLoad handler in the tree reads today (grep basis:
   `handle_list_hosts` in `crates/core/agent-ssh-runtime/src/surface_runtime.rs`, proxmox `handle_list` in
   `crates/plugins/infrastructure/proxmox/src/surfaces.rs`, and the shared
   `crates/plugins/notifications/core/src/list_channels.rs` helper — all `.as_u64()` on `page`/`per_page`).
   Unparsable value → 422 `schema_validation_failed`.
3. **Everything else:** if the key is declared in the interaction's new `params` descriptor list, parse strictly per
   its declared `SchemaContract` (failure → 422 `schema_validation_failed`); if undeclared, pass through as a JSON
   string. String passthrough is verified safe for the entire current DataLoad population: every non-reserved param
   consumed by any DataLoad handler is a `Uuid`/`Option<Uuid>` (serde `Uuid` deserializes from JSON strings) or a
   string — including the typed-struct handlers using the `parse_action_params::<T>()` convention
   (`crates/plugins/releases/docker/src/surfaces.rs`, `crates/plugins/infrastructure/proxmox/src/surfaces.rs`,
   `crates/plugins/notifications/email/src/surfaces.rs`).

Reserved/envelope keys are documented in one static `#[derive(Deserialize, utoipa::IntoParams)]` struct referenced
as `params(<Struct>)`, satisfying `ci/verify_no_inline_query_params.sh` (ADR-0025). Dynamic per-interaction params
cannot appear in static OpenAPI (interactions are runtime registry data — structural limit, documented in the
OpenAPI operation description). **Explicitly not deliverable:** per-interaction typed SDK signatures.

The plan must re-verify the "only page/per_page are numeric" inventory with a fresh grep at implementation time —
inventory claims go stale.

Query-string params appear in access logs and browser history. Acceptable for DataLoad params
(tenant-identifying values at most — never secrets, enforced by the admission rule below); stated in
`docs/security/surfaces.md`.

### 3. Per-field param declarations (new, opt-in)

New shared type in `crates/shared/surfaces` (new module `params.rs`):

```rust
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParamFieldDescriptor {
    pub key: String,
    pub schema: SchemaContract, // reuse the existing 8-variant enum
    #[serde(default)]
    pub required: bool,
}
```

New additive field on `InteractionDescriptor` (`crates/shared/surfaces/src/interaction.rs`):

```rust
#[serde(default, skip_serializing_if = "Vec::is_empty")]
pub params: Vec<ParamFieldDescriptor>,
```

Wire-safe: unknown struct fields are dropped silently by older peers (no `deny_unknown_fields` anywhere in
`crates/shared/surfaces` or `crates/shared/wire`); same additive pattern as `sensitive_fields`.

Uses (one declaration, four consumers):

- **GET coercion** (§2 tier 3).
- **POST body validation:** `validate_input_schema` (`crates/ui/surface-proxy/src/proxy/validation.rs`)
  additionally walks `params` per-field after the existing top-level `schema_matches` check — `required` field
  absent or type-mismatched → the existing `SchemaValidationFailed` error path. Undeclared body keys still pass (no
  new rejection class rides along the migration).
- **CLI:** when `form_ui` is `None` and `params` is non-empty, the dynamic clap tree
  (`crates/ui/cli/src/commands/surfaces.rs`) builds typed args from the declarations
  (String/Integer/Number/Boolean value parsers) instead of raw-JSON mode.
- **Frontend:** the read model already delivers descriptors at runtime; the surface renderer can type-coerce and
  validate before dispatch (non-blocking enhancement, not a deliverable of this spec).

**Migration cost: zero declarations required.** Every currently-shipped DataLoad keeps working with reserved-key
coercion + string passthrough alone. Declarations are for new/future interactions and for providers that opt in.

### 4. Admission, wire validation, enforcement

- **Descriptor validation** (`InteractionDescriptor::validate_for_provider`,
  `crates/shared/surfaces/src/interaction.rs`) gains three rules, enforced for every provider through the single
  admission choke point `validate_registration_admission_locked` (`crates/ui/surface-proxy/src/registry.rs` —
  shared by `register_service` (remote/WS) and `bootstrap_plugin`/`bootstrap_builtin` (in-process); the plan must
  verify this call linkage by grep):
  1. `params[].key` must not collide with a reserved/envelope key (`page`, `per_page`, `target_provider_id`,
     `timeout_seconds`).
  2. On `DataLoad` interactions, `params[].schema` must be a scalar (`String`/`Integer`/`Number`/`Boolean`) — a
     query string cannot carry `Array`/`Object`/`Null`.
  3. `DataLoad` interactions must not declare `sensitive_fields` (vacuous today — no DataLoad declares any — but it
     closes the "secret in a GET query string" class permanently).
- **Wire bounds** (`validate_surface_interaction`, `crates/shared/wire/src/wire_validate_impls.rs`): bound
  `params.len()` with the existing `MAX_SURFACE_FIELDS` and each `key` with `MAX_SHORT_STRING_LEN`
  (`crates/shared/wire/src/limits.rs`) — satisfies the "Wire protocol payloads must implement `WireValidate`" rule.
- **Advisory telemetry for remote providers** (CI cannot inspect their source): when a `DataLoad` interaction is
  admitted with empty `params`, emit a structured `tracing::warn!` (provider id + interaction id fields, message
  matching the target file's message shape) and increment a metric
  (`surface_registration_dataload_missing_params_total`, `metrics` crate — already used in web-api).
  Non-rejecting: a hard block would break additive compatibility for not-yet-updated remote providers.
- **CI guard for in-repo providers:** new `ci/verify_dataload_declares_params.sh` +
  `ci/verify_dataload_declares_params_allowlist.txt`, modeled on the existing
  `ci/verify_typed_audit_actions.sh` + allowlist pair. Flags `InteractionKind::DataLoad` constructions with no
  `params` declaration; genuinely param-less DataLoads (e.g. `get_smtp`, `get_global_telegram`) go on the allowlist.

### 5. Consumer migration (atomic)

- Frontend: `SurfaceReadPanel.svelte` hydration and `SurfaceForm.svelte` preload/option-load DataLoads switch to
  the GET call; a thin hand-written helper appends dynamic DataLoad query params to the generated GET call
  (everything else stays on the generated SDK from the predecessor spec).
- `crates/shared/openapi-client`: new `read_surface_interaction` (GET, query-param encoding) alongside
  `invoke_surface_interaction` (POST); path constants updated. Gated by `cargo xtask openapi-client-check`.
- CLI: `dynamic_invoke` dispatches DataLoad interactions through the new GET client method; all other kinds
  unchanged.

## Error semantics (delta only)

| Case                                       | Status        | Code                            |
| ------------------------------------------ | ------------- | ------------------------------- |
| POST on DataLoad / GET on other kinds      | 405 + `Allow` | `method_not_allowed` (new code) |
| Reserved or declared GET param fails parse | 422           | `schema_validation_failed`      |

Everything else keeps the existing mapping in `routes/surfaces.rs` (`action_error_code` table).

## Testing

New endpoint tests use the shared `TestApp` harness (`crates/ui/web-api/src/test_harness/`). Required coverage
(each its own test — enumerated members, not a representative):

1. GET DataLoad happy path: query params land in provider `params` (reserved keys as numbers, undeclared as
   strings).
2. POST on a DataLoad interaction → 405, `Allow: GET`, `ErrorResponse` envelope.
3. GET on a `MutationAction` interaction → 405, `Allow: POST`.
4. Unknown interaction id → 404 (not 405) on both methods.
5. Caller lacking the interaction permission → 403 on GET **before** any kind/method disclosure.
6. Declared param strict-parse failure → 422 `schema_validation_failed`; declared param success coerces to the
   declared type.
7. `page=abc` → 422; `page=2` → provider sees `Number(2)`.
8. HEAD on a DataLoad route does not reach the provider (assert via recording provider stub).
9. `Cache-Control: private, no-store` present on the GET invoke response.
10. Admission rejections: reserved-key collision; `Array` schema on DataLoad; `sensitive_fields` on DataLoad —
    each as its own `validate_for_provider` unit test, plus one registration-path test proving the admission choke
    point enforces them.
11. Wire bounds: `params` vec over `MAX_SURFACE_FIELDS` rejected by `WireValidate`.
12. POST per-field validation: `required` declared field absent → 422; undeclared body key still passes.
13. Advisory path: DataLoad admitted with empty `params` registers successfully and increments the metric/warn
    (assert non-rejection at minimum).
14. Frontend: `SurfaceReadPanel`/`SurfaceForm` unit tests updated for GET hydration; CLI test for DataLoad → GET
    dispatch.

Verification commands and their scope: `cargo test --all-features` (full workspace; requires `frontend/build/` —
build the frontend first), scoped `cargo clippy --all-targets --no-default-features --features db-sqlite`,
`./scripts/regen-api.sh` then a clean-diff check on `crates/ui/web-api/openapi.json` +
`frontend/src/lib/api/generated/`, `cargo xtask openapi-client-check`, `bash ci/verify_no_inline_query_params.sh`,
`python3 ci/verify_db_access_policy.py`, and the new `bash ci/verify_dataload_declares_params.sh`.

## Deliverables

**Code** — GET route + method/kind gate + resolution order + HEAD short-circuit + cache header
(`routes/surfaces.rs`, `router.rs`); `ParamFieldDescriptor` + `InteractionDescriptor.params` + admission rules
(`crates/shared/surfaces`); GET coercion layer; POST per-field validation (`surface-proxy`); wire bounds
(`crates/shared/wire`); reserved-keys `IntoParams` struct + OpenAPI registration for the GET operation;
openapi-client GET method; CLI GET dispatch; frontend hydration migration; advisory warn+metric; CI guard script +
allowlist.

**Docs (non-optional):**

- `docs/api/surfaces.md` — method split, query tiers, resolution order, 405 semantics, params declarations.
- `docs/development/surfaces.md` — provider authoring guidance (declare `params`; DataLoad params are strings
  unless declared/reserved).
- `docs/security/surfaces.md` — GET query-string exposure statement, DataLoad `sensitive_fields` admission rule.
- New ADR `docs/adr/0028-surfaces-rest-api-and-param-typing.md` (number = next free at implementation time) — REST
  shape, method/kind gate, query-typing tiers, rejected alternatives, typed-extractor exception.
- `crates/ui/web-api/db_access_policy.toml` — entries for renamed/new handlers.
- Regenerated `crates/ui/web-api/openapi.json` + `frontend/src/lib/api/generated/` committed with the originating
  change.
- `CONTEXT.md`: no change — no new domain vocabulary (ParamFieldDescriptor/reserved keys are code-level terms).
- `crates/shared/wire/asyncapi.yaml`: no surfaces sub-schema exists there today (surfaces descriptor contract is
  documented in `docs/api/surfaces.md`); the plan must re-verify with grep and update whichever file actually
  carries the `SurfaceRegistration`/`InteractionDescriptor` payload documentation.

## Deferred (named follow-ups, out of scope)

1. **Phase 2 — `#[derive(SurfaceParams)]`**: proc-macro (new crate; `uptrakit-shared-macros` is
   `macro_rules!`-only) generating `declared_params()` from the same structs `parse_action_params::<T>()`
   deserializes, making declaration/handler drift compile-impossible; migrate remaining map-poking DataLoad
   handlers to typed structs. Explicitly does **not** emit `utoipa::IntoParams` (no per-interaction utoipa path
   exists; would drag utoipa into plugin crates).
2. **Typed sort/filter reserved keys** driven by `DataSourceSorting`/`DataSourceFiltering` (declared in
   `crates/shared/surfaces/src/data.rs` but populated nowhere outside tests today).
3. **Pagination clamp unification**: source `handle_list_hosts`-style clamps from the paired
   `DataSourceDescriptor.pagination` instead of duplicated literals.
4. **Hard-block policy** for remote DataLoads registering without `params` declarations (product decision;
   requires a deprecation window).
5. **Interaction-id REST naming sweep** (user-approved 2026-07-16; rides the D9 two-system unification follow-up
   deferred by `2026-07-15-proxmox-guest-flow-provider-invocable-design.md`, which halves the touch-points per
   rename by collapsing the legacy `SurfaceActionDescriptor` library + `ControllerSurfaceAction` dispatch map into
   registered `InteractionDescriptor`s first). Convention: kebab-case only (no snake_case, no `mqtt.`-style provider
   prefixes — the surface id already namespaces); DataLoad ids are noun phrases naming the resource read (no
   `list-`/`get-`/`load-`/`preload-` prefixes); non-DataLoad ids stay verb phrases (bare verbs `create`/`edit`/
   `delete`/`test`/`discover`/`match`/`unmatch`/`bootstrap` are fine). Skew discipline: ids declared in service
   binaries (`ssh-agent.hosts`, `mqtt.clients`) keep a one-release dual-dispatch alias (dispatch accepts the old id;
   only the new id registers); in-process plugin ids rename atomically (frontend is data-driven and ships with the
   controller). Interaction ids are not persisted anywhere (no DB columns, no MQTT topics — verified 2026-07-16), so
   renames are storage-safe. Approved rename table:

   | Surface                                  | Current                                                                                                    | New                                                                      |
   | ---------------------------------------- | ---------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------ |
   | `ssh-agent.hosts`                        | `list-hosts`                                                                                               | `hosts`                                                                  |
   | `proxmox.hosts`                          | `list`                                                                                                     | `mappings`                                                               |
   | `proxmox.host-info`                      | `get-info`                                                                                                 | `info`                                                                   |
   | `proxmox.settings.update-hooks`          | `preload-global-defaults` / `load-backup-target-options`                                                   | `global-defaults` / `backup-target-options`                              |
   | `proxmox.settings.resource-scaling`      | `preload-scaling-global-defaults`                                                                          | `scaling-global-defaults`                                                |
   | `proxmox.software-item.update-hooks`     | `preload-item-overrides` / `load-backup-target-options`                                                    | `item-overrides` / `backup-target-options`                               |
   | `proxmox.software-item.resource-scaling` | `preload-scaling-item-overrides`                                                                           | `scaling-item-overrides`                                                 |
   | `docker.item-host-actions`               | `get-current-tag`                                                                                          | `current-tag`                                                            |
   | `notifications.{email,telegram,webhook}` | `list`                                                                                                     | `channels`                                                               |
   | `notifications.email`                    | `get_smtp` / `configure_smtp`                                                                              | `smtp` / `configure-smtp`                                                |
   | `notifications.email.global_smtp`        | `get_global_smtp` / `save_global_smtp` / `test_global_smtp_email`                                          | `global-smtp` / `save-global-smtp` / `test-global-smtp`                  |
   | `notifications.telegram.global_settings` | `get_global_telegram` / `save_global_telegram`                                                             | `global-telegram` / `save-global-telegram`                               |
   | `mqtt.clients`                           | `mqtt.list-clients` / `mqtt.get-client` / `mqtt.create-client` / `mqtt.edit-client` / `mqtt.delete-client` | `clients` / `client` / `create-client` / `edit-client` / `delete-client` |

## Open questions

None.

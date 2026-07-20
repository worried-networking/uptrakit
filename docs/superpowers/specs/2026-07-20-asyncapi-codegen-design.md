# AsyncAPI Codegen for the Wire Protocol — Design

**Date:** 2026-07-20
**Status:** Approved
**Crate:** `uptrakit-wire` (`crates/shared/wire/`)

## Problem

`crates/shared/wire/asyncapi.yaml` is hand-maintained and has drifted chronically from the wire implementation. Inventory as of
2026-07-20 (all counts derived from untruncated greps at authoring time; re-derive before relying on them):

- 12 of 63 real wire messages are absent from the yaml entirely: all five surface messages (`surface_registration`,
  service-side `surface_action_request`/`surface_action_response`, controller-side `surface_action_request`/`surface_action_cancel`/
  `surface_action_response`), plus `register`, `audit_event`, `token_revoked`, `software_states_changed`,
  `workload_claim_sync_request`, `workload_claim_sync_response`. Nothing documented in the yaml is absent from code.
- Field-level drift in 7 of 10 sampled messages: stale `host_package_id` (code serializes `host_software_item_id`), missing
  `UpdateResultPayload.resumable`, missing `VersionCheckResult.installed_display_version`/`not_ready`, removed
  `DisconnectingPayload.active_mqtt_clients` still documented, `test_plugin_config` structurally wrong (yaml requires an
  `assignment` object the code never serializes), `ServiceSettingsPayload.capabilities` enum listing 5 of the 15 typed
  `Capability` variants, workload payload schemas omitting the envelope fields every other schema inlines.
- The existing guard (`AsyncApiSpec` validator + `spec_conformance_*` tests in `src/tests.rs`) is sample-based: it checks
  required-fields/`const`/enum membership for hand-picked messages only. It cannot see missing schemas, undocumented
  messages, or extra/renamed fields — structurally the same blindness
  [ADR-0025](../../adr/0025-drift-proof-openapi-params.md) documents for the OpenAPI golden test. No CI gate or script
  touches the yaml.
- The drift is not recency lag (yaml and src last changed the same day); messages were simply never added.

`asyncapi.yaml` has no programmatic consumers beyond `include_str!` in tests; it exists as reference documentation for
future WS integrations, referenced from `docs/api/wire-protocol.md`, `AGENTS.md`, `ARCHITECTURE.md`, and the docs index.

## Decision

Make the Rust wire types the single source of truth and generate `asyncapi.yaml` from them, mirroring the repo's
established drift-guard idiom (ADR-0025/ADR-0026 + `scripts/regen-api.sh` + golden staleness test). The first generation
replaces the drifted document and thereby fixes every gap above in one step. This decision gets its own ADR (0029).

Spec-first generation (AsyncAPI Generator templates → Rust) was rejected: no production-grade Rust template exists, it
drags the Node toolchain into Rust CI, generated code would violate the workspace lint policy, and it inverts the repo's
code-is-truth idiom (same grounds on which ADR-0026 rejected client codegen). Validation-only hardening was rejected:
sample-based validation cannot reach zero drift, and a faithful schema comparator requires deriving the schemas anyway —
at which point generation is the fix (ADR-0025's own reasoning). A manual yaml fix followed by later codegen was
rejected as throwaway work.

## Design

### 1. Schema derivation (schemars)

- Add `schemars = "1.2.1"` to root `[workspace.dependencies]`; `uptrakit-wire` references it `workspace = true` behind a
  new **additive** cargo feature `schema` (off by default; complies with the additive-only feature rule). Enable
  schemars features `uuid1` and `preserve_order` (deterministic key order for the golden gate).
- `#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]` on `ServiceMessage`, `ControllerMessage`, both
  envelope structs, and every payload/shared type they reach. schemars 1.x natively supports the internally-tagged
  layout (`#[serde(tag = "type", rename_all = "snake_case")]`) both enums use, plus `rename`/`default`/`flatten`.
- The payload types reach two other workspace crates: `uptrakit-shared-types` (e.g. `DiscoveredSoftware`,
  `ConfigTestKind`, `UpdateCategory`, `SecretString`) and `uptrakit-surfaces` (the surface descriptor types carried by
  the five surface messages). Each of those crates gains its own **additive** `schema` feature with the same
  `cfg_attr` derives on the reached types; `uptrakit-wire`'s `schema` feature enables both
  (`schema = ["uptrakit-shared-types/schema", "uptrakit-surfaces/schema"]`). Per reached type, prefer the derive (so
  rustdoc and known enum values render into the schema); fall back to a wire-side
  `#[cfg_attr(feature = "schema", schemars(with = "..."))]` override matching the serialized form only where a derive
  is disproportionate (e.g. `SecretString` → `with = "String"`). The exact reached-type set is discovered by the
  compiler during implementation, not enumerated here.
- Explicit `#[cfg_attr(feature = "schema", schemars(with = "..."))]` overrides where serde and schemars diverge:
  - fields using `serde_helpers::utc_datetime_millis` → `with = "i64"` (epoch milliseconds),
  - fields using `serde_helpers::duration_seconds` → `with = "u32"`,
  - fields using `serde_helpers::option_duration_seconds` → `with = "Option<u32>"`,
  - every `time::UtcDateTime`-typed field (schemars has no `time`-crate feature; only the helper-module fields above
    exist today — verify by grepping `UtcDateTime` in `payloads.rs` during implementation).
- Known, accepted representation limits (documented in the ADR and in the generated `info.description`):
  - `#[serde(other)]` catch-all variants (`Unknown`) are excluded from the generated message list via
    `#[cfg_attr(feature = "schema", schemars(skip))]` — they are a deserialize-only forward-compat mechanism, not a wire
    message.
  - Wire-safe enums with `Other(String)` catch-alls (e.g. `UpdateCategory`, `Capability`) are represented honestly as
    open `string` schemas with the known values listed in the schema `description`, never as closed `enum:` lists.
  - Deserialize-only `#[serde(alias)]` attributes are not represented — moot, because this spec removes them (§4).
- Rustdoc comments (`///`) on payload structs and fields flow into schema `description` fields automatically via
  schemars, so per-field documentation lives in code and regenerates with it.

### 2. Document assembly (asyncapi-rust)

- Add `asyncapi-rust = "0.4.0"` to root `[workspace.dependencies]`; wire crate dev/feature-gated use only. Chosen for
  consistency with the web-api code-first approach: it targets AsyncAPI 3.0, is built directly on schemars 1.x, its
  headline use case is an internally-tagged WS message enum, and its output is IndexMap-backed (byte-stable). Risk
  accepted knowingly: 0.x with two breaking releases in eight months, single maintainer — pinned version; breaking bumps
  are scheduled chores.
- A `#[cfg(feature = "schema")]` module `spec_gen` in `uptrakit-wire` builds the complete AsyncAPI 3.0.0 document in
  Rust: `info` (short description pointing at `docs/api/wire-protocol.md` for narrative), the `/api/v1/ws/service`
  channel, send/receive operations for the two message directions, `components/messages` (one entry per real enum
  variant, `const`-discriminated) and `components/schemas` (schemars output, shared `$defs` hoisted). The **entire**
  `asyncapi.yaml` becomes generated output; there is no hand-edited region left in the file. A generated-file header
  comment names the regen command.
- The envelope-flatten layout must be preserved as documented today: each message payload schema presents one flat
  object carrying `protocol_version`, `seq`, `trace_context`, optional `pagination` (service direction), `type`
  discriminator, and the payload fields — matching what `#[serde(flatten)]` actually puts on the wire.
- **Feasibility spike is the first implementation task**: prove asyncapi-rust 0.4.0 can express (a) the flattened
  envelope schema shape, (b) AsyncAPI 3.0 channels/operations as currently structured, (c) deterministic byte-stable
  YAML output. **Named fallback** if any of these fail: keep schemars for all schemas and assemble the document
  sections by hand with `serde_yaml_ng` (already a dev-dependency) inside the same module — the assembly surface is
  small since all prose leaves the file (§5). The fallback changes no other part of this design.

### 3. Golden staleness gate (web-api idiom)

- New test in `uptrakit-wire` (feature-gated on `schema`), named with the `asyncapi_` prefix: normal run asserts the
  committed `asyncapi.yaml` is byte-identical to the generated document; with `UPDATE_ASYNCAPI=1` set it rewrites the
  file instead — exactly the `UPDATE_OPENAPI=1` pattern in `crates/ui/web-api`.
- New `scripts/regen-asyncapi.sh` mirroring `scripts/regen-api.sh`:
  `UPDATE_ASYNCAPI=1 cargo test -p uptrakit-wire --features schema asyncapi_`.
- Enforcement scope (stated per the feature-gate lesson): the CI quality gate `cargo test --all-features` runs the
  golden test; the pre-push hook's `cargo nextest run --no-default-features --features db-sqlite` does **not** compile
  the `schema` feature, so pre-push will not catch staleness — CI is the enforcing gate. The command that exercises the
  gate locally is `cargo test -p uptrakit-wire --features schema asyncapi_`.

### 4. Serde alias removal

Delete all seven deserialize-only `#[serde(alias)]` attributes in the wire crate — `batch_host_package_update_result`
and `execute_batch_host_package_update` on the message enums (`messages.rs`), and the `host_package_id` (×4) and
`host_package_hosts` aliases in `payloads.rs` (locate by `grep -n 'alias' crates/shared/wire/src/`; line numbers drift).
Justification verified against release history: all three renames land in commit `256c3dbfd` and every one of the
existing release tags contains that commit (`git tag --no-contains 256c3dbfd` is empty), so no released binary ever
emitted the old names; the aliases only served unreleased dev-era peers. Before deletion, grep both old names across
the whole workspace (including `#[cfg(test)]` code) and rewrite or delete every referencing test — the plan must
enumerate each hit.

### 5. Prose eviction with preservation inventory

All narrative prose leaves `asyncapi.yaml`; the file focuses on the wire API itself. Migration is inventory-driven, not
a rewrite (regeneration is how unlisted content dies):

- Read every prose region of the current yaml end-to-end: the `info.description` lifecycle narrative (enrollment steps,
  CSR-based issuance, reconnection semantics, workload-claim notes), per-message `description` blocks, the
  `serviceEnvelope`/`controllerEnvelope` explanatory notes, and the "Known capabilities" list.
- For each item, record one of: (a) already present in `docs/api/wire-protocol.md` (cite the section), (b) migrated
  there (new/extended section), or (c) moved into rustdoc on the corresponding Rust type, from where it re-enters the
  generated yaml as a schema/message `description`. Semantic per-message notes prefer (c); flow narrative prefers (b).
- No hand-maintained capability list survives anywhere in prose — point at `Capability` in
  `crates/shared/wire/src/capabilities.rs` instead.
- Verification is mechanical, both directions: a stale-string grep (removed prose absent from the yaml) **and** a
  presence grep (each inventoried survivor string found at its recorded destination).

### 6. Test cleanup

Retire the `AsyncApiSpec` validator and all `spec_conformance_*` tests in `src/tests.rs`: once the schemas derive from
the same Rust types via serde/schemars, asserting serialization against them is testing upstream crate behavior. The
envelope wire-shape serialization tests (flat JSON layout, seq/protocol_version presence) stay — they pin the on-wire
contract independent of schemars. `docs/development/testing.md` currently describes the sample-based mechanism and
overstates its coverage; that section is rewritten to describe the golden gate (correct-in-place, not a rewrite of the
whole doc).

### 7. Out-of-scope guard

`protocol_version` stays at 1: the canonical serialized format does not change (alias removal only drops deserialize
leniency for names no released peer sends). `WireValidate`, `limits.rs`, pagination, and all runtime wire behavior are
untouched. No frontend, controller, or service code changes beyond the wire crate's derive annotations.

## Deliverables

1. Workspace-root `[workspace.dependencies]`: `schemars` 1.2.1, `asyncapi-rust` 0.4.0 (registered there first; crate
   manifests use `workspace = true`). `cargo deny check` must pass with both.
2. `uptrakit-wire`: `schema` feature, `JsonSchema` derives + overrides, `spec_gen` module, golden test, alias removal,
   conformance-test retirement.
3. Regenerated `crates/shared/wire/asyncapi.yaml` (fully generated; reviewed diff against the old file checked against
   the §Problem inventory: 12 previously missing messages present, stale fields gone, capability representation open).
4. `scripts/regen-asyncapi.sh`.
5. Docs:
   - `docs/adr/0029-asyncapi-codegen.md` — new ADR recording this decision, the rejected alternatives, the accepted
     schemars representation limits, and the asyncapi-rust 0.x risk.
   - `docs/api/wire-protocol.md` — absorbs migrated narrative prose (§5).
   - `docs/development/testing.md` — sample-validator section replaced by golden-gate description (§6).
   - `docs/development/quality-gates.md` **and** the AGENTS.md quick-start block — add the regen command, same commit
     (AGENTS.md maintenance rule).
   - `AGENTS.md` — wire invariant wording still holds ("documented in asyncapi.yaml"); fix the stale crate name in the
     layout tree (`uptrakit-internal-wire` → `uptrakit-wire`).
   - `docs/README.md` — index entry for the new ADR.
6. Pending-specs tracker entry (working artifact, not committed).

## Verification

- `cargo test -p uptrakit-wire --features schema asyncapi_` green; rerunning `scripts/regen-asyncapi.sh` twice yields a
  byte-identical file (determinism).
- Full gates: `cargo fmt`, `cargo check`/`clippy` on both canonical feature sets, `cargo test --all-features`,
  `cargo deny check`, `markdownlint`.
- Generated-vs-old yaml diff reviewed against the §Problem inventory (the gap-fix proof).
- Prose migration greps (§5), both directions.
- Workspace-wide grep for the removed alias strings returns hits only in this spec/ADR/history documents.

## Deferred / out of scope

- Publishing or rendering `asyncapi.yaml` anywhere (website, AsyncAPI Studio) — it remains an in-repo reference.
- Documenting NATS-side message flows in AsyncAPI — the yaml covers the service↔controller WebSocket protocol only, as
  today.
- Hardening beyond the golden gate (e.g. instance-validation of runtime messages against the generated schemas) — the
  golden gate plus type-derivation makes this redundant.
- Any `protocol_version` bump or wire behavior change.

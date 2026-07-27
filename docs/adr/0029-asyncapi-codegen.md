# 0029 — AsyncAPI Codegen for the Wire Protocol

**Date:** 2026-07-20 **Status:** Accepted

## Context

`crates/shared/wire/asyncapi.yaml` was hand-maintained and had drifted chronically from the wire
implementation. An inventory taken at spec time found 12 of 63 real wire messages absent from the yaml
entirely (all five surface messages, plus `register`, `audit_event`, `token_revoked`,
`software_states_changed`, `workload_claim_sync_request`, `workload_claim_sync_response`), field-level
drift in 7 of 10 sampled messages (stale `host_package_id` where code serializes
`host_software_item_id`, missing `UpdateResultPayload.resumable`, missing
`VersionCheckResult.installed_display_version`/`not_ready`, a removed `active_mqtt_clients` field still
documented, a structurally wrong `test_plugin_config` schema, and a `ServiceSettingsPayload.capabilities`
enum listing 5 of the 15 typed `Capability` variants), and nothing documented in the yaml missing from
code. The prior guard — an `AsyncApiSpec` validator plus `spec_conformance_*` tests — was sample-based:
it checked required-fields/`const`/enum membership for hand-picked messages only, and could not see
missing schemas, undocumented messages, or extra/renamed fields — structurally the same blindness
[ADR-0025](0025-drift-proof-openapi-params.md) documents for the OpenAPI golden test. No CI gate or
script touched the yaml, and the drift was not recency lag: the yaml and the source it should describe
last changed on the same day, because messages were simply never added.

`asyncapi.yaml` has no programmatic consumers beyond `include_str!` in tests; it exists as reference
documentation for future WebSocket integrations, referenced from `docs/api/wire-protocol.md`,
`AGENTS.md`, `ARCHITECTURE.md`, and the docs index.

## Decision

The Rust wire types are the single source of truth; `asyncapi.yaml` is fully generated from them. Schema
derivation uses `schemars` (feature-gated `schema` derives across `uptrakit-wire`,
`uptrakit-shared-types`, and `uptrakit-surfaces`); the complete AsyncAPI 3.0.0 document is hand-assembled
in a test-only `spec_gen` module using `serde_yaml_ng` (already a wire-crate dev-dependency) — there is
no hand-edited region left in the file. A golden staleness test
(`asyncapi_yaml_is_up_to_date`, feature-gated on `schema`) asserts the committed yaml is byte-identical
to the generated document, mirroring the repo's `openapi.json` idiom
([ADR-0025](0025-drift-proof-openapi-params.md), `regen-api.sh`); `scripts/regen-asyncapi.sh` regenerates
it via `UPDATE_ASYNCAPI=1 cargo test -p uptrakit-wire --all-features asyncapi_`. The first generation
replaced the drifted document outright, fixing every gap in the inventory above in one step.

CI's `--all-features` coverage run is the enforcing gate. The pre-push hook's
`cargo nextest run --no-default-features --features db-sqlite` does not compile the `schema` feature, so
pre-push does not catch staleness — this mirrors an accepted, already-documented limitation of the
minimal-feature-set pre-push gate elsewhere in the repo, not a new gap.

### Rejected alternatives

- **Spec-first generation** (AsyncAPI Generator templates → Rust): no production-grade Rust template
  exists, it would drag the Node toolchain into Rust CI, generated code would violate the workspace lint
  policy (`warnings = "deny"`, `clippy::all = "deny"`, `#[allow]` banned), and it inverts the repo's
  code-is-truth idiom — the same grounds on which [ADR-0026](0026-openapi-client-drift-guard.md) rejected
  client codegen.
- **Validation-only hardening** of the sample-based `AsyncApiSpec` checker: sample-based validation
  cannot reach zero drift, and a faithful schema comparator would need to derive the schemas anyway — at
  which point generation is the fix.
- **Manual yaml fix followed by later codegen**: throwaway work.
- **Deleting `asyncapi.yaml` outright** — the cheapest fix for a document with no programmatic consumer —
  was rejected by explicit product decision: the document is kept as an interchange-format reference for
  future WebSocket integrators (loadable into AsyncAPI tooling, usable for third-party client
  generation), a value rustdoc alone cannot provide.
- **asyncapi-rust 0.4.0 as the document assembler**: initially preferred for consistency with the web-api
  code-first approach, then rejected on evidence verified by reading its published source. Two blockers:
  (a) `ToAsyncApiMessage` operates on the tagged enum alone, while the envelope fields
  (`protocol_version`, `seq`, `trace_context`, `pagination`) live on the wrapping
  `ServiceEnvelope`/`ControllerEnvelope` structs, so the repo's flattened-envelope wire layout cannot be
  expressed; (b) its codegen reads only per-variant `#[serde(rename = ...)]` and has no `rename_all`
  handling at all, so both message enums' `rename_all = "snake_case"` discriminants and message-map keys
  would emit as bare PascalCase variant identifiers (`"Ping"`, not `"ping"`) — working around this would
  mean adding dozens of per-variant `rename` attributes to reshape wire code to fit a 0.x doc tool, which
  was rejected. Supporting immaturity signals: a single maintainer; roughly seven months of dormancy
  followed by two breaking releases within hours of each other on one day (2026-06-19); 0.4.0 itself
  roughly a month old at spec time. **Reversal condition:** re-evaluate replacing the hand assembly if a
  future asyncapi-rust release gains both `rename_all` handling and an envelope/flatten mechanism; until
  then it is not registered in the workspace at all.

### Accepted representation limits

- `#[serde(other)]` catch-all variants (deserialize-only forward-compat, e.g. `Unknown`) are excluded
  from the generated message list via `#[cfg_attr(feature = "schema", schemars(skip))]` — they are not a
  wire message.
- Wire-safe enums with an `Other(String)` catch-all (`Capability`, `EnrollmentStatus`, `UpdateCategory`,
  and the rest of the `wire_safe_enum!`/hand-rolled population) are represented honestly as open `string`
  schemas with known values listed in the schema description, never as closed `enum:` lists — these types
  hand-implement `Serialize`/`Deserialize` and cannot use the `JsonSchema` derive without silently
  documenting the Rust enum shape instead of the wire string. `wire_safe_enum!` now emits a manual,
  feature-gated `schemars::JsonSchema` impl alongside its existing manual `utoipa` impl; each known-value
  list is derived programmatically from the same source the `Serialize` impl uses, never a hardcoded
  string list.
- Deserialize-only `#[serde(alias)]` attributes are not represented in the schema — moot, because this
  change also removes them (see below).

### Serde alias removal

The seven deserialize-only `#[serde(alias)]` attributes in the wire crate were deleted (two on the
message enums, five on payload fields including four `host_package_id` variants and
`host_package_hosts`). Verified against release history: the renames they compensated for all land in a
single commit, and every existing release tag contains that commit, so no released binary ever emitted
the old names — the aliases only served unreleased dev-era peers.

### Trust model

The golden gate detects **staleness** (committed yaml vs. generated document) and turns every wire-shape
change into a reviewable `asyncapi.yaml` diff in the pull request — the same diff-review trust model the
repo already accepts for `openapi.json`. It does **not** hard-fail on an accidental serde change the way
the retired sample-based `AsyncApiSpec` checks did for their hand-picked sample: a self-derived spec
cannot disagree with the code it derives from. Cross-version wire compatibility remains untested, exactly
as before this change. A `schemars` minor-version bump that changes derived output flips the golden test
red into a reviewable regen diff — caught, not silent.

### Standing obligation

Any future custom `Serialize`/`Deserialize` impl on a type reached by the schema derivation (directly or
transitively, in `uptrakit-wire`, `uptrakit-shared-types`, or `uptrakit-surfaces`) requires a matching
manual `JsonSchema` impl. The `#[derive(JsonSchema)]` shortcut silently documents the Rust type's default
shape, which is wrong the moment `Serialize` is hand-written — the compiler cannot catch this
divergence, and the golden gate cannot either, because the generated schema simply agrees with itself.

### schemars MSRV vs. workspace `rust-version`

`uptrakit-wire` pins `schemars = { workspace = true }` resolving to **1.2.1** in the current lockfile
(confirmed: `Cargo.lock` pins `schemars 1.2.1` for `uptrakit-wire`, `uptrakit-shared-types`,
`uptrakit-surfaces`, `uptrakit-mcp`, and `uptrakit-web-api-types`; a separate `schemars 0.9.0` in the
lockfile is an unrelated transitive dependency of `serde_with` only). Verified directly from the vendored
crate manifest (`schemars-1.2.1/Cargo.toml`): `rust-version = "1.74"`. The workspace baseline some crates
declare is `rust-version = "1.91"` (see `AGENTS.md`). schemars' MSRV is comfortably below the workspace
floor, so it imposes no additional toolchain constraint.

## Consequences

- `cargo ... --all-features` builds now compile `schemars` into `uptrakit-shared-types` and
  `uptrakit-surfaces` (feature unification across the workspace's single `--all-features` build), a
  one-time compile-time cost.
- The generated document's fitness for AsyncAPI tooling (AsyncAPI Studio / `@asyncapi/parser`) was
  verified once at authoring time — deliberately not re-checked per commit, to keep Node out of Rust CI.
  `scripts/regen-asyncapi.sh` prints a reminder to re-run that validator whenever the message/schema set
  changes, so future regens carry a prompt the diff review alone would not provide.
- Adding a wire message or changing a payload shape requires running `./scripts/regen-asyncapi.sh` and
  committing the resulting `asyncapi.yaml` diff — the same discipline the repo already expects for
  `openapi.json` via `./scripts/regen-api.sh`.
- Zero net-new workspace dependencies: `schemars` reuses the pre-existing `schemars = { version = "1" }`
  root entry (already consumed by `uptrakit-mcp`); `serde_yaml_ng` stays a plain dev-dependency.

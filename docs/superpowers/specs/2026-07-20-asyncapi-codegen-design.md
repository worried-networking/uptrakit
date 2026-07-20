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

Rejected alternatives (all recorded in ADR-0029):

- **Spec-first generation** (AsyncAPI Generator templates → Rust): no production-grade Rust template exists, it drags
  the Node toolchain into Rust CI, generated code would violate the workspace lint policy, and it inverts the repo's
  code-is-truth idiom (same grounds on which ADR-0026 rejected client codegen).
- **Validation-only hardening**: sample-based validation cannot reach zero drift, and a faithful schema comparator
  requires deriving the schemas anyway — at which point generation is the fix (ADR-0025's own reasoning).
- **Manual yaml fix followed by later codegen**: throwaway work.
- **Deleting `asyncapi.yaml` outright** (cheapest fix for a consumer-less doc): rejected by explicit product decision —
  the document is kept as an interchange-format reference for future WS integrators (loadable into AsyncAPI tooling,
  usable for third-party client generation), value rustdoc cannot provide.
- **asyncapi-rust 0.4.0 as the document assembler**: initially selected for consistency with the web-api code-first
  approach, then rejected on verified evidence — see §2.

Honest statement of what the golden gate does and does not guard (also §6): it detects **staleness** (committed yaml vs
code) and turns every wire-shape change into a reviewable `asyncapi.yaml` diff in the PR — the same diff-review trust
model the repo already uses for `openapi.json`. It does **not** hard-fail on an accidental serde change the way a
pinned-expectation test would: a self-derived spec cannot disagree with the code it derives from. Cross-version wire
compatibility remains untested, exactly as before this spec.

## Design

### 1. Schema derivation (schemars)

- schemars is **already registered** in root `[workspace.dependencies]` as `schemars = { version = "1" }` (consumed by
  `uptrakit-mcp`; resolves to 1.2.1 in the current lockfile). Reuse that entry — do not re-pin to an exact patch
  version (the workspace convention is loose major-version pins). `uptrakit-wire` references it `workspace = true`
  behind a new **additive** cargo feature `schema` (off by default; complies with the additive-only feature rule).
  Per the dependency-policy convention (root entry = version only; consuming crates declare the additional features
  they need), the schemars features `uuid1` and `preserve_order` go in `uptrakit-wire`'s own manifest —
  `schemars = { workspace = true, features = ["uuid1", "preserve_order"], optional = true }` — not on the root entry.
  Note on `preserve_order`: schemars output is deterministic either way (default is alphabetical via `BTreeMap`); the
  feature switches to declaration order for readability. Determinism does not depend on it.
  Resolution heads-up for implementation: schemars 1.2.1's `ref-cast` chain can pull `syn 3.x` in an unconstrained
  resolve while today's lockfile holds `syn 2.x` — `cargo deny check` (multiple-versions = deny) is the gate that
  catches this during lockfile regen; not a design change, just expect it.
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
  - Wire-safe enums with `Other(String)` catch-alls (e.g. `UpdateCategory`, `Capability`, `EnrollmentStatus`) are
    represented honestly as open `string` schemas with the known values listed in the schema `description`, never as
    closed `enum:` lists. **These types cannot use the `JsonSchema` derive**: they implement `Serialize`/`Deserialize`
    by hand (or via `wire_safe_enum!`), so a derive would silently document the Rust enum shape instead of the wire
    string — a semantic bug the compiler cannot catch. The repo already solves this exact problem for OpenAPI:
    `wire_safe_enum!` (`crates/shared/macros/src/lib.rs`) emits a manual `utoipa::PartialSchema`/`ToSchema` impl
    describing the serde wire format. Mirror it: extend `wire_safe_enum!` with a feature-gated manual
    `schemars::JsonSchema` impl (open string schema + known values in the description), and write equivalent manual
    impls for the hand-rolled (non-macro) types. Implementation must inventory the affected population by grepping
    `impl Serialize for` / `impl<'de> Deserialize` across every crate the payloads reach — known members today:
    `Capability`, `EnrollmentStatus` (`capabilities.rs`), `UpdateCategory` (`shared/types`); the grep is the
    authoritative list (the named types are examples, not an exhaustive or mutually-exclusive enumeration).
    Implementation constraint: each manual impl's known-value list must be derived programmatically from the same
    source the `Serialize` impl uses (variant iteration / shared const table), never a hardcoded string list — a
    hardcoded list drifts silently because the golden gate cannot see self-derived output disagreeing with itself.
    `uptrakit-shared-macros` is therefore a touched crate.
  - Deserialize-only `#[serde(alias)]` attributes are not represented — moot, because this spec removes them (§4).
- Rustdoc comments (`///`) on payload structs and fields flow into schema `description` fields automatically via
  schemars, so per-field documentation lives in code and regenerates with it.

### 2. Document assembly (hand-assembled via serde_yaml_ng)

- The `#[cfg(all(test, feature = "schema"))]` module `spec_gen` in `uptrakit-wire` assembles the complete AsyncAPI
  3.0.0 document in Rust and serializes it with `serde_yaml_ng` (already a wire-crate dev-dependency, which covers
  test-only code — no manifest change for it). **No net-new external dependency.** Document content: `info` (short description pointing
  at `docs/api/wire-protocol.md` for narrative), the `/api/v1/ws/service` channel, send/receive operations for the two
  message directions, `components/messages` (one entry per real enum variant, `const`-discriminated, snake_case keys
  matching the serde wire strings) and `components/schemas` (schemars output, shared `$defs` hoisted). The **entire**
  `asyncapi.yaml` becomes generated output; there is no hand-edited region left in the file. A generated-file header
  comment names the regen command. Output must be deterministic (byte-identical across reruns) — verified in §Verification.
- **Named implementation risk — `$ref` namespace translation.** schemars emits definitions under `$defs` with
  `$ref: "#/$defs/Foo"` (JSON Schema 2020-12 dialect); AsyncAPI `components/schemas` requires
  `$ref: "#/components/schemas/Foo"`. The hoisting step is a translation, not a move — and broken refs are still
  deterministic, so the golden gate cannot see them. Guard with a first-party test: the generated document contains no
  `#/$defs/` occurrence and every `$ref` string resolves to an existing key within the document. Verify schemars
  1.2.1's actual ref path and emitted dialect keywords (`$schema`, 2020-12-isms) during implementation rather than
  assuming.
- The envelope-flatten layout must be preserved as documented today: each message payload schema presents one flat
  object carrying `protocol_version`, `seq`, `trace_context`, optional `pagination` (service direction), `type`
  discriminator, and the payload fields — matching what `#[serde(flatten)]` actually puts on the wire.
- **asyncapi-rust 0.4.0 was evaluated as the assembler and rejected on verified evidence** (initially preferred for
  consistency with the web-api code-first approach). Two blockers, both confirmed by reading its published source:
  (a) `ToAsyncApiMessage` operates on the tagged enum alone, while the envelope fields live on the wrapping
  `ServiceEnvelope`/`ControllerEnvelope` structs — the flattened envelope layout cannot be expressed; (b) its codegen
  reads only per-variant `#[serde(rename = ...)]` and has no `rename_all` handling at all, so both enums'
  `rename_all = "snake_case"` discriminants and message-map keys would emit as bare PascalCase variant identifiers
  (`"Ping"`, not `"ping"`). Adding 63 per-variant `rename` attributes to work around (b) is rejected — reshaping wire
  code to fit a 0.x doc tool. Supporting signals: single maintainer; ~7 months of dormancy followed by two breaking
  releases within hours on one day (2026-06-19); 0.4.0 roughly a month old at spec time. **Reversal condition** (ADR
  records it): if a future asyncapi-rust release gains `rename_all` support and an envelope/flatten mechanism,
  re-evaluate replacing the hand assembly; until then it is not registered in the workspace at all.

### 3. Golden staleness gate (web-api idiom)

- New test in `uptrakit-wire` (feature-gated on `schema`), named with the `asyncapi_` prefix: normal run asserts the
  committed `asyncapi.yaml` is byte-identical to the generated document; with `UPDATE_ASYNCAPI=1` set it rewrites the
  file instead — exactly the `UPDATE_OPENAPI=1` pattern in `crates/ui/web-api`.
- New `scripts/regen-asyncapi.sh` mirroring `scripts/regen-api.sh`:
  `UPDATE_ASYNCAPI=1 cargo test -p uptrakit-wire --all-features asyncapi_` — `--all-features` (not `--features schema`)
  deliberately mirrors `regen-api.sh` and future-proofs against the "legitimate feature subset" false-failure trap the
  web-api golden test documents, should `uptrakit-wire` ever grow another spec-affecting feature.
- Enforcement scope (stated per the feature-gate lesson): CI exercises the golden test through the coverage job's
  `cargo llvm-cov --workspace --all-features` run (the documented equivalent of the canonical `cargo test
--all-features` gate); the pre-push hook's `cargo nextest run --no-default-features --features db-sqlite` does
  **not** compile the `schema` feature, so pre-push will not catch staleness — CI is the enforcing gate. The command
  that exercises the gate locally is `cargo test -p uptrakit-wire --all-features asyncapi_`.

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

Retire the `AsyncApiSpec` validator and all `spec_conformance_*` tests in `src/tests.rs` — scoped precisely: the
retirement rationale ("testing upstream crate behavior") applies to **derived** schemas only. The envelope wire-shape
serialization tests (flat JSON layout, seq/protocol_version presence) stay — they pin the on-wire contract independent
of schemars. **New first-party tests this spec adds** (the manual impls are new logic, per the cover-new-logic rule):

- per wire-safe/hand-rolled enum with a manual `JsonSchema` impl (`Capability`, `EnrollmentStatus`, `UpdateCategory`,
  plus the `wire_safe_enum!`-generated impls): assert the generated schema is an open `string` with **no** `enum:`
  array — the exact regression (closed enum where open string is required) the manual impls exist to prevent;
- the `$ref`-resolution test from §2 (no `#/$defs/`, all refs resolve).

Trust-model change stated honestly (see §Decision): the retired tests' residual change-detection value (a hard-red on
an accidental serde change for sampled messages) is replaced by the golden gate's diff-review model, not by an
equivalent hard-red; this matches the repo's accepted `openapi.json` regime. `docs/development/testing.md` currently
describes the sample-based mechanism and overstates its coverage; that section is rewritten to describe the golden
gate and this trust model (correct-in-place, not a rewrite of the whole doc).

### 7. Out-of-scope guard

`protocol_version` stays at 1: the canonical serialized format does not change (alias removal only drops deserialize
leniency for names no released peer sends). `WireValidate`, `limits.rs`, pagination, and all runtime wire behavior are
untouched. No frontend, controller, or service code changes beyond the wire crate's derive annotations.

## Deliverables

1. **Zero net-new workspace dependencies**: schemars reused from the existing `schemars = { version = "1" }` root
   entry; `serde_yaml_ng` stays a plain dev-dependency (test-only generator code needs no optional prod entry).
   Crate manifests use `workspace = true`. `cargo deny check` must pass.
2. `uptrakit-wire`: `schema` feature, `JsonSchema` derives + overrides, `spec_gen` module, golden test, alias removal,
   conformance-test retirement. `uptrakit-shared-macros`: `wire_safe_enum!` extended with the manual `JsonSchema`
   emission (§1).
3. Regenerated `crates/shared/wire/asyncapi.yaml` (fully generated; reviewed diff against the old file checked against
   the §Problem inventory: 12 previously missing messages present, stale fields gone, capability representation open).
4. `scripts/regen-asyncapi.sh`.
5. Docs:
   - `docs/adr/0029-asyncapi-codegen.md` — new ADR recording this decision, the rejected alternatives (including
     yaml deletion and asyncapi-rust with its reversal condition), the accepted schemars representation limits, the
     golden-gate trust model (staleness detection + diff review, not hard-red change detection), the `--all-features`
     compile-time cost of feature unification pulling schemars into `uptrakit-shared-types`/`uptrakit-surfaces`, and
     the standing obligation that any future custom `Serialize` impl in a schema-reached crate needs a matching manual
     `JsonSchema` impl, the one-time AsyncAPI-validator authoring gate and its residual tension (validity not
     re-checked per commit), and a note that a schemars minor bump changing output flips the golden test red into a
     reviewable regen diff (caught, not silent). Implementation verifies schemars 1.2.1's MSRV against the workspace
     `rust-version`.
   - `docs/api/wire-protocol.md` — absorbs migrated narrative prose (§5).
   - `docs/development/testing.md` — sample-validator section replaced by golden-gate description (§6).
   - `docs/development/quality-gates.md` **and** the AGENTS.md quick-start block — add the regen command, same commit
     (AGENTS.md maintenance rule).
   - `AGENTS.md` — wire invariant wording still holds ("documented in asyncapi.yaml"); fix the stale crate name in the
     layout tree (`uptrakit-internal-wire` → `uptrakit-wire`). No `docs/README.md` change: it carries no ADR index
     (verified — zero ADR mentions), and this spec does not introduce one.
6. Pending-specs tracker entry (working artifact, not committed).

## Verification

- `cargo test -p uptrakit-wire --all-features asyncapi_` green; rerunning `scripts/regen-asyncapi.sh` twice yields a
  byte-identical file (determinism).
- Full gates: `cargo fmt`, `cargo check`/`clippy` on both canonical feature sets, `cargo test --all-features`
  (workspace-wide `--all-features` includes `embed-frontend` — build `frontend/` first per the AGENTS.md note),
  `cargo deny check`, `markdownlint`.
- Generated-vs-old yaml diff reviewed against the §Problem inventory (the gap-fix proof).
- **AsyncAPI validity**: the first generated document is loaded through an AsyncAPI validator (AsyncAPI Studio or
  `@asyncapi/parser`) and passes — a one-time authoring gate, deliberately not CI (keeps Node out of Rust CI). The
  ADR states the residual tension honestly: the doc's fitness for AsyncAPI tooling is verified at authoring and on
  manual regen review, not per-commit. `scripts/regen-asyncapi.sh` prints a one-line reminder to re-run the validator
  whenever the message/schema set changes, so future regens have a prompt the diff review alone would not provide.
- The §2 `$ref`-resolution test and the §6 open-string manual-impl tests are green.
- Prose migration greps (§5), both directions.
- Workspace-wide grep for the removed alias strings returns hits only in this spec/ADR/history documents.

## Deferred / out of scope

- Publishing or rendering `asyncapi.yaml` anywhere (website, AsyncAPI Studio) — it remains an in-repo reference.
- Documenting NATS-side message flows in AsyncAPI — the yaml covers the service↔controller WebSocket protocol only, as
  today.
- Hardening beyond the golden gate (e.g. instance-validation of runtime messages against the generated schemas) — the
  golden gate plus type-derivation makes this redundant.
- Any `protocol_version` bump or wire behavior change.

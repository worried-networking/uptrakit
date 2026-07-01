# 0025 — Drift-Proof OpenAPI Parameters via Derived `IntoParams`

**Date:** 2026-07-01 **Status:** Accepted

## Context

The OpenAPI spec (`crates/ui/web-api/openapi.json`) is generated from `#[utoipa::path]` annotations and
then drives the committed frontend TypeScript client. For query parameters, the historical style declared
them twice: once as the handler's `Query<SomeStruct>` extractor (which does the actual deserialization),
and again as a hand-maintained `params(("field" = Type, Query, description = "…"))` list inside the
`#[utoipa::path]` attribute. These two lists can silently diverge.

The `openapi_json_is_up_to_date` golden test (`crates/ui/web-api/src/integration_tests/openapi_spec.rs`)
does **not** catch this class of bug. It only asserts that the committed `openapi.json` equals what the
current annotations regenerate — so if a `params(...)` block is missing a field the extractor accepts,
the handler and the committed spec stay mutually consistent (both wrong), the test passes, and the
generated client silently loses the parameter. This actually happened: `list_software_items`' manual
block omitted the `query` (name filter) and `plugin_type` params, so the frontend software name-filter
feature broke while every backend test stayed green. It was found only by an end-to-end test that
exercised real URL construction.

The same shape recurs for **enum schemas**: a hand-written `utoipa::PartialSchema` that hardcodes
`enum_values` can drift from the enum's serde wire strings.

As background, all existing handlers were migrated to the derived form (and the two remaining raw-route
query handlers converted) in the change set that preceded this ADR; this record fixes the convention so
it cannot regress.

## Decision

Author OpenAPI parameters and schemas so the spec is **derived from a single source of truth**, never
hand-maintained alongside it. Concretely:

- **Query / request params** use `params(<IntoParamsStruct>)` over the handler's `Query<Struct>`
  extractor — never a `params(("field" = …, Query, …))` list duplicating struct fields. Field `///`
  doc-comments are the parameter descriptions.
- **Derive gating** differs between the shared `uptrakit-web-api-types` crate (feature-gated) and local
  `uptrakit-web-api` route structs (unconditional) — coding-standards has the exact form.
- **Path params** stay inline; a handler with both keeps its Path tuples inline and adds the `IntoParams`
  struct as a further entry in the same `params(...)` block.
- **Enum schemas** source `enum_values` from one place — `Self::all()` (`strum::EnumIter`) or the
  `wire_safe_enum!` macro's `$wire` list; an `Other(String)` catch-all enum that can't derive `EnumIter`
  hardcodes its values but is paired with a guard test asserting schema == `as_str()` set.

The mechanics, gating, and canonical examples live in
[`docs/development/coding-standards.md`](../development/coding-standards.md) ("OpenAPI parameter & schema
authoring (drift-proof)") — not duplicated here.

**Enforcement:** a CI guard, `ci/verify_no_inline_query_params.sh` (wired into `.husky/pre-push` and the
`semantic-boundary` job of `.github/workflows/ci.yml`), fails on any inline `Query`-param tuple in
`crates/ui/web-api/src`, with an allowlist for a genuine non-`Query<Struct>`-backed exception. This turns
the convention from a review expectation into a deterministic gate.

## Alternatives Considered

1. **Keep hand-maintained inline params** — rejected. It is the drift source itself, and the golden test
   is structurally blind to it (both sides regenerate from the same wrong annotation).
2. **Add only a drift-guard test that compares handler params to the struct** — rejected. Rust has no
   field reflection, so materializing "the params the struct implies" requires deriving `IntoParams`
   anyway; once derived, `params(<Struct>)` is the fix and the manual list disappears.
3. **Derive `IntoParams` + a grep CI guard** (chosen) — the spec parameters are compile-time-tied to the
   struct, and the guard prevents a new hand-maintained block from re-introducing the drift.

## Consequences

- New endpoints author query/request params via `params(<IntoParamsStruct>)`; existing ones migrate when
  touched. Parameters are compile-time-coupled to the extractor struct.
- Reviewers and CI reject new inline `Query`-param tuples; a legitimate non-`Query<Struct>` exception is
  an explicit, reviewed allowlist entry.
- The one case that cannot auto-source its schema — an `Other(String)` catch-all enum (no `EnumIter`) —
  stays hardcoded but is test-guarded, so it fails CI on drift rather than shipping a wrong spec.
- Descriptions move from the `#[utoipa::path]` attribute to struct-field `///` doc-comments, keeping the
  documentation next to the type.

# OpenAPI Client Drift Guard — Design

**Date:** 2026-07-01 **Status:** Draft (pending plan) **Crate:** `uptrakit-openapi-client`
(`crates/shared/openapi-client/`)

## Problem

`uptrakit-openapi-client` is a hand-written typed HTTP client (~38 `impl UptrakitClient` endpoint
modules + `paths.rs` + a `mock.rs` harness with typed section accessors). The frontend TypeScript
client was migrated to autogeneration from `crates/ui/web-api/openapi.json`, and the same
drift-reduction is wanted for the Rust client.

The Rust client's situation differs decisively from the frontend's: it already reuses the **same**
request/response structs the server serializes (`uptrakit-web-api-types` / `uptrakit-shared-types`),
so **type drift is impossible by construction**. The one real, unguarded risk is **endpoint
coverage drift**: the server adds an operation and no one adds a client method — a _silent
omission_ that produces no diff in the client crate for a reviewer to notice and no test failure.

## Goal

Close the endpoint-coverage-drift gap with the smallest, most maintainable mechanism, while
touching none of the properties the crate deliberately provides (type reuse, the `mock` feature,
SSE, retry, pagination, stable public method names, publishability, real docs.rs signatures).

## Chosen approach: a bidirectional coverage guard (no code generation)

Add the check as **`cargo xtask openapi-client-check`** — a subcommand of a new `xtask` umbrella
crate at `/xtask/`, realizing the pre-existing (currently dangling) `cargo xtask` alias in
`.cargo/config.toml`. It `walkdir`-walks + `syn`-parses the client source, reads `openapi.json`, and
asserts the client's _set of operations_ tracks the spec's; a non-zero exit fails CI. The client
stays hand-written; nothing is generated.

The repo currently has one comparable static-analysis gate, `audit-coverage-check`, as a
`crates/**/tools/<name>/` crate — the only working precedent. Rather than add a second tools-crate,
this **establishes the xtask convention** (the alias was already added in anticipation) and, as a
fast-follow in the same effort, **migrates `audit-coverage-check` into xtask** so the repo converges
on a single dev-tooling home (`cargo xtask <command>`). The guard's analysis logic reuses the same
building blocks the precedent used (`syn` visitors, `walkdir`, `ExitCode`).

### Why this over code generation (GAN-derived decision)

A scoped code generator was designed and then rejected after a Generator/Critic review grounded in
the actual code. Reasons:

- **Types already can't drift** (shared structs), so a generator would regenerate only the endpoint
  layer — and reuse of `web-api-types` (wire-safe `Other(String)`, `#[non_exhaustive]`, `&Uuid`,
  custom `Deserialize`/`Validate`) is exactly what any generator destroys.
- **The endpoint files are hand/generated hybrids at method granularity** — 24 of 39 carry
  co-located hand-written `#[cfg(test)] mod tests`, and the spec-absent `list_all_*` pagination
  companions are interleaved with generatable siblings. "Emit a file per group" clobbers them.
- **An override-manifest would absorb the interesting endpoints** — every list/query endpoint needs
  query-struct + return-type + section hints (utoipa flattens `IntoParams`, so the query-struct
  name is not recoverable from the spec), leaving the generator to autonomously own only trivial
  `get_x(id)`/`delete_x(id)` boilerplate. Net maintenance delta is plausibly negative.
- **Generated inherent methods must live in-crate**, subject to `warnings=deny`/`clippy::all=deny`
  with `#[allow]` banned and `#[expect]` fragile — no clean escape.

The guard delivers the actual drift-prevention (the silent-omission class) at a fraction of the
effort, with zero owned generated code. See ADR 0026.

### Rejected alternatives

- **Scoped code generation (committed `src/generated/` + override-manifest).** Rejected as above:
  hybrid-file clobbering, manifest absorbing the non-trivial endpoints, negative maintenance delta.
- **Off-the-shelf generators (progenitor, openapi-generator).** Generate their own types
  (duplicate `web-api-types`, drop wire-safe `Other(String)` / `#[non_exhaustive]` / `&Uuid`), no
  mock/SSE/retry, JVM or 3.1→3.0 friction (utoipa 5.5 emits 3.1 only; progenitor 0.14 is 3.0-only).
- **Do nothing (hand-written + review discipline).** Wins on any single PR but is structurally
  blind to _absence_: a spec-only endpoint yields no client diff and no failing test. The guard's
  bidirectional name-coverage is exactly the check review cannot do by eye.

## Architecture

```text
crates/ui/web-api/openapi.json          # source of truth, already CI-gated (ADR 0025)
crates/shared/openapi-client/src/*.rs   # hand-written client (unchanged)
        │
        ▼
xtask (bin+lib; clap dispatch; deps: clap + serde_json + syn(visit) + proc-macro2 + walkdir)
   └─ subcommand `openapi-client-check`  (module xtask::openapi_client_check)
        ├─ read openapi.json             -> set of (operationId, normalized path, method)
        ├─ walkdir+syn-parse client src/ -> pub async fn on `impl UptrakitClient`; paths.rs
        │                                   const/fn -> extracted+normalized path templates
        ├─ apply ledgers                 -> RENAME_MAP / SPEC_ONLY / CLIENT_ONLY / PATHS_CLIENT_ONLY
        └─ assert (§ Assertions)         -> non-zero exit + eprintln on any drift
        │
        ▼
CI backend-lint job: `cargo xtask openapi-client-check`  (alongside `cargo xtask audit-coverage-check`
                                                          after the fast-follow migration)
```

No `src/generated/`, no macros, no manifest, no new committed generated code. Paths are resolved
`current_dir()`-relative (invocation from workspace root is guaranteed by `cargo xtask …`) —
**not** via `cargo_metadata`, matching the precedent.

## Assertions (core)

1. **Name coverage (bidirectional).** Every spec `operationId` resolves — via `RENAME_MAP` or
   identity — to exactly one client method (a `pub async fn` on `impl UptrakitClient`), and every
   such client method resolves back to one `operationId` or a `CLIENT_ONLY` entry. Missing either
   direction fails. This is the load-bearing check (the silent-omission class). Roughly two-thirds
   of operationIds match a method name by identity today; the rest are `RENAME_MAP` or `SPEC_ONLY`
   (see Ledgers — these are one-time bootstrap tables, enumerated at implementation). The
   `list_all_<x>` pagination companions are **structurally auto-exempted** on the reverse direction —
   a `pub async fn list_all_<x>` needs no `CLIENT_ONLY` entry as long as a sibling `list_<x>`
   resolves to an operationId — so adding a paginated endpoint never requires a ledger edit.
2. **`paths.rs` ↔ spec (secondary, low-weight).** Every `paths.rs` const/fn maps to a real spec
   path template and vice versa, after **placeholder normalization**: both sides rewrite every
   `{name}` segment to a canonical `{}` before comparison (client fns name params `{item_id}` /
   `{target_id}` where the spec uses `{id}` — 7 such divergences exist, so exact-string comparison
   would false-fail). Path templates are extracted from each `paths.rs` `fn` by reading the first
   string-literal token of its `format!` body via `syn`. `PATHS_CLIENT_ONLY` lists spec-absent path
   consts (`health::HEALTHZ`, `events::STREAM`, the `surfaces` path fns). Catches a dead path const
   or an unrouted spec path; no verb check is claimed. If the `format!`-literal extraction proves
   fragile in practice it may be reduced to consts-only or dropped — assertion 1 is the primary
   guarantee.

### Explicitly NOT checked (core)

- **Param and response type/shape** — rides on **type reuse + the compiler**: a client method that
  takes a `web-api-types` query struct cannot silently omit a field the struct defines, and response
  types are the same structs the server serializes.
- **Mock-accessor coverage** — see Deferred. The `mock.rs` harness is **hand-maintained**; today it
  covers only ~half the client methods (18 endpoint categories have no mock section), so requiring
  parity would force a large mock build-out or an unmaintainable association table. Not in the core
  guard.

## Ledgers (the maintainability artifact)

Small, reviewed, self-invalidating tables in the tool crate (Rust `const` slices or a sidecar
`toml`). Exact contents are enumerated at implementation by diffing the real `openapi.json` against
the client's public methods; approximate scale is given so implementors size the bootstrap:

- **`RENAME_MAP`** (`operationId → method_name`, the largest — order of ~30 entries) — legitimate
  divergences between the two namespaces (spec `operationId`s vs the stable public method names the
  CLI depends on), e.g. `create_provider → create_oidc_provider`, `token → oauth_token`,
  `deactivate_service → remove_service`. New endpoints that match by identity need no entry; the
  map only records existing divergences.
- **`SPEC_ONLY`** (order of ~20 entries) — `operationId`s that intentionally have **no** client
  method: unimplemented feature areas (MFA/2FA, email-change, profile, instance-plugin admin,
  oauth/zeroconf global settings, config-state/reload) plus true non-client ops (OCSP, WebSocket,
  the `oidc_callback` 302 redirect). Semantics are **decoupled from encoding**: `healthz` (text) and
  `oauth_token` (form-urlencoded) _have_ methods and stay covered — they are **not** listed here.
- **`CLIENT_ONLY`** — `pub async fn` on `UptrakitClient` with no spec operation: `raw_request`, the
  three SSE stream methods (`stream_update_output`, `stream_events`, `stream_batch_progress`), the
  `unassign_host`-with-`ignore` variant, `healthz`, and the spec-absent `surfaces` methods. (The
  `list_all_*` companions are **not** listed — they are structurally auto-exempted per Assertion 1.)
- **`PATHS_CLIENT_ONLY`** — path consts/fns with no spec path (Assertion 2): `health::HEALTHZ`,
  `events::STREAM`, the `surfaces` path fns.
- **Hard error:** any name appearing in **both** `RENAME_MAP`/`SPEC_ONLY` and `CLIENT_ONLY` —
  double-booking would hide the pairing the guard protects. Unused ledger entries also fail, so the
  tables cannot rot silently.

## Deferred tiers (defined, not built)

- **Mock-accessor coverage** — assert every client method (minus streaming/raw) has a matching mock
  `on_*` accessor. Requires either completing `mock.rs` to full coverage or a hand-maintained
  method→`(section, on_*)` association table (the naming is not derivable: `list_hosts` →
  `hosts().on_list()`). Build once the mock surface is intentionally completed; until then the mock
  is declared hand-maintained.
- **Param-coverage** — `syn`-parse the `web-api-types` query struct a method uses, resolve its
  serde field names (`rename`/`rename_all`/`flatten`/`skip` aware), assert they **cover** the spec
  operation's query params, catching the residual ADR-0025 class (a method rewired to a _narrower_
  struct). Type reuse already makes the dangerous form structurally impossible, so build this only
  if a **second real param-drift incident** justifies the serde machinery. A documented known gap.

## Where it runs

- CI `backend-lint` job (the precedent's home), added as `cargo xtask openapi-client-check`,
  non-zero fails. The migrated `cargo xtask audit-coverage-check` runs in the same job.
- Behavior is validated by fixture-based tests in `xtask/tests/` — lib-function tests over inline
  fixtures plus one real-workspace check (running `cargo xtask openapi-client-check` against the
  committed spec + client), mirroring `audit-coverage-check`'s test architecture.

Adding a new client endpoint: implement the method (+ `paths.rs` const), or the guard fails.
Renames go in `RENAME_MAP`; intentional non-client ops go in `SPEC_ONLY`. All ledger edits are
small and reviewed.

## Crate structure (xtask umbrella)

The `xtask` crate at `/xtask/` is `[lib]` + `[[bin]]` (both targets in one crate, like
`audit-coverage-check`), added to `workspace.members`, **no `publish` field**, inheriting
`[workspace.lints]`. The existing `.cargo/config.toml` alias `xtask = "run --package xtask --"` makes
`cargo xtask <command>` work.

- `src/main.rs` — `clap`-derived subcommand dispatch; maps each command's `Result` to an `ExitCode`.
- `src/lib.rs` — `pub mod openapi_client_check;` (+ `pub mod audit_coverage_check;` after migration).
- `src/openapi_client_check/` — `mod.rs` (`pub fn run(root) -> Result<Vec<Violation>>`) + `spec`,
  `client`, `ledgers`, `normalize`, `check` submodules.
- `tests/` — per-subcommand fixture tests + the real-workspace check.

## Standards conformance (snapshot-bound)

Checked against `.superpowers/standards-snapshot.md`:

- **Panic policy (rule 27):** production tool code uses **no `unwrap()`/`expect()`/`panic!()`**;
  errors propagate as `Result<T, String>` and `main()`/`cli()` map them to an `eprintln!` +
  non-zero exit, mirroring `audit-coverage-check`.
- **Error handling — documented deviation from rule 26:** rule 26 mandates `rootcause::Report` for
  the main codebase, but the sole dev-tool precedent (`audit-coverage-check`) uses `Result<T, String>`
  with no `rootcause` dep. The `xtask` crate follows the precedent so it is uniform after the audit
  migration (converting the working audit modules to `rootcause` would be churn/risk).
- **Lint suppression / lints:** inherits `[workspace.lints]` (`warnings=deny`, `clippy::all=deny`);
  no `#[allow]`; any suppression uses `#[expect(..., reason = "…")]`.
- **No change to the client's public surface, errors, or types** — the hand-written client and its
  `ClientError` / wire-safe enums are untouched.
- **Testing (rule 31):** the guard tests _our_ coverage invariant, not upstream crates; fixture
  tests cover parsing + ledger logic.
- **Dev-tooling idiom:** `xtask` umbrella (`[lib]`+`[[bin]]`, `clap` dispatch), `walkdir`+`syn`
  analysis, `current_dir()`-relative paths, `tests/` fixtures — reusing the `audit-coverage-check`
  building blocks; invoked `cargo xtask …` in `backend-lint`. The fast-follow migration converges
  the repo on one dev-tooling home.
- **Conventional Commits:** implementation commits use `feat(openapi-client)` / `build(xtask)` /
  `refactor(xtask)` / `ci` / `docs` scopes.

## Dependencies

**No new external dependency.** The `xtask` crate reuses workspace deps: `clap = "4"` (subcommand
dispatch, features `["derive"]`), `syn = { workspace = true, features = ["visit"] }`,
`proc-macro2 = { workspace = true }`, `walkdir = { workspace = true }`, `serde_json = "1"`, plus
`toml`/`serde` carried over when `audit-coverage-check` migrates in. (`cargo_metadata` is **not**
used — paths resolve from `current_dir()`; `prettyplease` from the earlier codegen design is gone.)

## Documentation deliverables

- **New ADR** (`docs/adr/0026-openapi-client-drift-guard.md`): records that the Rust client stays
  hand-written (affirming, not superseding, the `docs/development/openapi-client.md` "Hand-written
  instead of code-generated" decision), that code generation was evaluated and rejected (hybrid
  files, manifest absorbing non-trivial endpoints, negative maintenance delta, semantic-fidelity
  loss), and that a bidirectional coverage guard closes the endpoint-drift gap; notes the deferred
  mock-coverage and param-coverage tiers and their triggers; and records the **`xtask` umbrella** as
  the repo's dev-tooling convention (realizing the existing alias; `audit-coverage-check` migrated in).
- **Update** `docs/development/openapi-client.md`: extend "Keeping the client in sync" to describe
  the guard, the four ledgers, the both-lists hard error, and the placeholder-normalization rule;
  keep "Adding a new endpoint" and note the guard enforces coverage.
- **Update** `CONTRIBUTING.md` (or the linked dev doc): a short worked example for adding an
  endpoint, pointing at one real endpoint file as the template.
- **Update** the CI workflow (`backend-lint` job) to run `cargo xtask openapi-client-check` (and
  `cargo xtask audit-coverage-check` after migration).

## In scope: xtask convention + audit-coverage-check migration (fast-follow)

To avoid two dev-tooling conventions, this effort **establishes the `xtask` umbrella** and, as a
clearly-separated fast-follow, **migrates the existing `audit-coverage-check`** into it: move its
`src/` modules under `xtask::audit_coverage_check`, add the `audit-coverage-check` subcommand, move
its `tests/`, rewire its CI step to `cargo xtask audit-coverage-check`, and delete the old
`crates/shared/audit-log/tools/audit-coverage-check/` crate + its `workspace.members` entry. This is
behavior-preserving (the audit gate keeps failing on the same conditions) and is planned as distinct,
reviewable tasks after the guard lands green.

## Deferred / out of scope

- **All code generation** (scoped bespoke, off-the-shelf, generate-types-from-spec).
- **Mock-accessor coverage** and **param-coverage** — the two deferred tiers above.
- **crates.io self-containment / type snapshot** (the 2026-04-28 publishing plan) — orthogonal.
- **Verb-level `paths.rs` checking** and any method-body AST path resolution — deliberately excluded
  as brittle and low-value.
- **Migrating other dev scripts into xtask** beyond `audit-coverage-check` — not now.

## Open questions

None blocking. The exact `RENAME_MAP` / `SPEC_ONLY` / `CLIENT_ONLY` / `PATHS_CLIENT_ONLY` contents,
and whether the ledgers live as Rust consts or a sidecar `toml`, are settled during implementation
by diffing the real `openapi.json` operationIds and paths against the client's public methods and
`paths.rs`.

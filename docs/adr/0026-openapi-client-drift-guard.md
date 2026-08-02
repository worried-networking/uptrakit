# 0026 — OpenAPI Client Drift Guard (no code generation)

Date: 2026-07-01

## Status

Accepted

## Context

`uptrakit-openapi-client` is hand-written and reuses `uptrakit-web-api-types` verbatim, so client
request/response **type** drift is impossible by construction. The only unguarded risk is endpoint
**coverage** drift: the server adds an operation and no one adds a client method — a silent omission
with no client-side diff and no failing test.

## Decision

Add `cargo xtask openapi-client-check` — a subcommand of a new `xtask` umbrella crate (realizing the
pre-existing `.cargo/config.toml` alias) that `syn`-parses the client and reads `openapi.json`,
asserting bidirectional coverage (operationId ↔ method, `paths.rs` ↔ spec) via reviewed
`RENAME_MAP` / `SPEC_ONLY` / `CLIENT_ONLY` / `PATHS_CLIENT_ONLY` ledgers. It runs in CI
(`backend-lint`) and fails on drift. **The client stays hand-written; nothing is generated.**

To avoid two dev-tooling conventions, `xtask` becomes the repo's dev-tooling home and the existing
`audit-coverage-check` is migrated into it as a second subcommand (behavior-preserving).

Code generation was evaluated and **rejected**: shared types can't drift; the endpoint files are
hand/generated hybrids at method granularity (co-located tests, interleaved `list_all_*`); an
override-manifest would absorb the non-trivial endpoints, leaving a generator to own only trivial
`get_x(id)`/`delete_x(id)` boilerplate — a plausibly negative maintenance delta; and generated
inherent methods must live in-crate under `warnings=deny`/`clippy::all=deny` with `#[allow]` banned.

Param/type shape is intentionally not checked (rides on type reuse + the compiler). Two tiers are
defined but deferred: **mock-accessor coverage** (until `mock.rs` is intentionally completed) and
**param-coverage** (until a second real param-drift incident).

## Consequences

Adding an endpoint requires the method (+ `paths.rs` const) or a reviewed ledger entry; the
`list_all_<x>` companions are auto-exempted structurally. The guard closes the silent-omission gap
that review cannot see, with zero generated code. This **affirms** (does not supersede) the
"Hand-written instead of code-generated" note in `docs/development/openapi-client.md`. `xtask` is now
the home for repo dev-tooling gates.

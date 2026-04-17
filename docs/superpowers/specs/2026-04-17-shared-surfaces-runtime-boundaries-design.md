# Shared Surfaces Runtime Boundaries — Design

## Problem

The shared surfaces contract layer is carrying too much maintainability debt in two places.

First, registration validation has grown into large, centralized walkers that accumulate every
new rule and every new `SurfaceNode` branch. The result is high change risk: adding a new node,
new capability rule, or new reference type requires editing sprawling validation functions with
duplicated error construction and repeated traversal logic.

Second, the smaller public value APIs around the shared surfaces model are not consistently
expressing their contracts. Public fallible helpers often omit rustdoc `# Errors` sections, and
small validated types still miss common Rust affordances like `#[must_use]` and `const fn`
where they would strengthen call-site clarity.

This track addresses the shared contract layer itself plus a narrow shared-type slice that supports
those contracts, rather than the higher-level runtime dispatchers that consume them.

## Covered Findings

- Finding 3: Break up the surface registration validator.
- Finding 9: Apply small-type Rust idioms consistently within the first shared-contract hardening
  slice owned by this track.
- Finding 10: Document fallible public APIs with rustdoc error contracts in `uptrakit-surfaces`
  public APIs touched by this track.

## Goals

- Make surface registration validation modular enough that new rules can be added locally.
- Make public shared-surface APIs easier to consume correctly without reading implementation.
- Tighten small shared types so ownership intent and ignored-result hazards are explicit.
- Establish a documentation standard for public fallible APIs in `uptrakit-surfaces`.
- Establish the first reusable hardening slice for these idioms in `uptrakit-surfaces` plus the
  companion `uptrakit-shared-types` API touched here.

## Non-Goals

- No redesign of the surface JSON model itself.
- No transport or runtime dispatch changes in `web-api` or `agent-ssh`.
- No broad project-wide rustdoc rewrite outside the targeted shared crates in this track.
- No stylistic churn that does not improve API clarity or contract expression.

## Design

### 1. Modularize registration validation

`SurfaceRegistration::validate_against()` and the recursive root-node reference walk should stop
acting as the single home for every registration rule. The design target is a validation module
that separates:

- registration-level checks
- per-surface descriptor checks
- interaction/data-source contract checks
- root-node reference validation
- capability compatibility checks

The traversal logic should be shared rather than rewritten per rule. A visitor-style walk or a
small set of recursive helpers is acceptable as long as the rule boundaries become explicit and
new `SurfaceNode` support can be added without extending one giant function.

Error creation should also be normalized. Today many branches construct near-identical
`SurfaceRegistrationError` values inline. The design target is a smaller set of focused helpers
that keep message formatting close to the rule being enforced while removing repetitive boilerplate.

### 2. Treat public fallible APIs as documented contracts

Public constructors and validators in `crates/shared/surfaces` should document their failure
conditions with `# Errors` sections. This applies to identifier constructors, validation helpers,
and other exported `Result`-returning functions.

The target is not “more docs everywhere”; it is “the exported API tells callers what can fail
without requiring source inspection”. The enforcement mechanism should be explicit:
targeted `clippy::missing_errors_doc` coverage for the `uptrakit-surfaces` crate in scope, wired
into the relevant lint/CI command path for this track, such as:

```sh
cargo clippy -p uptrakit-surfaces --all-targets -- -D clippy::missing_errors_doc
```

### 3. Tighten small validated types

Small shared value APIs in `crates/shared/surfaces` and representative companion types in
`crates/shared/types` should express intent more clearly:

- `#[must_use]` on pure constructors, parsers, and validation predicates where ignoring the result
  is likely to be a mistake.
- `const fn` for trivial accessors when stable and semantically appropriate.
- `Copy` for small marker-like enums only where it materially simplifies usage and does not blur
  ownership semantics.

This track should prefer semantic tightening over blanket lint cleanup. The goal is clearer APIs,
not satisfying every pedantic suggestion.

## File Map

Primary files expected in scope:

- `crates/shared/surfaces/src/protocol.rs`
- `crates/shared/surfaces/src/ids.rs`
- `crates/shared/surfaces/src/data.rs`
- `crates/shared/surfaces/src/interaction.rs`
- `crates/shared/surfaces/src/surface.rs`
- `crates/shared/types/src/network.rs`

Likely supporting docs/config:

- `docs/development/coding-standards.md`
- `docs/development/rust-idioms.md` as cross-cutting guidance, not a track-owned design artifact
- relevant crate-level lint configuration if the team chooses enforcement

`crates/shared/types/src/network.rs` is in scope as the first representative companion API outside
`uptrakit-surfaces` for Finding 9, so the small-type idiom work does not collapse back to a
single-crate cleanup.

## Acceptance Criteria

- Registration validation is split across explicit rule-focused validators/helpers for
  registration-level, descriptor-level, interaction/data-source, root-node reference, and
  capability compatibility checks rather than one central walker carrying all rule branches.
- Exported fallible APIs in `uptrakit-surfaces` document their failure conditions with rustdoc
  `# Errors` sections.
- `clippy::missing_errors_doc` is adopted as the documented enforcement mechanism for the
  `uptrakit-surfaces` crate in this track, via:

```sh
cargo clippy -p uptrakit-surfaces --all-targets -- -D clippy::missing_errors_doc
```

- Public constructors, parsers, and validation predicates in the targeted shared crates that return
  owned values or boolean validity signals adopt `#[must_use]` unless an explicit documented
  exception exists.
- Trivial identifier-style accessors in scope adopt `const fn` where stable and semantically valid,
  and small marker-like enums in scope either derive `Copy` where it materially simplifies usage or
  have an explicit rationale for remaining non-`Copy`.
- Validation modularization preserves or expands targeted test coverage for registration rules and
  root-node reference checks rather than treating the refactor as structure-only.
- The resulting structure is documented in `docs/development/coding-standards.md`, with optional
  cross-cutting reinforcement in `docs/development/rust-idioms.md`, rather than left as tribal
  knowledge.

## Recommended Sequencing

This track can run independently of the typed-config track. If the work is serialized, plugin API
typing should still land first overall, but this track can then proceed in parallel with typed
config because it changes separate shared-contract code. It should land before the large runtime
decomposition work so the shared surface contracts are cleaner first.

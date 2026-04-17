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

This track addresses the shared contract layer itself rather than the higher-level runtime
dispatchers that consume it.

## Covered Findings

- Break up the surface registration validator.
- Apply small-type Rust idioms consistently in shared crates.
- Document fallible public APIs with rustdoc error contracts.

## Goals

- Make surface registration validation modular enough that new rules can be added locally.
- Make public shared-surface APIs easier to consume correctly without reading implementation.
- Tighten small shared types so ownership intent and ignored-result hazards are explicit.
- Establish a documentation standard for public fallible APIs in shared crates.

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
without requiring source inspection”. The design should be enforceable through lint configuration
or targeted CI checks for the shared crates covered by this track.

### 3. Tighten small validated types

Small shared value APIs should express intent more clearly:

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

Likely supporting docs/config:

- `docs/development/rust-idioms.md`
- `docs/development/coding-standards.md`
- relevant crate-level lint configuration if the team chooses enforcement

## Acceptance Criteria

- Surface registration validation is decomposed into smaller rule-focused units rather than one
  primary monolith.
- Adding a new `SurfaceNode` or validation rule requires editing a localized validator rather than
  threading behavior through one large function.
- Exported fallible APIs in the targeted shared crates document their failure conditions with
  rustdoc `# Errors` sections.
- Small shared value APIs adopt `#[must_use]`, `const fn`, and `Copy` only where they sharpen the
  contract and remain easy to understand.
- The resulting structure is documented as project guidance rather than left as tribal knowledge.

## Recommended Sequencing

This track should land after the plugin API typing and typed config boundary tracks have defined
their boundary expectations, but before the large runtime decomposition work. It hardens the shared
contracts that those runtime refactors will consume.

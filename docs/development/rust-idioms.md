# Rust Idioms

This document covers maintainability-oriented Rust practices for Uptrakit. It complements
[Coding Standards](coding-standards.md), which defines the mandatory baseline rules.

## Goals

- Keep ownership and control flow obvious.
- Push dynamic or stringly data to the edges.
- Prefer types that make invalid states unrepresentable.
- Split modules before they become dispatch engines or validation monoliths.
- Make public APIs self-documenting for both humans and tools.

## Module Design

- Prefer focused modules with one primary responsibility.
- Split files when they start combining transport, validation, orchestration, and data shaping.
- Prefer helper functions or submodules over giant `match` expressions and hundred-line methods.
- Keep boundary code thin. Put parsing, validation, and domain decisions in separately testable helpers.

Good candidates for extraction:

- Surface/action dispatchers that mix lookup, authorization, idempotency, transport, and response shaping.
- Validation functions that walk large enums and construct many near-identical errors.
- Service runtime files that contain registration, request parsing, background task spawning, and workflow execution.

## Typed Boundaries

- Prefer typed request/response/config structs over `serde_json::Value`.
- Prefer typed enums or newtypes over raw `String` mode flags and identifiers.
- If a boundary must stay dynamic for plugin extensibility, convert to typed data
  immediately after deserialization and keep the untyped representation out of
  business logic.
- Avoid `HashMap<String, serde_json::Value>` for internal settings snapshots. Deserialize into a typed struct instead.

Use raw JSON only at the edge:

1. Receive dynamic payload.
2. Validate and deserialize once.
3. Operate on typed Rust values internally.
4. Serialize again only when returning to a dynamic boundary.

## Error Types

- Prefer typed error enums over `Result<T, String>` for reusable library boundaries.
- Convert typed errors into user-facing strings only at UI, CLI, or plugin interop edges.
- Preserve context with `rootcause::Report`, `report!`, `bail!`, and `.context_to()?`.
- When an API is intentionally user-facing, document that the returned string is presentation text, not a stable programmatic contract.

## Type Erasure

- Avoid `dyn Any` in core extension points unless there is no practical alternative.
- Prefer narrow traits that expose exactly what an extension needs.
- If type erasure is unavoidable, isolate downcasts inside one adapter layer and keep the rest of the code strongly typed.

## Async API Shape

- Avoid repeating raw `Pin<Box<dyn Future<...>>>` signatures across traits.
- Introduce shared aliases such as `type SurfaceActionFuture<'a> = ...` and `type SurfaceActionResult = ...`.
- Prefer typed request/response structs for async trait boundaries instead of several loosely related parameters.

## Small Value Types

- Add `#[must_use]` to pure constructors, parsers, validation predicates, and value-transforming helpers.
- Derive `Copy` for small C-like enums that are cheap to copy and frequently passed around as markers.
- Use `const fn` for trivial accessors and constructors when available.
- Prefer newtypes for validated identifiers and domain-specific strings.

## Reuse Over Repetition

- When multiple plugins or services implement the same command pipeline shape, extract a shared helper or trait template.
- Prefer iterator pipelines and small mapping helpers over repeated hand-written `Vec<String>` construction and repeated error fan-out code.
- Keep parsing logic and command execution logic separate so each can be reused and tested independently.

## Numeric Conversions

- Avoid unchecked `as` casts for time, size, and count conversions.
- Prefer `try_from`, saturating conversions, or explicit bounds checks.
- Log or clamp intentionally when truncation is acceptable; do not rely on implicit narrowing.

## Rustdoc Contracts

- Public functions that return `Result` should include a `# Errors` section.
- Public functions that can panic should include a `# Panics` section.
- Document invariants for newtypes, builder methods, and validation helpers.
- Keep doc comments aligned with the type-level contract, not just the current implementation.

## Clippy Usage

- `cargo clippy` is the baseline.
- For touched shared crates or reusable libraries, run a targeted pedantic pass when practical:

```sh
cargo clippy -p <crate> --all-targets -- -W clippy::pedantic -W clippy::nursery
```

- Treat pedantic findings as review input, not as blindly applied style churn.
- When suppressing a lint, leave a short comment explaining why the non-idiomatic shape is intentional.

## Review Checklist

- Did this change reduce or increase type erasure?
- Did any new `serde_json::Value`, `HashMap<String, Value>`, or `Result<_, String>` cross an internal boundary?
- Did any module become responsible for multiple independent concerns?
- Are helper return values hard to ignore when ignoring them would be a bug?
- Are public fallible APIs documented with `# Errors`?
- Did any new cast rely on `as` where `try_from` would be clearer?

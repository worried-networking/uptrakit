# Code Review: `uptrakit-shared-macros`

**Date:** 2026-02-17
**Reviewer:** Claude Opus 4.6 (automated)
**Scope:** Architecture, code quality, macro correctness, coding standards
**Overall quality: GOOD -- correct macro, but lacks dedicated tests**

---

## Architecture

The crate exports a single declarative macro (`impl_report_conversion!`) that generates `rootcause::ReportConversion`
implementations. It is the foundation of the project's cross-crate error propagation strategy, used at 30+ call sites
across the workspace.

Three arms:

1. **Simple variant mapping:** `impl_report_conversion!(SourceError => TargetError::Variant)`
2. **Closure-based transform:** `impl_report_conversion!(SourceError => TargetError, |e| TargetError::Variant(...))`
3. **Multiple at once:** comma-separated list (simple variant only)

---

## Findings

### PASS: Macro hygiene and path resolution

The macro uses fully qualified absolute paths (`rootcause::ReportConversion`, `rootcause::prelude::markers::Mutable`,
etc.) for rootcause types -- correct since `rootcause` is a dependency of this crate, not `$crate` itself. The recursive
multi-arm uses `$crate::impl_report_conversion!` for proper self-invocation.

### PASS: Generated `ReportConversion` impls match the trait exactly

Cross-referenced against the actual `rootcause` source. The macro hardcodes `O = Mutable`, which is correct because
`IntoReport` always produces `Mutable` reports. Both simple-variant and closure arms generate type-correct
implementations.

### PASS: Multi-segment source type paths handled correctly

The `$source:ty` fragment specifier correctly handles paths like `tokio_tungstenite::tungstenite::Error`. Confirmed by
widespread usage.

### LOW: `$target:ident` prevents multi-segment target paths

The fragment specifier only accepts a single identifier (e.g., `MyError`), not paths like `module::MyError`. Acceptable
given that `ReportConversion` impls are always written where the target type is defined.

### LOW: Closure-based arm lacks "Expands to" example

The simple variant arm has an "Expands to" section in the doc comment. The closure arm does not. Adding one would help
readability.

### LOW: Implicit requirement on `rootcause` dependency name

The macro requires downstream crates have `rootcause` under that exact name. If a crate renames the dependency, the
macro fails. Acceptable given workspace conventions, but worth a brief doc note.

### INFORMATIONAL: All doc examples use `/// ```ignore`

Doc examples are not compiled or tested. Understandable since the macro depends on `rootcause` types unavailable in
standalone doc-test context, but means examples could drift out of sync.

---

## Summary

| Category               | Status     | Notes                                          |
| ---------------------- | ---------- | ---------------------------------------------- |
| Macro correctness      | PASS       | Generated impls match trait exactly             |
| Macro hygiene          | PASS       | Correct path resolution, proper `$crate` usage |
| Edge case handling     | PASS       | Trailing commas, single-element multi-arm work  |
| Test coverage          | PASS       | Dedicated tests added for all three macro arms  |
| Documentation          | PASS       | Closure limitation documented in macro doc      |
| `unwrap`/`panic`       | N/A        | Macro crate; no runtime code                   |

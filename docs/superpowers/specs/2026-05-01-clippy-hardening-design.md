# Clippy Hardening

## Background

The workspace already sets `clippy::all = "deny"`, covering the five default lint groups
(correctness, suspicious, style, complexity, performance). This baseline is solid but leaves
high-value lints from the `restriction` and `pedantic` groups entirely unenforced.

Two gaps motivated this work:

1. **Mechanical panics.** The coding-standards.md already forbids `unwrap()`, `expect()`, and
   `panic!()` in production code. Enforcement is manual today — the lints close that gap.
2. **Stale suppressions.** All 95 existing `#[allow(...)]` attributes carry no rationale. A
   suppression without a reason is as bad as no lint: it silences the tool without recording
   why. Migrating to `#[expect(..., reason = "...")]` fixes this and additionally catches stale
   suppressions — a site where the lint no longer fires becomes a compile error.

## Scope

One PR. Sequence within the PR:

1. Add `clippy.toml` with test-mode exemptions.
2. Add `unfulfilled_lint_expectations = "deny"` to `[workspace.lints.rust]`.
3. Add `[lints] workspace = true` to `crates/shared/github-client/Cargo.toml` and
   `crates/ui/mcp/Cargo.toml` (currently missing).
4. Add all new lints to `[workspace.lints.clippy]` at `"warn"`.
5. Fix every violation or add `#[expect(..., reason = "...")]`. New suppressions use `#[expect]`
   from the start — step 4 enables `allow_attributes = "warn"`, so any `#[allow]` added here
   immediately warns.
6. Convert all 95 existing `#[allow(...)]` sites. Most become `#[expect(..., reason = "...")]`;
   see **Known Exceptions** below for sites that must retain `#[allow]`.
7. Promote all new lints from `"warn"` to `"deny"`.
8. Update `docs/development/coding-standards.md`.

## `clippy.toml` (new file, workspace root)

```toml
allow-unwrap-in-tests    = true
allow-expect-in-tests    = true
allow-panic-in-tests     = true
allow-dbg-in-tests       = true
allow-indexing-slicing-in-tests = true
```

These suppress the five high-noise restriction lints inside `#[cfg(test)]` modules and
`#[test]` functions without per-site `#[expect]` annotations.

Note: there is no `allow-unreachable-in-tests` key — `allow-panic-in-tests` suppresses `panic!`
but not `unreachable!`. Any `unreachable!()` calls inside test functions require a per-site
`#[expect(clippy::unreachable, reason = "...")]`.

## Lint Additions (`[workspace.lints.clippy]`)

All lints target `"deny"` as the final state. They are added at `"warn"` only during the
remediation pass, then promoted.

### Panic Prevention

| Lint               | What it catches                                                 |
| ------------------ | --------------------------------------------------------------- |
| `unwrap_used`      | `.unwrap()` on `Option` or `Result`                             |
| `expect_used`      | `.expect()` on `Option` or `Result`                             |
| `get_unwrap`       | `.get(i).unwrap()` — prefer `.get(i)?` or explicit error        |
| `unwrap_in_result` | `.unwrap()` or `.expect()` inside a function returning `Result` |
| `panic`            | explicit `panic!()` macro                                       |
| `todo`             | `todo!()` placeholder                                           |
| `unimplemented`    | `unimplemented!()` placeholder                                  |
| `unreachable`      | `unreachable!()` — must document the invariant via `reason`     |
| `indexing_slicing` | `arr[i]` or `&arr[a..b]` — panics on bad index                  |
| `string_slice`     | `&str[a..b]` — panics if index is inside a UTF-8 character      |

**Actual violation count** (measured via `cargo clippy --force-warn`): ~15 for
`unwrap_used` + `expect_used` combined; 5 for `indexing_slicing`; 5 for `string_slice`.
`panic`, `todo`, `unimplemented`, `unreachable`, `get_unwrap`, `unwrap_in_result`: 0–1 each.

**Double-fire interaction:** `unwrap_in_result` fires on the same expression as `unwrap_used`
(and `expect_used`) when `.unwrap()` / `.expect()` is called inside a `Result`-returning
function. Both lint names must appear in the `#[expect]` attribute:

```rust
#[expect(clippy::unwrap_used, clippy::unwrap_in_result, reason = "...")]
```

The `unreachable` lint does **not** conflict with the `#[non_exhaustive]` wildcard-arm pattern —
those arms call `tracing::warn!`, not `unreachable!`. The 11 production `unreachable!()` calls
are invariant assertions that each warrant a `reason`.

**Excluded:** `arithmetic_side_effects` (~85% noise in practice), `panic_in_result_fn` (already
covered by `panic`).

### Silent Failure Prevention

| Lint                          | What it catches                                           |
| ----------------------------- | --------------------------------------------------------- |
| `map_err_ignore`              | `.map_err(\|_\| …)` — discards the root-cause error       |
| `let_underscore_future`       | `let _ = async_fn()` — drops the future without awaiting  |
| `let_underscore_must_use`     | `let _ = must_use_value` — silently discards              |
| `unused_result_ok`            | `result.ok()` where the `Err` is meaningful               |
| `assertions_on_result_states` | `assert!(r.is_ok())` — hides the error message on failure |

Note: `let_underscore_future` is in `clippy::suspicious` and is already covered by
`clippy::all = "deny"`. It is listed here for completeness; the `#[expect]` migration applies
to any suppression sites regardless.

**Actual violation count:** 4 for `map_err_ignore`; 0 for the rest.

### Async Correctness

| Lint            | What it catches                                            |
| --------------- | ---------------------------------------------------------- |
| `large_futures` | `Future` large enough to risk a stack overflow when polled |

`large_futures` is in `clippy::pedantic` and is specified individually rather than enabling the
full pedantic group. `await_holding_lock` and `await_holding_refcell_ref` are already in
`clippy::suspicious` (covered by `all = "deny"`).

**Actual violation count:** 0.

### Memory and Unsafe Hygiene

| Lint                            | What it catches                            |
| ------------------------------- | ------------------------------------------ |
| `mem_forget`                    | `mem::forget(x)` — intentional memory leak |
| `undocumented_unsafe_blocks`    | `unsafe {}` without a `// SAFETY:` comment |
| `multiple_unsafe_ops_per_block` | more than one unsafe operation per block   |

17 `unsafe {}` blocks exist; 13 already have `// SAFETY:` comments. The `--force-warn` run
measured 2 violations (some crates failed to compile due to cascading errors, so the true
count may be slightly higher — at most 4).

**Actual violation count:** 2–4 for `undocumented_unsafe_blocks`; 0 for the rest.

### Numeric Correctness

| Lint        | What it catches                          |
| ----------- | ---------------------------------------- |
| `float_cmp` | direct `==` comparison on `f32` or `f64` |

`float_cmp` is in `clippy::pedantic`. **Actual violation count:** 0. Included for future-proofing.

**Excluded:** `cast_sign_loss`, `cast_possible_truncation`, `cast_possible_wrap`,
`cast_precision_loss`. The codebase has 213 numeric casts, mostly in SeaORM pagination and
query layers. Values are bounded by domain invariants (host counts, pagination offsets) that
are not near numeric limits. The noise-to-signal ratio is too high.

### Suppression Hygiene

| Lint                              | What it catches                                         |
| --------------------------------- | ------------------------------------------------------- |
| `allow_attributes`                | any `#[allow(...)]` — must use `#[expect(...)]` instead |
| `allow_attributes_without_reason` | `#[allow]` or `#[expect]` without `reason = "..."`      |

Both lints fire simultaneously on a bare `#[allow(foo)]` — one for using `#[allow]` at all,
one for the missing reason. This is expected; converting to `#[expect(foo, reason = "...")]`
clears both.

`#[expect(lint, reason = "...")]` is identical to `#[allow(lint)]` at runtime. Stale
suppressions are caught by `unfulfilled_lint_expectations`. This PR adds
`unfulfilled_lint_expectations = "deny"` explicitly to `[workspace.lints.rust]` to make the
guarantee independent of the `warnings = "deny"` umbrella setting.

Requires Rust ≥ 1.81; the workspace is on 1.95.

### Hygiene

| Lint        | What it catches                                                 |
| ----------- | --------------------------------------------------------------- |
| `dbg_macro` | `dbg!()` left in production code                                |
| `rc_mutex`  | `Rc<Mutex<T>>` — `Rc` is single-threaded, defeating the `Mutex` |

**Actual violation count:** 0 for both.

## Known Exceptions

Two patterns of `#[allow]` site cannot be mechanically converted to `#[expect]` and must retain
the `#[allow]` form. These are **feature-conditional sites** where the suppressed lint fires only
under some feature flag combinations — an `#[expect]` that is unsatisfied under the other
feature variant becomes an `unfulfilled_lint_expectations` compile error.

The implementation step must audit ALL `#[allow]` sites against the feature matrix before
migrating. Run:

```bash
grep -rn '#\[allow(' crates --include='*.rs'
```

Then for each site check whether surrounding code is inside a `#[cfg(feature = "...")]` block
or in a `macro_rules!` body — both make the lint fire selectively.

### Pattern: feature-conditional sites

Applies to sites where a lint fires only when a specific feature is **disabled** (e.g., a
variable is only used inside `#[cfg(feature = "foo")]`). Known examples:

- `crates/plugins/infrastructure/core/src/macros.rs:78,187,191` — inside `declare_plugin!` macro body; lints fire selectively at call sites by feature
- `crates/shared/service-sdk/src/lifecycle.rs:132` — `unreachable_code` when `zeroconf` is active
- `crates/ui/web-api/src/batch_progress_broadcaster.rs:110` — `unreachable_code` when `nats` is active
- Several `unused_mut` / `unused_assignments` / `unused_variables` sites in `controller-runtime`,
  `agent-core`, `docker`, `settings_global_combined` where assignments only occur inside
  `#[cfg(feature = "...")]` blocks

These sites retain `#[allow]` with a wrapping `#[expect]` that covers BOTH suppression lints:

```rust
#[expect(
    clippy::allow_attributes,
    clippy::allow_attributes_without_reason,
    reason = "feature-conditional: lint fires only when <feature> is disabled; #[expect] fails under the variant where it does not fire"
)]
#[allow(unused_mut)]
```

### Pattern: `cfg_attr` allow sites

Sites using `#[cfg_attr(not(feature = "..."), allow(lint))]` must be converted to use `expect`
inside the `cfg_attr` — `allow_attributes` fires on the inner `allow` token:

```rust
// Before
#[cfg_attr(not(feature = "foo"), allow(unused_mut))]

// After
#[cfg_attr(not(feature = "foo"), expect(unused_mut, reason = "only mutated inside #[cfg(feature = \"foo\")] block"))]
```

This form is valid since Rust 1.81 (`#[expect]` stabilized). Known `cfg_attr` sites exist in
`controller-runtime/src/lib.rs` (lines 276, 323, 376, 792, 794) and the docker plugin crate.

## `#[expect]` Migration

All 95 existing `#[allow(...)]` sites are converted to `#[expect(..., reason = "...")]`.
Breakdown:

- 22× `clippy::too_many_arguments` — reason pattern: `"mirrors the N fields of <record type>"`
- 13× `clippy::type_complexity` — reason pattern: `"SeaORM join chain; extracting a type alias
adds no clarity here"`
- 25× `dead_code` — reason documents why the item is kept (feature-gated, test helper, etc.)
- 16× `unused_*` (imports, variables, mut) — reason documents the cfg condition or workaround
- 19× remainder — site-specific reasons

New suppressions introduced during remediation follow the same `#[expect]` pattern.
The `#[allow]` form is banned workspace-wide by `allow_attributes = "deny"`.

## Coding Standards Update

`docs/development/coding-standards.md` gains a new **Lint Suppression** section (after
**Panic Policy**):

> Use `#[expect(lint_name, reason = "...")]`, never `#[allow(lint_name)]`. The `reason`
> field is mandatory (`allow_attributes_without_reason = "deny"`). When the lint stops
> firing at a site, the `#[expect]` becomes a compile error via `unfulfilled_lint_expectations`
> (promoted to error by `warnings = "deny"`), so stale suppressions are caught automatically.
>
> ```rust
> // ✓ Correct
> #[expect(clippy::too_many_arguments, reason = "mirrors the eight DB columns of Update")]
> fn create_update_record(…) { … }
>
> // ✗ Wrong — no reason, and will silently persist if the lint is fixed
> #[allow(clippy::too_many_arguments)]
> fn create_update_record(…) { … }
> ```
>
> When two lints fire on the same expression, list both in one attribute:
>
> ```rust
> #[expect(clippy::unwrap_used, clippy::unwrap_in_result, reason = "infallible: regex compiled from a literal")]
> let re = Regex::new(PATTERN).unwrap();
> ```

## Verification

Run both feature variants throughout the conversion pass (not just at the end) — `cfg_attr`
converted sites and feature-conditional exception sites may produce `unfulfilled_lint_expectations`
errors under one variant but not the other:

```bash
# Both commands must be clean after every batch of conversions:
cargo clippy --all-targets --no-default-features --features db-sqlite
cargo clippy --all-targets --all-features

# Tests must still pass (clippy.toml test exemptions let test .unwrap()/.expect() through):
cargo test --all-features
```

Expected outcome: zero warnings, zero errors from Clippy on all targets.

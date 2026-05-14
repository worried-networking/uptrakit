# Clone Reduction Refactors

**Date:** 2026-05-02
**Status:** Implemented (2026-05-14)

## Background

The workspace had 2500+ `.clone()` calls. The majority are cheap: `Arc`/SeaORM handle clones before
`tokio::spawn` are O(1) and correct. Three areas had avoidable heap allocations:

1. SeaORM model maps iterated by reference, forcing a full-model clone before `ActiveModel` conversion.
2. `Vec<Uuid>` cloned to satisfy `is_in()` ownership when only iteration is needed.
3. `Option::or(x.clone())` eagerly cloning when the receiver is already `Some`.

A fourth micro-fix (`FrameworkGenerationRange` missing `Copy`) was included as it follows from the same
audit. All four were independent; the spec produced three implementation plans that ran in parallel.

All three plans have been implemented as of 2026-05-14.

## Out of scope

- `db.clone()` / `pool.clone()` before `tokio::spawn` — Arc clones, correct and cheap.
- `Vec<String>` `.is_in()` clones — no improvement without API changes.
- `model.clone().into()` sites where the map is read after the loop — leave as-is.
- `model.clone().into()` single-model return-before pattern — where `before` is returned (or
  fields accessed) after `active` is created from it, the `clone()` is load-bearing. All such
  sites must be left as-is. See the implementation note under Plan A.
- String field-by-field clones in DB→DTO mapping loops — separate concern.

---

## Plan A — `web-api-queries`: consume-by-value + iter().copied()

**Crate:** `crates/ui/web-api-queries`

### Pattern 1: model.clone().into() → consume map by value

**Status: Implemented** — all HashMap-iteration sites were fixed or confirmed load-bearing.

All 13 originally listed sites followed this shape:

```rust
for (id, h) in &found {                          // found: HashMap<Uuid, SomeModel>
    let mut active: SomeModel::ActiveModel = h.clone().into();
    active.some_field = Set(value);
    active.update(db).await?;
}
```

`h.clone().into()` clones every field of the model (strings, timestamps, options) into `Unchanged`
slots that are never read. The fix is to consume the map by iterating by value:

```rust
for (id, h) in found {
    let mut active: SomeModel::ActiveModel = h.into();  // no clone
    active.some_field = Set(value);
    active.update(db).await?;
}
```

**Per-site rule:** apply only when `found` is not accessed **after** the consuming loop. Access
**before** the consuming loop (e.g. a `contains_key` validation pass that runs first) is not a
blocking condition — those borrows complete before the move. Do not restructure two-pass patterns
into a single loop; that changes behavior and is out of scope.

**Site audit results:**

| File                                 | Original line  | Status       | Notes                                                                                                                |
| ------------------------------------ | -------------- | ------------ | -------------------------------------------------------------------------------------------------------------------- |
| `queries/hosts.rs`                   | 426 (→438/467) | Out of scope | `before` returned after conversion — load-bearing                                                                    |
| `queries/services.rs`                | 557, 751, 805  | Fixed        |                                                                                                                      |
| `queries/system_services.rs`         | 398, 441, 488  | Fixed        |                                                                                                                      |
| `queries/plugin_configs.rs`          | 292 (→212/245) | Out of scope | `before` returned after conversion — load-bearing                                                                    |
| `queries/host_tags.rs`               | 398 (→524/569) | Out of scope | `before` returned after conversion — load-bearing                                                                    |
| `queries/update_batches/dispatch.rs` | 172 (→176)     | Follow-up    | `next_record.batch_id`/`.tenant_id` accessed after ActiveModel; extract as locals first then `into()` to avoid clone |
| `queries/notifications.rs`           | 149 (→371/492) | Out of scope | `existing`/`before` accessed after conversion — load-bearing                                                         |
| `queries/software_items/crud.rs`     | 690, 730       | Fixed        |                                                                                                                      |

The "out of scope" sites were audited during implementation and found to be the single-model
return-before pattern (not the HashMap-iteration pattern). In all cases `before`/`existing` is either
returned directly or its fields are accessed after the `ActiveModel` is created — consuming the value
into `active` would be a compile error. These clones are correct and no further work is needed.

### Pattern 2: Vec\<Uuid\> cloned for .is_in() → iter().copied()

**Status: Implemented** — all listed sites fixed.

`is_in` accepts `I: IntoIterator<Item = V>` where `V: Into<Expr>`. `Uuid: Copy` and
`Uuid: Into<Expr>`, so `iter().copied()` yields owned `Uuid` values without allocating a new `Vec`.
Note: `.iter()` alone yields `&Uuid`, which does **not** satisfy `V: Into<Expr>` — it would be a
compile error. Use `.iter().copied()`.

The simplest mechanical approach: replace every `.is_in(x.clone())` with
`.is_in(x.iter().copied())`. This is safe for both intermediate and last-use sites. For last uses
the owned form `.is_in(x)` also works (and is marginally cheaper), but the distinction is an
optimisation, not a requirement — the compiler will catch any mistakes. Note: `cargo clippy` (with
`all = "deny"`) may prefer `into_iter()` at confirmed last-use sites on owned `Vec` — verify with a
clippy pass before settling on a universal `.iter().copied()` rule.

```rust
// works for all sites
.filter(col.is_in(item_ids.iter().copied()))

// equivalent for last use (optional optimisation)
.filter(col.is_in(item_ids))
```

**Rule:** only apply to `Vec<Uuid>` (or other `Copy` element types). Leave `Vec<String>` sites
unchanged — `.iter().cloned()` is no improvement over `.clone()`.

**Sites fixed:**

| File                                   | Original lines               | Notes                                              |
| -------------------------------------- | ---------------------------- | -------------------------------------------------- |
| `queries/software_states.rs`           | 98, 136, 369, 404, 604       | line 625 was already a bare last-use — left as-is  |
| `queries/update_history.rs`            | 117 (→121), 126 (→130)       | line 134 was already bare — left as-is             |
| `routes/service_ws/handler/updates.rs` | 361, 382, 393, 407, 408, 472 | in `crates/ui/web-api`, included here for cohesion |

### Quality gates

```sh
cargo fmt --all
cargo check --no-default-features --features db-sqlite
cargo check --all-features
cargo clippy --all-targets --no-default-features --features db-sqlite
cargo clippy --all-targets --all-features
cargo test --no-default-features --features db-sqlite
cargo test --all-features
cargo deny check
```

---

## Plan B — `surface-proxy`: Option::or → or_else

**Crate:** `crates/ui/surface-proxy`
**File:** `src/proxy.rs`

**Status: Implemented** — proxy.rs was significantly refactored (now 946 lines; original spec
referenced sites up to line ~1372). No `.or(x.clone())` patterns remain in the file.

`Option::or(expr)` evaluates `expr` eagerly — the clone runs even when the receiver is `Some`.
`Option::or_else(|| expr)` is lazy.

```rust
// before
.or(requested_name.clone())

// after
.or_else(|| requested_name.clone())
```

No behavioral change. The cloned value is identical; only the allocation is deferred. The values
being cloned are `Option<String>` fields extracted from request params — the clone on the `None`
path is unavoidable since the downstream audit struct requires owned `String`. `or_else` is
already optimal for this type.

### Quality gates

```sh
cargo fmt --all
cargo check --no-default-features --features db-sqlite
cargo check --all-features
cargo clippy --all-targets --no-default-features --features db-sqlite
cargo clippy --all-targets --all-features
cargo test --no-default-features --features db-sqlite
cargo test --all-features
cargo deny check
```

---

## Plan C — `shared/surfaces`: derive Copy on FrameworkGenerationRange

**Crate:** `crates/shared/surfaces`
**File:** `src/surface.rs:470` (original spec cited line 435; drifted during implementation)

**Status: Implemented** — `Copy` is derived.

`FrameworkGenerationRange` contains two `FrameworkGeneration` fields, each two `u16` — all `Copy`.
The struct is a pure value type with no heap allocation. It should derive `Copy`.

```rust
// before
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameworkGenerationRange {

// after
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameworkGenerationRange {
```

This eliminates the explicit `.clone()` at `crates/ui/surface-proxy/src/registry.rs:836` where
a `SurfaceRegistrationPolicy` is constructed during provider registration.

**Check `CapabilitySet`:** `CapabilitySet(BTreeSet<Capability>)` — heap-allocated, cannot be `Copy`.
Left unchanged.

**Known gap:** `FrameworkGenerationRange` does not carry `#[non_exhaustive]`, which is required by
project standards for public structs in shared crates. This was not addressed by Plan C. Adding it
is a follow-up: it would require consumers using struct literal syntax (`registry.rs:111`,
`surfaces/tests/protocol.rs:24`, `surfaces/tests/ids.rs:78`) to add `..` to pattern matches and
switch to constructor-based builds. `Copy + #[non_exhaustive]` is fully legal in Rust.

One consumer (`surfaces/tests/ids.rs:78`) uses a `const` struct literal. A
`const fn new(min: FrameworkGeneration, max: FrameworkGeneration) -> Self` constructor resolves
all three consumer sites, including this `const` site — `const fn` calls are valid in `const`
initializers. No `Default` impl or struct-update syntax is needed.

### Quality gates

```sh
cargo fmt --all
cargo check --no-default-features --features db-sqlite
cargo check --all-features
cargo clippy --all-targets --no-default-features --features db-sqlite
cargo clippy --all-targets --all-features
cargo test --no-default-features --features db-sqlite
cargo test --all-features
cargo deny check
```

---

## Execution order

All three plans were independent — no shared files, no ordering constraints.

| Plan | Effort                                | Crate(s)                     | Status      |
| ---- | ------------------------------------- | ---------------------------- | ----------- |
| C    | Trivial (1 line)                      | `shared/surfaces`            | Implemented |
| B    | Small (mechanical edits)              | `surface-proxy`              | Implemented |
| A    | Medium (20+ sites, per-site judgment) | `web-api-queries`, `web-api` | Implemented |

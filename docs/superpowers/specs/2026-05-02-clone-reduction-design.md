# Clone Reduction Refactors

**Date:** 2026-05-02
**Status:** Spec

## Background

The workspace has 2500+ `.clone()` calls. The majority are cheap: `Arc`/SeaORM handle clones before
`tokio::spawn` are O(1) and correct. Three areas have avoidable heap allocations:

1. SeaORM model maps iterated by reference, forcing a full-model clone before `ActiveModel` conversion.
2. `Vec<Uuid>` cloned to satisfy `is_in()` ownership when only iteration is needed.
3. `Option::or(x.clone())` eagerly cloning when the receiver is already `Some`.

A fourth micro-fix (`FrameworkGenerationRange` missing `Copy`) is included as it follows from the same
audit. All four are independent; the spec produces three implementation plans that can run in parallel.

## Out of scope

- `db.clone()` / `pool.clone()` before `tokio::spawn` — Arc clones, correct and cheap.
- `Vec<String>` `.is_in()` clones — no improvement without API changes.
- `model.clone().into()` sites where the map is read after the loop — leave as-is.
- String field-by-field clones in DB→DTO mapping loops — separate concern.

---

## Plan A — `web-api-queries`: consume-by-value + iter().copied()

**Crate:** `crates/ui/web-api-queries`

### Pattern 1: model.clone().into() → consume map by value

All 13 sites follow this shape:

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

**Known sites (verify each):**

| File                                 | Line | Safe to consume? |
| ------------------------------------ | ---- | ---------------- |
| `queries/hosts.rs`                   | 426  | verify           |
| `queries/services.rs`                | 557  | verify           |
| `queries/services.rs`                | 751  | verify           |
| `queries/services.rs`                | 805  | verify           |
| `queries/system_services.rs`         | 398  | verify           |
| `queries/system_services.rs`         | 441  | verify           |
| `queries/system_services.rs`         | 488  | verify           |
| `queries/plugin_configs.rs`          | 292  | verify           |
| `queries/host_tags.rs`               | 398  | verify           |
| `queries/update_batches/dispatch.rs` | 172  | verify           |
| `queries/notifications.rs`           | 149  | verify           |
| `queries/software_items/crud.rs`     | 690  | verify           |
| `queries/software_items/crud.rs`     | 730  | verify           |

### Pattern 2: Vec\<Uuid\> cloned for .is_in() → iter().copied()

`is_in` accepts `I: IntoIterator<Item = V>` where `V: Into<Expr>`. `Uuid: Copy` and
`Uuid: Into<Expr>`, so `iter().copied()` yields owned `Uuid` values without allocating a new `Vec`.
Note: `.iter()` alone yields `&Uuid`, which does **not** satisfy `V: Into<Expr>` — it would be a
compile error. Use `.iter().copied()`.

The simplest mechanical approach: replace every `.is_in(x.clone())` with
`.is_in(x.iter().copied())`. This is safe for both intermediate and last-use sites. For last uses
the owned form `.is_in(x)` also works (and is marginally cheaper), but the distinction is an
optimisation, not a requirement — the compiler will catch any mistakes.

```rust
// works for all sites
.filter(col.is_in(item_ids.iter().copied()))

// equivalent for last use (optional optimisation)
.filter(col.is_in(item_ids))
```

**Rule:** only apply to `Vec<Uuid>` (or other `Copy` element types). Leave `Vec<String>` sites
unchanged — `.iter().cloned()` is no improvement over `.clone()`.

**Known sites:**

| File                                   | Lines                        | Notes                                              |
| -------------------------------------- | ---------------------------- | -------------------------------------------------- |
| `queries/software_states.rs`           | 98, 136, 369, 404, 604       | line 625 is already a bare last-use — leave it     |
| `queries/update_history.rs`            | 117, 126                     | line 134 is already bare — leave it                |
| `routes/service_ws/handler/updates.rs` | 361, 382, 393, 407, 408, 472 | in `crates/ui/web-api`, included here for cohesion |

Apply `.iter().copied()` to all listed sites. Optionally use the bare owned form for confirmed
last-use lines (98→136, 369→404, 604→625 for `software_states.rs`; 117→126 for `update_history.rs`).

### Quality gates

```sh
cargo fmt --all
cargo check --no-default-features --features db-sqlite
cargo check --all-features
cargo clippy --all-targets --all-features
cargo test --all-features
```

---

## Plan B — `surface-proxy`: Option::or → or_else

**Crate:** `crates/ui/surface-proxy`
**File:** `src/proxy.rs`

`Option::or(expr)` evaluates `expr` eagerly — the clone runs even when the receiver is `Some`.
`Option::or_else(|| expr)` is lazy.

```rust
// before
.or(requested_name.clone())

// after
.or_else(|| requested_name.clone())
```

**Known sites (approx lines):** 1014, 1182, 1187, 1207, 1212, 1366, 1372.

Scan the full file for `.or(` with a `.clone()` argument to catch any additional sites before
applying the fix. Some adjacent lines already use `or_else` — normalise to `or_else` consistently
throughout the file to avoid mixed idioms.

No behavioral change. The cloned value is identical; only the allocation is deferred. The values
being cloned are `Option<String>` fields extracted from request params — the clone on the `None`
path is unavoidable since the downstream audit struct requires owned `String`. `or_else` is
already optimal for this type. All 7 sites are in plain `match` arms in synchronous functions —
no `move` closures or `async` blocks involved, so the borrow in `|| x.clone()` is trivially safe.

### Quality gates

```sh
cargo fmt --all
cargo check --no-default-features --features db-sqlite
cargo check --all-features
cargo clippy --all-targets --all-features
cargo test --all-features
```

---

## Plan C — `shared/surfaces`: derive Copy on FrameworkGenerationRange

**Crate:** `crates/shared/surfaces`
**File:** `src/surface.rs:435`

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
Leave unchanged.

After adding `Copy`, verify no existing code breaks (e.g. move-after-use errors surfaced by the
compiler now that copies happen implicitly). `cargo check` will catch any such sites.

### Quality gates

```sh
cargo fmt --all
cargo check --no-default-features --features db-sqlite
cargo check --all-features
cargo clippy --all-targets --all-features
cargo test --all-features
```

---

## Execution order

All three plans are independent — no shared files, no ordering constraints. Recommended execution:

| Plan | Effort                                | Crate(s)                     |
| ---- | ------------------------------------- | ---------------------------- |
| C    | Trivial (1 line)                      | `shared/surfaces`            |
| B    | Small (8 mechanical edits)            | `surface-proxy`              |
| A    | Medium (20+ sites, per-site judgment) | `web-api-queries`, `web-api` |

Can be run in parallel by subagents or sequentially C → B → A.

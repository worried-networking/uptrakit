# Clone Reduction — Plan C: FrameworkGenerationRange Copy derive

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans`
> to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Derive `Copy` on `FrameworkGenerationRange` in `crates/shared/surfaces`, eliminating
the explicit `.clone()` call when constructing `SurfaceRegistrationPolicy` during provider
registration.

**Architecture:** `FrameworkGenerationRange` holds two `FrameworkGeneration` fields, each two
`u16` — all `Copy`. The struct is a pure value type with no heap allocation. Adding `Copy` to its
derive list allows direct field access without cloning; the explicit `.clone()` at `registry.rs:836`
is then redundant and must be removed by hand (it still compiles — only Clippy flags it).
`cargo check` will surface any previously-hidden move errors.

**Tech Stack:** Rust, `crates/shared/surfaces`, `crates/ui/surface-proxy`

---

## Task 1: Add Copy to FrameworkGenerationRange

**Files:**

- Modify: `crates/shared/surfaces/src/surface.rs:435`

- [ ] **Step 1: Apply the change**

In `crates/shared/surfaces/src/surface.rs`, at line 435, replace:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameworkGenerationRange {
```

with:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameworkGenerationRange {
```

- [ ] **Step 2: Verify compilation across full workspace**

```sh
cargo check --all-features
```

Expected: no errors. If any crate fails because it assumed `FrameworkGenerationRange` was not
`Copy` (e.g. a move that now becomes a copy silently), the compiler will surface it here.

- [ ] **Step 3: Remove the now-redundant .clone() at registry.rs:836**

After adding `Copy`, calling `.clone()` on `FrameworkGenerationRange` still compiles — `rustc`
does not warn about it. Only Clippy flags it as `clone_on_copy`. Remove it now by hand.

In `crates/ui/surface-proxy/src/registry.rs`, replace:

```rust
        if let Err(err) = registration.validate_against(&surfaces::SurfaceRegistrationPolicy {
            supported_generation: self.config.supported_generation.clone(),
            required_capabilities: self.config.required_capabilities.clone(),
        }) {
```

with:

```rust
        if let Err(err) = registration.validate_against(&surfaces::SurfaceRegistrationPolicy {
            supported_generation: self.config.supported_generation,
            required_capabilities: self.config.required_capabilities.clone(),
        }) {
```

Note: `required_capabilities` is `CapabilitySet(BTreeSet<Capability>)` — heap-allocated, not
`Copy`. Its `.clone()` is unchanged.

Alternatively, `cargo clippy --fix -p uptrakit-surface-proxy --all-features -- -A clippy::all -W clippy::clone_on_copy`
will auto-remove it, but the manual edit above is simpler.

- [ ] **Step 4: Verify compilation**

```sh
cargo check --all-features
```

Expected: no errors.

- [ ] **Step 5: Commit**

```sh
git add crates/shared/surfaces/src/surface.rs \
        crates/ui/surface-proxy/src/registry.rs
git commit -m "refactor(surfaces): derive Copy on FrameworkGenerationRange, remove redundant clone"
```

---

## Task 2: Quality gates

- [ ] **Step 1: Format**

```sh
cargo fmt --all
```

- [ ] **Step 2: Full check (both feature sets)**

```sh
cargo check --no-default-features --features db-sqlite
cargo check --all-features
```

Expected: no errors.

- [ ] **Step 3: Clippy**

```sh
cargo clippy --all-targets --all-features
```

Expected: no new warnings. Specifically confirm `clippy::clone_on_copy` is not fired for
`FrameworkGenerationRange` anywhere else in the workspace.

- [ ] **Step 4: Tests**

```sh
cargo test -p uptrakit-surfaces --all-features
cargo test -p uptrakit-surface-proxy --all-features
```

Expected: all pass.

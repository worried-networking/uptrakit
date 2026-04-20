# Shared Surfaces Runtime Boundaries Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to
> implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Split the shared surface registration validator into rule-focused helpers, add explicit `# Errors` contracts to exported fallible APIs in
`uptrakit-surfaces`, and tighten the first shared-contract slice of small-type Rust idioms.

**Architecture:** Refactor the validator first so test coverage anchors stay stable, then harden exported APIs with rustdoc and small-type affordances
(`#[must_use]`, `const fn`, carefully chosen `Copy`), then wire the documented lint/doc guidance so the hardening does not regress.

**Tech Stack:** Rust workspace crates (`uptrakit-surfaces`, `uptrakit-shared-types`), unit tests in `crates/shared/surfaces/tests`, Clippy
`missing_errors_doc`, Markdown docs

---

## File Structure

### Validation split

- Modify: [`crates/shared/surfaces/src/protocol.rs`](/Users/andreyyantsen/Development/uptrakit/crates/shared/surfaces/src/protocol.rs) Responsibility:
  split `validate_against` into explicit rule-focused helpers and normalize error construction.
- Modify: [`crates/shared/surfaces/tests/protocol.rs`](/Users/andreyyantsen/Development/uptrakit/crates/shared/surfaces/tests/protocol.rs)
  Responsibility: preserve rule coverage while allowing helper extraction.

### Small-type and rustdoc hardening

- Modify: [`crates/shared/surfaces/src/ids.rs`](/Users/andreyyantsen/Development/uptrakit/crates/shared/surfaces/src/ids.rs)
- Modify: [`crates/shared/surfaces/src/data.rs`](/Users/andreyyantsen/Development/uptrakit/crates/shared/surfaces/src/data.rs)
- Modify: [`crates/shared/surfaces/src/interaction.rs`](/Users/andreyyantsen/Development/uptrakit/crates/shared/surfaces/src/interaction.rs)
- Modify: [`crates/shared/surfaces/src/surface.rs`](/Users/andreyyantsen/Development/uptrakit/crates/shared/surfaces/src/surface.rs)
- Modify: [`crates/shared/types/src/network.rs`](/Users/andreyyantsen/Development/uptrakit/crates/shared/types/src/network.rs)
- Modify: [`crates/shared/surfaces/tests/ids.rs`](/Users/andreyyantsen/Development/uptrakit/crates/shared/surfaces/tests/ids.rs)

### Documentation

- Modify: [`docs/development/coding-standards.md`](/Users/andreyyantsen/Development/uptrakit/docs/development/coding-standards.md)
- Optionally modify: [`docs/development/rust-idioms.md`](/Users/andreyyantsen/Development/uptrakit/docs/development/rust-idioms.md)

### Verification commands

- `cargo fmt --all`
- `cargo test -p uptrakit-surfaces`
- `cargo check -p uptrakit-surfaces`
- `cargo clippy -p uptrakit-surfaces --all-targets -- -D clippy::missing_errors_doc`
- `cargo check -p uptrakit-shared-types`
- `markdownlint --config .markdownlint.json docs/development/coding-standards.md`
- `test ! -f docs/development/rust-idioms.md || markdownlint --config .markdownlint.json docs/development/rust-idioms.md`

### Task 1: Split Registration Validation Into Rule-Focused Helpers

**Files:**

- Modify: [`crates/shared/surfaces/src/protocol.rs`](/Users/andreyyantsen/Development/uptrakit/crates/shared/surfaces/src/protocol.rs)
- Modify: [`crates/shared/surfaces/tests/protocol.rs`](/Users/andreyyantsen/Development/uptrakit/crates/shared/surfaces/tests/protocol.rs)

- [ ] **Step 1: Snapshot the current protocol tests before refactoring**

Run:

```bash
cargo test -p uptrakit-surfaces --test protocol
```

Expected: PASS and establishes the current validation behavior baseline.

- [ ] **Step 2: Extract rule-focused helpers in `protocol.rs`**

Refactor:

```rust
impl SurfaceRegistration {
    pub fn validate_against(
        &self,
        policy: &SurfaceRegistrationPolicy,
    ) -> Result<(), SurfaceRegistrationError> {
        validate_generation(self, policy)?;
        validate_capability_contract(self, policy)?;
        validate_surface_descriptors(self)?;
        validate_root_nodes(self)?;
        Ok(())
    }
}
```

Use a shared error helper:

```rust
fn invalid_contract(message: impl Into<String>) -> SurfaceRegistrationError {
    SurfaceRegistrationError::new(
        SurfaceRegistrationErrorCode::InvalidContract,
        message.into(),
    )
}
```

- [ ] **Step 3: Re-run the protocol test suite**

Run:

```bash
cargo test -p uptrakit-surfaces --test protocol
```

Expected: PASS with no behavioral regressions.

- [ ] **Step 4: Commit**

```bash
git add crates/shared/surfaces/src/protocol.rs crates/shared/surfaces/tests/protocol.rs
git commit -m "refactor: split shared surface registration validation"
```

### Task 2: Add `# Errors` Contracts And Enforce Them

**Files:**

- Modify: [`crates/shared/surfaces/src/ids.rs`](/Users/andreyyantsen/Development/uptrakit/crates/shared/surfaces/src/ids.rs)
- Modify: [`crates/shared/surfaces/src/data.rs`](/Users/andreyyantsen/Development/uptrakit/crates/shared/surfaces/src/data.rs)
- Modify: [`crates/shared/surfaces/src/interaction.rs`](/Users/andreyyantsen/Development/uptrakit/crates/shared/surfaces/src/interaction.rs)
- Modify: [`crates/shared/surfaces/src/protocol.rs`](/Users/andreyyantsen/Development/uptrakit/crates/shared/surfaces/src/protocol.rs)

- [ ] **Step 1: Run the lint first to capture the current failures**

Run:

```bash
cargo clippy -p uptrakit-surfaces --all-targets -- -D clippy::missing_errors_doc
```

Expected: FAIL with a `missing_errors_doc` lint on one or more public `Result`-returning function such as `validate_surface_identifier` or
`SurfaceRegistration::validate_against`.

- [ ] **Step 2: Add explicit `# Errors` sections to all exported fallible APIs in scope**

Example rustdoc pattern:

```rust
/// Validate a surface identifier.
///
/// # Errors
///
/// Returns [`IdentifierError`] when the value is empty, too long, or contains
/// characters outside the allowed surface identifier contract.
pub fn validate_surface_identifier(value: &str) -> Result<(), IdentifierError> { /* ... */ }
```

```rust
/// Validate this registration against a provider policy.
///
/// # Errors
///
/// Returns [`SurfaceRegistrationError`] when the framework generation,
/// capabilities, root-node references, or slot/data-source contracts violate
/// the supplied policy.
pub fn validate_against(...) -> Result<(), SurfaceRegistrationError> { /* ... */ }
```

- [ ] **Step 3: Re-run the Clippy enforcement command**

Run:

```bash
cargo test -p uptrakit-surfaces
cargo clippy -p uptrakit-surfaces --all-targets -- -D clippy::missing_errors_doc
```

Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/shared/surfaces/src/ids.rs crates/shared/surfaces/src/data.rs crates/shared/surfaces/src/interaction.rs crates/shared/surfaces/src/protocol.rs
git commit -m "docs: add explicit errors contracts to shared surfaces APIs"
```

### Task 3: Tighten Small Types In The First Hardening Slice

**Files:**

- Modify: [`crates/shared/surfaces/src/ids.rs`](/Users/andreyyantsen/Development/uptrakit/crates/shared/surfaces/src/ids.rs)
- Modify: [`crates/shared/surfaces/src/data.rs`](/Users/andreyyantsen/Development/uptrakit/crates/shared/surfaces/src/data.rs)
- Modify: [`crates/shared/surfaces/src/surface.rs`](/Users/andreyyantsen/Development/uptrakit/crates/shared/surfaces/src/surface.rs)
- Modify: [`crates/shared/types/src/network.rs`](/Users/andreyyantsen/Development/uptrakit/crates/shared/types/src/network.rs)
- Modify: [`crates/shared/surfaces/tests/ids.rs`](/Users/andreyyantsen/Development/uptrakit/crates/shared/surfaces/tests/ids.rs)

- [ ] **Step 1: Add a small call-site test for `#[must_use]`/`const fn` targets**

Add or adapt tests that exercise the intended API surface, for example:

```rust
#[test]
fn generated_identifier_new_accepts_valid_value() {
    let id = SurfaceId::new("dashboard.main").expect("valid identifier");
    assert_eq!(id.as_str(), "dashboard.main");
}
```

- [ ] **Step 2: Apply the targeted small-type affordances**

Examples:

```rust
#[must_use]
pub fn is_valid_surface_identifier(value: &str) -> bool { /* ... */ }
```

```rust
pub const fn as_str(&self) -> &str {
    &self.0
}
```

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Targeting {
    Universal,
    Targeted,
}
```

```rust
#[must_use]
pub fn is_private_ip(addr: IpAddr) -> bool { /* ... */ }
```

- [ ] **Step 3: Run package checks**

Run:

```bash
cargo test -p uptrakit-surfaces
cargo check -p uptrakit-shared-types
```

Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/shared/surfaces/src/ids.rs crates/shared/surfaces/src/data.rs crates/shared/surfaces/src/surface.rs crates/shared/types/src/network.rs crates/shared/surfaces/tests/ids.rs
git commit -m "refactor: tighten shared surface small types"
```

### Task 4: Document The New Shared-Contract Rules

**Files:**

- Modify: [`docs/development/coding-standards.md`](/Users/andreyyantsen/Development/uptrakit/docs/development/coding-standards.md)
- Optionally modify: [`docs/development/rust-idioms.md`](/Users/andreyyantsen/Development/uptrakit/docs/development/rust-idioms.md)

- [ ] **Step 1: Add the shared-contract hardening guidance to coding standards**

Add guidance like:

```md
### Shared contract crates

- Public fallible APIs in `uptrakit-surfaces` must document `# Errors`.
- Run `cargo clippy -p uptrakit-surfaces --all-targets -- -D clippy::missing_errors_doc` when touching shared surface contracts.
- Prefer `#[must_use]`, `const fn`, and carefully chosen `Copy` on small shared value APIs when they clarify call-site intent.
```

- [ ] **Step 2: Lint the docs and re-run the enforcement command**

Run:

```bash
cargo fmt --all
markdownlint --config .markdownlint.json docs/development/coding-standards.md
test ! -f docs/development/rust-idioms.md || markdownlint --config .markdownlint.json docs/development/rust-idioms.md
cargo clippy -p uptrakit-surfaces --all-targets -- -D clippy::missing_errors_doc
```

Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add docs/development/coding-standards.md
git add docs/development/rust-idioms.md 2>/dev/null || true
git commit -m "docs: document shared surface contract hardening rules"
```

## Self-Review

- Spec coverage: Task 1 covers validator decomposition. Task 2 covers the rustdoc `# Errors` contract and Clippy enforcement. Task 3 covers the first
  shared small-type hardening slice. Task 4 covers documentation.
- Placeholder scan: no unfinished-plan markers remain.
- Type consistency: all tasks refer to the same Clippy command and the same targeted files.

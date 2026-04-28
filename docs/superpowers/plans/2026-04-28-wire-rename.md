# Wire Crate Rename Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task.
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rename `uptrakit-internal-wire` → `uptrakit-wire` across the workspace and fix
`uptrakit-surfaces` missing from `[workspace.dependencies]`.

**Architecture:** Two sequential commits. First: add `uptrakit-surfaces` to workspace deps,
switch the wire crate's path dep to `workspace = true`, and add `publish = false` to both
wire and surfaces crates. Second: mechanical sed-based rename of the wire crate name and all
references (~22 Cargo.toml files, ~142 Rust source files). No behavior changes — compilation
check is the only meaningful test.

**Tech Stack:** Bash (sed, find), cargo check, cargo test --no-run.

---

## File Structure

**Task 1 — workspace/publish fixes:**

- Modify: `Cargo.toml` — add `uptrakit-surfaces` to `[workspace.dependencies]`
- Modify: `crates/shared/wire/Cargo.toml` — switch surfaces dep to workspace; add `publish = false`
- Modify: `crates/shared/surfaces/Cargo.toml` — add `publish = false`

**Task 2 — mechanical rename:**

- Modify: `crates/shared/wire/Cargo.toml` — package name → `uptrakit-wire`
- Modify: `Cargo.toml` — workspace dep key rename
- Modify: 21 other `Cargo.toml` files — dep reference rename
- Modify: ~142 Rust source files — `uptrakit_internal_wire::` → `uptrakit_wire::`

---

### Task 1: Fix workspace surfaces dep and publish flags

**Files:**

- Modify: `Cargo.toml`
- Modify: `crates/shared/wire/Cargo.toml`
- Modify: `crates/shared/surfaces/Cargo.toml`

- [ ] **Step 1: Verify baseline compiles**

```bash
cargo check --all-features 2>&1 | tail -3
```

Expected: `Finished` with no errors. Abort if there are errors — fix them before continuing.

- [ ] **Step 2: Add `uptrakit-surfaces` to `[workspace.dependencies]` in root `Cargo.toml`**

In `Cargo.toml`, locate the internal crates block in `[workspace.dependencies]`
(around line 116, near `uptrakit-internal-wire`). Add:

```toml
uptrakit-surfaces = { path = "crates/shared/surfaces" }
```

- [ ] **Step 3: Switch wire crate's surfaces dep to workspace**

In `crates/shared/wire/Cargo.toml`, change:

```toml
uptrakit-surfaces = { path = "../surfaces" }
```

to:

```toml
uptrakit-surfaces = { workspace = true }
```

- [ ] **Step 4: Add `publish = false` to wire crate**

In `crates/shared/wire/Cargo.toml`, add after `version.workspace = true`:

```toml
publish = false
```

- [ ] **Step 5: Add `publish = false` to surfaces crate**

In `crates/shared/surfaces/Cargo.toml`, add after `version.workspace = true`:

```toml
publish = false
```

- [ ] **Step 6: Add `publish = false` to `[workspace.package]` in root `Cargo.toml`**

This sets the workspace-level default. Internal crates that have no explicit `publish`
field can opt in to this default via `publish.workspace = true`. Existing crates with
explicit `publish = false` are unaffected. Future published crates (`uptrakit-service-sdk`,
`uptrakit-openapi-client`) will use explicit `publish = true` to override.

In `Cargo.toml`, in the `[workspace.package]` section, add after `edition = "2024"`:

```toml
publish = false
```

- [ ] **Step 7: Run cargo check**

```bash
cargo check --all-features 2>&1 | tail -3
```

Expected: `Finished` with no errors.

- [ ] **Step 8: Commit**

```bash
git add Cargo.toml crates/shared/wire/Cargo.toml crates/shared/surfaces/Cargo.toml
git commit -m "fix(workspace): add uptrakit-surfaces to workspace deps; lock publish defaults"
```

---

### Task 2: Rename uptrakit-internal-wire to uptrakit-wire

**Files:**

- Modify: `crates/shared/wire/Cargo.toml` (package name)
- Modify: `Cargo.toml` (workspace dep key)
- Modify: 21 other `Cargo.toml` files (dep reference)
- Modify: ~142 Rust source files (use paths, doc comments)

The following `Cargo.toml` files reference `uptrakit-internal-wire` and will be updated by the
bulk sed command in Step 2:

```text
Cargo.toml (workspace root — dep key)
crates/shared/wire/Cargo.toml (package name)
crates/core/controller/Cargo.toml
crates/core/agent-ssh/Cargo.toml
crates/ui/web-api/Cargo.toml
crates/shared/service-sdk/Cargo.toml
crates/ui/cli/Cargo.toml
crates/shared/service-platform/Cargo.toml
crates/core/scheduler-runtime/Cargo.toml
crates/core/mqtt-runtime/Cargo.toml
crates/core/agent-ssh-runtime/Cargo.toml
crates/core/agent-runtime/Cargo.toml
crates/ui/web-api-queries/Cargo.toml
crates/shared/web-api-types/Cargo.toml
crates/shared/scheduler-engine/Cargo.toml
crates/shared/nats/Cargo.toml
crates/shared/github-client/Cargo.toml
crates/shared/agent-core/Cargo.toml
crates/plugins/infrastructure/core/Cargo.toml
crates/core/scheduler/Cargo.toml
crates/core/mqtt/Cargo.toml
crates/core/integration-tests/Cargo.toml
crates/core/agent/Cargo.toml
```

- [ ] **Step 1: Verify baseline compiles**

```bash
cargo check --all-features 2>&1 | tail -3
```

Expected: `Finished` with no errors.

- [ ] **Step 2: Bulk rename in all Cargo.toml files**

Renames `name = "uptrakit-internal-wire"`, the workspace dep key
`uptrakit-internal-wire = ...`, and all dep references:

```bash
find . -name "Cargo.toml" -not -path "*/target/*" \
  -exec sed -i.bak 's/uptrakit-internal-wire/uptrakit-wire/g' {} \;
find . -name "Cargo.toml.bak" -not -path "*/target/*" -delete
```

- [ ] **Step 3: Bulk rename in all Rust source files**

Renames `uptrakit_internal_wire` (Rust module path separator is `_`) to `uptrakit_wire`
in all `use` statements, doc comments (`uptrakit_internal_wire::` in doc examples), and any
other occurrences:

```bash
find . -name "*.rs" -not -path "*/target/*" \
  -exec sed -i.bak 's/uptrakit_internal_wire/uptrakit_wire/g' {} \;
find . -name "*.rs.bak" -not -path "*/target/*" -delete
```

- [ ] **Step 4: Run cargo check**

```bash
cargo check --all-features 2>&1 | tail -5
```

Expected: `Finished` with no errors. If `unresolved import` errors appear, search for any
remaining occurrences: `grep -r "uptrakit.internal.wire" . --include="*.rs" --include="*.toml"
-l` and fix manually.

- [ ] **Step 5: Verify test compilation**

```bash
cargo test --all-features --no-run 2>&1 | tail -5
```

Expected: no compile errors.

- [ ] **Step 6: Commit**

`git add -u` stages both the renamed source files AND the updated `Cargo.lock` (which
cargo regenerated in Step 4 to reflect the new crate name).

```bash
git add -u
git commit -m "refactor(workspace): rename uptrakit-internal-wire to uptrakit-wire

Mechanical rename throughout the workspace — no behavior changes.
~23 Cargo.toml files and ~142 Rust source files updated via sed."
```

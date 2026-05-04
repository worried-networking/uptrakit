# Surfaces Button Icons & Label Display Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wire icon support end-to-end for Surfaces (Rust descriptors → wire validation → Svelte
rendering), introduce adaptive label visibility (`labelDisplay` = `'always' | 'auto' | 'icon-only'`)
in a new `SurfaceActionButton` wrapper, and refactor the Proxmox VE Hosts and SSH Hosts surfaces to
use icons.

**Architecture:** Plugin authors set `icon: Option<String>` (kebab-case lucide names) on
`SurfaceActionDescriptor` / `surfaces::InteractionDescriptor`. A shared `validate_icon_name` in the
`surfaces` crate enforces the kebab-case regex on the wire and at the surfaces-side
`validate_for_provider`. Frontend `resolveIcon(name)` (in a new unified `frontend/src/lib/icons.ts`
that replaces `nav-icons.ts`) maps kebab-case to lucide components with a logged-error fallback. A
new `SurfaceActionButton.svelte` owns icon resolve, effective-display, label-span rendering, and
`<span title=…>` tooltip wrapping. `SurfaceInteractionButton` and `SurfaceWorkflow` delegate to it.
`Button.svelte` is unchanged.

**Tech Stack:** Rust (workspace clippy lints; `parking_lot`; `thiserror`; `rootcause::Report`);
SvelteKit 5 with Svelte 5 runes + Snippets; Tailwind v4 (`@tailwindcss/vite` 4.2.4) with native
container queries; `lucide-svelte` 1.0.1 (Svelte 4-style class components, type
`ComponentType<SvelteComponent>`); vitest + `@testing-library/svelte`.

**Snapshot rules referenced** (`.superpowers/standards-snapshot.md`):

- forbid `unwrap`/`expect`/`panic` in production code (clippy `unwrap_used = deny`,
  `expect_used = deny`, `panic = deny`, `indexing_slicing = deny`)
- apply `#[non_exhaustive]` to public enums with future variants
  (`docs/development/coding-standards.md#Public Enum Extensibility`)
- use `#[expect(lint, reason = "...")]` never `#[allow(...)]`
- forbid `Result<T, String>` outside web-api validation
- commit format: Conventional Commits (`<type>(scope): <description>`)
- TypeScript: `strict = true`, `checkJs = true`
- Prettier: tabs, single quotes, no trailing comma, width 120
- markdownlint: line length 150, code blocks/tables exempt
- mandatory pre-PR gates: `cargo fmt --all`, `cargo check --all-features`,
  `cargo clippy --all-targets --all-features`, `cargo test --all-features`, `cargo deny check`,
  `cd frontend && npm run check && npm run test && npm run build`,
  `markdownlint --config .markdownlint.json '**/*.md'`

**Domain glossary terms used:** Surface, Slot, Plugin, Service, Operator (per `CONTEXT.md`).

---

## File Structure

Files this plan creates or modifies:

**Rust — shared/surfaces (upstream of wire):**

- `crates/shared/surfaces/src/validation.rs` — NEW. `MAX_ICON_NAME_LEN`, `IconNameError`,
  `validate_icon_name`. Keeps validation isolated from `lib.rs` glue.
- `crates/shared/surfaces/src/lib.rs` — register and re-export `validation` module.
- `crates/shared/surfaces/src/interaction.rs` — add `icon: Option<String>` field to
  `InteractionDescriptor`; extend `InteractionValidationError` with
  `IconInvalid { interaction_id, reason }`; call `validate_icon_name` from `validate_for_provider`.

**Rust — shared/wire (downstream consumer):**

- `crates/shared/wire/src/limits.rs` — delete `MAX_NAV_ICON_LEN`.
- `crates/shared/wire/src/wire_validate_impls.rs` — call `surfaces::validate_icon_name` for
  `nav_icon` and every `interactions[].icon`; map `IconNameError` to `WireValidationError`. Update
  PascalCase-accepting test to use kebab-case.

**Rust — plugin authoring:**

- `crates/plugins/infrastructure/core/src/surface_form_authoring.rs` — add `icon: Option<String>`
  field + `with_icon` builder to `SurfaceActionDescriptor`.

**Rust — surface providers:**

- `crates/core/agent-ssh/src/surface_runtime.rs` — forward `action.icon` into
  `InteractionDescriptor.icon` at line 614 conversion; set icons on `bootstrap`, `sync-host`,
  `remove-host`.
- `crates/plugins/infrastructure/proxmox/src/plugin.rs` — set `icon` field on the five Proxmox VE
  Hosts interaction literals.
- `crates/plugins/infrastructure/proxmox/src/agent/plugin.rs` — `with_icon("boxes")` on
  `bootstrap_proxmox_guest_action`.

**Frontend — icon registry & contract:**

- `frontend/src/lib/icons.ts` — NEW (replaces `nav-icons.ts`). Kebab-case keys, logged-error
  fallback, `resolveIcon(name)`.
- `frontend/src/lib/icons.test.ts` — NEW (replaces `nav-icons.test.ts`).
- `frontend/src/lib/nav-icons.ts` — DELETED.
- `frontend/src/lib/nav-icons.test.ts` — DELETED.
- `frontend/src/lib/surfaces/contract.ts` — add `icon?: string` to `InteractionDescriptor`.
- `frontend/src/lib/surfaces/registry.svelte.ts` — switch to `resolveIcon`, kebab-case `'box'`
  fallback.
- `frontend/src/lib/surfaces/registry.test.ts` — update PascalCase fixtures to kebab-case.
- `frontend/src/lib/surfaces/label-display.ts` — NEW. `LabelDisplay` const-object + type union.

**Frontend — components:**

- `frontend/src/lib/components/surfaces/SurfaceActionButton.svelte` — NEW. Owns icon resolve,
  effective-display, label-span, tooltip wrap.
- `frontend/src/lib/components/surfaces/SurfaceActionButton.test.ts` — NEW.
- `frontend/src/lib/components/surfaces/SurfaceInteractionButton.svelte` — accept `labelDisplay`;
  delegate to `SurfaceActionButton` in both render branches.
- `frontend/src/lib/components/surfaces/SurfaceInteractionButton.test.ts` — extend forwarding tests.
- `frontend/src/lib/components/surfaces/SurfaceActionBar.svelte` — `@container/buttons` class on
  outer `<div>`; pass `labelDisplay='auto'` when icon set.
- `frontend/src/lib/components/surfaces/SurfaceActionBar.test.ts` — extend.
- `frontend/src/lib/components/surfaces/SurfaceTable.svelte` — entity-link row-actions `<div>`
  becomes `flex flex-nowrap items-center gap-1 @container/buttons`; rowActions snippet path gets a
  NEW wrapping `<div>` with the same classes; cell `<td>` adds `whitespace-nowrap`; pass
  `labelDisplay='icon-only'` when icon set.
- `frontend/src/lib/components/surfaces/SurfaceTable.test.ts` — extend.
- `frontend/src/lib/components/surfaces/SurfaceWorkflow.svelte` — accept `labelDisplay`; replace
  inline `<Button>` trigger with `<SurfaceActionButton>`.
- `frontend/src/lib/components/surfaces/SurfaceWorkflow.test.ts` — extend forwarding tests.
- `frontend/src/lib/components/Button.svelte` — **unchanged**.

**Frontend — dev preview:**

- `frontend/src/routes/dev/button-preview/+page.svelte` — extend with `SurfaceActionButton`
  examples.

**Docs:**

- `docs/development/plugin-guidelines.md` — new "Action icons" section.
- `docs/superpowers/specs/2026-05-02-icon-rollout-design.md` — banner pointing to the new spec for
  the kebab-case migration.

---

## Sequencing

Tasks 1–3 land Rust validation + descriptor field (`surfaces` crate is upstream of `wire`, so this
order avoids cross-crate breakage). Tasks 4–7 update Rust producers + wire validation now that the
field exists. Tasks 8–11 land the frontend icon registry independent of producers. Tasks 12–15
introduce the new shared `SurfaceActionButton` and its delegators. Tasks 16–18 wire surface action
bar / table / workflow callers. Tasks 19–21 wire dev preview, plugin guidelines doc, and the
icon-rollout banner. Task 22 runs full quality gates and verifies acceptance criteria.

---

## Task 1: Add `validate_icon_name` to the `surfaces` crate

**Files:**

- Create: `crates/shared/surfaces/src/validation.rs`
- Modify: `crates/shared/surfaces/src/lib.rs`
- Test: same file (Rust unit tests live alongside the implementation)

**Snapshot rules in scope:**

- `apply #[non_exhaustive] to public enums with future variants`
- `forbid unwrap/expect/panic in production code`
- `cargo clippy | indexing_slicing = deny`

- [ ] **Step 1: Write the failing tests**

Append to `crates/shared/surfaces/src/validation.rs` (file does not exist yet — create it with the
test module first):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_icon_name_accepts_kebab_case() {
        assert!(validate_icon_name("trash-2").is_ok());
        assert!(validate_icon_name("refresh-cw").is_ok());
        assert!(validate_icon_name("server-cog").is_ok());
        assert!(validate_icon_name("plug-zap").is_ok());
        assert!(validate_icon_name("box").is_ok());
        assert!(validate_icon_name("ab").is_ok());
    }

    #[test]
    fn validate_icon_name_rejects_empty() {
        assert_eq!(validate_icon_name(""), Err(IconNameError::Empty));
    }

    #[test]
    fn validate_icon_name_rejects_oversized() {
        let long = "a".repeat(MAX_ICON_NAME_LEN + 1);
        assert_eq!(validate_icon_name(&long), Err(IconNameError::TooLong));
    }

    #[test]
    fn validate_icon_name_rejects_pascal_case() {
        assert_eq!(validate_icon_name("Trash2"), Err(IconNameError::InvalidFormat));
        assert_eq!(validate_icon_name("Package"), Err(IconNameError::InvalidFormat));
    }

    #[test]
    fn validate_icon_name_rejects_underscore() {
        assert_eq!(validate_icon_name("trash_2"), Err(IconNameError::InvalidFormat));
    }

    #[test]
    fn validate_icon_name_rejects_leading_or_trailing_dash() {
        assert_eq!(validate_icon_name("-trash"), Err(IconNameError::InvalidFormat));
        assert_eq!(validate_icon_name("trash-"), Err(IconNameError::InvalidFormat));
    }

    #[test]
    fn validate_icon_name_rejects_single_char() {
        assert_eq!(validate_icon_name("x"), Err(IconNameError::InvalidFormat));
    }

    #[test]
    fn validate_icon_name_rejects_uppercase_or_punctuation_in_middle() {
        assert_eq!(validate_icon_name("traSh-2"), Err(IconNameError::InvalidFormat));
        assert_eq!(validate_icon_name("trash 2"), Err(IconNameError::InvalidFormat));
    }
}
```

- [ ] **Step 2: Run the tests to verify failure**

Run: `cargo test -p uptrakit-surfaces validation::tests` Expected: FAIL — compile error:
`validation` module unknown / `validate_icon_name` undefined.

- [ ] **Step 3: Implement the validator and register the module**

Replace `crates/shared/surfaces/src/validation.rs` with the production code preceded by the test
module from Step 1:

```rust
//! Shared icon-name validation used by the wire layer and by
//! `InteractionDescriptor::validate_for_provider`.

use thiserror::Error;

/// Maximum length (in bytes) of a Lucide-canonical icon name.
pub const MAX_ICON_NAME_LEN: usize = 64;

/// Errors reported by [`validate_icon_name`].
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum IconNameError {
    #[error("icon name must not be empty")]
    Empty,
    #[error("icon name exceeds {MAX_ICON_NAME_LEN} characters")]
    TooLong,
    #[error("icon name must match lucide kebab-case (lowercase letters, digits, hyphens)")]
    InvalidFormat,
}

/// Validates a candidate Lucide icon name.
///
/// Accepts ASCII kebab-case identifiers matching the regex
/// `^[a-z][a-z0-9-]*[a-z0-9]$` (≥ 2 chars, no leading/trailing dash).
///
/// # Errors
///
/// Returns [`IconNameError::Empty`] for empty strings,
/// [`IconNameError::TooLong`] for inputs longer than [`MAX_ICON_NAME_LEN`],
/// and [`IconNameError::InvalidFormat`] for any other shape mismatch.
pub fn validate_icon_name(name: &str) -> Result<(), IconNameError> {
    if name.is_empty() {
        return Err(IconNameError::Empty);
    }
    if name.len() > MAX_ICON_NAME_LEN {
        return Err(IconNameError::TooLong);
    }
    // Slice destructuring satisfies clippy::unwrap_used = deny and
    // clippy::indexing_slicing = deny without an #[expect(...)] escape:
    // the empty- and single-element cases fall through to the `else`.
    let bytes = name.as_bytes();
    let [first, middle @ .., last] = bytes else {
        return Err(IconNameError::InvalidFormat);
    };
    if !first.is_ascii_lowercase() {
        return Err(IconNameError::InvalidFormat);
    }
    if !(last.is_ascii_lowercase() || last.is_ascii_digit()) {
        return Err(IconNameError::InvalidFormat);
    }
    for &byte in middle {
        let allowed = byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-';
        if !allowed {
            return Err(IconNameError::InvalidFormat);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    // ... tests from Step 1 unchanged
}
```

(Keep the test module from Step 1 — paste the production code above it. Final file has both.)

Then update `crates/shared/surfaces/src/lib.rs`:

```rust
mod data;
mod form;
mod ids;
mod interaction;
mod protocol;
mod slot;
mod surface;
mod validation;

pub use data::*;
pub use form::*;
pub use ids::*;
pub use interaction::*;
pub use protocol::*;
pub use slot::*;
pub use surface::*;
pub use validation::*;
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p uptrakit-surfaces validation::tests` Expected: PASS — all eight tests green.

- [ ] **Step 5: Run the per-crate quality gates**

Run:
`cargo fmt -p uptrakit-surfaces && cargo clippy -p uptrakit-surfaces --all-targets --all-features -- -D warnings`
Expected: Empty output (no clippy or fmt diagnostics).

- [ ] **Step 6: Commit**

```bash
git add crates/shared/surfaces/src/validation.rs crates/shared/surfaces/src/lib.rs
git commit -m "feat(surfaces): add validate_icon_name and IconNameError"
```

---

## Task 2: Add `icon` field + validation to `surfaces::InteractionDescriptor`

**Files:**

- Modify: `crates/shared/surfaces/src/interaction.rs`
- Test: same file (existing `#[cfg(test)]` module)

**Snapshot rules in scope:**

- `apply #[non_exhaustive] to public enums with future variants`
- `wrap errors in rootcause::Report with typed enums (thiserror)` — `InteractionValidationError` is
  already a typed `thiserror` enum; we extend it.

- [ ] **Step 1: Write the failing tests**

Append to the existing test module in `crates/shared/surfaces/src/interaction.rs` (find
`#[cfg(test)] mod tests` near the bottom — if no such module exists yet, create one at the end of
the file using the same pattern as `validation.rs`):

```rust
#[test]
fn validate_for_provider_accepts_kebab_icon() {
    let descriptor = InteractionDescriptor {
        interaction_id: InteractionId::new("act").unwrap(),
        kind: InteractionKind::MutationAction,
        label: "Action".to_string(),
        required_permission: None,
        input_schema: None,
        result_schema: None,
        sensitive_fields: vec![],
        timeout_seconds: None,
        confirmation: None,
        transport: InteractionTransport::ControllerLocal,
        workflow_steps: vec![],
        form_ui: None,
        icon: Some("trash-2".to_string()),
    };
    assert!(descriptor.validate_for_provider(ProviderKind::Plugin).is_ok());
}

#[test]
fn validate_for_provider_rejects_pascal_icon() {
    let mut descriptor = InteractionDescriptor {
        interaction_id: InteractionId::new("act").unwrap(),
        kind: InteractionKind::MutationAction,
        label: "Action".to_string(),
        required_permission: None,
        input_schema: None,
        result_schema: None,
        sensitive_fields: vec![],
        timeout_seconds: None,
        confirmation: None,
        transport: InteractionTransport::ControllerLocal,
        workflow_steps: vec![],
        form_ui: None,
        icon: Some("Trash2".to_string()),
    };
    let err = descriptor
        .validate_for_provider(ProviderKind::Plugin)
        .unwrap_err();
    assert!(matches!(err, InteractionValidationError::IconInvalid { .. }));

    descriptor.icon = Some(String::new());
    let err = descriptor
        .validate_for_provider(ProviderKind::Plugin)
        .unwrap_err();
    assert!(matches!(err, InteractionValidationError::IconInvalid { .. }));
}

#[test]
fn validate_for_provider_accepts_missing_icon() {
    let descriptor = InteractionDescriptor {
        interaction_id: InteractionId::new("act").unwrap(),
        kind: InteractionKind::MutationAction,
        label: "Action".to_string(),
        required_permission: None,
        input_schema: None,
        result_schema: None,
        sensitive_fields: vec![],
        timeout_seconds: None,
        confirmation: None,
        transport: InteractionTransport::ControllerLocal,
        workflow_steps: vec![],
        form_ui: None,
        icon: None,
    };
    assert!(descriptor.validate_for_provider(ProviderKind::Plugin).is_ok());
}
```

- [ ] **Step 2: Run the tests to verify failure**

Run: `cargo test -p uptrakit-surfaces interaction::` Expected: FAIL — `InteractionDescriptor` has no
`icon` field; `InteractionValidationError::IconInvalid` does not exist.

- [ ] **Step 3: Implement the field, error variant, and validator call**

Edit `crates/shared/surfaces/src/interaction.rs`:

In `InteractionDescriptor` (line 43 area) add the field:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InteractionDescriptor {
    pub interaction_id: InteractionId,
    pub kind: InteractionKind,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_permission: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_schema: Option<SchemaContract>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_schema: Option<SchemaContract>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sensitive_fields: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirmation: Option<InteractionConfirmation>,
    pub transport: InteractionTransport,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub workflow_steps: Vec<WorkflowStepDescriptor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub form_ui: Option<FormUiDescriptor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
}
```

In `InteractionValidationError` (line 86 area) add a variant **at the end** so existing positional
users keep working — the enum is `#[non_exhaustive]` so add-only is fine:

```rust
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum InteractionValidationError {
    // ... existing variants unchanged ...
    #[error(
        "interaction `{interaction_id}` has invalid icon: {reason}"
    )]
    IconInvalid {
        interaction_id: InteractionId,
        reason: crate::IconNameError,
    },
}
```

(Confirm the existing enum carries `#[non_exhaustive]`; if not, add it as part of this change — the
snapshot rule mandates it.)

In `validate_for_provider` (line 125 area), add — just before `Ok(())` at the end:

```rust
        if let Some(icon) = &self.icon {
            crate::validate_icon_name(icon).map_err(|reason| {
                InteractionValidationError::IconInvalid {
                    interaction_id: self.interaction_id.clone(),
                    reason,
                }
            })?;
        }

        Ok(())
```

- [ ] **Step 4: Cascade `icon: None,` across every workspace struct literal**

`InteractionDescriptor` is `#[non_exhaustive]` so external crates can't use struct literals at all,
but internal workspace crates do — and adding the new public `icon` field breaks every existing
literal until each one sets it. There are roughly 74 sites across 17 files. Discover them:

```bash
grep -rn 'InteractionDescriptor\s*{' --include='*.rs' crates/ \
    | grep -v ':[[:space:]]*//' \
    | grep -v '/.worktrees/'
```

Walk every hit and add `icon: None,` next to the existing terminal field. Touched files include
(non-exhaustive — confirm via the grep above):

- `crates/plugins/notifications/{email,telegram,webhook}/src/plugin.rs`
- `crates/plugins/infrastructure/proxmox/src/plugin.rs` (all surfaces, not only `proxmox_hosts_surface`;
  Task 5 will replace these `None`s with real icons for the five Proxmox VE Hosts interactions)
- `crates/plugins/releases/docker/src/plugin.rs`
- `crates/core/mqtt-runtime/src/surface_runtime.rs`
- `crates/core/agent-ssh/src/surface_runtime.rs` (the conversion site at line 614 — Task 4 will then
  replace `None` with `action.icon.clone()`)
- `crates/core/agent-ssh/src/surface_runtime/registration/*.rs`
- `crates/ui/surface-proxy/src/registry.rs`
- `crates/ui/surface-proxy/src/proxy/tests/**/*.rs`
- `crates/ui/cli/{tests/command_execution.rs,src/commands/surfaces.rs}`
- `crates/ui/web-api/src/routes/{surfaces.rs,service_ws/handler/mod.rs}`
- `crates/shared/wire/src/wire_validate_impls.rs` (test fixtures inside the existing test module)

This step is mechanical but mandatory: skipping any literal leaves the workspace red. Use sed if you
prefer, but verify each site visually — some are inside macro invocations or test helpers that may
need slightly different formatting.

- [ ] **Step 5: Run the surfaces-crate tests to verify they pass**

Run: `cargo test -p uptrakit-surfaces interaction::`
Expected: PASS — all interaction tests including the three new ones.

- [ ] **Step 6: Run the workspace-wide check before committing**

Run: `cargo check --all-features`
Expected: compile success across the entire workspace. If any literal was missed, fix it now —
do not commit a red workspace.

- [ ] **Step 7: Quality gates for touched crates**

Run:

```bash
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
```

Expected: empty output. (Workspace-wide because the cascade touched many crates.)

- [ ] **Step 8: Commit**

Stage everything the cascade touched plus the surfaces-crate change. The commit must include every
struct-literal site that gained `icon: None,` so the workspace stays compilable at every commit.

```bash
git add -u crates/
git status   # sanity check — every modified .rs in the listed crates should be staged
git commit -m "feat(surfaces): add icon field to InteractionDescriptor and validate it"
```

---

## Task 3: Add `icon` field + `with_icon` builder to `SurfaceActionDescriptor`

**Files:**

- Modify: `crates/plugins/infrastructure/core/src/surface_form_authoring.rs`
- Test: same file (Rust unit tests)

**Snapshot rules in scope:**

- `apply #[non_exhaustive] to public enums with future variants` (`SurfaceActionDescriptor` is
  `#[non_exhaustive]` — confirm before edit)
- `cargo clippy | unwrap_used = deny` (no `unwrap` in production code)

- [ ] **Step 1: Write the failing test**

Append to the test module at the bottom of
`crates/plugins/infrastructure/core/src/surface_form_authoring.rs` (locate `#[cfg(test)] mod tests`;
add one if missing):

```rust
#[test]
fn surface_action_descriptor_with_icon_sets_field() {
    let descriptor = SurfaceActionDescriptor::new("sync-host", "Sync Host")
        .with_icon("refresh-cw");
    assert_eq!(descriptor.icon.as_deref(), Some("refresh-cw"));
}

#[test]
fn surface_action_descriptor_default_icon_is_none() {
    let descriptor = SurfaceActionDescriptor::new("sync-host", "Sync Host");
    assert!(descriptor.icon.is_none());
}
```

- [ ] **Step 2: Run the tests to verify failure**

Run: `cargo test -p uptrakit-plugin-infrastructure-core surface_action_descriptor_with_icon`
Expected: FAIL — `with_icon` not defined; `icon` field not on struct.

- [ ] **Step 3: Add the field and the builder**

Edit `crates/plugins/infrastructure/core/src/surface_form_authoring.rs`:

In `SurfaceActionDescriptor` struct (≈ line 249) add the field next to `label`:

```rust
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfaceActionDescriptor {
    pub action_id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    // ... existing fields unchanged
}
```

In `impl SurfaceActionDescriptor::new` (≈ line 282) initialize the field:

```rust
pub fn new(action_id: impl Into<String>, label: impl Into<String>) -> Self {
    Self {
        action_id: action_id.into(),
        label: label.into(),
        icon: None,
        ui: None,
        permission: String::new(),
        destructive: false,
        timeout_seconds: None,
        api_submit: None,
        row_visible_when: None,
        confirm_entity_field: None,
        batch_action: false,
    }
}
```

Add the builder method to the same `impl` block, next to `with_permission`:

```rust
/// Set the lucide-canonical kebab-case icon name (e.g. `"refresh-cw"`).
/// Validation lives in the wire layer; this builder accepts the value verbatim.
#[must_use]
pub fn with_icon(mut self, icon: impl Into<String>) -> Self {
    self.icon = Some(icon.into());
    self
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p uptrakit-plugin-infrastructure-core surface_action_descriptor` Expected: PASS —
both new tests green; existing tests still green.

- [ ] **Step 5: Quality gates for the plugin-infrastructure-core crate**

Run:
`cargo fmt -p uptrakit-plugin-infrastructure-core && cargo clippy -p uptrakit-plugin-infrastructure-core --all-targets --all-features -- -D warnings`
Expected: empty output.

- [ ] **Step 6: Commit**

```bash
git add crates/plugins/infrastructure/core/src/surface_form_authoring.rs
git commit -m "feat(plugin-core): add icon field and with_icon builder to SurfaceActionDescriptor"
```

---

## Task 4: Forward `icon` through SSH `surface_runtime` interaction conversion + apply icons to SSH Hosts actions

**Files:**

- Modify: `crates/core/agent-ssh/src/surface_runtime.rs:614` (interaction-conversion site) and the
  action definitions for `bootstrap`, `sync-host`, `remove-host`.
- Test: same file (existing `#[cfg(test)]` module).

**Snapshot rules in scope:**

- `forbid unwrap/expect/panic in production code`
- icon names: `bootstrap` → `server-cog`, `sync-host` → `refresh-cw`, `remove-host` → `trash-2` (per
  spec § 11)

- [ ] **Step 1: Write the failing test**

Append to the test module in `crates/core/agent-ssh/src/surface_runtime.rs` (search for
`#[cfg(test)] mod tests`; the file already has one near the bottom):

```rust
#[test]
fn ssh_host_actions_carry_their_icons() {
    let actions = build_actions();
    let by_id: std::collections::HashMap<&str, &SurfaceActionDescriptor> = actions
        .iter()
        .map(|a| (a.action_id.as_str(), a))
        .collect();

    assert_eq!(by_id["bootstrap"].icon.as_deref(), Some("server-cog"));
    assert_eq!(by_id["sync-host"].icon.as_deref(), Some("refresh-cw"));
    assert_eq!(by_id["remove-host"].icon.as_deref(), Some("trash-2"));
}

#[test]
fn surface_action_icon_is_forwarded_to_interaction_descriptor() {
    let registration = build_surface_registration(None, &Default::default(), None, None);
    let surface = registration
        .surfaces
        .iter()
        .find(|s| s.descriptor.surface_id.as_str() == SSH_HOSTS_SURFACE_ID)
        .expect("ssh hosts surface present");

    let bootstrap = surface
        .interactions
        .iter()
        .find(|i| i.interaction_id.as_str() == "bootstrap")
        .expect("bootstrap interaction present");
    assert_eq!(bootstrap.icon.as_deref(), Some("server-cog"));

    let sync = surface
        .interactions
        .iter()
        .find(|i| i.interaction_id.as_str() == "sync-host")
        .expect("sync-host interaction present");
    assert_eq!(sync.icon.as_deref(), Some("refresh-cw"));

    let remove = surface
        .interactions
        .iter()
        .find(|i| i.interaction_id.as_str() == "remove-host")
        .expect("remove-host interaction present");
    assert_eq!(remove.icon.as_deref(), Some("trash-2"));
}
```

(Imports: `SurfaceActionDescriptor` is already in scope; if not, add
`use uptrakit_plugin_infrastructure_registry::SurfaceActionDescriptor;`. `SSH_HOSTS_SURFACE_ID` is
module-local.)

- [ ] **Step 2: Run the tests to verify failure**

Run:
`cargo test -p uptrakit-agent-ssh surface_runtime::tests::ssh_host_actions_carry_their_icons surface_runtime::tests::surface_action_icon_is_forwarded`
Expected: FAIL — actions have no `icon`; conversion drops the field.

- [ ] **Step 3: Apply icons in `build_actions` and forward in the conversion**

Find the `bootstrap_action` builder (≈ line 943) and add `.with_icon("server-cog")`:

```rust
SurfaceActionDescriptor::new("bootstrap", "Bootstrap Host")
    .with_icon("server-cog")
    .with_permission(uptrakit_shared_types::Permission::UpdateHosts)
    // ... rest unchanged
```

Find `sync_host_action` (≈ line 805 / `SurfaceActionDescriptor::new("sync-host", "Sync Host")` at
line 862):

```rust
SurfaceActionDescriptor::new("sync-host", "Sync Host")
    .with_icon("refresh-cw")
    // ... rest unchanged
```

Find the `remove-host` action (≈ line 171 inside `build_actions`):

```rust
SurfaceActionDescriptor::new("remove-host", "Remove Host")
    .with_icon("trash-2")
    .with_permission(uptrakit_shared_types::Permission::UpdateHosts)
    // ... rest unchanged
```

Now forward the icon through the interaction conversion. In
`crates/core/agent-ssh/src/surface_runtime.rs:614`, change the literal:

```rust
        interactions.push(InteractionDescriptor {
            interaction_id,
            kind,
            label: action.label.clone(),
            required_permission: permission_or_none(&action.permission),
            input_schema: Some(surfaces::SchemaContract::Object),
            result_schema: Some(surfaces::SchemaContract::Any),
            sensitive_fields: sensitive_fields.into_iter().collect(),
            timeout_seconds,
            confirmation,
            transport: InteractionTransport::ProviderProxied,
            workflow_steps,
            form_ui,
            icon: action.icon.clone(),
        });
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p uptrakit-agent-ssh surface_runtime` Expected: PASS — both new tests, existing
tests still green.

- [ ] **Step 5: Quality gates**

Run:
`cargo fmt -p uptrakit-agent-ssh && cargo clippy -p uptrakit-agent-ssh --all-targets --all-features -- -D warnings`
Expected: empty output.

- [ ] **Step 6: Commit**

```bash
git add crates/core/agent-ssh/src/surface_runtime.rs
git commit -m "feat(agent-ssh): apply icons to SSH host actions and forward to interactions"
```

---

## Task 5: Apply `icon` to Proxmox VE Hosts surface interactions

**Files:**

- Modify: `crates/plugins/infrastructure/proxmox/src/plugin.rs:219-345` (the five
  `InteractionDescriptor` literals in `proxmox_hosts_surface`)
- Test: a new unit test inside the same file's existing test module (or add one if absent).

**Icon mapping (per spec § 11):** `discover` → `radar`; `test-connection` → `plug-zap`;
`approve-match` → `check`; `match` → `link`; `unmatch` → `unlink`. The `list` interaction stays
icon-less.

- [ ] **Step 1: Write the failing test**

Append to the test module at the bottom of `crates/plugins/infrastructure/proxmox/src/plugin.rs`
(locate `#[cfg(test)] mod tests`; create one at end-of-file if absent):

```rust
#[test]
fn proxmox_hosts_surface_interactions_carry_icons() {
    let registrations = proxmox_surface_registrations();
    let proxmox = registrations
        .first()
        .expect("plugin returns at least one registration");
    let hosts_surface = proxmox
        .surfaces
        .iter()
        .find(|s| s.descriptor.surface_id.as_str() == "proxmox.hosts")
        .expect("proxmox.hosts surface present");

    let by_id: std::collections::HashMap<&str, &surfaces::InteractionDescriptor> = hosts_surface
        .interactions
        .iter()
        .map(|i| (i.interaction_id.as_str(), i))
        .collect();

    assert_eq!(by_id["discover"].icon.as_deref(), Some("radar"));
    assert_eq!(by_id["test-connection"].icon.as_deref(), Some("plug-zap"));
    assert_eq!(by_id["approve-match"].icon.as_deref(), Some("check"));
    assert_eq!(by_id["match"].icon.as_deref(), Some("link"));
    assert_eq!(by_id["unmatch"].icon.as_deref(), Some("unlink"));
    assert!(by_id["list"].icon.is_none());
}
```

(`proxmox_surface_registrations()` is the existing helper at `crates/plugins/infrastructure/proxmox/src/plugin.rs:96` returning
`Vec<surfaces::SurfaceRegistration>`. The existing test `proxmox_hosts_surface_has_full_table_layout`
in the same file calls it; mirror that pattern.)

- [ ] **Step 2: Run the test to verify failure**

Run:
`cargo test -p uptrakit-plugin-infrastructure-proxmox proxmox_hosts_surface_interactions_carry_icons`
Expected: FAIL — every `icon.as_deref()` returns `None`.

- [ ] **Step 3: Set the `icon` field on each of the five interactions**

In `crates/plugins/infrastructure/proxmox/src/plugin.rs`, for each interaction literal in
`proxmox_hosts_surface()` (≈ lines 220–345), add the `icon` field. The existing literals look like:

```rust
surfaces::InteractionDescriptor {
    interaction_id: surfaces::InteractionId::new("discover").expect("literal"),
    kind: surfaces::InteractionKind::MutationAction,
    label: "Discover".to_string(),
    // ...
    form_ui: None,
},
```

Add `icon: Some("<name>".to_string()),` immediately after the existing terminal field. Apply per the
table above. The `list` interaction (line ≈ 221) gets `icon: None,`. Example for `discover`:

```rust
surfaces::InteractionDescriptor {
    interaction_id: surfaces::InteractionId::new("discover").expect("literal"),
    kind: surfaces::InteractionKind::MutationAction,
    label: "Discover".to_string(),
    required_permission: Some(Permission::UpdateHosts.to_string()),
    input_schema: Some(surfaces::SchemaContract::Object),
    result_schema: Some(surfaces::SchemaContract::Any),
    sensitive_fields: vec![],
    timeout_seconds: Some(120),
    confirmation: None,
    transport: surfaces::InteractionTransport::ControllerLocal,
    workflow_steps: vec![],
    form_ui: None,
    icon: Some("radar".to_string()),
},
```

Repeat for `test-connection` (`"plug-zap"`), `approve-match` (`"check"`), `match` (`"link"`),
`unmatch` (`"unlink"`), and `list` (`None`).

There are additional `InteractionDescriptor` literals in the same file for the other Proxmox
surfaces (e.g. `proxmox_host_info_surface`, `proxmox_settings_update_protection_surface`,
`proxmox_software_item_update_protection_surface`). Add `icon: None` to each of them so the new
field is set everywhere — leaving them out would be a compile error since the struct is
`#[non_exhaustive]` from the trait side but the literal must still set every public field.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p uptrakit-plugin-infrastructure-proxmox` Expected: PASS — new test green,
existing tests green.

- [ ] **Step 5: Quality gates**

Run:
`cargo fmt -p uptrakit-plugin-infrastructure-proxmox && cargo clippy -p uptrakit-plugin-infrastructure-proxmox --all-targets --all-features -- -D warnings`
Expected: empty output.

- [ ] **Step 6: Commit**

```bash
git add crates/plugins/infrastructure/proxmox/src/plugin.rs
git commit -m "feat(plugin-proxmox): assign lucide icons to Proxmox VE Hosts interactions"
```

---

## Task 6: Apply `with_icon("boxes")` to `bootstrap_proxmox_guest_action`

**Files:**

- Modify: `crates/plugins/infrastructure/proxmox/src/agent/plugin.rs:597`
- Test: same file's existing test module (or add one).

- [ ] **Step 1: Write the failing test**

Append to the test module at the bottom of
`crates/plugins/infrastructure/proxmox/src/agent/plugin.rs`:

```rust
#[test]
fn bootstrap_proxmox_guest_action_has_boxes_icon() {
    let action = bootstrap_proxmox_guest_action();
    assert_eq!(action.action_id, "bootstrap-proxmox-guest");
    assert_eq!(action.icon.as_deref(), Some("boxes"));
}
```

- [ ] **Step 2: Run the test**

Run:
`cargo test -p uptrakit-plugin-infrastructure-proxmox bootstrap_proxmox_guest_action_has_boxes_icon`
Expected: FAIL — `action.icon` is `None`.

- [ ] **Step 3: Apply the icon**

Edit `crates/plugins/infrastructure/proxmox/src/agent/plugin.rs:597`:

```rust
fn bootstrap_proxmox_guest_action() -> SurfaceActionDescriptor {
    SurfaceActionDescriptor::new("bootstrap-proxmox-guest", "Bootstrap Discovered Guest")
        .with_icon("boxes")
        .with_permission(uptrakit_shared_types::Permission::UpdateHosts)
        .with_timeout(300)
        // ... rest unchanged
```

- [ ] **Step 4: Run the test**

Run:
`cargo test -p uptrakit-plugin-infrastructure-proxmox bootstrap_proxmox_guest_action_has_boxes_icon`
Expected: PASS.

- [ ] **Step 5: Quality gates**

Run:
`cargo fmt -p uptrakit-plugin-infrastructure-proxmox && cargo clippy -p uptrakit-plugin-infrastructure-proxmox --all-targets --all-features -- -D warnings`
Expected: empty output.

- [ ] **Step 6: Commit**

```bash
git add crates/plugins/infrastructure/proxmox/src/agent/plugin.rs
git commit -m "feat(plugin-proxmox): assign boxes icon to bootstrap-proxmox-guest action"
```

---

## Task 7: Switch wire validation to `surfaces::validate_icon_name` for both `nav_icon` and `interactions[].icon`

**Files:**

- Modify: `crates/shared/wire/src/limits.rs` (delete `MAX_NAV_ICON_LEN`)
- Modify: `crates/shared/wire/src/wire_validate_impls.rs` (use shared validator; iterate interaction
  icons; update PascalCase test)
- Test: existing test module in `wire_validate_impls.rs`

**Snapshot rules in scope:**

- `forbid Result<T, String> except user-facing validation` — wire `WireValidationError` is the
  existing typed error; we map into it.

**Backward-compat note:** This task tightens `nav_icon` validation from "non-empty + length" to the
strict kebab-case regex. In-tree producers do not call `SurfaceDescriptor::nav_icon(...)` today (per
the spec's grep), but external Services / out-of-tree Plugins enrolled against an upgraded
controller will fail re-enrollment if they send PascalCase values. This is intentional — the spec
calls it out under Risks ("Kebab-case migration of `nav_icon`") — but the implementer should expect
to coordinate the controller upgrade with any deployed services that emit a `nav_icon`.

- [ ] **Step 1: Write the failing tests**

In the test module at the bottom of `crates/shared/wire/src/wire_validate_impls.rs`, find the
existing `surface_registration_accepts_valid_nav_icon` (≈ line 1896). **Replace** it (PascalCase no
longer accepted) and add new tests:

```rust
#[test]
fn surface_registration_accepts_valid_nav_icon() {
    let mut payload = test_surface_registration();
    payload.surfaces[0].descriptor.nav_icon = Some("package".to_string());
    assert!(payload.wire_validate().is_ok());
}

#[test]
fn surface_registration_rejects_pascal_case_nav_icon() {
    let mut payload = test_surface_registration();
    payload.surfaces[0].descriptor.nav_icon = Some("Package".to_string());
    let err = payload.wire_validate().unwrap_err();
    assert_eq!(err.field, "surfaces[].descriptor.nav_icon");
}

#[test]
fn surface_registration_rejects_empty_interaction_icon() {
    let mut payload = test_surface_registration();
    payload.surfaces[0].interactions[0].icon = Some(String::new());
    let err = payload.wire_validate().unwrap_err();
    assert_eq!(err.field, "surfaces[].interactions[].icon");
}

#[test]
fn surface_registration_rejects_oversized_interaction_icon() {
    let mut payload = test_surface_registration();
    payload.surfaces[0].interactions[0].icon = Some("a".repeat(65));
    let err = payload.wire_validate().unwrap_err();
    assert_eq!(err.field, "surfaces[].interactions[].icon");
}

#[test]
fn surface_registration_rejects_pascal_case_interaction_icon() {
    let mut payload = test_surface_registration();
    payload.surfaces[0].interactions[0].icon = Some("Trash2".to_string());
    let err = payload.wire_validate().unwrap_err();
    assert_eq!(err.field, "surfaces[].interactions[].icon");
}

#[test]
fn surface_registration_rejects_underscore_interaction_icon() {
    let mut payload = test_surface_registration();
    payload.surfaces[0].interactions[0].icon = Some("trash_2".to_string());
    let err = payload.wire_validate().unwrap_err();
    assert_eq!(err.field, "surfaces[].interactions[].icon");
}

#[test]
fn surface_registration_accepts_valid_interaction_icon() {
    let mut payload = test_surface_registration();
    payload.surfaces[0].interactions[0].icon = Some("trash-2".to_string());
    assert!(payload.wire_validate().is_ok());
}
```

- [ ] **Step 2: Run the tests to verify failure**

Run: `cargo test -p uptrakit-wire surface_registration_` Expected: FAIL — five new tests fail
(compile error: `interactions[0].icon` does not exist on the wire view of `InteractionDescriptor` —
but it does after Task 2; confirm). The four pre-existing `nav_icon` tests still pass; the renamed
`accepts_valid_nav_icon` test fails because the validator currently accepts `"Package"` but we will
reject it after the migration. PascalCase-rejection test fails because the validator only checks
length, not charset.

- [ ] **Step 3: Switch validation to the shared helper and add interaction-icon iteration**

Edit `crates/shared/wire/src/limits.rs` — delete the `MAX_NAV_ICON_LEN` constant entirely (≈ line
199). Any imports referencing it elsewhere must move to `surfaces::MAX_ICON_NAME_LEN`.

Edit `crates/shared/wire/src/wire_validate_impls.rs`:

Replace the existing `nav_icon` block (≈ lines 733–741):

```rust
            if let Some(nav_icon) = &surface.descriptor.nav_icon {
                surfaces::validate_icon_name(nav_icon).map_err(|err| WireValidationError {
                    field: "surfaces[].descriptor.nav_icon",
                    message: err.to_string(),
                })?;
            }
```

After the existing
`for interaction in &surface.interactions { validate_surface_interaction(interaction)?; }` loop (≈
line 753), add an inline icon validation pass — or, cleaner, fold it into
`validate_surface_interaction`. The cleanest option: extend `validate_surface_interaction` to call
`validate_icon_name`. Locate that function (search for `fn validate_surface_interaction`) and add at
the start (or end) of its body:

```rust
    if let Some(icon) = &interaction.icon {
        surfaces::validate_icon_name(icon).map_err(|err| WireValidationError {
            field: "surfaces[].interactions[].icon",
            message: err.to_string(),
        })?;
    }
```

Remove the `MAX_NAV_ICON_LEN` import at the top of `wire_validate_impls.rs` (the constant no longer
exists).

- [ ] **Step 4: Run the tests**

Run: `cargo test -p uptrakit-wire surface_registration_` Expected: PASS — all (existing + new) tests
green.

- [ ] **Step 5: Quality gates**

Run:
`cargo fmt -p uptrakit-wire -p uptrakit-surfaces && cargo clippy -p uptrakit-wire -p uptrakit-surfaces --all-targets --all-features -- -D warnings`
Expected: empty output.

- [ ] **Step 6: Workspace-wide check (catches any other call site that imported
      `MAX_NAV_ICON_LEN`)**

Run: `cargo check --all-features` Expected: compile success. If a call site outside
`wire_validate_impls.rs` imports the deleted constant, fix it by importing
`surfaces::MAX_ICON_NAME_LEN` instead.

- [ ] **Step 7: Commit**

```bash
git add crates/shared/wire/src/limits.rs crates/shared/wire/src/wire_validate_impls.rs
git commit -m "feat(wire): validate icon names via shared kebab-case validator"
```

---

## Task 8: Create the unified frontend icon registry `frontend/src/lib/icons.ts`

**Files:**

- Create: `frontend/src/lib/icons.ts`
- Create: `frontend/src/lib/icons.test.ts`

**Snapshot rules in scope:**

- `TypeScript: strict = true, checkJs = true`
- `Prettier: tabs, single quotes, trailing comma = none, width = 120`

- [ ] **Step 1: Write the failing tests**

Create `frontend/src/lib/icons.test.ts`:

```ts
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { Box, Trash2 } from "lucide-svelte";
import { ICONS, resolveIcon } from "./icons";

describe("resolveIcon", () => {
  let consoleErrorSpy: ReturnType<typeof vi.spyOn>;

  beforeEach(() => {
    consoleErrorSpy = vi.spyOn(console, "error").mockImplementation(() => {});
  });

  afterEach(() => {
    consoleErrorSpy.mockRestore();
  });

  it("resolves a known kebab-case icon name", () => {
    const result = resolveIcon("trash-2");
    expect(result.ok).toBe(true);
    expect(result.component).toBe(Trash2);
    expect(consoleErrorSpy).not.toHaveBeenCalled();
  });

  it("returns the Box fallback and logs an error for an unknown name", () => {
    const result = resolveIcon("Trash2");
    expect(result.ok).toBe(false);
    expect(result.component).toBe(Box);
    expect(consoleErrorSpy).toHaveBeenCalledWith('[surfaces] Unknown icon name: "Trash2"');
  });

  it("returns the Box fallback without logging when the name is null", () => {
    const result = resolveIcon(null);
    expect(result.ok).toBe(false);
    expect(result.component).toBe(Box);
    expect(consoleErrorSpy).not.toHaveBeenCalled();
  });

  it("returns the Box fallback without logging when the name is undefined", () => {
    const result = resolveIcon(undefined);
    expect(result.ok).toBe(false);
    expect(result.component).toBe(Box);
    expect(consoleErrorSpy).not.toHaveBeenCalled();
  });

  it("contains every key referenced by the surface refactor", () => {
    const required = [
      "box",
      "boxes",
      "check",
      "link",
      "plug-zap",
      "radar",
      "refresh-cw",
      "server-cog",
      "trash-2",
      "unlink",
    ];
    for (const key of required) {
      expect(ICONS[key], `expected key "${key}" in ICONS`).toBeDefined();
    }
  });
});
```

- [ ] **Step 2: Run the tests to verify failure**

Run: `cd frontend && npx vitest run src/lib/icons.test.ts` Expected: FAIL —
`Cannot find module './icons'`.

- [ ] **Step 3: Implement the registry**

Create `frontend/src/lib/icons.ts`:

```ts
import type { ComponentType, SvelteComponent } from "svelte";
import {
  Box,
  Boxes,
  Check,
  Cpu,
  Database,
  FileText,
  Globe,
  HardDrive,
  History,
  Layers,
  Link,
  Package,
  PlugZap,
  Puzzle,
  Radar,
  RefreshCw,
  ScrollText,
  Server,
  ServerCog,
  Settings,
  Shield,
  Tag,
  Tags,
  Trash2,
  Unlink,
  Wrench,
} from "lucide-svelte";

export type IconComponent = ComponentType<SvelteComponent>;

export const ICONS: Record<string, IconComponent> = {
  box: Box,
  boxes: Boxes,
  check: Check,
  cpu: Cpu,
  database: Database,
  "file-text": FileText,
  globe: Globe,
  "hard-drive": HardDrive,
  history: History,
  layers: Layers,
  link: Link,
  package: Package,
  "plug-zap": PlugZap,
  puzzle: Puzzle,
  radar: Radar,
  "refresh-cw": RefreshCw,
  "scroll-text": ScrollText,
  server: Server,
  "server-cog": ServerCog,
  settings: Settings,
  shield: Shield,
  tag: Tag,
  tags: Tags,
  "trash-2": Trash2,
  unlink: Unlink,
  wrench: Wrench,
};

export interface ResolvedIcon {
  component: IconComponent;
  ok: boolean;
}

const FALLBACK: IconComponent = Box;

export function resolveIcon(name: string | null | undefined): ResolvedIcon {
  if (!name) {
    return { component: FALLBACK, ok: false };
  }
  const component = ICONS[name];
  if (!component) {
    console.error(`[surfaces] Unknown icon name: "${name}"`);
    return { component: FALLBACK, ok: false };
  }
  return { component, ok: true };
}
```

- [ ] **Step 4: Run the tests**

Run: `cd frontend && npx vitest run src/lib/icons.test.ts` Expected: PASS — all five tests green.

- [ ] **Step 5: Format and lint**

Run:
`cd frontend && npx prettier --write src/lib/icons.ts src/lib/icons.test.ts && npm run lint -- src/lib/icons.ts src/lib/icons.test.ts`
Expected: prettier reports the files were already formatted (or formats them); ESLint clean.

- [ ] **Step 6: Commit**

```bash
git add frontend/src/lib/icons.ts frontend/src/lib/icons.test.ts
git commit -m "feat(frontend): add unified kebab-case icon registry with logged fallback"
```

---

## Task 9: Migrate `registry.svelte.ts` and `+layout.svelte` to `resolveIcon`, then delete `nav-icons.ts`

**Files:**

- Modify: `frontend/src/lib/surfaces/registry.svelte.ts:55`
- Modify: `frontend/src/lib/surfaces/registry.test.ts` (update PascalCase fixtures)
- Modify: `frontend/src/routes/+layout.svelte:47,280` (also imports `resolveNavIcon`).
- Delete: `frontend/src/lib/nav-icons.ts`
- Delete: `frontend/src/lib/nav-icons.test.ts`

- [ ] **Step 1: Update the test fixtures**

In `frontend/src/lib/surfaces/registry.test.ts`, find the `nav_icon` fixture near line 243 (the test
that "passes nav_icon through as icon when set") and any test data that uses PascalCase strings like
`'Package'` or `'Server'`. Replace with kebab-case (e.g. `'package'`, `'server'`). Also add a new
test:

```ts
it("falls back to 'box' when nav_icon is absent", () => {
  const surface = makeSurface({ nav_icon: undefined });
  const items = resolveSurfacePageNavItems([surface]);
  expect(items[0].icon).toBe("box");
});
```

(Use the existing `makeSurface`/`makeFixture` helper — match its actual name in the file.)

- [ ] **Step 2: Run the registry tests to verify failure**

Run: `cd frontend && npx vitest run src/lib/surfaces/registry.test.ts` Expected: FAIL — `'Box'`
(PascalCase) is still the fallback in `registry.svelte.ts`.

- [ ] **Step 3: Update `registry.svelte.ts`**

Edit `frontend/src/lib/surfaces/registry.svelte.ts:55`. `resolveSurfacePageNavItems` keeps its
current contract (returns `icon: string`); component-resolution lives downstream in `+layout.svelte`.
The change here is just the lowercase fallback:

```ts
icon: surface.nav_icon ?? "box";
```

- [ ] **Step 4: Update `+layout.svelte` to use `resolveIcon`**

`+layout.svelte` is the only live consumer of `resolveNavIcon`. Edit
`frontend/src/routes/+layout.svelte:47` — replace the import:

```svelte
import { resolveIcon } from '$lib/icons';
```

Edit `frontend/src/routes/+layout.svelte:280` — pick the component out of `resolveIcon`'s
`{ component, ok }` return shape:

```svelte
icon: resolveIcon(item.icon).component
```

Run `grep -rn "resolveNavIcon\|nav-icons" frontend/src` and confirm no remaining references outside
this plan / spec docs.

- [ ] **Step 5: Delete the old files**

```bash
git rm frontend/src/lib/nav-icons.ts frontend/src/lib/nav-icons.test.ts
```

- [ ] **Step 6: Run the test suite**

Run: `cd frontend && npm run check && npx vitest run` Expected: PASS — all tests green;
`npm run check` reports no missing module.

- [ ] **Step 7: Format and lint**

Run:

```bash
cd frontend
npx prettier --write src/lib/surfaces/registry.svelte.ts \
                     src/lib/surfaces/registry.test.ts \
                     src/routes/+layout.svelte
npm run lint
```

Expected: clean.

- [ ] **Step 8: Commit**

```bash
git add frontend/src/lib/surfaces/registry.svelte.ts \
        frontend/src/lib/surfaces/registry.test.ts \
        frontend/src/routes/+layout.svelte \
        frontend/src/lib/nav-icons.ts \
        frontend/src/lib/nav-icons.test.ts
git commit -m "refactor(frontend): retire nav-icons.ts; use unified kebab-case icon registry"
```

---

## Task 10: Add `icon` field to the frontend `InteractionDescriptor` type

**Files:**

- Modify: `frontend/src/lib/surfaces/contract.ts:226-239`

- [ ] **Step 1: Edit the type**

```ts
export interface InteractionDescriptor {
  interaction_id: InteractionId;
  kind: InteractionKind;
  label: string;
  required_permission?: string;
  input_schema?: SchemaContract;
  result_schema?: SchemaContract;
  sensitive_fields?: string[];
  timeout_seconds?: number;
  confirmation?: InteractionConfirmation;
  transport: InteractionTransport;
  workflow_steps?: WorkflowStepDescriptor[];
  form_ui?: FormUiDescriptor;
  icon?: string;
}
```

- [ ] **Step 2: Type-check**

Run: `cd frontend && npm run check` Expected: no errors.

- [ ] **Step 3: Format**

Run: `cd frontend && npx prettier --write src/lib/surfaces/contract.ts`

- [ ] **Step 4: Commit**

```bash
git add frontend/src/lib/surfaces/contract.ts
git commit -m "feat(frontend): add optional icon field to InteractionDescriptor type"
```

---

## Task 11: Add the `LabelDisplay` const-object type

**Files:**

- Create: `frontend/src/lib/surfaces/label-display.ts`

- [ ] **Step 1: Create the file**

```ts
export const LabelDisplay = {
  Always: "always",
  Auto: "auto",
  IconOnly: "icon-only",
} as const;

export type LabelDisplay = (typeof LabelDisplay)[keyof typeof LabelDisplay];
```

- [ ] **Step 2: Type-check + format**

Run: `cd frontend && npm run check && npx prettier --write src/lib/surfaces/label-display.ts`
Expected: clean.

- [ ] **Step 3: Commit**

```bash
git add frontend/src/lib/surfaces/label-display.ts
git commit -m "feat(frontend): add LabelDisplay type for surface buttons"
```

---

## Task 12: Implement `SurfaceActionButton.svelte` with full test matrix

**Files:**

- Create: `frontend/src/lib/components/surfaces/SurfaceActionButton.svelte`
- Create: `frontend/src/lib/components/surfaces/SurfaceActionButton.test.ts`

**Snapshot rules in scope:**

- `TypeScript: strict = true`
- `Prettier: tabs, single quotes`
- This component is the single owner of the icon + label-display contract per spec § 4.

- [ ] **Step 1: Write the failing tests**

Create `frontend/src/lib/components/surfaces/SurfaceActionButton.test.ts`:

```ts
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render } from "@testing-library/svelte";
import SurfaceActionButton from "./SurfaceActionButton.svelte";

afterEach(() => cleanup());

describe("SurfaceActionButton", () => {
  it('renders the visible label when labelDisplay is "always"', () => {
    const { container } = render(SurfaceActionButton, {
      label: "Sync",
      icon: "refresh-cw",
      labelDisplay: "always",
      variant: "primary",
    });
    expect(container.textContent).toContain("Sync");
    expect(container.querySelector(".sr-only")).toBeNull();
    expect(container.querySelector(".button-label-auto")).toBeNull();
    expect(container.querySelector("span[title]")).toBeNull();
  });

  it("renders sr-only label and tooltip wrapper for icon-only with icon", () => {
    const { container } = render(SurfaceActionButton, {
      label: "Sync",
      icon: "refresh-cw",
      labelDisplay: "icon-only",
      variant: "primary",
    });
    const srOnly = container.querySelector(".sr-only");
    expect(srOnly?.textContent).toBe("Sync");
    const tooltipWrap = container.querySelector('span[title="Sync"]');
    expect(tooltipWrap).not.toBeNull();
    expect(tooltipWrap?.classList.contains("inline-flex")).toBe(true);
    const button = container.querySelector("button");
    expect(button?.getAttribute("aria-label")).toBe("Sync");
  });

  it("renders button-label-auto span and tooltip wrapper for auto with icon", () => {
    const { container } = render(SurfaceActionButton, {
      label: "Sync",
      icon: "refresh-cw",
      labelDisplay: "auto",
      variant: "primary",
    });
    const labelSpan = container.querySelector(".button-label-auto");
    expect(labelSpan?.textContent).toBe("Sync");
    const tooltipWrap = container.querySelector('span[title="Sync"]');
    expect(tooltipWrap).not.toBeNull();
  });

  it("forces always when labelDisplay is icon-only but icon is missing", () => {
    const { container } = render(SurfaceActionButton, {
      label: "Sync",
      icon: undefined,
      labelDisplay: "icon-only",
      variant: "primary",
    });
    expect(container.textContent).toContain("Sync");
    expect(container.querySelector(".sr-only")).toBeNull();
    expect(container.querySelector("span[title]")).toBeNull();
  });

  it("forces always when labelDisplay is auto but icon is missing", () => {
    const { container } = render(SurfaceActionButton, {
      label: "Sync",
      icon: undefined,
      labelDisplay: "auto",
      variant: "primary",
    });
    expect(container.textContent).toContain("Sync");
    expect(container.querySelector(".button-label-auto")).toBeNull();
    expect(container.querySelector("span[title]")).toBeNull();
  });

  it("omits the tooltip wrapper when disabled is true even with icon-only", () => {
    const { container } = render(SurfaceActionButton, {
      label: "Sync",
      icon: "refresh-cw",
      labelDisplay: "icon-only",
      disabled: true,
      variant: "primary",
    });
    expect(container.querySelector(".sr-only")?.textContent).toBe("Sync");
    expect(container.querySelector("span[title]")).toBeNull();
    const button = container.querySelector("button");
    expect(button?.getAttribute("aria-label")).toBe("Sync");
    expect(button?.disabled).toBe(true);
  });

  it("renders the leading icon when the resolver succeeds", () => {
    const { container } = render(SurfaceActionButton, {
      label: "Sync",
      icon: "refresh-cw",
      variant: "primary",
    });
    // lucide-svelte renders an <svg>; presence indicates icon was passed.
    expect(container.querySelector("svg")).not.toBeNull();
  });

  it("does not render an icon when the resolver fails (unknown name)", () => {
    const consoleSpy = vi.spyOn(console, "error").mockImplementation(() => {});
    const { container } = render(SurfaceActionButton, {
      label: "Sync",
      icon: "NotARealIcon",
      labelDisplay: "icon-only",
      variant: "primary",
    });
    // Resolver logged the failure and forced effectiveDisplay='always'.
    expect(consoleSpy).toHaveBeenCalled();
    expect(container.textContent).toContain("Sync");
    expect(container.querySelector(".sr-only")).toBeNull();
    consoleSpy.mockRestore();
  });
});
```

(`vi` is included in the import at the top of Step 1 — used by the resolver-failure test case below.)

- [ ] **Step 2: Run the tests to verify failure**

Run: `cd frontend && npx vitest run src/lib/components/surfaces/SurfaceActionButton.test.ts`
Expected: FAIL — `SurfaceActionButton.svelte` does not exist.

- [ ] **Step 3: Implement `SurfaceActionButton.svelte`**

Create `frontend/src/lib/components/surfaces/SurfaceActionButton.svelte`:

```svelte
<script lang="ts" module>
	import type { ButtonVariant, ButtonSize } from '$lib/components/Button.svelte';
	import type { LabelDisplay } from '$lib/surfaces/label-display';

	export interface SurfaceActionButtonProps {
		label: string;
		icon?: string | null;
		labelDisplay?: LabelDisplay;
		variant: ButtonVariant;
		size?: ButtonSize;
		loading?: boolean;
		disabled?: boolean;
		onclick?: () => void;
		dataUi?: string;
	}
</script>

<script lang="ts">
	import Button from '$lib/components/Button.svelte';
	import { resolveIcon } from '$lib/icons';

	let {
		label,
		icon,
		labelDisplay = 'always',
		variant,
		size = 'md',
		loading = false,
		disabled = false,
		onclick,
		dataUi
	}: SurfaceActionButtonProps = $props();

	const resolved = $derived(resolveIcon(icon));
	const effectiveDisplay = $derived(resolved.ok ? labelDisplay : 'always');
	const iconSize = $derived(size === 'sm' ? 14 : 16);
	const showTooltipWrap = $derived(effectiveDisplay !== 'always' && !disabled);
	const ariaLabel = $derived(effectiveDisplay !== 'always' ? label : undefined);
</script>

{#snippet leadingIcon()}
	{#if resolved.ok}
		{@const Component = resolved.component}
		<Component size={iconSize} aria-hidden="true" />
	{/if}
{/snippet}

{#snippet body()}
	<Button
		{variant}
		{size}
		{loading}
		{disabled}
		{onclick}
		{ariaLabel}
		data-ui={dataUi}
		leadingIcon={resolved.ok ? leadingIcon : undefined}
	>
		{#if effectiveDisplay === 'always'}
			{label}
		{:else if effectiveDisplay === 'auto'}
			<span class="button-label-auto @max-[28em]/buttons:sr-only">{label}</span>
		{:else}
			<span class="sr-only">{label}</span>
		{/if}
	</Button>
{/snippet}

{#if showTooltipWrap}
	<span title={label} class="inline-flex">
		{@render body()}
	</span>
{:else}
	{@render body()}
{/if}
```

Note on the wrapper class: `class="inline-flex"` matches the layout the existing context-gated
wrapper produces at `SurfaceInteractionButton.svelte:102` (a plain inline `<span>` whose intrinsic
size is the child Button's). `display: contents` is intentionally avoided — its interaction with
the native `title` attribute is browser-inconsistent (Safari has reported regressions in this area),
and the proven pattern in the existing code is the inline span. Verify in the dev preview at Task 18
that the tooltip appears on hover for all three label-display states.

- [ ] **Step 4: Run the tests**

Run: `cd frontend && npx vitest run src/lib/components/surfaces/SurfaceActionButton.test.ts`
Expected: PASS — all eight tests green.

- [ ] **Step 5: Type-check + format**

Run:
`cd frontend && npm run check && npx prettier --write src/lib/components/surfaces/SurfaceActionButton.svelte src/lib/components/surfaces/SurfaceActionButton.test.ts`
Expected: no type errors; format clean.

- [ ] **Step 6: Commit**

```bash
git add frontend/src/lib/components/surfaces/SurfaceActionButton.svelte frontend/src/lib/components/surfaces/SurfaceActionButton.test.ts
git commit -m "feat(frontend): add SurfaceActionButton with adaptive labelDisplay and icon"
```

---

## Task 13: Refactor `SurfaceInteractionButton` to delegate to `SurfaceActionButton`

**Files:**

- Modify: `frontend/src/lib/components/surfaces/SurfaceInteractionButton.svelte`
- Modify: `frontend/src/lib/components/surfaces/SurfaceInteractionButton.test.ts`

- [ ] **Step 1: Write the new forwarding tests**

Append to `frontend/src/lib/components/surfaces/SurfaceInteractionButton.test.ts`:

```ts
it("passes labelDisplay and icon down to SurfaceActionButton", async () => {
  const interaction: InteractionDescriptor = {
    interaction_id: "sync-host",
    kind: "mutation_action",
    label: "Sync",
    transport: { mode: "controller_local" },
    icon: "refresh-cw",
  };

  const { container } = render(SurfaceInteractionButton, {
    surfaceId: "ssh-agent.hosts",
    interaction,
    interactions: [interaction],
    labelDisplay: "icon-only",
  });

  expect(container.querySelector(".sr-only")?.textContent).toBe("Sync");
  expect(container.querySelector('span[title="Sync"]')).not.toBeNull();
  expect(container.querySelector("svg")).not.toBeNull();
});

it("keeps the context-gated outer span and disables the inner button", () => {
  const interaction: InteractionDescriptor = {
    interaction_id: "sync-host",
    kind: "mutation_action",
    label: "Sync",
    transport: { mode: "controller_local" },
    icon: "refresh-cw",
  };

  const { container } = render(SurfaceInteractionButton, {
    surfaceId: "ssh-agent.hosts",
    interaction,
    interactions: [interaction],
    baseParams: {},
    labelDisplay: "icon-only",
    requiredContextParam: "config_id",
  });

  const gateSpan = container.querySelector('span[title="Select a configuration first"]');
  expect(gateSpan).not.toBeNull();
  const button = container.querySelector("button");
  expect(button?.disabled).toBe(true);
});
```

- [ ] **Step 2: Run the tests**

Run: `cd frontend && npx vitest run src/lib/components/surfaces/SurfaceInteractionButton.test.ts`
Expected: FAIL — `labelDisplay` prop unknown; component still renders inline `<Button>`.

- [ ] **Step 3: Refactor the component**

Edit `frontend/src/lib/components/surfaces/SurfaceInteractionButton.svelte`. Replace the imports and
prop block:

```svelte
<script lang="ts">
	import ConfirmDialog from '$lib/components/ConfirmDialog.svelte';
	import Callout from '$lib/components/ui/Callout.svelte';
	import SurfaceActionButton from './SurfaceActionButton.svelte';
	import { invokeSurfaceInteraction } from '$lib/api';
	import SurfaceForm from './SurfaceForm.svelte';
	import SurfaceWorkflow from './SurfaceWorkflow.svelte';
	import SurfaceModal from './SurfaceModal.svelte';
	import { buildSurfaceInteractionRequest, type SurfaceEncryptionContext } from '$lib/surfaces/interactions';
	import { showError, showSuccess } from '$lib/notifications.svelte';
	import type { InteractionDescriptor } from '$lib/surfaces/contract';
	import type { LabelDisplay } from '$lib/surfaces/label-display';

	let {
		surfaceId,
		interaction,
		interactions = [],
		targetProviderId,
		encryptionContext,
		baseParams = {},
		rowSeed,
		size = 'md',
		oncomplete,
		requiredContextParam,
		labelDisplay = 'always'
	}: {
		surfaceId: string;
		interaction: InteractionDescriptor;
		interactions?: InteractionDescriptor[];
		targetProviderId?: string;
		encryptionContext?: SurfaceEncryptionContext;
		baseParams?: Record<string, unknown>;
		rowSeed?: Record<string, unknown>;
		size?: 'sm' | 'md';
		oncomplete?: (result: unknown) => void | Promise<void>;
		requiredContextParam?: string;
		labelDisplay?: LabelDisplay;
	} = $props();
```

(Keep all existing `$state`, `$derived`, and helper code below the prop block intact.)

In the workflow delegation branch, forward `labelDisplay`:

```svelte
{:else if isWorkflow}
	<SurfaceWorkflow
		{surfaceId}
		{interaction}
		{interactions}
		{targetProviderId}
		{encryptionContext}
		baseParams={formBaseParams}
		{size}
		{oncomplete}
		{labelDisplay}
	/>
```

Replace the active branch (around lines 100–116) and the context-gated branch (around lines 101–106)
with `SurfaceActionButton` calls:

```svelte
{:else}
	{#if isContextGated}
		<span title="Select a configuration first">
			<SurfaceActionButton
				label={actionLabel}
				icon={interaction.icon}
				{labelDisplay}
				variant={interaction.confirmation?.severity === 'danger' ? 'danger' : 'primary'}
				{size}
				disabled
			/>
		</span>
	{:else}
		<SurfaceActionButton
			label={actionLabel}
			icon={interaction.icon}
			{labelDisplay}
			variant={interaction.confirmation?.severity === 'danger' ? 'danger' : 'primary'}
			{size}
			{loading}
			onclick={requestAction}
		/>
	{/if}

	{#if showModal}
		<!-- ... existing SurfaceModal block unchanged ... -->
	{/if}

	{#if showConfirm && interaction.confirmation}
		<!-- ... existing ConfirmDialog block unchanged ... -->
	{/if}
{/if}
```

(Keep `Button` import out — `SurfaceActionButton` replaces every direct Button render in this file.)

- [ ] **Step 4: Run the tests**

Run: `cd frontend && npx vitest run src/lib/components/surfaces/SurfaceInteractionButton.test.ts`
Expected: PASS — all existing + new tests green.

- [ ] **Step 5: Type-check, format, lint**

Run:

```bash
cd frontend && npm run check
npx prettier --write src/lib/components/surfaces/SurfaceInteractionButton.svelte \
                     src/lib/components/surfaces/SurfaceInteractionButton.test.ts
npm run lint
```

Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add frontend/src/lib/components/surfaces/SurfaceInteractionButton.svelte frontend/src/lib/components/surfaces/SurfaceInteractionButton.test.ts
git commit -m "refactor(frontend): delegate SurfaceInteractionButton rendering to SurfaceActionButton"
```

---

## Task 14: Refactor `SurfaceWorkflow` to delegate the trigger to `SurfaceActionButton` and accept `labelDisplay`

**Files:**

- Modify: `frontend/src/lib/components/surfaces/SurfaceWorkflow.svelte`
- Modify: `frontend/src/lib/components/surfaces/SurfaceWorkflow.test.ts`

- [ ] **Step 1: Write the failing forwarding tests**

Append to `frontend/src/lib/components/surfaces/SurfaceWorkflow.test.ts`:

```ts
it("renders the workflow trigger via SurfaceActionButton with icon", () => {
  const interaction: InteractionDescriptor = {
    interaction_id: "wizard",
    kind: "workflow",
    label: "Run Wizard",
    transport: { mode: "controller_local" },
    icon: "server-cog",
    workflow_steps: [
      {
        step_id: "s1",
        label: "Step 1",
        input_schema: "object",
        result_schema: "any",
      },
    ],
  };

  const { container } = render(SurfaceWorkflow, {
    surfaceId: "demo",
    interaction,
    interactions: [interaction],
    labelDisplay: "icon-only",
  });

  expect(container.querySelector(".sr-only")?.textContent).toBe("Run Wizard");
  expect(container.querySelector('span[title="Run Wizard"]')).not.toBeNull();
  expect(container.querySelector("svg")).not.toBeNull();
});
```

- [ ] **Step 2: Run the test**

Run: `cd frontend && npx vitest run src/lib/components/surfaces/SurfaceWorkflow.test.ts` Expected:
FAIL — `labelDisplay` prop unknown; trigger still uses raw `Button`.

- [ ] **Step 3: Refactor the component**

Edit `frontend/src/lib/components/surfaces/SurfaceWorkflow.svelte`:

In the `<script>` imports, swap `Button` for `SurfaceActionButton`:

```svelte
import SurfaceActionButton from './SurfaceActionButton.svelte';
import type { LabelDisplay } from '$lib/surfaces/label-display';
```

(Keep the other imports. `Modal` remains in use for the wizard modal.)

In the prop destructure (line 16 area), add:

```ts
let {
  surfaceId,
  interaction,
  interactions = [],
  targetProviderId,
  encryptionContext,
  baseParams = {},
  size = "md",
  oncomplete,
  labelDisplay = "always",
}: {
  surfaceId: string;
  interaction: InteractionDescriptor;
  interactions?: InteractionDescriptor[];
  targetProviderId?: string;
  encryptionContext?: SurfaceEncryptionContext;
  baseParams?: Record<string, unknown>;
  size?: "sm" | "md";
  oncomplete?: (result: unknown) => void | Promise<void>;
  labelDisplay?: LabelDisplay;
} = $props();
```

Replace the trigger render around line 339:

```svelte
{#if actionLabel.length === 0 || showContractIssue}
	<Callout tone="warning" title="Action unavailable" message="This action is not available right now." />
{:else}
	<SurfaceActionButton
		label={actionLabel}
		icon={interaction.icon}
		{labelDisplay}
		variant={confirmVariantForSeverity}
		{size}
		{loading}
		onclick={startWorkflow}
		dataUi="workflow-trigger"
	/>
{/if}
```

Remove the now-unused `Button` import only if no other site in the file uses it (the modal footer
may still — check, leave the import if so).

- [ ] **Step 4: Run the tests**

Run: `cd frontend && npx vitest run src/lib/components/surfaces/SurfaceWorkflow.test.ts` Expected:
PASS.

- [ ] **Step 5: Type-check, format, lint**

Run:

```bash
cd frontend && npm run check
npx prettier --write src/lib/components/surfaces/SurfaceWorkflow.svelte \
                     src/lib/components/surfaces/SurfaceWorkflow.test.ts
npm run lint
```

Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add frontend/src/lib/components/surfaces/SurfaceWorkflow.svelte frontend/src/lib/components/surfaces/SurfaceWorkflow.test.ts
git commit -m "refactor(frontend): delegate SurfaceWorkflow trigger to SurfaceActionButton"
```

---

## Task 15: `SurfaceActionBar` adds `@container/buttons` and `labelDisplay='auto'` when icon set

**Files:**

- Modify: `frontend/src/lib/components/surfaces/SurfaceActionBar.svelte`
- Modify: `frontend/src/lib/components/surfaces/SurfaceActionBar.test.ts`

- [ ] **Step 1: Write the failing tests**

Append to `frontend/src/lib/components/surfaces/SurfaceActionBar.test.ts`:

```ts
it("marks the outer wrapper as a button container query parent", () => {
  const interaction: InteractionDescriptor = {
    interaction_id: "sync",
    kind: "mutation_action",
    label: "Sync",
    transport: { mode: "controller_local" },
    icon: "refresh-cw",
  };

  const { container } = render(SurfaceActionBar, {
    surfaceId: "demo",
    actionIds: ["sync"],
    interactions: [interaction],
  });

  const bar = container.querySelector('[data-ui="surface-action-bar"]');
  expect(bar).not.toBeNull();
  expect(bar?.classList.contains("@container/buttons")).toBe(true);
});

it('passes labelDisplay="auto" to children when interaction has icon', () => {
  const interaction: InteractionDescriptor = {
    interaction_id: "sync",
    kind: "mutation_action",
    label: "Sync",
    transport: { mode: "controller_local" },
    icon: "refresh-cw",
  };

  const { container } = render(SurfaceActionBar, {
    surfaceId: "demo",
    actionIds: ["sync"],
    interactions: [interaction],
  });

  // labelDisplay='auto' renders the label inside .button-label-auto.
  expect(container.querySelector(".button-label-auto")?.textContent).toBe("Sync");
});

it('passes labelDisplay="always" to children when interaction has no icon', () => {
  const interaction: InteractionDescriptor = {
    interaction_id: "sync",
    kind: "mutation_action",
    label: "Sync",
    transport: { mode: "controller_local" },
  };

  const { container } = render(SurfaceActionBar, {
    surfaceId: "demo",
    actionIds: ["sync"],
    interactions: [interaction],
  });

  expect(container.querySelector(".button-label-auto")).toBeNull();
  expect(container.textContent).toContain("Sync");
});
```

- [ ] **Step 2: Run the tests**

Run: `cd frontend && npx vitest run src/lib/components/surfaces/SurfaceActionBar.test.ts` Expected:
FAIL — outer wrapper missing `@container/buttons`; child receives no `labelDisplay`.

- [ ] **Step 3: Update `SurfaceActionBar.svelte`**

In `frontend/src/lib/components/surfaces/SurfaceActionBar.svelte`, change the outer `<div>` (line 57
area):

```svelte
<div class="@container/buttons flex flex-wrap justify-end gap-2" data-ui="surface-action-bar">
	{#each resolvedActions as interaction (interaction.interaction_id)}
		<SurfaceInteractionButton
			{surfaceId}
			{interaction}
			{interactions}
			{targetProviderId}
			{encryptionContext}
			{baseParams}
			labelDisplay={interaction.icon ? 'auto' : 'always'}
			requiredContextParam={requiredForInteractionIds.includes(interaction.interaction_id)
				? requiredContextParam
				: undefined}
			oncomplete={async () => {
				notifySurfaceReload();
			}}
		/>
	{/each}
</div>
```

- [ ] **Step 4: Run the tests**

Run: `cd frontend && npx vitest run src/lib/components/surfaces/SurfaceActionBar.test.ts` Expected:
PASS.

- [ ] **Step 5: Type-check, format, lint**

Run:

```bash
cd frontend && npm run check
npx prettier --write src/lib/components/surfaces/SurfaceActionBar.svelte \
                     src/lib/components/surfaces/SurfaceActionBar.test.ts
npm run lint
```

Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add frontend/src/lib/components/surfaces/SurfaceActionBar.svelte frontend/src/lib/components/surfaces/SurfaceActionBar.test.ts
git commit -m "feat(frontend): SurfaceActionBar opts into container query and auto labelDisplay"
```

---

## Task 16: `SurfaceTable` row actions become single-line, `labelDisplay='icon-only'`

**Files:**

- Modify: `frontend/src/lib/components/surfaces/SurfaceTable.svelte`
- Modify: `frontend/src/lib/components/surfaces/SurfaceTable.test.ts`

- [ ] **Step 1: Write the failing tests**

Append to `frontend/src/lib/components/surfaces/SurfaceTable.test.ts`:

```ts
it("row-action wrapper uses flex-nowrap and @container/buttons (entity-link path)", async () => {
  // Arrange a surface with a column whose cell_type.kind === 'entity_link'
  // and one row action that has an icon. The exact harness mirrors existing
  // tests in this file — copy the entity-link fixture and add an icon-bearing
  // interaction.
  // ... fixture omitted for brevity; reuse existing helper ...

  const { container } = await renderEntityLinkTable({
    rowActionInteraction: {
      interaction_id: "remove",
      kind: "mutation_action",
      label: "Remove",
      transport: { mode: "controller_local" },
      icon: "trash-2",
    },
  });

  const wrappers = container.querySelectorAll("td .flex.flex-nowrap.\\@container\\/buttons");
  expect(wrappers.length).toBeGreaterThan(0);
  expect(container.querySelector(".sr-only")?.textContent).toBe("Remove");
});

it("row-action wrapper uses flex-nowrap and @container/buttons (rowActions snippet path)", async () => {
  // Same fixture but for a non-entity-link table that uses DataTable's
  // rowActions snippet path.
  // ... fixture omitted; reuse the existing helper ...

  const { container } = await renderPlainRowActionsTable({
    rowActionInteraction: {
      interaction_id: "remove",
      kind: "mutation_action",
      label: "Remove",
      transport: { mode: "controller_local" },
      icon: "trash-2",
    },
  });

  const wrapper = container.querySelector(".flex.flex-nowrap.\\@container\\/buttons");
  expect(wrapper).not.toBeNull();
  expect(container.querySelector(".sr-only")?.textContent).toBe("Remove");
});

it('passes labelDisplay="always" when interaction has no icon (entity-link path)', async () => {
  const { container } = await renderEntityLinkTable({
    rowActionInteraction: {
      interaction_id: "remove",
      kind: "mutation_action",
      label: "Remove",
      transport: { mode: "controller_local" },
    },
  });

  expect(container.querySelector(".sr-only")).toBeNull();
  expect(container.textContent).toContain("Remove");
});
```

(If `renderEntityLinkTable` / `renderPlainRowActionsTable` helpers don't exist, write inline test
fixtures matching the existing test file's pattern; the file already has working table fixtures —
reuse them.)

- [ ] **Step 2: Run the tests**

Run: `cd frontend && npx vitest run src/lib/components/surfaces/SurfaceTable.test.ts` Expected: FAIL
— wrappers still `flex flex-wrap`; no `@container/buttons`; rowActions snippet path has no wrapping
div.

- [ ] **Step 3: Edit the entity-link branch (line ≈ 261)**

```svelte
{#if hasRowActions}
	<td class="table-cell-pad whitespace-nowrap">
		<div class="@container/buttons flex flex-nowrap items-center gap-1">
			{#each resolvedRowActions as { rowAction, interaction } (rowAction.interaction_id)}
				{#if isRowActionVisible(rowAction, rowRecord)}
					<SurfaceInteractionButton
						{surfaceId}
						{interaction}
						{interactions}
						{targetProviderId}
						{encryptionContext}
						baseParams={rowParams(rowRecord)}
						rowSeed={rowRecord}
						size="sm"
						labelDisplay={interaction.icon ? 'icon-only' : 'always'}
						oncomplete={async () => {
							await loadPage(currentPage);
						}}
					/>
				{/if}
			{/each}
		</div>
	</td>
{/if}
```

- [ ] **Step 4: Edit the rowActions snippet branch (line ≈ 311) to add a wrapping `<div>`**

```svelte
{#snippet rowActions(row)}
	<div class="@container/buttons flex flex-nowrap items-center gap-1">
		{#each resolvedRowActions as { rowAction, interaction } (rowAction.interaction_id)}
			{#if isRowActionVisible(rowAction, row)}
				<SurfaceInteractionButton
					{surfaceId}
					{interaction}
					{interactions}
					{targetProviderId}
					{encryptionContext}
					baseParams={rowParams(row)}
					rowSeed={row}
					size="sm"
					labelDisplay={interaction.icon ? 'icon-only' : 'always'}
					oncomplete={async () => {
						await loadPage(currentPage);
					}}
				/>
			{/if}
		{/each}
	</div>
{/snippet}
```

The `DataTable` consumer of the `rowActions` snippet wraps the snippet output inside its own `<td>`.
The new inner `<div>` adds the container query parent and the nowrap layout. Verify that
`DataTable.svelte`'s rowActions cell does not also need `whitespace-nowrap` — check the file; if the
cell inside DataTable has no whitespace handling, leave it alone since the inner `<div>` controls
layout.

- [ ] **Step 5: Run the tests**

Run: `cd frontend && npx vitest run src/lib/components/surfaces/SurfaceTable.test.ts` Expected:
PASS.

- [ ] **Step 6: Type-check, format, lint**

Run:

```bash
cd frontend && npm run check
npx prettier --write src/lib/components/surfaces/SurfaceTable.svelte \
                     src/lib/components/surfaces/SurfaceTable.test.ts
npm run lint
```

Expected: clean.

- [ ] **Step 7: Commit**

```bash
git add frontend/src/lib/components/surfaces/SurfaceTable.svelte frontend/src/lib/components/surfaces/SurfaceTable.test.ts
git commit -m "feat(frontend): SurfaceTable row actions go single-line and icon-only when icon set"
```

---

## Task 17: Verify Tailwind v4 emits the container-query CSS at build time

**Files:**

- Inspect: build output (verification only — both classes are already applied in Task 12 / 15 / 16).

- [ ] **Step 1: Build the frontend**

Run: `cd frontend && npm run build`
Expected: build succeeds.

- [ ] **Step 2: Confirm the generated CSS contains the container-query rule**

Run:

```bash
grep -r "container-name: buttons" frontend/.svelte-kit/output/ frontend/build/ 2>/dev/null | head -3
```

Expected: at least one hit (Tailwind generated `container-type: inline-size; container-name: buttons`
from `@container/buttons`).

Run:

```bash
grep -r "@container buttons" frontend/.svelte-kit/output/ frontend/build/ 2>/dev/null | head -3
```

Expected: at least one hit, including a rule whose body sets `position: absolute; ...` (the
generated `sr-only` block from `@max-[28em]/buttons:sr-only`).

- [ ] **Step 3: If either grep returns nothing, stop and diagnose**

The Tailwind utility classes are applied in `SurfaceActionButton.svelte` (`@max-[28em]/buttons:sr-only`),
`SurfaceActionBar.svelte` (`@container/buttons`), and `SurfaceTable.svelte` (`@container/buttons` in
both row-action branches). If the generated CSS does not contain the expected rules, the most likely
causes are:

1. The Tailwind config's `content` glob does not include `src/lib/components/surfaces/`. Inspect
   `frontend/tailwind.config.*` (or the `@tailwindcss/vite` setup in `vite.config.ts`).
2. The class name was typed differently in source (e.g. `@container/Buttons` capitalized).
3. The tailwind v4 version installed is < 4.0.0-alpha.20 (container queries shipped in alpha-20).
   Check `frontend/package.json` and `package-lock.json`.

Resolve the cause before continuing — Task 17 is a hard gate: Tasks 18+ depend on the visual
collapse working.

This task makes no commits when the gate passes.

---

## Task 18: Extend the dev button preview with `SurfaceActionButton` examples

**Files:**

- Modify: `frontend/src/routes/dev/button-preview/+page.svelte`

- [ ] **Step 1: Append a section showing each effective display state**

In `frontend/src/routes/dev/button-preview/+page.svelte`, add a new `<section>` after the existing
examples:

```svelte
<section class="mt-8">
	<h2 class="mb-3 text-sm font-bold uppercase tracking-wide">SurfaceActionButton — labelDisplay states</h2>
	<div class="@container/buttons flex flex-wrap items-center gap-3">
		<SurfaceActionButton label="Always" icon="refresh-cw" labelDisplay="always" variant="primary" onclick={noop} />
		<SurfaceActionButton label="Auto (collapses ≤ 28em)" icon="refresh-cw" labelDisplay="auto" variant="primary" onclick={noop} />
		<SurfaceActionButton label="Icon only" icon="trash-2" labelDisplay="icon-only" variant="danger" onclick={noop} />
		<SurfaceActionButton label="Icon-only without icon (degrades)" labelDisplay="icon-only" variant="ghost" onclick={noop} />
	</div>
</section>
```

Add the import to the script block:

```svelte
import SurfaceActionButton from '$lib/components/surfaces/SurfaceActionButton.svelte';
```

- [ ] **Step 2: Build and visually confirm**

Run: `cd frontend && npm run build` Expected: build succeeds.

(Manual visual verification: open the dev server `npm run dev` and navigate to
`/dev/button-preview`. Resize the browser window narrower than 28em and observe the "Auto" variant
collapsing to icon-only. Verify icons render. This is a visual check, not automated.)

- [ ] **Step 3: Format and lint**

Run:
`cd frontend && npx prettier --write src/routes/dev/button-preview/+page.svelte && npm run lint`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add frontend/src/routes/dev/button-preview/+page.svelte
git commit -m "docs(frontend): add SurfaceActionButton examples to dev button preview"
```

---

## Task 19: Add the "Action icons" section to plugin guidelines

**Files:**

- Modify: `docs/development/plugin-guidelines.md`

- [ ] **Step 1: Append the new section**

Add to the end of `docs/development/plugin-guidelines.md`:

````markdown
## Action icons

Plugin authors may declare a `lucide-svelte` icon alongside any `SurfaceActionDescriptor` label.
Icons render through the shared `SurfaceActionButton` component and adapt to the surface they appear
in (`'auto'` collapse in action bars; `'icon-only'` default in DataTable row actions).

### Identifier scheme

Icon names are Lucide-canonical kebab-case: `refresh-cw`, `trash-2`, `server-cog`. Wire validation
enforces the regex `^[a-z][a-z0-9-]*[a-z0-9]$` and a 64-character limit; PascalCase or snake_case
names are rejected at enrollment time.

### Source

The curated allowlist lives at `frontend/src/lib/icons.ts`. Only names present in that file render
correctly; any other name logs a `console.error` in the dashboard and falls back to text-only.

### Adding a new icon

Single-PR change to `frontend/src/lib/icons.ts`: import the lucide component and add its kebab-case
key to the `ICONS` map. No wire-crate release is required — the field is `Option<String>`.

### Authoring example

```rust
SurfaceActionDescriptor::new("sync-host", "Sync")
    .with_icon("refresh-cw")
    .with_permission(Permission::UpdateHosts.to_string())
```

### Where icons render

- **Action-bar buttons** use `labelDisplay='auto'` (the label collapses to icon-only on narrow
  containers, ≤ 28em).
- **DataTable row-action buttons** use `labelDisplay='icon-only'` (default-hide when an icon is
  set).
- **Workflow triggers** inherit `labelDisplay` from their parent context (action bar or row action).
````

- [ ] **Step 2: Run markdownlint**

Run: `markdownlint --config .markdownlint.json docs/development/plugin-guidelines.md` Expected:
empty output.

- [ ] **Step 3: Commit**

```bash
git add docs/development/plugin-guidelines.md
git commit -m "docs(plugin-guidelines): document the action-icon allowlist and authoring contract"
```

---

## Task 20: Add a banner to the older `2026-05-02-icon-rollout-design.md`

**Files:**

- Modify: `docs/superpowers/specs/2026-05-02-icon-rollout-design.md`

- [ ] **Step 1: Insert the banner immediately after the H1**

```markdown
# Icon Rollout — Design

> **Update (2026-05-04):** the kebab-case identifier scheme described in
> `docs/superpowers/specs/2026-05-04-surfaces-button-icons-and-label-display-design.md` supersedes
> the PascalCase keys recorded below. Icons now live in a unified `frontend/src/lib/icons.ts`
> registry; `nav-icons.ts` is removed.

## Goal

...
```

(Leave the rest of the document untouched.)

- [ ] **Step 2: Run markdownlint**

Run:
`markdownlint --config .markdownlint.json docs/superpowers/specs/2026-05-02-icon-rollout-design.md`
Expected: empty output.

- [ ] **Step 3: Commit**

```bash
git add docs/superpowers/specs/2026-05-02-icon-rollout-design.md
git commit -m "docs(spec): point icon-rollout spec to the kebab-case migration spec"
```

---

## Task 21: Sweep for any orphaned `nav-icons` references

**Files:**

- Inspect: workspace-wide search.

- [ ] **Step 1: Search the whole repo**

Run:

```bash
grep -rn 'nav-icons\|resolveNavIcon\|SURFACE_NAV_ICONS\|MAX_NAV_ICON_LEN' \
    --include='*.ts' --include='*.svelte' --include='*.rs' --include='*.md' \
    /Users/andreyyantsen/Development/uptrakit \
    | grep -v '.worktrees\|node_modules\|.svelte-kit\|/build/\|target/'
```

Expected: only matches inside spec/plan markdown documents (historical references) and inside this
plan file itself. No live code references.

- [ ] **Step 2: If a live code reference appears**

Update the offending file to use `resolveIcon` from `$lib/icons` (TS) or
`surfaces::MAX_ICON_NAME_LEN` (Rust). Re-run the per-crate gates.

- [ ] **Step 3: Commit if any change was needed**

```bash
git add <touched files>
git commit -m "refactor: drop the last nav-icons / MAX_NAV_ICON_LEN references"
```

(If no change was needed, no commit.)

---

## Task 22: Full quality gates and acceptance walkthrough

**Files:**

- None (verification only)

- [ ] **Step 1: Rust workspace gates**

Run sequentially:

```bash
cargo fmt --all
cargo check --no-default-features --features db-sqlite
cargo check --all-features
cargo clippy --all-targets --no-default-features --features db-sqlite -- -D warnings
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
cargo deny check
```

Expected: every command exits 0.

- [ ] **Step 2: Frontend gates**

```bash
cd frontend
npm run lint
npm run format:check
npm run check
npm run test
npm run build
```

Expected: every command exits 0.

- [ ] **Step 3: Markdown gates**

```bash
markdownlint --config .markdownlint.json '**/*.md'
```

Expected: empty output.

- [ ] **Step 4: Walk through the acceptance criteria**

For each acceptance criterion in the spec § Acceptance criteria, confirm it holds:

- [ ] Plugin authors can declare `with_icon("trash-2")` on `SurfaceActionDescriptor` and the icon
      renders inside the button — verified by `surface_action_descriptor_with_icon_sets_field`
      (Task 3) plus `SurfaceActionButton.test.ts` icon presence test (Task 12).
- [ ] `SurfaceDescriptor::nav_icon` and `InteractionDescriptor::icon` both fail wire validation on
      PascalCase, snake_case, oversized, or empty values — verified by Task 7 wire tests.
- [ ] Proxmox VE Hosts surface buttons render with the assigned icons; row actions render icon-only
      on a single line — verified by Task 5 + Task 16.
- [ ] SSH Hosts surface buttons render with the assigned icons — verified by Task 4.
- [ ] An unknown icon name in any interaction degrades to text-only and logs a single
      `console.error` — verified by `icons.test.ts` (Task 8) + `SurfaceActionButton.test.ts` "does
      not render an icon when the resolver fails".
- [ ] `cargo test --all-features` and `cd frontend && npm run test` pass — Steps 1 + 2 of this task.
- [ ] `docs/development/plugin-guidelines.md` documents the icon allowlist and the workflow for
      adding a new icon — verified by Task 19.

- [ ] **Step 5: No final commit**

This task is verification-only; no code changed. Move on to PR creation when ready.

---

## Self-Review (run before declaring the plan complete)

### Spec coverage

Walking each section of
`docs/superpowers/specs/2026-05-04-surfaces-button-icons-and-label-display-design.md`:

- § 1 Wire field — Tasks 2, 3.
- § 2 Validation — Tasks 1, 2, 7.
- § 3 Frontend `icons.ts` — Tasks 8, 9.
- § 4 `labelDisplay` in surface layer — Tasks 11, 12.
- § 5 Container query CSS — implicit in component class lists (Tasks 12, 15, 16); verified by
  Task 17.
- § 6 `SurfaceTable` row layout — Task 16.
- § 7 `SurfaceInteractionButton` — Task 13.
- § 8 `SurfaceActionBar` — Task 15.
- § 9 `SurfaceTable` row actions labelDisplay — Task 16.
- § 10 `SurfaceWorkflow` — Task 14.
- § 11 Icon assignments — Tasks 4, 5, 6.
- § 12 Documentation — Tasks 19, 20.

### Placeholder scan

No "TBD", "TODO", "implement later", "fill in details", "add appropriate error handling", or
"similar to Task N" patterns appear in the steps. Test fixtures are concrete; commands include exact
expected output.

### Type consistency

- `IconNameError`, `MAX_ICON_NAME_LEN`, `validate_icon_name` — defined in Task 1, consumed in Tasks
  2, 7.
- `InteractionDescriptor.icon: Option<String>` — added in Task 2, used in Tasks 5, 7.
- `SurfaceActionDescriptor.icon` + `with_icon()` — added in Task 3, used in Tasks 4, 6.
- `SurfaceActionButton` props (`label`, `icon`, `labelDisplay`, `variant`, `size`, `loading`,
  `disabled`, `onclick`, `dataUi`) — defined in Task 12, called with the same names in Tasks 13, 14.
- `LabelDisplay` const-object — defined in Task 11, imported by Tasks 12, 13, 14.
- `resolveIcon(name)` returning `{ component, ok }` — defined in Task 8, consumed by Task 12 and
  (via `SurfaceActionButton`) by Tasks 13, 14, 15, 16.

### Snapshot conformance

- `unwrap_used = deny` / `indexing_slicing = deny` — Task 1's slice-pattern destructuring satisfies
  both.
- `#[non_exhaustive]` — applied to `IconNameError` (Task 1), confirmed/preserved on
  `InteractionValidationError` (Task 2) and `SurfaceActionDescriptor` (Task 3).
- `Result<T, String>` ban — every Rust error path uses `IconNameError` / `WireValidationError` /
  `InteractionValidationError`.
- Conventional Commits — every commit message uses `feat(scope):`, `refactor(scope):`,
  `docs(scope):`, or `fix(scope):`.
- Prettier (tabs, single quotes) — every TS/Svelte step ends with `npx prettier --write` on the
  touched files.
- markdownlint line-length — long table rows in spec / plan are inside tables (exempt per
  `.markdownlint.json`).
- All Rust gates (fmt, clippy, test, deny) and frontend gates (lint, check, test, build) run in
  Task 22.

No gaps detected. Plan is internally consistent and snapshot-aligned.

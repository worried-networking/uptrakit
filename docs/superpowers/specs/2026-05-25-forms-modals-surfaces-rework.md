# Forms, Modals & Surfaces Rework

**Date:** 2026-05-25
**Status:** Approved for planning

---

## Overview

Nine targeted improvements to unify UI patterns across built-in and surface-rendered pages:
button placement, form alignment, dirty-state drafting, title casing, tab grouping, and new
surface contract fields. No new pages or routes. No breaking API changes.

---

## Requirement 1 — SSH Hosts: Bootstrap Buttons Above Table

**File:** `crates/core/agent-ssh-runtime/src/surface_runtime.rs`

In `build_surface_parts()`, the current child order inside the root `Section` node is
`[Table, ActionBar]`. The Proxmox VE Hosts surface already uses `[ActionBar, Table]`.
Reorder the `children` vec so `ActionBar` comes before `Table`.

```rust
// Before
let root = SurfaceNode::Section {
    title: None,
    children: vec![
        SurfaceNode::Table { … },
        SurfaceNode::ActionBar { action_ids: primary_ids },
    ],
};

// After
let root = SurfaceNode::Section {
    title: None,
    children: vec![
        SurfaceNode::ActionBar { action_ids: primary_ids },
        SurfaceNode::Table { … },
    ],
};
```

Any unit tests in `surface_runtime.rs` that assert child ordering in the root section must
be updated to assert `ActionBar` appears before the `Table` node. Audit all tests in the file
for child-order assertions and update each one.

---

## Requirement 2 — Form Action Buttons: Bottom-Right Alignment

All form action button rows (Save/Discard, Reset Data, Rotate CA) must use `justify-end`:

```svelte
<!-- Before -->
<div class="flex gap-2">…</div>

<!-- After -->
<div class="flex gap-2 justify-end">…</div>
```

**Affected files** (exhaustive audit required — these are known cases):

| File                                                           | Buttons                                |
| -------------------------------------------------------------- | -------------------------------------- |
| `frontend/src/routes/settings/AgentCertificateSettings.svelte` | Save, Discard                          |
| `frontend/src/routes/settings/McpAccessTab.svelte`             | Save, Discard (OAuth settings section) |
| `frontend/src/routes/settings/DangerZone.svelte`               | Reset Data                             |
| `frontend/src/routes/settings/GlobalSettingsTab.svelte`        | Rotate CA                              |
| `frontend/src/lib/components/surfaces/SchemaForm.svelte`       | Submit (surface forms)                 |

**Audit scope:** grep `"flex gap-2"` across `frontend/src/routes/` and
`frontend/src/lib/components/` and fix every form-action button row that lacks `justify-end`.

**Note:** `SchemaForm.svelte`'s submit button row may not use a `flex gap-2` wrapper — inspect
the file directly and apply the same `justify-end` treatment regardless of the current wrapper
structure.

**Rule:** confirmation-dialog triggers (Reset Data, Rotate CA) are **not** modal-triggers.
They stay in the card body but must be right-aligned. They do **not** move to the
`SectionCard` header.

---

## Requirement 3 — Modal-Trigger Buttons → SectionCard Header

Any button that opens a `ModalShell` (not a `ConfirmDialog`) belongs in the `SectionCard`
`{#snippet actions()}` slot, not the card body.

**Affected built-in components:**

| Component                        | Button                           | Current location                                                        |
| -------------------------------- | -------------------------------- | ----------------------------------------------------------------------- |
| `OidcProvidersSettings.svelte`   | "Add Provider"                   | `<div class="mb-4 flex items-center justify-between">` inside card body |
| `EnrollmentTokenSettings.svelte` | "Create Token"                   | Same inline div                                                         |
| `SystemServicesSettings.svelte`  | "Create System Enrollment Token" | Same inline div                                                         |

**Pattern to apply** (matches `McpAccessTab.svelte` "Register" button — already correct):

```svelte
<SectionCard title="OIDC Providers">
  {#snippet actions()}
    <Button variant="primary" onclick={openCreateOidc}>Add Provider</Button>
  {/snippet}
  <!-- table body, no button here -->
</SectionCard>
```

Remove the `<div class="mb-4 flex items-center justify-between">` wrapper from the card body
after moving the button.

**Audit scope:** grep for `showModal\|showCreateDialog\|showOidcModal` patterns inside
`SectionCard` body content across all settings components.

---

## Requirement 4 — SectionCard Title Casing: Manual Audit

Audit every `SectionCard title=` prop in `frontend/src/routes/` and `frontend/src/lib/`.
Apply title case manually. No transform utility. No new lint rule.

**Known fixes:**

| File                  | Current              | Fixed                |
| --------------------- | -------------------- | -------------------- |
| `McpAccessTab.svelte` | "Registered clients" | "Registered Clients" |
| `McpAccessTab.svelte` | "OAuth settings"     | "OAuth Settings"     |

**Audit method:** `grep -rn 'title="' frontend/src/routes/ frontend/src/lib/` — review every
match and fix non-title-case strings.

**Acronym rules:** SSH, OIDC, MCP, CA, OAuth, SMTP, API, URL remain uppercase.
Articles and prepositions (a, an, the, of, in, for, with) stay lowercase mid-title.

**Rust side:** audit `title` fields in surface `Section` node definitions across
`crates/core/agent-ssh-runtime/` and `crates/plugins/` for the same casing rule.

---

## Requirement 5 — Surface Section Header Action Buttons (New Feature)

### Contract changes

**TypeScript — `frontend/src/lib/surfaces/contract.ts`:**

```typescript
export type SurfaceNode = {
  kind: "section";
  title?: string;
  header_action_ids?: InteractionId[]; // NEW — modal/workflow triggers only
  children?: SurfaceNode[];
};
// …all other variants unchanged
```

No new `SurfaceCapability` flag is required. `header_action_ids` is a backwards-compatible
extension of the existing `section_node` capability. Validation is at the interaction-kind
level (Rust registration), not at the slot allowlist level.

**Rust — `crates/shared/surfaces/src/surface.rs` (the file defining `SurfaceNode`):**

```rust
pub enum SurfaceNode {
    Section {
        title: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        header_action_ids: Vec<InteractionId>,  // NEW — defaults to empty vec
        children: Vec<SurfaceNode>,
    },
    // …all other variants unchanged
}
```

`header_action_ids` serializes as `"header_action_ids"` (snake_case, matches frontend).
`#[serde(default)]` ensures deserialization from existing payloads that omit the field succeeds.

**Migration:** `SurfaceNode` is `#[non_exhaustive]`. Adding a field to the `Section` variant
is serde-compatible (with `#[serde(default)]`) but breaks all Rust construction sites that
use struct-literal syntax. Per project coding standards, `#[non_exhaustive]` types must use
constructor functions, not raw struct literals. The correct migration path is:

1. Add associated constructor functions to `SurfaceNode`:

   ```rust
   impl SurfaceNode {
       pub fn section(title: impl Into<Option<String>>, children: Vec<SurfaceNode>) -> Self {
           SurfaceNode::Section { title: title.into(), header_action_ids: vec![], children }
       }
       pub fn section_with_header_actions(
           title: impl Into<Option<String>>,
           header_action_ids: Vec<InteractionId>,
           children: Vec<SurfaceNode>,
       ) -> Self {
           SurfaceNode::Section { title: title.into(), header_action_ids, children }
       }
   }
   ```

2. Migrate every `SurfaceNode::Section { … }` struct literal across all crates to use
   `SurfaceNode::section(…)` or `SurfaceNode::section_with_header_actions(…)`.
   Search: `grep -rn 'SurfaceNode::Section {' crates/`.
3. **Include `crates/shared/surfaces/src/surface.rs` itself**: tests inside the defining
   crate use raw struct literals and also fail to compile when the new field is added.
   Update or replace these with the constructor functions too.
4. **Update rustdoc examples**: `SurfaceDescriptorBuilder` documentation likely contains
   a `.root_node(SurfaceNode::Section { title: None, children: vec![] })` example. This
   is compiled by `cargo test --doc` and will break. Update to use the constructor.
5. Do not add `header_action_ids: vec![]` to raw struct literals — that continues the
   anti-pattern and must be updated on every future extension.

### Rust validation (surface registration)

At registration time, for every `header_action_id` in a `Section` node:

1. Resolve the referenced interaction from the surface's `interactions` list.
2. Assert its `kind ∈ {InteractionKind::Workflow, InteractionKind::MutationAction}`.
3. **Additionally assert** that the interaction's `form_ui` is `None`. `MutationAction`
   interactions that carry `form_ui: Some(_)` render an inline form expansion, which breaks
   the card header layout. Reject regardless of whether `form_ui.fields` is empty — if
   `form_ui` is set, reject it. Only `Workflow` interactions and `MutationAction` interactions
   with `form_ui: None` (zero-field confirm-style mutations) are valid in a section header.
   Plugin authors should use `ConfirmableAction` for confirmable zero-field mutations instead.
4. If not found, wrong kind, or has `form_ui: Some(_)`: emit `SurfaceProviderRejectionCode::SchemaOrLimitFailure`.

### Frontend rendering — `SurfaceRenderer.svelte`

The settings page (`settings/+page.svelte`) already wraps each surface in a `SectionCard`.
Unconditionally replacing the `SurfaceRenderer` section branch with another `SectionCard`
would produce a card-inside-a-card double-wrap for every existing surface.

**Rule:** the `SurfaceRenderer` section branch **only** promotes to `SectionCard` when
`header_action_ids` is non-empty. When empty, keep the existing `<div class="space-y-4">`
(with optional `<h3 class="text-subsection-title">` for titled sections) — no structural
change to the zero-header-actions rendering path.

In Svelte 5, `{#snippet}` declared inside an `{#if}` block is scoped to that block and can
be referenced as a prop within the same block. Declare and reference within the same branch:

```svelte
{#if node.kind === 'section'}
  {@const headerInteractions = (node.header_action_ids ?? [])
    .map(id => findInteraction(id))
    .filter((i): i is InteractionDescriptor => i !== undefined)}

  {#if headerInteractions.length > 0}
    {#snippet sectionActions()}
      {#each headerInteractions as interaction (interaction.interaction_id)}
        <SurfaceInteractionButton
          {surfaceId}
          {interaction}
          {interactions}
          {targetProviderId}
          {encryptionContext}
          {baseParams}
        />
      {/each}
    {/snippet}

    <SectionCard title={node.title} actions={sectionActions}>
      <!-- existing children rendering -->
    </SectionCard>
  {:else}
    <!-- existing rendering: <div class="space-y-4"> with optional <h3> -->
  {/if}
{/if}
```

Unknown interaction kinds (resolved but not `workflow`/`mutation_action` without form_ui)
are silently skipped in the frontend — Rust validation is the enforcement gate.

### Design language update

`docs/development/ui/surfaces.md` — Surface Primitives table:

Add row:

```text
| `Section` (with `header_action_ids`) | Renders action buttons in `SectionCard` header row via `{#snippet actions()}`. Only `modal_trigger` and `workflow_trigger` interactions are valid here. Validation enforced at registration. |
```

Also add a note to the `Section` primitive row:

> `header_action_ids` accepts an array of interaction IDs. Each referenced interaction must be
> kind `modal_trigger` or `workflow_trigger`. The host renders them as buttons in the card
> header — right-aligned, same position as built-in modal-trigger buttons. Action bars and
> form submits must not appear here.

---

## Requirement 6 — Surface Form Drafting

### Scope

Only `SchemaForm.svelte` (structured forms with `form_ui` fields). Raw JSON-payload forms
(no `form_ui`) remain stateless.

### Behavior specification

| State                  | Save button                 | Discard button   |
| ---------------------- | --------------------------- | ---------------- |
| Loading initial values | Visible, disabled           | Hidden           |
| Loaded, not dirty      | Visible, disabled           | Hidden           |
| Dirty, valid           | Visible, **enabled**        | Visible, enabled |
| Dirty, invalid         | Visible, disabled           | Visible, enabled |
| Submitting             | Visible, disabled (loading) | Hidden           |

**Dirty detection:** compare each field's current value against the value returned by
`pre_load_interaction`. Use the same `valuesEqual` semantics as `createFormDraft` —
`null`, `undefined`, `''`, and `NaN` are all treated as "empty" and equal.

**Field dirty highlight:** pass `dirty={isFieldDirty(field.name)}` to each `FormFieldRow`.
The existing `dirty` prop on `FormFieldRow` already renders a visual indicator.

**Implementation approach:**

`SchemaForm.svelte` already uses `$effect` (not `onMount`) to load initial values when
`preLoadInteraction` is provided. Integrate draft state using `createFormDraft` from
`$lib/forms/draft.svelte` — do NOT re-implement dirty tracking inline:

```typescript
// Inside SchemaForm.svelte script
import { createFormDraft } from "$lib/forms/draft.svelte";

// Replace the existing flat $state field variables with a single draft instance.
// Initial defaults match the current field default values.
type FieldRecord = Record<string, unknown>;
const form = createFormDraft<FieldRecord>({});

// Replace existing $effect that sets field values:
$effect(() => {
  if (loadedValues) {
    // loadedValues = result of preLoadInteraction
    form.load(loadedValues);
  }
});
```

`createFormDraft` provides:

- `form.draft` — reactive draft object bound to each form field
- `form.isDirty` — `true` when any field differs from loaded server values
- `form.isFieldDirty(name)` — per-field dirty indicator passed to `FormFieldRow`
- `form.commit(values)` — reset dirty state after successful save
- `form.discard()` — revert draft to loaded values

**Multi-select fields:** `createFormDraft`'s `valuesEqual` uses `a === b` scalar equality.
Two distinct `string[]` arrays are never `===`, so storing multi-select values as arrays
in the draft would cause permanently-dirty state. Instead, represent multi-select values
as **sorted NUL-joined strings** in the draft, so `valuesEqual` works as-is:

```typescript
// Normalize a SvelteSet to a draft-comparable string:
function setToDraftString(set: SvelteSet<string>): string {
  return [...set].sort().join("\0");
}
// Reconstruct a SvelteSet from a draft string:
function draftStringToSet(s: string): SvelteSet<string> {
  return new SvelteSet(s ? s.split("\0") : []);
}
```

In `normalizeForDraft`, for multi-select fields: `[...v].sort().join('\0')` where `v` is
the server-returned `string[]`. After `form.load()`, reconstruct each `SvelteSet` from
`draftStringToSet(form.draft[fieldKey])`. On user toggle (add/remove from set), sync the
updated set back to `form.draft[fieldKey]` via `setToDraftString(multiSets[fieldKey])`.
`isFieldDirty(name)` on multi-select fields then correctly compares the current
joined-string value against the loaded joined-string — no changes to `valuesEqual`.

**Type normalization:** `SchemaForm.svelte` stores all field values as strings internally
(coercing via `String(rawValue)`). Server values returned by `pre_load_interaction` arrive
as typed JSON (`number`, `boolean`, `string[]`, etc.). Before calling `form.load()`, normalize
server values to match the string representation the form would produce:

```typescript
function normalizeForDraft(
  raw: Record<string, unknown>,
): Record<string, unknown> {
  return Object.fromEntries(
    Object.entries(raw).map(([k, v]) => [
      k,
      Array.isArray(v)
        ? [...(v as string[])].sort().join("\0") // multi-select: NUL-joined sorted string
        : v === null || v === undefined
          ? "" // empty sentinel
          : String(v), // number/bool → string
    ]),
  );
}
```

Call `form.load(normalizeForDraft(serverResponse))` after loading initial values, and
`form.commit(normalizeForDraft(result))` after a successful submit. This ensures
`valuesEqual(draft[k], serverValues[k])` compares like types and the form is not dirty
immediately after load for numeric or boolean fields.

**Multi-select fields:** normalize `SvelteSet` contents to a sorted `string[]` before
storing in the draft. After `form.load()`, reconstruct `multiSets` from `form.draft`.
`isFieldDirty` for multi-select fields compares the current `SvelteSet` as a sorted array
against the sorted array in `serverValues`.

**When no `preLoadInteraction` is provided:** `form.isDirty` is always false, Save
is always disabled. Correct behavior for create-only forms.

**On successful submit:** if the interaction result is a non-null object, call
`form.commit(normalizeForDraft(result))` so the form returns to a not-dirty state without
a full reload. If the result is not an object (null or primitive), call
`loadInitialValues()` again to re-sync and then `form.load(normalizeForDraft(reloaded))`.

### Design language update

`docs/development/ui/surfaces.md` — `Form` primitive row, add:

> When `pre_load_interaction_id` is set, the form enters draft mode: values are loaded from
> the server on mount, Save is always visible but disabled until the draft diverges from the
> loaded state, and dirty fields are highlighted via `FormFieldRow`'s `dirty` prop.
> Raw JSON forms (no `form_ui`) are stateless and do not support draft mode.

---

## Requirement 7 — Enrollment Tokens: Remove Load/Refresh Buttons

**Files:** `EnrollmentTokenSettings.svelte`, `SystemServicesSettings.svelte`

**Changes:**

1. Remove the `{#if tokens === null}` / `{:else}` branch that renders "Load Tokens" /
   "Refresh" buttons. The `onMount(() => void loadTokens())` call already handles initial
   load — no manual trigger needed.

2. Move the "Create Token" / "Create System Enrollment Token" button from the inline div
   inside the card body into `SectionCard`'s `{#snippet actions()}` (per Requirement 3).

3. Remove the now-empty button container div (`<div class="mb-4 flex items-center justify-between">`).

4. No retry button on load failure. Failed load handling is a deferred cross-cutting
   improvement (tracked in deferred items).

**Result:** the card body starts directly with the token created callout (if present) and
the data table.

**Enrollment token component unification** (`EnrollmentTokenList.svelte`) is **deferred**
to a follow-up spec.

---

## Requirement 8 — Notification Channels: Single Tab Grouping

**Files:** each notification plugin's `plugin.rs` — the `SurfaceDescriptor::builder()` call
that registers the surface:

| File                                                  | Surface ID               |
| ----------------------------------------------------- | ------------------------ |
| `crates/plugins/notifications/webhook/src/plugin.rs`  | `notifications.webhook`  |
| `crates/plugins/notifications/telegram/src/plugin.rs` | `notifications.telegram` |
| `crates/plugins/notifications/email/src/plugin.rs`    | `notifications.email`    |

Add to each builder (the `tab_group` builder method takes both id and label in one call):

```rust
.tab_group("notification-channels", "Notification Channels")
```

**Telegram exception:** the Telegram plugin registers TWO surfaces — `notifications.telegram`
(the channels list, gets `tab_group`) and `notifications.telegram.global_settings` (the
instance-scoped SMTP-equivalent surface that renders on `SLOT_SETTINGS_BELOW_GLOBAL`).
The global settings surface must **not** receive `tab_group` — it belongs in the global
settings area, not the notification channels tab. Only `notifications.telegram` gets the
`tab_group` call.

The frontend `settings/+page.svelte` already handles `tab_group` grouping — no frontend
changes needed.

**`BUILTIN_TAB_IDS` in `settings/+page.svelte`:** must **not** include
`"notification-channels"`. It is surface-provided, not built-in. Confirm the constant does
not include it (current code does not — no change required).

**Section titles inside each surface** ("Webhook Channels", "Telegram Channels",
"Email Channels") remain unchanged.

**Parity impact:** three separate `settings.tabs` surface entries collapse into one grouped
tab. Update any parity fixture that references these tab IDs by their individual surface IDs,
to instead reference the combined `"notification-channels"` tab group entry.

---

## Requirement 9 — Unified Submit Label: `submit_label` Field

### Contract changes

**TypeScript — `frontend/src/lib/surfaces/contract.ts`:**

```typescript
export interface InteractionDescriptor {
  // …existing fields…
  submit_label?: string; // NEW — overrides default "Save" for form_submit interactions
}
```

**Rust — `InteractionDescriptor` struct:**

```rust
pub struct InteractionDescriptor {
    // …existing fields…
    #[serde(skip_serializing_if = "Option::is_none")]
    pub submit_label: Option<String>,
}
```

### Rendering

**`SurfaceForm.svelte`** receives the full `interaction: InteractionDescriptor` prop.
It must resolve `submit_label` and pass it as the `submitLabel` prop to `SchemaForm.svelte`.
`SchemaForm.svelte` receives `submitLabel: string` only — it has no access to
`interaction` directly:

```typescript
// In SurfaceForm.svelte:
const effectiveSubmitLabel = $derived(
  interaction.submit_label?.trim() || "Save",
);
// Pass to SchemaForm:
// <SchemaForm submitLabel={effectiveSubmitLabel} … />
```

**`SchemaForm.svelte`:** change the `submitLabel` prop default from `'Submit'` to `'Save'`:

```typescript
let { submitLabel = 'Save', … }: { submitLabel?: string; … } = $props();
```

### Built-in form audit

Replace non-`"Save"` primary action button labels in modal footers where the action
saves a record (creates or updates):

| Component                        | Context                    | Current label | Fixed label |
| -------------------------------- | -------------------------- | ------------- | ----------- |
| `OidcProvidersSettings.svelte`   | Modal footer — create path | "Create"      | "Save"      |
| `OidcProvidersSettings.svelte`   | Modal footer — edit path   | "Update"      | "Save"      |
| `EnrollmentTokenSettings.svelte` | Modal footer — create      | "Create"      | "Save"      |
| `SystemServicesSettings.svelte`  | Modal footer — create      | "Create"      | "Save"      |

**Keep as-is** (destructive or non-save actions):

| Component                        | Label                    | Reason                  |
| -------------------------------- | ------------------------ | ----------------------- |
| `DangerZone.svelte`              | "Reset All Data"         | Destructive, not a save |
| `GlobalSettingsTab.svelte`       | "Rotate CA"              | Destructive action      |
| All `ConfirmDialog` confirmLabel | "Revoke", "Delete", etc. | Confirmable actions     |
| `ModalShell` cancel buttons      | "Cancel"                 | Not a save              |

---

## Documentation Deliverables

All changes below are part of this spec's implementation — not deferred.

### `docs/development/ui/surfaces.md`

| Change                                            | Detail                                                                                                                                                                    |
| ------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Update Surface Primitives table — `Section` row   | Note `header_action_ids` support; link to new Section Layout Rules section                                                                                                |
| Update Surface Primitives table — `ActionBar` row | State must appear before any `Table` sibling — buttons above data                                                                                                         |
| Update Surface Primitives table — `Form` row      | Note draft mode when `pre_load_interaction_id` is set; link to Form Draft Mode section                                                                                    |
| Add **Section Layout Rules** section              | Document ActionBar-before-Table ordering rule with correct/incorrect Rust examples                                                                                        |
| Add **Section Header Actions** section            | Full contract for `header_action_ids`: valid kinds (`Workflow`, `MutationAction` with `form_ui: None`), Rust constructor usage, TypeScript shape, registration validation |
| Add **Form Draft Mode** section                   | Save/Discard state table, dirty field highlighting, multi-select normalization, `submit_label` field with Rust and TypeScript definitions                                 |

### `docs/development/ui/primitives.md`

| Change                           | Detail                                                                                                                                                                                                  |
| -------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Update `SectionCard` section     | Add button placement rules: modal triggers → `{#snippet actions()}` header; form saves and confirm-dialog triggers → card body with `flex gap-2 justify-end`; title casing rule with acronym exceptions |
| Update `createFormDraft` example | Fix button row to `flex gap-2 justify-end`; Discard only when dirty; add form action button rules block (Save always visible/disabled when clean; label always "Save"; dirty field prop)                |

---

## Deferred / Out of Scope

| Item                                                                  | Tracking                                                                                                                                                                                                                                                                                                                                                         |
| --------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Enrollment token component unification (`EnrollmentTokenList.svelte`) | Follow-up spec                                                                                                                                                                                                                                                                                                                                                   |
| Failed load error handling / retry patterns                           | Cross-cutting follow-up for all forms                                                                                                                                                                                                                                                                                                                            |
| Title casing enforcement via ESLint rule or `SectionCard` transform   | Explicitly rejected — manual audit only                                                                                                                                                                                                                                                                                                                          |
| Design language docs restructure (`docs/development/ui/`)             | Follow-up spec — developer experience and maintainability focus; includes verifying and removing stale `Status:` markers, consolidating fragmented rules, improving discoverability. New patterns introduced by this spec (form action placement, SectionCard rules, surface layout rules, draft mode, `submit_label`) must be easily findable post-restructure. |

---

## Implementation Sequencing Constraint

**Requirement 5 (Rust side) must land first.** Adding `header_action_ids` to `SurfaceNode::Section`
and migrating all construction sites to `SurfaceNode::section(…)` is a workspace-wide compile
break. Any crate that references `SurfaceNode::Section { … }` fails to compile until the
migration is complete. Requirements 1 and 4 (Rust surface titles) also touch `SurfaceNode`
and must merge after R5. All frontend requirements (R2, R3, R6, R7, R9) and Rust plugin
requirements (R8) can proceed in parallel once R5's Rust side compiles cleanly.

Suggested commit order: **R5-Rust** → **R1, R4-Rust, R8** → **R2, R3, R6, R7, R9, R5-Frontend**.

---

## Quality Gates

All existing quality gates apply. Additional checks for this spec:

- `cargo check --all-features` — surface contract Rust changes compile
- `cargo test -p uptrakit-surface-proxy` — registration validation tests pass
- `frontend/npm run check` — TypeScript types compile for new contract fields
- `frontend/npm run test` — `SurfaceRenderer`, `SchemaForm`, `SurfaceActionBar` unit tests pass
- `frontend/npm run test:e2e -- ui-parity` — run after R8; if tab consolidation changes
  snapshots, run `npm run test:e2e -- ui-parity --update-snapshots`, review the diff
  (must show only the notification channels tab consolidation), then commit the updated snapshots
- **R9 label audit gate:** `grep -rn '"Create"\|"Update"\|"Submit"' frontend/src/routes/settings/`
  — review every match; any remaining modal footer "Create"/"Update"/"Submit" label on a
  save-record action is a gap and must be fixed
- Manual visual check: SSH Hosts page shows bootstrap buttons above table
- Manual visual check: OIDC Providers "Add Provider" in card header
- Manual visual check: Notification Channels combined tab in Settings

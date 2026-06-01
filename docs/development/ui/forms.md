<!-- markdownlint-disable MD013 -->

# Forms

Canonical home for form primitives, form action button placement, draft-state tracking, and surface form behaviour.

This page consolidates rules that previously lived in three places: form primitives in `primitives.md`, draft-mode behaviour in `surfaces.md`, and
form-related layout notes in `layout.md`. If you are adding a new form-shaped thing — a field row, a draft factory, a Save/Discard button row, a
surface form override — it goes here.

All primitives below are exported from `frontend/src/lib/components/ui/index.ts` (with form-only components also re-exported from
`frontend/src/lib/components/forms/`).

**Status:** `Implemented` (all sections unless noted)

---

## Form Primitives

### FormFieldRow

Labeled form field wrapper with hint and error display. Provides ARIA `aria-describedby` context to child `Input` and `Textarea` components
automatically.

```typescript
// frontend/src/lib/components/ui/FormFieldRow.svelte
{
  label: string;
  hint?: string;          // secondary helper text below the label
  error?: string;         // error message; triggers red text below the field
  inputId?: string;       // connects label[for] and generates error id
  required?: boolean;     // shows red asterisk next to label
  dirty?: boolean;        // shows left-side accent border (used by draft mode)
  children: Snippet;      // the input control(s)
}
```

Usage:

```svelte
<FormFieldRow label="Email address" inputId="user-email" required>
  <Input id="user-email" type="email" bind:value={email} />
</FormFieldRow>

<FormFieldRow
  label="Webhook URL"
  inputId="webhook-url"
  hint="Must be HTTPS."
  error={urlError}
>
  <Input id="webhook-url" type="url" bind:value={webhookUrl} error={urlError} />
</FormFieldRow>
```

Label-column width is set by the `FormLayout` context (see FormLayout Context below). No manual width override is needed when using `FormFieldRow`.

---

### FormFieldReadOnly

Static-value sibling of `FormFieldRow`. Use this for read-only URLs, fingerprints, IDs, or other non-interactive values displayed inside a form
section so they align with surrounding `FormFieldRow` inputs (both primitives read the same `FormLayout` context for label-column width).

```typescript
// frontend/src/lib/components/forms/FormFieldReadOnly.svelte
{
  label: string;
  hint?: string;          // secondary helper text below the label
  value?: string;         // text rendered in the value column
  mono?: boolean;         // default false; when true, applies font-mono to the value text
  children?: Snippet;     // overrides `value` rendering (badges, links, custom content)
}
```

Usage:

```svelte
<FormFieldReadOnly label="Current URL" mono value={currentUrl} />

<FormFieldReadOnly label="CA Fingerprint" mono value={fingerprint} hint="SHA-256." />

<FormFieldReadOnly label="Status">
  <StatusBadge tone="success" label="Active" />
</FormFieldReadOnly>
```

Layout: identical `@container/field` + container-query behaviour as `FormFieldRow`. Use this primitive instead of hand-rolling a
`<div class="grid grid-cols-[…rem_1fr]">` — hand-rolled grids will not track the modal/page context split and will misalign with sibling
`FormFieldRow` rows.

---

### Input

Single-line text input. Supports error state, disabled state, and bindable value.

```typescript
// frontend/src/lib/components/Input.svelte
export type InputType = 'text' | 'email' | 'password' | 'url' | 'number' | 'search';

{
  id: string;
  type: InputType;
  value: string | number;         // bindable
  el?: HTMLInputElement;          // bindable ref to the underlying <input>
  name?: string;
  placeholder?: string;
  autocomplete?: string;
  disabled?: boolean;
  required?: boolean;
  error?: string;                 // sets aria-invalid and error styling
  min?: number | string;
  max?: number | string;
  oninput?: (e: Event) => void;
  onblur?: (e: FocusEvent) => void;
  onkeydown?: (e: KeyboardEvent) => void;
  'aria-describedby'?: string;
  'aria-label'?: string;
  class?: string;
}
```

When placed inside `<FormFieldRow inputId="...">`, `aria-describedby` is wired automatically via Svelte context — no manual prop needed.

Error state: `border-[var(--color-danger-border)] bg-[var(--color-danger-bg)]` via `aria-invalid`. Height: `h-8` (`32px`). Radius: `3px`.

---

### Textarea

Multi-line text input. Identical token contract to `Input`.

```typescript
// frontend/src/lib/components/Textarea.svelte
export type TextareaVariant = 'default' | 'mono';

{
  id: string;
  value: string;            // bindable
  name?: string;
  placeholder?: string;
  rows?: number;
  disabled?: boolean;
  required?: boolean;
  error?: string;
  variant?: TextareaVariant;   // 'mono' applies font-mono text-[13px]
  oninput?: (e: Event) => void;
  onblur?: (e: FocusEvent) => void;
  'aria-describedby'?: string;
  class?: string;
}
```

Minimum height `4rem` (`min-h-[4rem]`), resizable vertically.

---

### Checkbox

Single checkbox input with bindable `checked` and `indeterminate` states.

```typescript
// frontend/src/lib/components/Checkbox.svelte
{
  id: string;
  checked?: boolean;          // bindable, default false
  indeterminate?: boolean;    // bindable, default false
  name?: string;
  disabled?: boolean;
  onchange?: (e: Event) => void;
  class?: string;
  'aria-label'?: string;
}
```

Color note: `@tailwindcss/forms` sets `appearance: none` on checkboxes making `accent-color` inert. The checked fill uses `currentColor` from the
`text-[var(--accent)]` class. Do not use `accent-[var(--accent)]`.

`Checkbox` is also the canonical boolean-toggle primitive — there is no track-and-thumb switch component.

---

### RadioCardGroup

Horizontal card-tile selector for mutually exclusive string options. No radio indicators — selection is conveyed by accent border and background tint
only.

**Location:** `frontend/src/lib/components/forms/RadioCardGroup.svelte` **Import:** `import { RadioCardGroup } from '$lib/components/forms';`

```typescript
// frontend/src/lib/components/forms/RadioCardGroup.svelte
export type RadioCardOption = {
  value: string;
  label: string;
  tooltip?: string;  // shown in a Tooltip bubble; omit to render the card without an info icon
};

{
  name: string;                          // ARIA label for the group
  value: string;                         // currently selected value
  options: RadioCardOption[];            // { value, label, tooltip? }[]
  onchange: (value: string) => void;    // called when selection changes
  disabled?: boolean;                    // disables all cards
}
```

**Accessibility:** `role="radiogroup"` on container; each card has `role="radio"` and `aria-checked`. Each card element is a `<div role="radio">`
rather than `<button>` so the tooltip `<button>` can nest inside it without producing invalid HTML; no ARIA behaviour changed.

**Example:**

```svelte
<RadioCardGroup
  name="registration-mode"
  value={form.draft.mode}
  options={[
    { value: 'open', label: 'Open', tooltip: 'Anyone can register.' },
    { value: 'invite', label: 'Invite Only', tooltip: 'Token required.' },
    { value: 'closed', label: 'Closed', tooltip: 'No new accounts.' }
  ]}
  onchange={(v) => form.update('mode', v)}
/>
```

---

## FormLayout Context

`FormFieldRow` and `FormFieldReadOnly` derive their label-column width from a `FormLayout` context value supplied by their ancestor. The grid switches
to label-beside-input layout via an anonymous container query on a named CSS container (`@container/field`) so it behaves correctly inside
multi-column grids — labels stack when the cell is too narrow regardless of viewport width.

| Context value      | Container threshold | Label column       |
| ------------------ | ------------------- | ------------------ |
| `FormLayout.Modal` | `24rem`             | `minmax(0, 11rem)` |
| `FormLayout.Page`  | `32rem`             | `minmax(0, 20rem)` |

- `Modal.svelte` (`ModalShell`) sets `FormLayout.Modal` automatically. Nothing to do in the modal body.
- Pages default to `FormLayout.Page`. No manual override needed.
- Source: `frontend/src/lib/components/forms/form-layout-context.ts`.

Do not hand-roll a `<div class="grid grid-cols-[…rem_1fr]">` for a single field — it will not respect the context and will misalign with sibling
`FormFieldRow` rows.

---

## Form Action Buttons

Two distinct button categories appear around forms. They go in different locations.

| Button category                                                | Location                                                         | Examples                             |
| -------------------------------------------------------------- | ---------------------------------------------------------------- | ------------------------------------ |
| Modal/workflow trigger (opens a new dialog or starts a wizard) | `SectionCard` `{#snippet actions()}` slot in the card header     | "Add Provider", "Create Token"       |
| Form action (saves or discards the card's own form)            | Card body, `<div class="flex gap-2 justify-end">` at the bottom  | Save, Discard, Reset Data, Rotate CA |
| Confirmation-dialog trigger (opens a `ConfirmDialog`)          | Card body, right-aligned (same `<div class="flex justify-end">`) | Delete, Revoke, Reset                |

Confirmation-dialog triggers are **not** modal-triggers; they stay in the card body. Only buttons that open a `ModalShell` (a dialog with arbitrary
form content) or start a `WorkflowTrigger` belong in the header actions slot.

### Save/Discard pattern

For any form that saves to the backend, use this exact button row at the bottom of the card body:

```svelte
<div class="flex gap-2 justify-end">
  {#if form.isDirty}
    <Button variant="ghost" onclick={() => form.discard()}>Discard</Button>
  {/if}
  <Button variant="primary" disabled={!form.isDirty || !isValid} onclick={save}>Save</Button>
</div>
```

Rules:

- The submit label is always **"Save"** for built-in forms. Surface forms may override via `submit_label` (see below).
- `Save` is always visible; disabled when not dirty or invalid.
- `Discard` is rendered only when dirty.
- Pass `dirty={form.isFieldDirty(key)}` to each `FormFieldRow` to show the left-accent dirty indicator.

### Modal/workflow trigger pattern

```svelte
<SectionCard title="OIDC Providers">
  {#snippet actions()}
    <Button variant="primary" onclick={openCreate}>Add Provider</Button>
  {/snippet}
  <!-- table or body content -->
</SectionCard>
```

Titles in `SectionCard` must follow **Title Case** (e.g. "OAuth Settings", not "OAuth settings").

---

## createFormDraft

Svelte 5 reactive factory for the settings draft pattern: tracks server-committed state versus in-progress edits, computes dirty state, and provides
load/commit/discard lifecycle methods.

**Location:** `frontend/src/lib/forms/draft.svelte.ts` **Import:** `import { createFormDraft } from '$lib/forms/draft.svelte';`

```typescript
interface FormDraft<T> {
  readonly draft: T; // current in-progress edits
  readonly serverValues: T; // last committed server state
  readonly isDirty: boolean; // any field differs from serverValues
  isFieldDirty(key: keyof T): boolean;
  update<K extends keyof T>(key: K, value: T[K]): void;
  load(values: T): void; // on data fetch — sets both draft and serverValues
  commit(updated: T): void; // on successful save — sets both to the server response
  discard(): void; // reset draft to serverValues
}
```

**When to use:** any editable settings form that needs a Save/Discard pair with disabled-when-clean Save button and per-field dirty indicators.

**Critical:** do **not** destructure the return value — `const { draft } = form` takes a snapshot. Always access through `form.draft`, `form.isDirty`,
etc.

**Example:**

```svelte
<script lang="ts">
  import { createFormDraft } from '$lib/forms/draft.svelte';

  let form = createFormDraft({ mode: 'open', maxUsers: 100 });
</script>

<FormFieldRow label="Mode" dirty={form.isFieldDirty('mode')}>
  <RadioCardGroup
    name="mode"
    value={form.draft.mode}
    options={[...]}
    onchange={(v) => form.update('mode', v)}
  />
</FormFieldRow>

<FormFieldRow label="Max Users" dirty={form.isFieldDirty('maxUsers')}>
  <Input
    id="max-users"
    type="number"
    value={form.draft.maxUsers}
    oninput={(e) => form.update('maxUsers', +e.currentTarget.value)}
  />
</FormFieldRow>

<div class="flex gap-2 justify-end">
  {#if form.isDirty}
    <Button variant="ghost" onclick={() => form.discard()}>Discard</Button>
  {/if}
  <Button variant="primary" disabled={!form.isDirty} onclick={() => save()}>
    Save
  </Button>
</div>
```

`createFormDraft` has a sibling factory for URL-state binding: `createUrlParam`, documented under `Filter Primitives` in `primitives.md`. Both are
Svelte 5 reactive factories that wrap a `$state`/`$derived` source with a stable read/write API.

---

## Surface Form Draft Mode

Surface forms backed by `pre_load_interaction` automatically enter draft mode and get the same dirty-tracking, Save/Discard behaviour as built-in
forms backed by `createFormDraft` — the surface runtime wires it for you. The form fetches the current server values on mount, then tracks dirty state
field-by-field.

### Behavior

| Condition                                            | Save button        | Discard button |
| ---------------------------------------------------- | ------------------ | -------------- |
| `pre_load_interaction` absent (create mode), valid   | Enabled            | Hidden         |
| `pre_load_interaction` absent (create mode), invalid | Disabled           | Hidden         |
| Edit mode, values match server baseline              | Disabled           | Hidden         |
| Edit mode, at least one field changed                | Enabled            | Visible        |
| Submitting or loading initial values                 | Disabled (spinner) | Hidden         |

- **Dirty fields** receive a left-side accent border (the `dirty` prop on `FormFieldRow`).
- **Discard** restores all fields to the last server-fetched values without a network round-trip.
- **Save** commits the current values as the new baseline on success — no reload needed to re-enable Save for subsequent edits.

### Caveats

- The JSON-payload fallback (no `form_ui` / `fields`) is **not** in draft mode. It remains stateless.
- Multi-select dirty detection uses sorted NUL-joined string comparison. Field order from the server does not affect dirty state.

---

## submit_label

Optional override for the submit-button label on a surface `Form` interaction. Default is `"Save"` (same as built-in forms — see Form Action Buttons
above).

| Layer                 | Shape                                                     |
| --------------------- | --------------------------------------------------------- |
| Rust (wire)           | `submit_label: Option<String>` on `InteractionDescriptor` |
| TypeScript (frontend) | `submit_label?: string` on `InteractionDescriptor`        |
| Default               | `"Save"` (frontend fallback)                              |

### Validation

Enforced at both registration layers:

| Check                               | Outcome  |
| ----------------------------------- | -------- |
| Empty string or whitespace-only     | Rejected |
| Length > 50 characters              | Rejected |
| Valid non-empty up to 50 characters | Accepted |

Source: `crates/shared/surfaces/src/interaction.rs` (validation), `frontend/src/lib/components/surfaces/SurfaceForm.svelte`
(`interaction.submit_label?.trim() || submitLabel || 'Save'`).

### When to override

Use `submit_label` only when "Save" is genuinely wrong for the action — for example, a workflow step whose semantics are "Enroll" or "Run check". Do
not override to micro-vary the wording. Keep it consistent with the rest of the product.

---

## Notes

- For the surface-specific runtime container (slot registry, parity contract, surface primitives table) see `surfaces.md`.
- For form-action button placement inside `SectionCard`, the canonical rule lives in the Form Action Buttons section above; `primitives.md`
  `SectionCard` cross-references it for the body half.

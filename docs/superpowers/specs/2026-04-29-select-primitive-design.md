# Select Primitive & Forms Folder Design

**Date:** 2026-04-29
**Status:** Approved

## Overview

Add a `Select` form primitive consistent with `Input`, `Textarea`, and `Checkbox`.
Reorganise form-specific components into a dedicated `forms/` folder with a barrel export.
Migrate all inline `<select>` elements at known call sites to use the new component.

## Component API

`src/lib/components/forms/Select.svelte`

```ts
type SelectOption = { value: string; label: string };

type SelectProps = {
  id: string;
  value: string;           // bindable
  options: SelectOption[];
  name?: string;
  placeholder?: string;    // renders as disabled first <option value="">
  disabled?: boolean;
  required?: boolean;
  error?: string;
  onchange?: (e: Event) => void;
  onblur?: (e: FocusEvent) => void;
  'aria-describedby'?: string;
  'aria-label'?: string;
  class?: string;
};
```

**Styling:** Identical `BASE` class string to `Input` — `h-8 w-full px-[10px] rounded-card`, CSS variable
theming, focus ring `shadow-[0_0_0_3px_rgba(var(--accent-rgb),0.25)]`, error state via
`aria-[invalid=true]` selectors, `disabled:opacity-40 disabled:cursor-not-allowed`,
`transition-[background,border-color] duration-fast`.

**Placeholder option:** When `placeholder` prop is provided, renders
`<option value="" disabled>{placeholder}</option>` as first child — not re-selectable after user picks a value.

**Error state:** `error` prop sets `aria-invalid="true"`. Reads `form-field-row:aria-describedby` context
(same as `Input`) to auto-wire `aria-describedby` when used inside `FormFieldRow`.

**Value type:** `string` only. Native `<select>` always yields strings; numeric coercion is caller responsibility.

**Events:** `onchange` (primary), `onblur` (for validation-on-blur parity with `Input`). No `oninput` or `onkeydown` — not applicable to selects.

## File Reorganisation

New folder: `src/lib/components/forms/`

**Moves from `src/lib/components/`:**

- `Input.svelte` + `Input.test.ts`
- `Textarea.svelte` + `Textarea.test.ts`
- `Checkbox.svelte` + `Checkbox.test.ts`
- `CheckboxList.svelte`

**Moves from `src/lib/components/ui/`:**

- `FormFieldRow.svelte` + `FormFieldRow.test.ts`
- `FormFieldRow` export removed from `ui/index.ts`

**New barrel:** `src/lib/components/forms/index.ts` re-exports all moved components plus `Select` (and their exported types).

**Import updates:** All ~50 import sites across `src/lib/components/`, `src/routes/`, and test files updated to reference the new paths.

## Migrations

### SchemaForm.svelte

Replace inline `<select class="select">` + manual `{#each}` options block with `<Select>`:

```svelte
<Select
  {id}
  bind:value={values[field.key]}
  options={resolvedOptions(field)}
  placeholder="Select..."
  {required}
  error={fieldErrors[field.key]}
  onchange={() => clearFieldError(field.key)}
/>
```

SchemaForm's loading state (spinner while fetching `select_source`) remains untouched.

### audit-logs/+page.svelte

Replace both inline `<select class="select">` elements in the filter panel:

- `filter-outcome` — static options for outcome filter
- `filter-actor-type` — static options for actor type filter

Both switch to `<Select>` with equivalent `bind:value` and static `options` arrays.

### dev/form-primitive-preview/+page.svelte

Add `Select` demo section showing:

- Default state with options
- With placeholder
- Error state
- Disabled state

## Out of Scope

- Custom dropdown (no search, no rich option rendering, no portal)
- Multi-select (handled separately by `CheckboxList`)
- Async option loading changes in `SchemaForm` (behaviour unchanged)
- `Button` component (stays in `src/lib/components/` root)

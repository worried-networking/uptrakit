# Select Primitive & Forms Folder Design

**Date:** 2026-04-29
**Status:** Approved

## Overview

Add a `Select` form primitive consistent with `Input`, `Textarea`, and `Checkbox`.
Reorganise form-specific components into a dedicated `forms/` folder with a barrel export.
Migrate all inline `<select class="select">` elements across the codebase to use the new component.

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

**Styling:** Same `BASE` class string as `Input` with one omission: drop
`placeholder:text-[var(--text-muted)]` — that pseudo-class is meaningless on `<select>`.
All other tokens carry over: `h-8 w-full px-[10px] rounded-card`, CSS variable theming,
focus ring `shadow-[0_0_0_3px_rgba(var(--accent-rgb),0.25)]`, error state via
`aria-[invalid=true]` selectors, `disabled:opacity-40 disabled:cursor-not-allowed`,
`transition-[background,border-color] duration-fast`.

`w-full` is part of BASE. The `class` prop is **additive only** — it appends tokens not
already in BASE. The codebase does not use `tailwind-merge`, so passing a conflicting
width class (e.g., `w-auto`) is not a reliable override. Migration targets that require
non-full-width sizing (e.g., `software/+page.svelte` filter select with `w-auto`) must be
excluded from migration and left as native `<select>` elements.

Selects using `flex-1` in a flex container (e.g., `AssignToHostModal` hook selects at
lines ~383 and ~508) may be migrated with `class="flex-1"`. In a flex context, `flex-1`
governs sizing and `w-full` is inert — the layout is correct without special handling.

**Placeholder option:** When `placeholder` prop is provided, renders
`<option value="" disabled>{placeholder}</option>` as first child — not re-selectable
after user picks a value. Callers that bind `value=""` initially will show the placeholder;
callers that do not need an empty state should omit the `placeholder` prop and pre-set
`value` to a valid option.

**Initial value + required contract:** If `required` and no `placeholder`, callers must
initialise `value` to a non-empty option. If `required` with `placeholder`, the component
renders `value=""` initially — the browser blocks form submission until user selects a
real option.

**Error state:** `error` prop sets `aria-invalid="true"`. Reads `form-field-row:aria-describedby`
context (same as `Input`) to auto-wire `aria-describedby` when used inside `FormFieldRow`.

**Value type:** `string` only. Native `<select>` always yields strings; numeric coercion
is caller responsibility.

**Events:** `onchange` (primary), `onblur` (for validation-on-blur parity with `Input`).
No `oninput` or `onkeydown` — not applicable to selects.

## File Reorganisation

New folder: `src/lib/components/forms/`

**Moves from `src/lib/components/`:**

- `Input.svelte` + `Input.test.ts`
- `Textarea.svelte` + `Textarea.test.ts`
- `Checkbox.svelte` + `Checkbox.test.ts`
- `CheckboxList.svelte` (no test file)

**Moves from `src/lib/components/ui/`:**

- `FormFieldRow.svelte` + `FormFieldRow.test.ts`
- `FormFieldRow` export removed from `ui/index.ts`; all callers update import path directly —
  no re-export shim in `ui/index.ts`

**New barrel:** `src/lib/components/forms/index.ts` with explicit exports:

```ts
export { default as Input } from './Input.svelte';
export type { InputProps, InputType } from './Input.svelte';

export { default as Textarea } from './Textarea.svelte';
export type { TextareaProps, TextareaVariant } from './Textarea.svelte';

export { default as Checkbox } from './Checkbox.svelte';
export type { CheckboxProps } from './Checkbox.svelte';

export { default as CheckboxList } from './CheckboxList.svelte';
export type { CheckboxListItem } from './CheckboxList.svelte';

export { default as FormFieldRow } from './FormFieldRow.svelte';

export { default as Select } from './Select.svelte';
export type { SelectProps, SelectOption } from './Select.svelte';
```

**Import updates:** All import sites across `src/lib/components/`, `src/routes/`, and test
files updated. The grep commands below are the canonical discovery mechanism — named
examples in this spec are illustrative, not exhaustive.

```sh
# form component imports
grep -rn 'from.*components/Input\|from.*components/Textarea\|from.*components/Checkbox\|from.*components/CheckboxList' frontend/src
# FormFieldRow imports (~11 callers split their import into two statements after removal from ui/index.ts)
grep -rn 'FormFieldRow' frontend/src
```

Three non-route files will break at build time if missed:
`src/lib/components/forms/CheckboxList.svelte` (imports `Checkbox` from old path — update
after the move), `src/lib/components/ui/SoftwareGroupList.svelte` (imports `Checkbox`
directly), and `src/routes/public-entry.test.ts` (imports `Checkbox` directly).

## Migrations

Find all `<select>` targets with:

```sh
grep -rn '<select' frontend/src
```

**Exclusion criteria — do NOT migrate a select if it:**

- Uses `<optgroup>` — `EditHostAssignmentModal` has two selects with "Saved"/"Inline" optgroups
  (lines ~716 and ~1081). Exclude only those specific selects; other selects in that file
  (including the `resolvedOptions(field)`-pattern selects — same as SchemaForm) are eligible.
- Binds to a non-string value — `services/+page.svelte` uses `value={null}` on the placeholder
  option and binds to `string | null`; stays native.
- Requires non-full-width sizing — `software/+page.svelte` filter select uses `w-auto`; stays
  native (no `tailwind-merge` in project, `class` prop is additive only).
- Has per-row *conditionally varying* option sets — where different loop iterations need
  different option lists based on row-specific logic (e.g., `execution_site` select in
  `AssignToHostModal` conditionally adds a "Controller" option only when `role === 'fetch_releases'`).
  Distinguish from uniformly computed options: `SchemaForm`'s `resolvedOptions(field)` returns
  different values per field but is called identically for every row — that migrates. The
  `execution_site` selects whose option arrays are shaped by an `if` on the loop key do not.
- Is inside `ProviderSelector.svelte` (controlled/uncontrolled + per-option disabled — stays native).

All other inline `<select>` elements migrate to `<Select>`. The grep command is the
exhaustive source of truth — apply the exclusion criteria to each result mechanically.

**`id` for loop selects:** Selects inside `{#each}` loops lack an `id` on the native element.
Generate a unique `id` using the loop key, e.g., `id="assign-role-{role}-plugin-config"`.

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

### dev/form-primitive-preview/+page.svelte

Existing route. Add `Select` demo section showing:

- Default state with options
- With placeholder
- Error state
- Disabled state

## Out of Scope

- Custom dropdown (no search, no rich option rendering, no portal)
- Multi-select (handled separately by `CheckboxList`)
- Async option loading changes in `SchemaForm` (behaviour unchanged)
- `Button` component (stays in `src/lib/components/` root)
- `ProviderSelector.svelte` (controlled/uncontrolled + per-option disabled — stays native)
- Selects using `optgroup`, non-string bound values, non-full-width sizing, or per-row conditionally varying option sets

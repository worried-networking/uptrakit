# Form Primitives — `data-ui` Markers

**Date:** 2026-05-03
**Status:** Approved

## Overview

Add `data-ui` attributes to `Input`, `Textarea`, and `Checkbox` form primitives, matching the
pattern already established by `Select` (`data-ui="select"` at `Select.svelte:73`). This is
the out-of-scope follow-up explicitly listed in
`docs/superpowers/specs/2026-04-29-select-migration-leftovers-design.md`.

## Goals

- All four form primitives carry a `data-ui` marker for consistent selector targeting in tests
  and future tooling.
- Zero API surface change — no new props, no type exports.

## Non-Goals

- `CheckboxList.svelte` and `FormFieldRow.svelte` — composites, not form controls.
- Exposing `data-ui` as an overridable prop in any `*Props` type — no current caller needs this.
- Rolling out `data-ui` to non-primitive UI components.

## Naming Convention

Attribute value = component name lowercased, matching the established pattern:

| Component | Root element | `data-ui` value |
| --- | --- | --- |
| `Select.svelte` | `<select>` | `"select"` ✓ already present |
| `Input.svelte` | `<input>` | `"input"` |
| `Textarea.svelte` | `<textarea>` | `"textarea"` |
| `Checkbox.svelte` | `<input type="checkbox">` | `"checkbox"` |

`Checkbox` uses `"checkbox"` (not `"input"`) to distinguish from text inputs. The naming
is component-semantic, not element-type-literal — consistent with `Select`, which also
names itself after the component rather than the HTML element.

## Changes

### `frontend/src/lib/components/forms/Input.svelte`

Add `data-ui="input"` on the `<input>` element, between `aria-label` and `class`:

```svelte
aria-label={ariaLabel}
data-ui="input"
class={computedClass}
```

### `frontend/src/lib/components/forms/Textarea.svelte`

Add `data-ui="textarea"` on the `<textarea>` element, between `aria-describedby` and `class`:

```svelte
aria-describedby={ariaDescribedby}
data-ui="textarea"
class={computedClass}
```

### `frontend/src/lib/components/forms/Checkbox.svelte`

Add `data-ui="checkbox"` on the `<input type="checkbox">` element, between `onchange` and `class`:

```svelte
{onchange}
data-ui="checkbox"
class={computedClass}
```

## Tests

One new case per test file, placed after the existing attribute-forwarding tests.

### `Input.test.ts`

```ts
it('has data-ui="input" attribute', () => {
    const { container } = render(Input, baseInput());
    expect(container.querySelector('input')!.getAttribute('data-ui')).toBe('input');
});
```

### `Textarea.test.ts`

```ts
it('has data-ui="textarea" attribute', () => {
    const { container } = render(Textarea, base());
    expect(container.querySelector('textarea')!.getAttribute('data-ui')).toBe('textarea');
});
```

### `Checkbox.test.ts`

```ts
it('has data-ui="checkbox" attribute', () => {
    const { container } = render(Checkbox, baseCheckbox());
    expect(container.querySelector('input')!.getAttribute('data-ui')).toBe('checkbox');
});
```

## Acceptance Criteria

- [ ] `Input.svelte` root `<input>` has `data-ui="input"`.
- [ ] `Textarea.svelte` root `<textarea>` has `data-ui="textarea"`.
- [ ] `Checkbox.svelte` root `<input>` has `data-ui="checkbox"`.
- [ ] No changes to `InputProps`, `TextareaProps`, or `CheckboxProps` types.
- [ ] Three new test cases pass.
- [ ] `npm run check` clean.
- [ ] `npm run lint` clean.
- [ ] `npm run test` green.

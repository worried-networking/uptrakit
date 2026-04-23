# Form Primitive Adoption — Design

**Parent spec:** `docs/superpowers/specs/2026-04-16-ui-design-language-design.md`
**Audit source:** `docs/superpowers/specs/2026-04-23-design-alignment-gaps.md` §Category C

Replaces all raw `<input>`, `<input type="checkbox">`, and `<textarea>` elements that
should use the `<Input>`, `<Checkbox>`, and `<Textarea>` primitives from
`$lib/components/`.

---

## Overview

`<Input>`, `<Checkbox>`, and `<Textarea>` primitives were shipped in waves 2b and 2d of
the design-language rollout. Consumer migration was done in waves 3–6 for newly-created
route files and settings panels. The audit found 24 remaining call sites in shared
components and route files where raw HTML form elements are still used.

No behaviour changes. Drop-in primitive substitution.

---

## Design decisions

**Q1 — Scope of migration (which raw elements to migrate).**

- (chosen) Migrate: `<input type="text|email|password|search">`, `<input type="checkbox">`,
  `<textarea>`. These have spec primitives (`<Input>`, `<Checkbox>`, `<Textarea>`).
- Out of scope: `<input type="radio">` (no Radio primitive), `<select>` (no Select
  primitive), `<label>` (valid HTML wrapping, not a primitive violation).
- Reasoning: only migrate what has a direct primitive replacement; do not invent
  primitives not in the spec.

**Q2 — `<Input>` binding pattern.**

- (chosen) Preserve existing `bind:value` bindings. `<Input>` accepts `bind:value`.
  Pass any existing `class` overrides via the `class` prop if needed, but prefer
  removing override classes that duplicated Skeleton utilities.
- Reasoning: minimal diff; no logic change.

**Q3 — `<Checkbox>` binding pattern.**

- (chosen) Replace `<input type="checkbox" class="checkbox" bind:checked={x}>` with
  `<Checkbox bind:checked={x}>`. Preserve any `id`/`name` attributes as props.
- Reasoning: `<Checkbox>` wraps the input and applies design-token focus ring and size.

**Q4 — `<Textarea>` for monospace/code fields.**

- (chosen) Replace raw `<textarea class="textarea font-mono text-xs">` with
  `<Textarea variant="mono">`. `<Textarea variant="mono">` applies `font-mono text-[13px]`
  internally (design token for monospace fields is 13px, not 12px).
- Rejected: `<Textarea class="font-mono text-xs">` — `text-xs` is 12px; the monospace
  design token size is 13px; consumers must not override this with smaller classes.
- Reasoning: `variant="mono"` is the canonical path for monospace code fields; it applies
  the correct size from the design token rather than letting consumers guess at it.

---

## Goals

1. Replace every raw `<input type="text|email|password|search">` with `<Input>`.
2. Replace every raw `<input type="checkbox">` with `<Checkbox>`.
3. Replace every raw `<textarea>` with `<Textarea>` (use `variant="mono"` for monospace/code fields).
4. Remove now-redundant Skeleton utility classes (`class="input"`, `class="checkbox"`,
   `class="textarea"`) from migrated elements.

## Non-goals

- Replacing `<select>` (no primitive).
- Replacing `<input type="radio">` (no primitive).
- Changing form field layout, validation logic, or `bind:` wiring.

---

## Scope

### `lib/components/` files

| File | Line(s) | Change |
| --- | --- | --- |
| `CheckboxList.svelte` | 40 | `<input type="checkbox" class="checkbox">` → `<Checkbox>` |
| `AddSoftwareModal.svelte` | 67, 86 | `<input class="input w-full">` → `<Input class="w-full">` |
| `AddSoftwareModal.svelte` | 99 | `<input type="checkbox" class="checkbox">` → `<Checkbox>` |
| `AssignToHostModal.svelte` | 302 | `<input type="checkbox" class="checkbox">` → `<Checkbox>` |
| `AssignToHostModal.svelte` | 339, 465 | `<input class="input text-sm">` → `<Input class="text-sm">` |
| `EditHostAssignmentModal.svelte` | 738, 803 | `<input class="input text-sm">` → `<Input class="text-sm">` |
| `EditHostAssignmentModal.svelte` | 770, 891, 1090, 1220 | `<textarea class="textarea font-mono text-xs">` → `<Textarea variant="mono">` |
| `EditHostAssignmentModal.svelte` | 789, 915, 1106 | `<input type="checkbox" class="checkbox">` → `<Checkbox>` |
| `SoftwareMergeWizard.svelte` | 261 | `<input class="input" type="search">` → `<Input type="search">` |
| `surfaces/SurfaceForm.svelte` | 137 | `<textarea class="textarea font-mono text-xs">` → `<Textarea variant="mono">` |

### `routes/settings/` files

| File | Line(s) | Change |
| --- | --- | --- |
| `AuthenticationSettings.svelte` | 50 | `<input type="checkbox" class="checkbox">` → `<Checkbox>` |
| `AgentCertificateSettings.svelte` | 69 | `<input type="checkbox" class="checkbox">` → `<Checkbox>` |
| `AgentCertificateSettings.svelte` | 77 | `<input class="input">` → `<Input>` |
| `EnrollmentTokenSettings.svelte` | 246 | `<input type="checkbox" class="checkbox">` → `<Checkbox>` |
| `NotificationRulesSettings.svelte` | 284 | `<input type="checkbox" class="checkbox">` → `<Checkbox>` |
| `OidcProvidersSettings.svelte` | 336, 348 | `<input type="checkbox" class="checkbox">` → `<Checkbox>` |
| `RegistrationSettings.svelte` | 90 | `<input type="checkbox" class="checkbox">` → `<Checkbox>` |

### `routes/software/` files

| File | Line(s) | Change |
| --- | --- | --- |
| `+page.svelte` | 889, 940, 972, 1499 | `<input type="checkbox" class="checkbox">` → `<Checkbox>` |
| `+page.svelte` | 1487 | `<input type="text" class="input">` → `<Input>` |
| `[id]/+page.svelte` | 1119 | `<input type="text">` → `<Input>` |
| `[id]/+page.svelte` | 1129 | `<input type="checkbox">` → `<Checkbox>` |
| `IgnoreRulesTab.svelte` | 162, 184 | `<input type="checkbox" class="checkbox">` → `<Checkbox>` |
| `IgnoreRulesTab.svelte` | 249 | `<input type="text">` → `<Input>` |

### Other route files

| File | Line | Change |
| --- | --- | --- |
| `routes/services/+page.svelte` | 688 | `<input class="input w-full">` → `<Input class="w-full">` |
| `routes/system-services/+page.svelte` | 661 | `<input class="input w-full">` → `<Input class="w-full">` |

---

## Migration pattern

For each file:

1. Add import if not already present:

   ```svelte
   import Input from '$lib/components/Input.svelte';
   import Checkbox from '$lib/components/Checkbox.svelte';
   import Textarea from '$lib/components/Textarea.svelte';
   ```

   (Check existing imports; some files may already import one or more of these.)

2. Replace raw elements per the scope table. Remove `class="input"`, `class="checkbox"`,
   `class="textarea"` from the element (the primitive applies its own base styles).
   Keep any additional classes (e.g. `w-full`) on the `class` prop of the primitive.
   Use `variant="mono"` (not `class="font-mono text-xs"`) for monospace/code fields.

3. **`id` prop is required on all three primitives** — `Input`, `Checkbox`, and `Textarea`
   all have `id: string` as a non-optional prop. Raw elements that omit `id` must be given
   one. Derive ids from field context: e.g. `id="filter-text"`, `id="assign-force-checkbox"`,
   `id="host-config-notes"`. IDs must be unique within the page; use the containing section
   name as a prefix when multiple instances exist.
   Failing to supply `id` will produce a TypeScript error in `npm run check`.

4. Run `cd frontend && npm run check` — verify no binding type errors (all primitives
   accept `bind:value` / `bind:checked`) and no missing `id` prop errors.

5. Run `cd frontend && npm run test` — verify existing component tests still pass.

---

## Testing

- `npm run check` — zero type errors (including missing `id` prop errors).
- `npm run test` — all Vitest tests pass. Component tests that assert on the raw DOM
  element may need updating. Example: a test that does
  `wrapper.find('input[type="checkbox"]')` will still find the internal `<input>` rendered
  by `<Checkbox>` — no change needed. A test that asserts `class="checkbox"` on the element
  will break — update to assert on the primitive's rendered structure instead.

## Rollout

Single PR titled `"fix(frontend): replace raw form elements with Input/Checkbox/Textarea primitives"`.

No dependency on other gap sub-specs. Can land in any order.

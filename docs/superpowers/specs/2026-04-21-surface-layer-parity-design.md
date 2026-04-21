# Surface-Layer Parity — Design

**Parent spec:** `docs/superpowers/specs/2026-04-16-ui-design-language-design.md`
(§4.3 Buttons, §4.10 Form Validation, §5 Surfaces)

**Sub-spec #4 of the UI design-language rollout.** Depends on sub-spec #2
(Button primitive) merged; sub-spec #2b (Input + Checkbox + Link
primitives) merged. Parallel-safe with all #3 sub-specs.

## Overview

Bring the surface-layer components (plugin-driven dynamic UI rendered
from server-provided schemas) into parity with the design language.
Files in `frontend/src/lib/components/surfaces/`:
`SurfaceInteractionButton.svelte` (143 lines), `SurfaceRenderer.svelte`
(227), `SurfaceWorkflow.svelte` (532), `SurfaceForm.svelte` (163),
`SchemaForm.svelte` (479), `SurfaceActionBar.svelte` (68),
`SurfaceReadPanel.svelte` (360), `SurfaceTable.svelte` (249),
`SurfaceSlot.svelte` (51), `SurfaceKeyValue.svelte` (28),
`SurfaceModal.svelte` (24). Surface-layer is shape-sensitive: server
data describes what to render, so the components need to map server
semantics (button styles like `primary | secondary | danger | ghost`) to
local variants cleanly.

## Design decisions

**Q1 — Server-side button-style enum mapping.**

- Options:
  - (chosen) Direct 1:1 mapping between server-side `ButtonStyle`
    ('primary', 'secondary', 'danger', 'ghost') and Button primitive
    `ButtonVariant`. Same string values already align. Update
    `SurfaceInteractionButton.svelte` to pass the value through, with
    a fallback to `'secondary'` on unknown values (matches the
    wire-safe Other(String) pattern from backend enum conventions).
  - Introduce a per-surface mapping layer. Rejected — wire format
    already mirrors internal variant names; extra layer adds nothing.
- Reasoning: wire protocol and internal variant names align by design;
  the fallback arm handles forward-compatible variants as the server
  evolves.

**Q2 — SchemaForm input migration to `<Input>` / `<Checkbox>`.**

- Options:
  - (chosen) Migrate every input field rendered by SchemaForm to the
    `<Input>` primitive (and `<Checkbox>` for boolean fields, `<Textarea>`
    for multiline/`textarea` fields). Schema types like
    `string | email | url | integer | number | password | search` map to
    InputType values directly; `boolean` maps to Checkbox; `textarea`
    maps to Textarea. Fields like `select`, `array` stay on their
    existing renderers (out of #2b / #2d scope).
  - Leave SchemaForm on raw inputs. Rejected — defeats the parity goal;
    surface-rendered forms should look identical to hand-coded forms.
- Reasoning: #2b primitives are the single source of truth for form
  visuals; SchemaForm is a consumer like any other, not a parallel path.

**Q3 — SurfaceActionBar vs BatchActionBar.**

- Options:
  - (chosen) Keep them distinct. SurfaceActionBar renders surface-declared
    actions (driven by surface schema); BatchActionBar is list-row bulk
    actions (driven by selection state). Both adopt Button primitive;
    each owns its own context.
  - Unify them. Rejected — different contracts; unification would force
    one to pretend to be the other.
- Reasoning: surface actions and batch actions look similar but have
  different data flows; local primitive use is enough parity.

**Q4 — Custom validation rendering in SchemaForm vs `<Input error>` prop.**

- Options:
  - (chosen) SchemaForm passes `error={fieldErrors[fieldName]}` to
    each `<Input>`; drops its bespoke error-rendering code.
  - Keep SchemaForm's bespoke rendering. Rejected — duplicates
    `<Input>` primitive contract.
- Reasoning: single source of error rendering. SchemaForm should own
  field-level validation state, not visual expression.

**Q5 — SurfaceReadPanel / SurfaceKeyValue / SurfaceTable button sites.**

- Options:
  - (chosen) Migrate every button surface (row actions, collapsible
    toggles, copy-value buttons) to Button primitive with appropriate
    variant. Table row actions follow #3f row-action convention
    (ghost, sm, icon).
  - Defer these read-only-panel buttons. Rejected — still renders
    preset-* classes; scope includes all surface components.
- Reasoning: consistent coverage across the surfaces/ directory.

**Q6 — SurfaceWorkflow step navigation.**

- Options:
  - (chosen) Adopt wizard pattern from #3f's SoftwareMergeWizard:
    Back=secondary, Next/Finish=primary, Cancel=ghost. Server schema
    already encodes step labels; local component handles variants.
  - Let server specify variants per button. Rejected — variant choice
    is a local UI concern; server should not dictate visual style
    beyond the base action intent.
- Reasoning: wizard button shape is a local convention; server
  describes semantics, client renders.

## Goals

1. Every button in every surface component renders through `<Button>`.
2. SchemaForm inputs render through `<Input>` / `<Checkbox>`; error
   state via the `error` prop.
3. SurfaceInteractionButton maps server `ButtonStyle` → local
   `ButtonVariant` 1:1 with safe fallback to `secondary`.
4. Link-shaped items rendered by surface components use `<Link>`
   primitive from #2b.

## Non-goals

- Surface wire-protocol changes — backend / shared-types scope.
- New surface types (textarea, select, array) — existing renderers
  stay.
- Plugin-authored CSS tokens — surfaces render against existing CSS
  variables.
- SurfaceModal shell refactor — outside Button scope.

## Scope

Files migrated:

- `SurfaceInteractionButton.svelte` — variant mapping + Button
  primitive render.
- `SurfaceRenderer.svelte` — any directly-rendered buttons.
- `SurfaceWorkflow.svelte` — wizard nav + per-step buttons.
- `SurfaceForm.svelte` — submit / reset / cancel.
- `SchemaForm.svelte` — migrate every input field to `<Input>` /
  `<Checkbox>` / `<Textarea>`; drop bespoke error rendering; wire
  `error` prop.
- `SurfaceActionBar.svelte` — render Button primitive for each action.
- `SurfaceReadPanel.svelte` — collapsible toggles, copy-value buttons.
- `SurfaceTable.svelte` — row-level action buttons.
- `SurfaceSlot.svelte` — any interactive content; mostly pass-through.
- `SurfaceKeyValue.svelte` — copy-value buttons (if present).
- `SurfaceModal.svelte` — close / action buttons.

## Migration pattern

Standard translation rules plus:

- `SurfaceInteractionButton`:

  ```svelte
  <script lang="ts">
    import Button from '$lib/components/Button.svelte';
    import type { ButtonVariant } from '$lib/components/Button.svelte';

    const { style, label, onclick, loading } = $props<{
      style?: string;
      label: string;
      onclick: () => void;
      loading?: boolean;
    }>();

    const KNOWN: ButtonVariant[] = ['primary', 'secondary', 'danger', 'ghost'];
    const variant: ButtonVariant = KNOWN.includes(style as ButtonVariant)
      ? (style as ButtonVariant)
      : 'secondary';
  </script>

  <Button variant={variant} loading={loading} onclick={onclick}>{label}</Button>
  ```

- `SchemaForm` field-type dispatch:

  ```svelte
  {#if field.type === 'boolean'}
    <Checkbox id={field.name} bind:checked={values[field.name]} />
  {:else if field.type === 'textarea'}
    <Textarea
      id={field.name}
      bind:value={values[field.name]}
      error={fieldErrors[field.name]}
      rows={field.rows ?? 4}
      required={field.required}
      variant={field.mono ? 'mono' : 'default'}
    />
  {:else}
    <Input
      id={field.name}
      type={mapSchemaType(field.type)}
      bind:value={values[field.name]}
      error={fieldErrors[field.name]}
      autocomplete={field.autocomplete}
      required={field.required}
    />
  {/if}
  ```

## Data flow

Template-level plus one function:
`mapSchemaType(schemaType): InputType` in `SchemaForm.svelte`. No
runtime protocol changes; error state routing goes through `<Input>` prop.

## Error handling

Button + Input discriminated unions catch invalid prop combos at
compile time. Surface protocol errors (unknown button style, unknown
schema type) hit fallback variants/types. Log once via `console.warn`
for unknown values — tracks forward-compatible protocol additions.

## Testing

### Unit tests

Extend every existing surface `.test.ts` file:

- `SurfaceInteractionButton.test.ts` — variant mapping for each known
  style; fallback to secondary for unknown; loading passthrough; known
  `KNOWN` array mirrors server wire enum.
- `SchemaForm.test.ts` — field-type → primitive dispatch; boolean →
  Checkbox; string → Input; error prop wired through; required flag
  respected.
- `SurfaceWorkflow.test.ts` — Back=secondary, Next=primary, Cancel=ghost;
  loading during submit.
- `SurfaceForm.test.ts` — submit / reset / cancel variants.
- `SurfaceActionBar.test.ts` — per-action variant passthrough.
- `SurfaceReadPanel.test.ts` / `SurfaceTable.test.ts` — row actions
  render ghost + sm.
- `SurfaceModal.test.ts` — close button ghost + icon-only.

### Integration / e2e

- `/dev/surface-preview` route (if absent, create) renders every
  surface component with representative schema + action sets in both
  themes. Playwright snapshot gate.
- Real-world route: re-baseline any route that hosts a plugin-rendered
  surface (e.g. plugin config modal). Delta enumeration: input radius
  `rounded-[3px]`, button height `h-[23px]`, wizard button gradient.

## Rollout

Single PR titled
"feat(frontend): bring surface-layer components into design-language parity (sub-spec #4)".

Prereq: sub-spec #2 + #2b merged.

1. `SurfaceInteractionButton.svelte` — variant mapping + Button render.
2. `SurfaceForm.svelte` — submit/reset/cancel migration.
3. `SchemaForm.svelte` — field-type dispatch + error-prop wiring.
4. `SurfaceWorkflow.svelte` — wizard nav migration.
5. `SurfaceActionBar.svelte` — per-action Button render.
6. `SurfaceReadPanel.svelte` / `SurfaceTable.svelte` / `SurfaceKeyValue.svelte`
   / `SurfaceSlot.svelte` — remaining button sites.
7. `SurfaceRenderer.svelte` / `SurfaceModal.svelte` — any remaining.
8. Extend unit tests per plan.
9. Add `/dev/surface-preview` if absent; Playwright snapshots.
10. Full frontend gate.

### Risk + rollback

Revert of one PR restores preset-* classes across surface layer.
Surface regression is user-visible everywhere plugins render custom UI
(plugin configuration modals, surface-driven wizards). Mitigated by
`/dev/surface-preview` Playwright coverage plus per-component unit
tests.

### Dependencies + ordering

- **Blocks on:** sub-spec #2 merged, sub-spec #2b merged, sub-spec
  #2c merged (`variant="secondary"` for SurfaceInteractionButton
  mapping + SurfaceWorkflow Back button), sub-spec #2d merged
  (Textarea primitive for SchemaForm string-textarea field type +
  SurfaceForm textarea site).
- **Blocks:** nothing downstream inside the #3 series (surfaces are a
  parallel concern).
- **Parallel-safe with:** every #3 sub-spec.

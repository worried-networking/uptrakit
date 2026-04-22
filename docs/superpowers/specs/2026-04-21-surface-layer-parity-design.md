# Surface-Layer Parity — Design

**Parent spec:** `docs/superpowers/specs/2026-04-16-ui-design-language-design.md` (§4.3 Buttons, §4.10 Form Validation,
§5 Surfaces)

**Sub-spec #4 of the UI design-language rollout.** Depends on sub-spec #2 (Button primitive) merged, sub-spec #2b
(Input + Checkbox + Link primitives) merged, sub-spec #2c merged (`variant="secondary"` + base- Button `ariaLabel`),
sub-spec #2d (Textarea primitive) merged, and **sub-spec #3k merged** — `ConfirmDialog` swaps its `confirmClass` prop
for `confirmVariant` there, and two files in this sub-spec's scope (`SurfaceInteractionButton` + `SurfaceWorkflow`) call
`<ConfirmDialog>` and must migrate their caller shape in the same PR.

## Overview

Bring the surface-layer components (plugin-driven dynamic UI rendered from server-provided descriptors) into parity with
the design language. Files live in `frontend/src/lib/components/surfaces/`. Source verification (see Scope below) shows
only five of the eleven files there actually host preset-\* `<button>` sites — the rest delegate through
`SurfaceInteractionButton` or render no interactive chrome at all. The spec is organized around the five files that
require work; the six delegating / inert files are called out explicitly so an implementer doesn't invent migration work
for them.

## Design decisions

**Q1 — Button variant source in `SurfaceInteractionButton`.**

- Options:
  - (chosen) Derive the local Button variant from the existing `interaction.confirmation?.severity` field, because that
    is the only variant signal the server protocol provides today. Source `SurfaceInteractionButton.svelte` (props at
    lines 12-32) has no `style`, `button_style`, or `variant` prop from the server; the pre-migration markup at lines
    99-101 uses a `presetClass` derived at line 50-52:
    `severity === 'danger' ? 'preset-filled-error-500' : 'preset-filled-primary-500'`. The migration maps that same
    severity check onto the primitive:
    `variant = interaction.confirmation?.severity === 'danger' ? 'danger' : 'primary'`. No new server field is
    introduced, no `KNOWN` array + `as ButtonVariant` fallback is invented.
  - Introduce a new server-side `ButtonStyle` enum covering `'primary' | 'secondary' | 'danger' | 'ghost'` and pass it
    through 1:1. Rejected — this is a backend wire-protocol addition, out of this sub-spec's frontend scope, and
    unnecessary for today's UI (every production interaction is either primary or danger). If a future interaction needs
    secondary or ghost semantics, extending the descriptor is an additive change, tracked separately.
- Reasoning: spec must match the protocol that exists, not one we wish existed. The two-variant fallback covers every
  production surface. An earlier draft of this spec invented a four-variant enum pattern
  (`KNOWN: ButtonVariant[] = [...]`) that the codebase does not support; that draft was stale and is removed.

**Q2 — `SchemaForm` input migration.**

- Options:
  - (chosen) Migrate `SchemaForm`'s per-field rendering against the actual `FieldType` enum exported from
    `frontend/src/lib/types.ts` (lines 864-874):
    `'text' | 'password' | 'number' | 'select' | 'multi_select' | 'textarea' | 'toggle' | 'hidden' | 'ssh_private_key' | string`.
    The field-type dispatch becomes:
    - `'text'` → `<Input type="text">`
    - `'password'` → `<Input type="password">`
    - `'number'` → `<Input type="number">`
    - `'ssh_private_key'` → `<Textarea variant="mono" rows={8}>` (multi-line key paste, monospace variant per #2d;
      `rows={8}` preserves the current source value)
    - `'textarea'` → `<Textarea rows={3}>` (preserves the current source value; do not use a generic `rows={4}`)
    - `'toggle'` → `<Checkbox>` (read/write via `values[field.key] === 'true'` string ↔ checkbox `bind:checked` boolean
      adapter — preserves the existing string serialization contract)
    - `'select'`, `'multi_select'` → unchanged; keep existing renderers (out of #2b / #2d scope — option-list primitive
      is a separate design concern)
    - `'hidden'` → `<input type="hidden">` raw (not a visible control; no primitive needed)
    - Unknown (catch-all `string` branch) → `<Input type="text">` + `console.warn` once per unknown type
      (forward-compatible)
  - Adopt the variant claimed by an earlier draft (`string | email | url | integer | number | password | search`).
    Rejected — those types are not part of `FieldType`; the draft invented them.
- Reasoning: migrate against the actual type enum. Unknown types fall back to `<Input type="text">` so the form still
  renders and a single `console.warn` flags the mismatch for the next spec pass.

**Q3 — `SurfaceActionBar` and `SurfaceTable` button rendering.**

- Options:
  - (chosen) Neither file renders `<button>` directly. `SurfaceActionBar` (lines 48-68) loops `resolvedActions` and
    renders `<SurfaceInteractionButton>` per action; `SurfaceTable` (around line 207) does the same for row-level
    actions. The migration of `SurfaceInteractionButton` (Q1) fully satisfies both. This sub-spec does not touch either
    file's source beyond verifying unit-test expectations still hold.
  - Re-implement Button primitive use directly inside `SurfaceActionBar`. Rejected — duplicates work and breaks the
    single-source-of-truth contract these files deliberately delegate to `SurfaceInteractionButton`.
- Reasoning: both are layout wrappers over `SurfaceInteractionButton`; once the child migrates, the wrappers inherit the
  visual change.

**Q4 — Loading contract (`#3c Q4`) on surface-layer form submits.**

- Three files contain `{submitting/loading/preLoading ? 'Xing…' : label}` text-swap expressions that are incompatible
  with the established #3c Q4 loading contract (bind `loading={flag}`, preserve static children, rely on the primitive's
  spinner + text preservation from #2c):
  - `SurfaceInteractionButton.svelte` line 100 — `{loading ? 'Processing...' : actionLabel}`.
  - `SurfaceForm.svelte` line 139 — `{submitting ? 'Submitting...' : effectiveSubmitLabel}`.
  - `SchemaForm.svelte` lines 470-476 — three-branch swap (`preLoading → 'Loading...'`, `loading → 'Processing...'`,
    else `submitLabel`).
- Options:
  - (chosen) Drop every text-swap. Use the primitive's `loading` prop on all three sites. Static children text is
    preserved during the loading window; the primitive renders the spinner centred over the text per #2c. For
    `SchemaForm`'s three-state button, bind `loading={loading || preLoading}` — the two flags collapse into one loading
    visual, matching every other consumer of the #3c Q4 contract. The only user-visible change is the loss of the
    `Loading…` vs `Processing…` distinction, which was not carrying semantic weight beyond "busy."
  - Preserve text-swap. Rejected — violates #3c Q4 and diverges from every other migrated surface.
- Reasoning: surface-layer consumers converge with non-surface consumers on the same loading shape. Uniformity beats the
  single preserved label distinction.

**Q5 — `SchemaForm` validation error rendering.**

- Options:
  - (chosen) Pass `error={fieldErrors[field.key]}` to `<Input>` and `<Textarea>` directly. The `<Input>` and
    `<Textarea>` primitives (#2b / #2d) own the error-row render (red border + message row). For `toggle` (checkbox)
    fields, `<Checkbox>` has **no `error` prop** — wrap with `<FormFieldRow>` instead, which owns the error message row
    and publishes `aria-describedby` context:

    ```svelte
    <FormFieldRow id="field-{field.key}" label={field.label} error={fieldErrors[field.key]}>
        <Checkbox
            id="field-{field.key}"
            checked={Boolean(formValues[field.key])}
            onchange={(e) => { formValues[field.key] = (e.target as HTMLInputElement).checked; }}
            disabled={loading}
        />
    </FormFieldRow>
    ```

    Drop any bespoke `aria-invalid` wiring that duplicates the primitive contract; keep `aria-invalid` only if the
    primitive itself still emits it. Never pass `error` directly to `<Checkbox>`.
  - Keep `SchemaForm`'s bespoke error rendering. Rejected — duplicates the primitive contract from #2b.
- Reasoning: single source of truth for field-level error visuals. `Checkbox.svelte` intentionally omits `error` — it
  is a bare input control; error display is the responsibility of the `FormFieldRow` wrapper.

**Q6 — `SurfaceWorkflow` wizard navigation.**

- Source has seven button sites plus a `<ConfirmDialog>` caller:
  - Line 339-347: the workflow-trigger entry button. Uses the SAME `presetClass` derivation at line 53 as
    `SurfaceInteractionButton` (`severity === 'danger' ? 'preset-filled-error-500' : 'preset-filled-primary-500'`) —
    migrates identically to Q1.
  - Line 484: Cancel footer (`preset-tonal-surface`).
  - Line 494: Back (`preset-tonal-surface`).
  - Line 497-499: `{isLastStep ? 'Done' : 'Execute'}` primary (review- next branch) — `preset-filled-primary-500`.
  - Line 501-503: `{isLastStep ? 'Run' : 'Continue'}` primary (form- submit branch).
  - Line 505-507: `{isLastStep ? 'Run' : 'Continue'}` primary (step- submit branch).
  - Line 509-511: `{isLastStep ? 'Done' : 'Continue'}` primary (no- submit branch).
  - Line 518: `<ConfirmDialog>` caller (default prop passthrough).
- Options:
  - (chosen) **Cancel and Back both map to `<Button variant="secondary">`.** Cancel and Back render with identical
    `preset-tonal-surface` classes today; promoting only one to ghost would introduce an unintended visual hierarchy.
    Preserve their pre-migration visual parity by assigning both the same variant. The four primary buttons (lines
    497/501/505/509) each map to `<Button variant="primary" loading={loading}>` with the existing dynamic children text
    preserved (the children strings `'Done'` / `'Execute'` / `'Run'` / `'Continue'` are legitimate content that varies
    with step state, distinct from a loading text-swap; they stay). The workflow-trigger (line 339) adopts the Q1
    severity-derived variant.
  - Adopt a "wizard pattern" (Back=secondary, Next/Finish=primary, Cancel=ghost). Rejected — makes Cancel visually
    distinct from Back purely to match a template that was never actually enforced in this codebase; any visual redesign
    of Cancel vs Back is a design change, not a migration concern.
- Reasoning: migration-only purity. The four primary button children are step-semantic labels (not loading text), so
  children text stays dynamic; Q4 only removes `loading`-bound swaps.

**Q7 — `SurfaceReadPanel` "Try again" retry button shape.**

- Two source files own the remaining preset-\* / custom-outline button sites outside of the
  interaction/workflow/form/schema families:
  - `SurfaceRenderer.svelte` line 186: `modal-trigger` with `preset-tonal-surface` →
    `<Button variant="secondary" type="button" data-ui="modal-trigger">`. Only button in the file; unrelated to retry.
  - `SurfaceReadPanel.svelte` lines 318-324 and 343-349: "Try again" retry button inside a `<Callout tone="danger">`. It
    is **not** on a preset-\* class — uses custom inline tailwind:
    `border border-[var(--color-error-border)] px-2 py-1 text-xs font-medium text-[var(--color-error)] transition-colors hover:bg-[var(--color-error-bg)]`.
    Shape is a compact danger- tinted outline affordance. Both sites call `retryHydration` and differ only in the
    `{#if descriptor.targeting === 'targeted'}` branch they sit under.
- Options:
  - (chosen) Migrate to `<Button variant="danger" size="sm">Try again</Button>`. The primitive's `danger` variant
    (filled gradient + red ring per parent §4.3) is the correct error- recovery shape; the current outline treatment is
    a bespoke one-off that predates the variant contract. This is a visible style change, but it converges the
    recovery-button visual across every `<Callout tone="danger">` caller (none of which have yet been migrated to the
    Button primitive — this is the first).
  - Preserve the outline style via `class` override. Rejected — the design language explicitly owns the danger-button
    shape; ad-hoc overrides defeat the parity goal.
- Reasoning: parent §4.3 treats `variant="danger"` as the recovery- from-error shape. Explicit mention here so the
  implementer doesn't skip the button as "no preset class to migrate."

**Q8 — `<ConfirmDialog>` caller migration coordination.**

- `SurfaceInteractionButton.svelte` line 129 and `SurfaceWorkflow.svelte` line 518 both call `<ConfirmDialog>`. Post-#3k
  the `confirmClass` prop is replaced by `confirmVariant` (`'primary' | 'danger'`).
- Options:
  - (chosen) Both callers pass
    `confirmVariant={interaction.confirmation ?.severity === 'danger' ? 'danger' : 'primary'}` — same severity mapping
    used by Q1 for the launcher button's own variant. The `presetClass` derived var at `SurfaceWorkflow` line 53 is
    deleted (unused after migration) and replaced by a `confirmVariant` derived var with the same severity check. The
    launcher button variant + the confirm dialog variant are driven by the same derivation, so they stay visually
    coherent.
  - Let callers keep the default (`confirmVariant='danger'` per #3k Q1). Rejected — non-danger severities would
    over-signal destruction (defaulting to danger for every non-danger interaction would visually lie about the action).
- Reasoning: cross-spec coordination with #3k. This is the reason #3k is listed as a prerequisite above.

## Goals

1. `SurfaceInteractionButton` renders through `<Button>`; variant derived from `interaction.confirmation?.severity` per
   Q1.
2. `SurfaceWorkflow` migrates every button site enumerated in Q6; passes `confirmVariant` to `<ConfirmDialog>` per Q8.
3. `SurfaceForm` and `SchemaForm` submit buttons migrate to `<Button variant="primary" loading={...}>` with the
   text-swap dropped per Q4.
4. `SchemaForm` per-field rendering dispatches against the actual `FieldType` enum; error-state rendering wires through
   the primitive `error` prop per Q5.
5. `SurfaceRenderer` modal-trigger (line 186) renders `<Button variant="secondary">` per Q7.
6. `SurfaceReadPanel` "Try again" retry buttons (lines 318-324 + 343-349) render `<Button variant="danger" size="sm">`
   per Q7; custom outline treatment retired.
7. `SurfaceActionBar` and `SurfaceTable` unchanged at the source level; visual change inherited from
   `SurfaceInteractionButton` migration.
8. `SurfaceModal`, `SurfaceKeyValue`, `SurfaceSlot` — unchanged; no buttons to migrate.

## Non-goals

- Server-side descriptor protocol changes (no new `ButtonStyle` enum, no new interaction fields) — backend /
  shared-types scope.
- New `FieldType` values — out of scope; the migration covers the current enum plus a warning fallback.
- `SurfaceModal`, `SurfaceKeyValue`, `SurfaceSlot` — no buttons, no form inputs; explicitly no work.
- `SurfaceActionBar` and `SurfaceTable` source edits — delegation to `SurfaceInteractionButton` covers them.
- Option-list primitive for `select` / `multi_select` fields — future sub-spec.
- Plugin-authored CSS tokens — surfaces render against existing CSS variables.

## Scope

Files migrated (button and input sites enumerated exhaustively against current source; adding sites not listed here is
out of scope):

### `frontend/src/lib/components/surfaces/SurfaceInteractionButton.svelte`

- Line 99-101: main action button. Migrate to
  `<Button variant={ interaction.confirmation?.severity === 'danger' ? 'danger' : 'primary'} size={size} loading={loading} onclick={requestAction}>{ actionLabel}</Button>`.
  The derived `presetClass` (line 50-52) and `buttonClass` (line 49) are deleted — size is passed through to the
  primitive directly. Text-swap `{loading ? 'Processing...' : actionLabel}` is dropped per Q4.
- Line 129 `<ConfirmDialog>`: add
  `confirmVariant={interaction .confirmation?.severity === 'danger' ? 'danger' : 'primary'}` per Q8.

### `frontend/src/lib/components/surfaces/SurfaceWorkflow.svelte`

- Line 53 `presetClass` derived var: deleted (unused post-migration). Replace with a `confirmVariantForSeverity` derived
  var returning `'danger' | 'primary'` for use at line 518.
- Line 339-347 workflow-trigger:

  ```svelte
  <Button
    variant={confirmVariantForSeverity}
    size={size}
    loading={loading}
    onclick={startWorkflow}
  >
    {actionLabel}
  </Button>
  ```

  `confirmVariantForSeverity` is already typed `'danger' | 'primary'` — the ternary `=== 'danger' ? 'danger' : 'primary'`
  is dead code and must not appear here. Text-swap `{loading ? 'Processing...' : actionLabel}` dropped per Q4.
- Line 484 Cancel footer: `<Button variant="secondary" disabled={ loading} onclick={...}>Cancel</Button>`.
- Line 494 Back: `<Button variant="secondary" disabled={loading} onclick={handleBack}>Back</Button>`.
- Line 497-499 Done/Execute review-next:
  `<Button variant="primary" loading={loading} onclick={handleReviewNext}>{isLastStep ? 'Done' : 'Execute'}</Button>`.
- Line 501-503 Run/Continue form submit:
  `<Button variant="primary" type="submit" form={WORKFLOW_FORM_ID} loading={loading}>{isLastStep ? 'Run' : 'Continue'}</Button>`.
- Line 505-507 Run/Continue step submit:
  `<Button variant="primary" loading={loading} onclick={() => void handleStepSubmit({})}>{ isLastStep ? 'Run' : 'Continue'}</Button>`.
- Line 509-511 Done/Continue no-submit:
  `<Button variant="primary" loading={loading} onclick={handleReviewNext}>{isLastStep ? 'Done' : 'Continue'}</Button>`.
- Line 518 `<ConfirmDialog>`: add `confirmVariant={ confirmVariantForSeverity}`.
- Line 426 `<label class="card ... preset-tonal-surface">`: explicitly OUT of scope — this class is on a `<label>`
  element for card-state styling, not a button.
- Lines 476-478 inline spinner `div`: explicitly OUT of scope — this is a loading-panel spinner, not a button.

### `frontend/src/lib/components/surfaces/SurfaceForm.svelte`

- Line 138: raw-payload fallback branch (`{:else}` path, active when `schemaFields.length === 0`). The primary
  form-submission path goes through `<SchemaForm>`, not through this button. The migration of line 138 is still correct
  and in scope — it is **not** the only code path, just the fallback path.
  `<Button variant="primary" type="submit" loading={submitting}>{effectiveSubmitLabel}</Button>`. Text-swap
  `{submitting ? 'Submitting...' : effectiveSubmitLabel}` dropped per Q4.
- Line ~145 `<ConfirmDialog>`: no `confirmVariant` prop needed. This is a destructive confirmation; it correctly
  inherits the `'danger'` default introduced by sub-spec #3k. Reviewed and intentionally left without explicit prop.

### `frontend/src/lib/components/surfaces/SchemaForm.svelte`

- Line 469 submit: `<Button type="submit" variant="primary" loading={ loading || preLoading}>{submitLabel}</Button>`.
  Three-branch text-swap (`preLoading → 'Loading...'`, `loading → 'Processing...'`, else `submitLabel`) collapsed to
  static `{submitLabel}` children per Q4.
- Per-field rendering sites: migrate against `FieldType` per Q2. The current source uses higher-level composition
  (`<CheckboxList>` + `<FormFieldRow>` imports, lines 6-7); the migration replaces raw `<input>` / `<textarea>` sites
  inside FormFieldRow with `<Input>` / `<Textarea>` / `<Checkbox>` per the Q2 dispatch.
- Validation: every primitive receives `error={fieldErrors[field.key]}` per Q5; bespoke `aria-invalid` duplication
  dropped.

### `frontend/src/lib/components/surfaces/SurfaceRenderer.svelte`

- Line 186 modal-trigger:
  `<Button variant="secondary" type="button" data-ui="modal-trigger" onclick={() => (modalOpen = true)}>{ interactionLabel(interaction)}</Button>`.
  This is the only button site in the file (227 lines) — retry buttons live in `SurfaceReadPanel`, not here.

### `frontend/src/lib/components/surfaces/SurfaceReadPanel.svelte`

- Lines 318-324 and 343-349 Retry "Try again": each site migrates to
  `<Button variant="danger" size="sm" type="button" onclick={ retryHydration}>Try again</Button>` per Q7. Both sites
  carry the same shape and live inside `<Callout tone="danger">`; they differ only in the
  `{#if descriptor.targeting === 'targeted'}` branch they sit under. The surrounding `<Callout>` is unchanged.

### Explicitly NOT migrated (source-level no-op)

- `SurfaceActionBar.svelte` — delegates to `<SurfaceInteractionButton>` (line 55). Visual change inherited.
- `SurfaceTable.svelte` — delegates to `<SurfaceInteractionButton>` (line 207). Visual change inherited.
- `SurfaceModal.svelte` (24 lines) — pure `<Modal>` wrapper, no buttons.
- `SurfaceKeyValue.svelte` (28 lines) — pure `<dl>`/`<dt>`/`<dd>` render, no buttons.
- `SurfaceSlot.svelte` (51 lines) — pure `<SurfaceRenderer>` wrapper, no buttons.

## Migration pattern

Standard translation rules (preset-filled-primary-500 → primary, preset-filled-error-500 → danger, preset-tonal-surface
→ secondary). Plus:

- Severity → variant helper:
  `const variantForSeverity = (severity?: string) => severity === 'danger' ? 'danger' : 'primary';`. Used by both
  `SurfaceInteractionButton` (Q1) and `SurfaceWorkflow` (Q6, Q8). Inline is fine — there's no shared module for two-line
  helpers.
- `SchemaForm` field dispatch:

  ```svelte
  {#if field.field_type === 'toggle'}
    <FormFieldRow id="field-{field.key}" label={field.label} error={fieldErrors[field.key]}>
      <Checkbox
        id={field.key}
        checked={values[field.key] === 'true'}
        onchange={(e) => { values[field.key] = e.currentTarget.checked ? 'true' : 'false'; }}
        disabled={loading}
      />
    </FormFieldRow>
  {:else if field.field_type === 'textarea' || field.field_type === 'ssh_private_key'}
    <Textarea
      id={field.key}
      bind:value={values[field.key]}
      error={fieldErrors[field.key]}
      rows={field.field_type === 'ssh_private_key' ? 8 : 3}
      variant={field.field_type === 'ssh_private_key' ? 'mono' : 'default'}
      required={field.required}
    />
  {:else if field.field_type === 'hidden'}
    <input type="hidden" name={field.key} bind:value={values[field.key]} />
  {:else if field.field_type === 'select' || field.field_type === 'multi_select'}
    <!-- unchanged: existing select / CheckboxList renderers -->
  {:else}
    <Input
      id={field.key}
      type={field.field_type === 'password' || field.field_type === 'number' ? field.field_type : 'text'}
      bind:value={values[field.key]}
      error={fieldErrors[field.key]}
      required={field.required}
    />
  {/if}
  ```

  Unknown `field_type` values fall through the last `{:else}` arm and render as `<Input type="text">`. Emit a single
  `console.warn` per unknown value (tracked via a module-local `Set<string>` to avoid warn spam).

## Data flow

Template-level changes only. `SchemaForm` adds a single `console.warn` call for unknown `field_type` values; no runtime
protocol or handler changes. No new props on `SurfaceInteractionButton` / `SurfaceWorkflow` / `SurfaceForm` /
`SchemaForm` / `SurfaceRenderer` / `SurfaceReadPanel` (the `size` prop on `SurfaceInteractionButton` already existed —
it just passes through to the primitive now).

## Error handling

Button and Input / Textarea / Checkbox discriminated unions catch invalid prop combos at compile time. `SchemaForm`'s
unknown- `field_type` fallback logs once, renders a text input, and continues. `SurfaceReadPanel`'s retry button is now
the primary recovery affordance inside its `<Callout tone="danger">`; its `onclick` handler (`retryHydration`) is
unchanged.

## Testing

### Unit tests

- `SurfaceInteractionButton.test.ts` — extend existing spec:
  - `confirmation.severity === 'danger'` → `<Button variant="danger">`.
  - Missing or non-'danger' severity → `<Button variant="primary">`.
  - `loading={true}` passthrough → `aria-busy="true"` on button; children text stays `actionLabel` (regression guard
    that text- swap is gone).
  - `size="sm"` passthrough asserts `h-[19px]`; `size="md"` asserts `h-[23px]`.
  - `<ConfirmDialog>` caller receives `confirmVariant` matching the same severity derivation.
- `SurfaceWorkflow.test.ts` — new or extended:
  - workflow-trigger (line 339) variant follows severity (same test matrix as `SurfaceInteractionButton`).
  - Cancel renders `variant="secondary"`; Back renders `variant="secondary"` (explicit parity — not ghost).
  - Each of the four primary step buttons renders `variant="primary"` with the correct children text per step state
    (`Done` vs `Execute` vs `Run` vs `Continue`) and `loading={loading}` passthrough.
  - ConfirmDialog (line 518) receives `confirmVariant` per severity.
- `SurfaceForm.test.ts` — submit renders `variant="primary"`; `loading={submitting}` wired; children text stays
  `effectiveSubmitLabel` across the loading window (regression guard).
- `SchemaForm.test.ts` — extend existing spec:
  - Field-type dispatch matrix: `'text'` → `<Input type="text">`, `'password'` → `<Input type="password">`, `'number'` →
    `<Input type="number">`, `'textarea'` → `<Textarea>`, `'ssh_private_key'` → `<Textarea variant="mono">`, `'toggle'`
    → `<Checkbox>`, `'hidden'` → `<input type="hidden">`, `'select'` → unchanged renderer, `'multi_select'` → unchanged
    renderer.
  - Unknown `field_type` (e.g. `'unexpected'`) → `<Input type="text">`
    - exactly one `console.warn` (assert via `vi.spyOn(console, 'warn')`).
  - `fieldErrors[field.key]` wires through to the primitive's `error` prop (check for the primitive's error-row render;
    do NOT assert bespoke `aria-invalid` on the raw input).
  - Submit button renders `variant="primary"` + `loading={loading || preLoading}`; children text stays `submitLabel`
    across both loading windows.
- `SurfaceRenderer.test.ts` — new or extended:
  - modal-trigger renders `<Button variant="secondary">` with `data-ui="modal-trigger"` preserved.
- `SurfaceReadPanel.test.ts` — extend existing spec (retry test already exists at line 579 / 630 of current source):
  - Retry button (both the `targeted` and `untargeted` branches) renders
    `<Button variant="danger" size="sm">Try again</Button>`; `onclick` invokes `retryHydration`. Existing
    `getByRole('button', { name: 'Try again' })` assertions continue to pass.

### Integration / e2e

- Re-baseline `/dev/surface-preview` (create if absent) with a fixture covering: one non-danger primary interaction, one
  danger interaction with confirmation, one workflow, one `SchemaForm` rendering every `FieldType` value, one
  retry-triggered error state. Dark and light themes.
- Re-baseline any production route that renders a plugin surface (e.g. plugin configuration modal, surface-driven
  wizards inside `/software` or `/hosts`). Delta enumeration per parent §9: input radius `rounded-[3px]`, button
  `h-[23px]` at size=md / `h-[19px]` at size=sm, danger gradient on retry.
- Snapshot masking (required for gate stability):
  - Mask in-flight spinner rotation on every `<Button loading>` site.
  - Mask transient toast banners raised by submit / retry flows.
  - Mask the `SurfaceWorkflow`'s loading-panel spinner (`<div class="... animate-spin ...">` at lines 476-478) — it
    still exists post-migration (not a button; not touched).

## Rollout

Single PR titled "feat(frontend): bring surface-layer components into design-language parity (sub-spec #4)".

Prereq: #2 + #2b + #2c + #2d + #3k merged.

1. `SurfaceInteractionButton.svelte` — migrate per Q1; `<ConfirmDialog>` caller passes `confirmVariant` per Q8.
2. `SurfaceWorkflow.svelte` — migrate all seven button sites per Q6; delete `presetClass` / `buttonClass` derived vars;
   `<ConfirmDialog>` caller passes `confirmVariant` per Q8.
3. `SurfaceForm.svelte` — migrate submit per Q4.
4. `SchemaForm.svelte` — migrate field dispatch per Q2; submit per Q4; error-prop wiring per Q5.
5. `SurfaceRenderer.svelte` — migrate modal-trigger (line 186) per Q7.
6. `SurfaceReadPanel.svelte` — migrate both retry sites (lines 318-324 + 343-349) per Q7.
7. Confirm `SurfaceActionBar` / `SurfaceTable` / `SurfaceModal` / `SurfaceKeyValue` / `SurfaceSlot` sources are
   untouched; re-grep each to verify no preset-\* `<button>` sites were added since this spec was written — if any are
   found, add them to the PR with the appropriate variant.
8. Extend unit tests per plan.
9. Add `/dev/surface-preview` if absent; re-baseline Playwright snapshots.
10. Full frontend gate.

### Risk + rollback

Revert of one PR restores preset classes across the surface layer. User-visible regression surface is every route that
hosts a plugin- rendered surface (plugin configuration modals, surface-driven wizards, host-tag custom rendering paths).
Mitigated by per-component unit tests, `/dev/surface-preview` Playwright coverage, and the compile-time gate from the
`ConfirmDialog` prop rename propagating from #3k.

### Dependencies + ordering

- **Blocks on:** sub-spec #2 merged (base Button with `leadingIcon` / `trailingIcon`); sub-spec #2b merged (Input /
  Checkbox / Link); sub-spec #2c merged (`variant="secondary"` + base-Button `ariaLabel`); sub-spec #2d merged (Textarea
  for `'textarea'` and `'ssh_private_key'` field types); **sub-spec #3k merged (ConfirmDialog `confirmVariant` prop
  rename)**.
- **Blocks:** nothing downstream inside the #3 series (surfaces are a parallel concern).
- **Parallel-safe with:** every #3 sub-spec whose scope does not touch surface-layer files (all of them — surfaces live
  in their own directory).

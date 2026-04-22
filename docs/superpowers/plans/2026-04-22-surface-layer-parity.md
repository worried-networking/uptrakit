# Surface-Layer Parity (#4) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task.
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Migrate all interactive buttons, inputs, checkboxes, textareas in the six
surface-layer components to the design-language primitives (Button, Input, Checkbox,
Textarea, FormFieldRow).

**Architecture:** Six components migrated in separate tasks. SchemaForm owns the
field-dispatch loop migration (Input/Checkbox/Textarea). SurfaceWorkflow owns
confirmVariant severity mapping. No new primitives created.

**Tech Stack:** Svelte 5, Button.svelte, Input.svelte, Checkbox.svelte, Textarea.svelte, FormFieldRow.svelte, Vitest, Playwright

---

## File map

| File | Change type | Summary |
| --- | --- | --- |
| `frontend/src/lib/components/surfaces/SurfaceInteractionButton.svelte` | Modify | Replace raw `<button>` with `<Button>` primitive; drop `presetClass`/`buttonClass`; add `confirmVariant` to `<ConfirmDialog>` |
| `frontend/src/lib/components/surfaces/SurfaceWorkflow.svelte` | Modify | Replace 7 raw `<button>` sites; delete `presetClass`/`buttonClass`; introduce `confirmVariantForSeverity`; add `confirmVariant` to `<ConfirmDialog>` |
| `frontend/src/lib/components/surfaces/SurfaceForm.svelte` | Modify | Replace one raw `<button>` (fallback branch) with `<Button>`; drop text-swap |
| `frontend/src/lib/components/surfaces/SchemaForm.svelte` | Modify | Replace raw `<button>` submit; replace `<input type="checkbox">` with `<Checkbox>`; replace raw `<textarea>` with `<Textarea>`; replace raw `<input>` with `<Input>`; drop bespoke `aria-invalid` |
| `frontend/src/lib/components/surfaces/SurfaceRenderer.svelte` | Modify | Replace one `<button class="btn preset-tonal-surface">` with `<Button variant="secondary">` |
| `frontend/src/lib/components/surfaces/SurfaceReadPanel.svelte` | Modify | Replace both custom-outline retry `<button>` sites with `<Button variant="danger" size="sm">` |
| `frontend/src/lib/components/surfaces/SurfaceInteractionButton.test.ts` | Modify | Add variant/loading/aria-busy/size/confirmVariant assertions |
| `frontend/src/lib/components/surfaces/SurfaceWorkflow.test.ts` | Modify | Add trigger variant, Cancel/Back secondary, four primary step buttons, confirmVariant assertions |
| `frontend/src/lib/components/surfaces/SurfaceForm.test.ts` | Modify | Add Button variant + loading wiring assertion |
| `frontend/src/lib/components/surfaces/SchemaForm.test.ts` | Modify | Add field-type dispatch matrix, error-prop wiring, unknown-type warn, submit primitive assertions |
| `frontend/src/lib/components/surfaces/SurfaceRenderer.test.ts` | Modify | Add modal-trigger `variant="secondary"` + `data-ui` assertion |
| `frontend/src/lib/components/surfaces/SurfaceReadPanel.test.ts` | Modify | Add retry button `variant="danger" size="sm"` assertions |

---

## Task 1: SurfaceInteractionButton — Button primitive + ConfirmDialog confirmVariant

**Files:**

- Modify: `frontend/src/lib/components/surfaces/SurfaceInteractionButton.svelte`
- Modify: `frontend/src/lib/components/surfaces/SurfaceInteractionButton.test.ts`

### Background

Current source (lines 49-52, 99-101, 128-129):

- `buttonClass` and `presetClass` are derived vars used to style the raw `<button>`.
- The button text-swaps `{loading ? 'Processing...' : actionLabel}`.
- `<ConfirmDialog>` at line 129 has no `confirmVariant` prop.

After migration:

- Both derived vars are deleted.
- The `<Button>` primitive receives `variant` derived from severity, `size` passed through, `loading={loading}`, `onclick={requestAction}`.
- Children text is always `{actionLabel}` (static — spinner covers the loading state).
- `<ConfirmDialog>` receives `confirmVariant` with the same severity derivation.

- [ ] **Step 1: Write the failing tests**

Add to `SurfaceInteractionButton.test.ts` inside the existing `describe` block:

```typescript
it('renders a primary Button for non-danger interactions', () => {
  const interaction: InteractionDescriptor = {
    interaction_id: 'do-thing',
    kind: 'mutation_action',
    label: 'Do Thing',
    transport: { mode: 'controller_local' }
  };

  render(SurfaceInteractionButton, {
    surfaceId: 'test.surface',
    interaction
  });

  const btn = screen.getByRole('button', { name: 'Do Thing' });
  // Button primitive applies h-[23px] for size=md
  expect(btn.className).toContain('h-[23px]');
  // No loading spinner initially
  expect(btn).not.toHaveAttribute('aria-busy');
});

it('renders a danger Button when severity is danger', () => {
  const interaction: InteractionDescriptor = {
    interaction_id: 'delete-thing',
    kind: 'mutation_action',
    label: 'Delete Thing',
    transport: { mode: 'controller_local' },
    confirmation: {
      title: 'Confirm',
      message: 'Are you sure?',
      severity: 'danger'
    }
  };

  render(SurfaceInteractionButton, {
    surfaceId: 'test.surface',
    interaction
  });

  const btn = screen.getByRole('button', { name: 'Delete Thing' });
  // Button primitive's danger variant contains error-bg token
  expect(btn.className).toContain('color-error');
});

it('sets aria-busy and preserves label text during loading', async () => {
  vi.mocked(invokeSurfaceInteraction).mockImplementation(
    () => new Promise(() => {}) // never resolves — keeps loading=true
  );
  const interaction: InteractionDescriptor = {
    interaction_id: 'slow-thing',
    kind: 'mutation_action',
    label: 'Slow Thing',
    transport: { mode: 'controller_local' }
  };

  render(SurfaceInteractionButton, {
    surfaceId: 'test.surface',
    interaction
  });

  const btn = screen.getByRole('button', { name: 'Slow Thing' });
  await fireEvent.click(btn);

  await waitFor(() => {
    expect(btn).toHaveAttribute('aria-busy', 'true');
  });
  // Children text is preserved during load — NOT replaced with 'Processing...'
  expect(btn.textContent).not.toContain('Processing');
  expect(screen.getByText('Slow Thing')).toBeInTheDocument();
});

it('renders size=sm Button with h-[19px]', () => {
  const interaction: InteractionDescriptor = {
    interaction_id: 'sm-thing',
    kind: 'mutation_action',
    label: 'Sm Thing',
    transport: { mode: 'controller_local' }
  };

  render(SurfaceInteractionButton, {
    surfaceId: 'test.surface',
    interaction,
    size: 'sm'
  });

  expect(screen.getByRole('button', { name: 'Sm Thing' }).className).toContain('h-[19px]');
});

it('no raw preset-filled-* or preset-tonal-* classes on any button', () => {
  const interaction: InteractionDescriptor = {
    interaction_id: 'any-thing',
    kind: 'mutation_action',
    label: 'Any Thing',
    transport: { mode: 'controller_local' }
  };

  const { container } = render(SurfaceInteractionButton, {
    surfaceId: 'test.surface',
    interaction
  });

  const buttons = container.querySelectorAll('button');
  buttons.forEach((b) => {
    expect(b.className).not.toMatch(/preset-filled|preset-tonal/);
  });
});
```

- [ ] **Step 2: Run tests to confirm they fail**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
npx vitest run src/lib/components/surfaces/SurfaceInteractionButton.test.ts
```

Expected: new tests fail (`h-[23px]` assertion fails because `<button>` has `btn` class not the primitive size class).

- [ ] **Step 3: Migrate SurfaceInteractionButton.svelte**

Open `frontend/src/lib/components/surfaces/SurfaceInteractionButton.svelte`.

a) Add `Button` import at top of `<script>`:

```svelte
import Button from '$lib/components/Button.svelte';
```

b) Delete lines 49-52 (the `buttonClass` and `presetClass` derived vars):

```svelte
// DELETE these two lines:
const buttonClass = $derived(size === 'sm' ? 'btn btn-sm text-xs' : 'btn');
const presetClass = $derived(
  interaction.confirmation?.severity === 'danger' ? 'preset-filled-error-500' : 'preset-filled-primary-500'
);
```

c) Replace the `<button>` block at lines 99-101:

Old:

```svelte
<button type="button" class="{buttonClass} {presetClass}" disabled={loading} onclick={requestAction}>
  {loading ? 'Processing...' : actionLabel}
</button>
```

New:

```svelte
<Button
  variant={interaction.confirmation?.severity === 'danger' ? 'danger' : 'primary'}
  {size}
  {loading}
  onclick={requestAction}
>
  {actionLabel}
</Button>
```

d) Add `confirmVariant` to `<ConfirmDialog>` at line ~129:

Old:

```svelte
{#if showConfirm && interaction.confirmation}
  <ConfirmDialog
    title={interaction.confirmation.title}
    messagePrefix={interaction.confirmation.message}
    entityName={actionLabel}
    {confirmLabel}
    onconfirm={() => {
```

New:

```svelte
{#if showConfirm && interaction.confirmation}
  <ConfirmDialog
    title={interaction.confirmation.title}
    messagePrefix={interaction.confirmation.message}
    entityName={actionLabel}
    {confirmLabel}
    confirmVariant={interaction.confirmation?.severity === 'danger' ? 'danger' : 'primary'}
    onconfirm={() => {
```

- [ ] **Step 4: Run tests to confirm they pass**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
npx vitest run src/lib/components/surfaces/SurfaceInteractionButton.test.ts
```

Expected: all tests pass.

- [ ] **Step 5: Format and type-check**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
npx prettier --write src/lib/components/surfaces/SurfaceInteractionButton.svelte
npx svelte-check --tsconfig ./tsconfig.json 2>&1 | grep -E "error|Error"
```

Expected: no errors.

- [ ] **Step 6: Commit**

```bash
cd /Users/andreyyantsen/Development/uptrakit
git add frontend/src/lib/components/surfaces/SurfaceInteractionButton.svelte \
        frontend/src/lib/components/surfaces/SurfaceInteractionButton.test.ts
git commit -m "feat(frontend): migrate SurfaceInteractionButton to Button primitive (#4)"
```

---

## Task 2: SurfaceWorkflow — all seven button sites + confirmVariant

**Files:**

- Modify: `frontend/src/lib/components/surfaces/SurfaceWorkflow.svelte`
- Modify: `frontend/src/lib/components/surfaces/SurfaceWorkflow.test.ts`

### Background

Current source has:

- Lines 51-54: `buttonClass` and `presetClass` derived vars.
- Line 339-347: workflow-trigger raw `<button>` with text-swap.
- Line 483-491: Cancel `<button class="btn preset-tonal-surface">`.
- Line 494: Back `<button class="btn preset-tonal-surface">`.
- Line 497-499: Done/Execute primary.
- Line 501-503: Run/Continue (form submit).
- Line 505-507: Run/Continue (step submit).
- Line 509-511: Done/Continue (no-submit).
- Line 518: `<ConfirmDialog>` with no `confirmVariant`.

After migration:

- `buttonClass` and `presetClass` deleted.
- New `confirmVariantForSeverity` derived var: `'danger' | 'primary'`.
- Trigger and four primary buttons use `<Button>` with `loading={loading}`.
- Cancel and Back use `<Button variant="secondary" disabled={loading}>`.
- `<ConfirmDialog>` receives `confirmVariant={confirmVariantForSeverity}`.

- [ ] **Step 1: Write the failing tests**

Add to `SurfaceWorkflow.test.ts` inside the existing `describe` block:

```typescript
it('renders workflow trigger with primary variant for non-danger severity', () => {
  const interaction: InteractionDescriptor = {
    interaction_id: 'bootstrap',
    kind: 'workflow',
    label: 'Bootstrap Host',
    transport: { mode: 'provider_proxied' },
    workflow_steps: [
      {
        step_id: 'execute',
        label: 'Execute',
        input_schema: 'object',
        result_schema: 'any',
        form_ui: { fields: [] }
      }
    ]
  };

  render(SurfaceWorkflow, {
    surfaceId: 'test.surface',
    interaction
  });

  const btn = screen.getByRole('button', { name: 'Bootstrap Host' });
  // primary variant has accent gradient
  expect(btn.className).toContain('h-[23px]');
  expect(btn.className).not.toMatch(/preset-filled|preset-tonal/);
});

it('renders workflow trigger with danger variant when severity is danger', () => {
  const interaction: InteractionDescriptor = {
    interaction_id: 'delete-workflow',
    kind: 'workflow',
    label: 'Delete Workflow',
    transport: { mode: 'provider_proxied' },
    confirmation: {
      title: 'Confirm',
      message: 'Are you sure?',
      severity: 'danger'
    },
    workflow_steps: [
      {
        step_id: 'execute',
        label: 'Execute',
        input_schema: 'object',
        result_schema: 'any',
        form_ui: { fields: [] }
      }
    ]
  };

  render(SurfaceWorkflow, {
    surfaceId: 'test.surface',
    interaction,
    interactions: [interaction]
  });

  const btn = screen.getByRole('button', { name: 'Delete Workflow' });
  expect(btn.className).toContain('color-error');
});

it('renders Cancel and Back as secondary variant buttons (not ghost)', async () => {
  vi.mocked(invokeSurfaceInteraction).mockResolvedValue({});
  const interaction: InteractionDescriptor = {
    interaction_id: 'multi-step',
    kind: 'workflow',
    label: 'Multi Step',
    transport: { mode: 'provider_proxied' },
    workflow_steps: [
      {
        step_id: 'step1',
        label: 'Step 1',
        input_schema: 'object',
        result_schema: 'any',
        submit_interaction_id: 'step1-submit',
        form_ui: { fields: [{ key: 'val', label: 'Val', field_type: 'text', required: false }] }
      },
      {
        step_id: 'step2',
        label: 'Step 2',
        input_schema: 'object',
        result_schema: 'any',
        form_ui: { fields: [] }
      }
    ]
  };
  const interactions: InteractionDescriptor[] = [
    interaction,
    {
      interaction_id: 'step1-submit',
      kind: 'mutation_action',
      label: 'Step1',
      transport: { mode: 'provider_proxied' }
    }
  ];

  render(SurfaceWorkflow, {
    surfaceId: 'test.surface',
    interaction,
    interactions
  });

  await fireEvent.click(screen.getByRole('button', { name: 'Multi Step' }));
  await fireEvent.click(screen.getByRole('button', { name: 'Continue' }));

  await waitFor(() => {
    // Back button is now visible on step 2
    const backBtn = screen.getByRole('button', { name: 'Back' });
    // secondary variant uses bg-raised token
    expect(backBtn.className).toContain('bg-[var(--bg-raised)]');
    expect(backBtn.className).not.toMatch(/preset-tonal/);
  });

  const cancelBtn = screen.getByRole('button', { name: 'Cancel' });
  expect(cancelBtn.className).toContain('bg-[var(--bg-raised)]');
  expect(cancelBtn.className).not.toMatch(/preset-tonal/);
});

it('renders four primary step buttons with correct children text', async () => {
  vi.mocked(invokeSurfaceInteraction).mockResolvedValue({});
  // Single-step workflow with form fields — renders Run/Continue form-submit path
  const interaction: InteractionDescriptor = {
    interaction_id: 'single',
    kind: 'workflow',
    label: 'Single Step',
    transport: { mode: 'provider_proxied' },
    workflow_steps: [
      {
        step_id: 'only',
        label: 'Only',
        input_schema: 'object',
        result_schema: 'any',
        submit_interaction_id: 'only-submit',
        form_ui: { fields: [{ key: 'v', label: 'V', field_type: 'text', required: false }] }
      }
    ]
  };
  const interactions: InteractionDescriptor[] = [
    interaction,
    {
      interaction_id: 'only-submit',
      kind: 'mutation_action',
      label: 'Only Submit',
      transport: { mode: 'provider_proxied' }
    }
  ];

  render(SurfaceWorkflow, {
    surfaceId: 'test.surface',
    interaction,
    interactions
  });

  await fireEvent.click(screen.getByRole('button', { name: 'Single Step' }));

  // isLastStep=true so form-submit branch should read 'Run'
  const runBtn = screen.getByRole('button', { name: 'Run' });
  expect(runBtn.className).toContain('h-[23px]');
  expect(runBtn.className).not.toMatch(/preset-filled/);
});

it('trigger loading state sets aria-busy and preserves label (no text-swap)', async () => {
  vi.mocked(invokeSurfaceInteraction).mockImplementation(() => new Promise(() => {}));
  const interaction: InteractionDescriptor = {
    interaction_id: 'long-workflow',
    kind: 'workflow',
    label: 'Long Workflow',
    transport: { mode: 'provider_proxied' },
    workflow_steps: [
      {
        step_id: 'step',
        label: 'Step',
        input_schema: 'object',
        result_schema: 'any',
        submit_interaction_id: 'step-submit',
        form_ui: { fields: [] }
      }
    ]
  };
  const interactions: InteractionDescriptor[] = [
    interaction,
    {
      interaction_id: 'step-submit',
      kind: 'mutation_action',
      label: 'Step Submit',
      transport: { mode: 'provider_proxied' }
    }
  ];

  render(SurfaceWorkflow, { surfaceId: 'test.surface', interaction, interactions });

  await fireEvent.click(screen.getByRole('button', { name: 'Long Workflow' }));
  const runBtn = screen.getByRole('button', { name: 'Run' });
  await fireEvent.click(runBtn);

  await waitFor(() => {
    expect(screen.getByRole('button', { name: 'Run' })).toHaveAttribute('aria-busy', 'true');
  });
  expect(screen.getByText('Run')).toBeInTheDocument();
  expect(screen.queryByText('Processing...')).not.toBeInTheDocument();
});

it('no raw preset-filled-* or preset-tonal-* classes on any button in modal', async () => {
  vi.mocked(invokeSurfaceInteraction).mockResolvedValue({});
  const interaction: InteractionDescriptor = {
    interaction_id: 'bootstrap',
    kind: 'workflow',
    label: 'Bootstrap Host',
    transport: { mode: 'provider_proxied' },
    workflow_steps: [
      {
        step_id: 'execute',
        label: 'Execute',
        input_schema: 'object',
        result_schema: 'any',
        form_ui: { fields: [] }
      }
    ]
  };

  const { container } = render(SurfaceWorkflow, {
    surfaceId: 'test.surface',
    interaction
  });

  await fireEvent.click(screen.getByRole('button', { name: 'Bootstrap Host' }));

  container.querySelectorAll('button').forEach((b) => {
    expect(b.className).not.toMatch(/preset-filled|preset-tonal/);
  });
});
```

- [ ] **Step 2: Run tests to confirm they fail**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
npx vitest run src/lib/components/surfaces/SurfaceWorkflow.test.ts
```

Expected: new tests fail on class assertions.

- [ ] **Step 3: Migrate SurfaceWorkflow.svelte**

Open `frontend/src/lib/components/surfaces/SurfaceWorkflow.svelte`.

a) Add `Button` import:

```svelte
import Button from '$lib/components/Button.svelte';
```

b) Delete the `buttonClass` derived var (line 51):

```svelte
// DELETE:
const buttonClass = $derived(size === 'sm' ? 'btn btn-sm text-xs' : 'btn');
```

c) Replace the `presetClass` derived var (lines 52-54) with `confirmVariantForSeverity`:

Old:

```svelte
const presetClass = $derived(
  interaction.confirmation?.severity === 'danger' ? 'preset-filled-error-500' : 'preset-filled-primary-500'
);
```

New:

```svelte
const confirmVariantForSeverity = $derived<'danger' | 'primary'>(
  interaction.confirmation?.severity === 'danger' ? 'danger' : 'primary'
);
```

d) Replace the workflow-trigger block (lines 339-347):

Old:

```svelte
<button
  type="button"
  class="{buttonClass} {presetClass}"
  data-ui="workflow-trigger"
  disabled={loading}
  onclick={startWorkflow}
>
  {loading ? 'Processing...' : actionLabel}
</button>
```

New:

```svelte
<Button
  variant={confirmVariantForSeverity}
  {size}
  {loading}
  data-ui="workflow-trigger"
  onclick={startWorkflow}
>
  {actionLabel}
</Button>
```

e) Replace Cancel footer button (lines 483-491):

Old:

```svelte
<button
  class="btn preset-tonal-surface"
  onclick={() => {
    showModal = false;
    resetWorkflowState();
  }}
  disabled={loading}
>
  Cancel
</button>
```

New:

```svelte
<Button
  variant="secondary"
  disabled={loading}
  onclick={() => {
    showModal = false;
    resetWorkflowState();
  }}
>
  Cancel
</Button>
```

f) Replace Back button (line 494):

Old:

```svelte
<button class="btn preset-tonal-surface" onclick={handleBack} disabled={loading}>Back</button>
```

New:

```svelte
<Button variant="secondary" disabled={loading} onclick={handleBack}>Back</Button>
```

g) Replace Done/Execute review-next button (lines 497-499):

Old:

```svelte
<button class="btn preset-filled-primary-500" onclick={handleReviewNext} disabled={loading}>
  {isLastStep ? 'Done' : 'Execute'}
</button>
```

New:

```svelte
<Button variant="primary" {loading} onclick={handleReviewNext}>
  {isLastStep ? 'Done' : 'Execute'}
</Button>
```

h) Replace Run/Continue form-submit button (lines 501-503):

Old:

```svelte
<button class="btn preset-filled-primary-500" type="submit" form={WORKFLOW_FORM_ID} disabled={loading}>
  {isLastStep ? 'Run' : 'Continue'}
</button>
```

New:

```svelte
<Button variant="primary" type="submit" form={WORKFLOW_FORM_ID} {loading}>
  {isLastStep ? 'Run' : 'Continue'}
</Button>
```

i) Replace Run/Continue step-submit button (lines 505-507):

Old:

```svelte
<button class="btn preset-filled-primary-500" onclick={() => void handleStepSubmit({})} disabled={loading}>
  {isLastStep ? 'Run' : 'Continue'}
</button>
```

New:

```svelte
<Button variant="primary" {loading} onclick={() => void handleStepSubmit({})}>
  {isLastStep ? 'Run' : 'Continue'}
</Button>
```

j) Replace Done/Continue no-submit button (lines 509-511):

Old:

```svelte
<button class="btn preset-filled-primary-500" onclick={handleReviewNext} disabled={loading}>
  {isLastStep ? 'Done' : 'Continue'}
</button>
```

New:

```svelte
<Button variant="primary" {loading} onclick={handleReviewNext}>
  {isLastStep ? 'Done' : 'Continue'}
</Button>
```

k) Add `confirmVariant` to `<ConfirmDialog>` (line ~518):

Old:

```svelte
{#if showConfirm && interaction.confirmation}
  <ConfirmDialog
    title={interaction.confirmation.title}
    messagePrefix={interaction.confirmation.message}
    entityName={actionLabel}
    {confirmLabel}
    onconfirm={() => {
```

New:

```svelte
{#if showConfirm && interaction.confirmation}
  <ConfirmDialog
    title={interaction.confirmation.title}
    messagePrefix={interaction.confirmation.message}
    entityName={actionLabel}
    {confirmLabel}
    confirmVariant={confirmVariantForSeverity}
    onconfirm={() => {
```

- [ ] **Step 4: Run tests to confirm they pass**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
npx vitest run src/lib/components/surfaces/SurfaceWorkflow.test.ts
```

Expected: all tests pass.

- [ ] **Step 5: Format and type-check**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
npx prettier --write src/lib/components/surfaces/SurfaceWorkflow.svelte
npx svelte-check --tsconfig ./tsconfig.json 2>&1 | grep -E "error|Error"
```

Expected: no errors.

- [ ] **Step 6: Commit**

```bash
cd /Users/andreyyantsen/Development/uptrakit
git add frontend/src/lib/components/surfaces/SurfaceWorkflow.svelte \
        frontend/src/lib/components/surfaces/SurfaceWorkflow.test.ts
git commit -m "feat(frontend): migrate SurfaceWorkflow to Button primitive (#4)"
```

---

## Task 3: SurfaceForm — fallback submit button migration

**Files:**

- Modify: `frontend/src/lib/components/surfaces/SurfaceForm.svelte`
- Modify: `frontend/src/lib/components/surfaces/SurfaceForm.test.ts`

### Background

`SurfaceForm` has two render branches. The primary path (`schemaFields.length > 0`) renders `<SchemaForm>` which owns
its own submit button (migrated in Task 4). The fallback path (`{:else}`) at line 138 renders a raw JSON-payload form
with its own `<button>`. Only the fallback button is in scope here. The `<ConfirmDialog>` at line ~145 intentionally
keeps no `confirmVariant` — it inherits the `'danger'` default from #3k.

- [ ] **Step 1: Write the failing test**

Add to `SurfaceForm.test.ts` inside the existing `describe` block:

```typescript
it('fallback submit renders Button primitive with primary variant and loading wired', async () => {
  // Interaction with NO form_ui fields — triggers the raw-payload fallback branch
  const interaction: InteractionDescriptor = {
    interaction_id: 'raw-submit',
    kind: 'form_submit',
    label: 'Raw Submit',
    transport: { mode: 'controller_local' },
    form_ui: { fields: [] }
  };

  render(SurfaceForm, {
    surfaceId: 'test.surface',
    interaction
  });

  const btn = screen.getByRole('button', { name: 'Submit' });
  // Button primitive h-[23px] for size=md
  expect(btn.className).toContain('h-[23px]');
  expect(btn.className).not.toMatch(/preset-filled|preset-tonal/);
});

it('fallback submit preserves effectiveSubmitLabel during loading (no text-swap)', async () => {
  vi.mocked(invokeSurfaceInteraction).mockImplementation(() => new Promise(() => {}));
  const interaction: InteractionDescriptor = {
    interaction_id: 'raw-submit',
    kind: 'form_submit',
    label: 'Raw Submit',
    transport: { mode: 'controller_local' },
    form_ui: { fields: [] }
  };

  render(SurfaceForm, {
    surfaceId: 'test.surface',
    interaction,
    submitLabel: 'Deploy'
  });

  const form = screen.getByRole('button', { name: 'Deploy' }).closest('form')!;
  await fireEvent.submit(form);

  await waitFor(() => {
    expect(screen.getByRole('button', { name: 'Deploy' })).toHaveAttribute('aria-busy', 'true');
  });
  expect(screen.queryByText('Submitting...')).not.toBeInTheDocument();
  expect(screen.getByText('Deploy')).toBeInTheDocument();
});
```

- [ ] **Step 2: Run tests to confirm they fail**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
npx vitest run src/lib/components/surfaces/SurfaceForm.test.ts
```

Expected: new tests fail (`h-[23px]` assertion fails on `btn` class).

- [ ] **Step 3: Migrate SurfaceForm.svelte**

Open `frontend/src/lib/components/surfaces/SurfaceForm.svelte`.

a) Add `Button` import in `<script>`:

```svelte
import Button from '$lib/components/Button.svelte';
```

b) Replace the fallback submit button at line 138:

Old:

```svelte
<button class="btn preset-filled-primary-500" type="submit" disabled={submitting}>
  {submitting ? 'Submitting...' : effectiveSubmitLabel}
</button>
```

New:

```svelte
<Button variant="primary" type="submit" loading={submitting}>
  {effectiveSubmitLabel}
</Button>
```

- [ ] **Step 4: Run tests to confirm they pass**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
npx vitest run src/lib/components/surfaces/SurfaceForm.test.ts
```

Expected: all tests pass.

- [ ] **Step 5: Format and type-check**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
npx prettier --write src/lib/components/surfaces/SurfaceForm.svelte
npx svelte-check --tsconfig ./tsconfig.json 2>&1 | grep -E "error|Error"
```

Expected: no errors.

- [ ] **Step 6: Commit**

```bash
cd /Users/andreyyantsen/Development/uptrakit
git add frontend/src/lib/components/surfaces/SurfaceForm.svelte \
        frontend/src/lib/components/surfaces/SurfaceForm.test.ts
git commit -m "feat(frontend): migrate SurfaceForm fallback submit to Button primitive (#4)"
```

---

## Task 4: SchemaForm — field dispatch + submit button migration

**Files:**

- Modify: `frontend/src/lib/components/surfaces/SchemaForm.svelte`
- Modify: `frontend/src/lib/components/surfaces/SchemaForm.test.ts`

### Background

Current source renders raw `<input>`, `<textarea>`, `<input type="checkbox">` inside `<FormFieldRow>` wrappers,
and a raw `<button>` submit with a three-branch text-swap. The migration:

- Imports `Button`, `Input`, `Checkbox`, `Textarea` primitives.
- Replaces raw controls with primitives, passing `error={fieldErrors[field.key]}` to `<Input>` and `<Textarea>`.
- `toggle` fields: replace `<input type="checkbox">` with `<Checkbox>`; the `<FormFieldRow>` wrapper already exists so error display is inherited.
- Drops all bespoke `aria-invalid` attributes (the primitives own this).
- Replaces the submit `<button>` with `<Button variant="primary" type="submit" loading={loading || preLoading}>`.
- Adds a module-level `warnedFieldTypes` Set to emit `console.warn` once per unknown `field_type`.

- [ ] **Step 1: Write the failing tests**

Add to `SchemaForm.test.ts` inside the existing `describe` block:

```typescript
it('renders text field as Input primitive (no raw input[type=text])', async () => {
  render(SchemaForm, {
    fields: [{ key: 'name', label: 'Name', field_type: 'text', required: false }] satisfies FormField[],
    onsubmit: vi.fn().mockResolvedValue(undefined)
  });

  const input = screen.getByRole('textbox', { name: /Name/i });
  // Button primitive uses h-8; Input primitive uses w-full rounded-[3px]
  expect(input.className).toContain('rounded-[3px]');
  // No raw Skeleton CSS class
  expect(input.className).not.toContain('input');
});

it('renders password field as Input type=password', () => {
  render(SchemaForm, {
    fields: [{ key: 'pwd', label: 'Password', field_type: 'password', required: false }] satisfies FormField[],
    onsubmit: vi.fn().mockResolvedValue(undefined)
  });

  const input = document.querySelector('input[type=password]') as HTMLInputElement;
  expect(input).not.toBeNull();
  expect(input.className).toContain('rounded-[3px]');
});

it('renders number field as Input type=number', () => {
  render(SchemaForm, {
    fields: [{ key: 'port', label: 'Port', field_type: 'number', required: false }] satisfies FormField[],
    onsubmit: vi.fn().mockResolvedValue(undefined)
  });

  const input = screen.getByRole('spinbutton', { name: /Port/i });
  expect(input.className).toContain('rounded-[3px]');
});

it('renders textarea field as Textarea primitive with rows=3', () => {
  render(SchemaForm, {
    fields: [{ key: 'notes', label: 'Notes', field_type: 'textarea', required: false }] satisfies FormField[],
    onsubmit: vi.fn().mockResolvedValue(undefined)
  });

  const ta = screen.getByRole('textbox', { name: /Notes/i }) as HTMLTextAreaElement;
  expect(ta.tagName).toBe('TEXTAREA');
  expect(ta.rows).toBe(3);
  // Textarea primitive uses rounded-[3px]
  expect(ta.className).toContain('rounded-[3px]');
  // No raw Skeleton CSS class
  expect(ta.className).not.toContain('textarea');
});

it('renders ssh_private_key field as mono Textarea with rows=8', () => {
  render(SchemaForm, {
    fields: [{ key: 'key', label: 'SSH Key', field_type: 'ssh_private_key', required: false }] satisfies FormField[],
    onsubmit: vi.fn().mockResolvedValue(undefined)
  });

  const ta = screen.getByRole('textbox', { name: /SSH Key/i }) as HTMLTextAreaElement;
  expect(ta.tagName).toBe('TEXTAREA');
  expect(ta.rows).toBe(8);
  // mono variant applies font-mono
  expect(ta.className).toContain('font-mono');
});

it('renders toggle field as Checkbox primitive inside FormFieldRow', () => {
  render(SchemaForm, {
    fields: [{ key: 'enabled', label: 'Enabled', field_type: 'toggle', required: false }] satisfies FormField[],
    onsubmit: vi.fn().mockResolvedValue(undefined)
  });

  const cb = screen.getByRole('checkbox', { name: /Enabled/i }) as HTMLInputElement;
  // Checkbox primitive uses accent- and rounded-[2px]
  expect(cb.className).toContain('rounded-[2px]');
  expect(cb.className).not.toContain('checkbox');
});

it('renders hidden field as raw input[type=hidden] (not a primitive)', () => {
  render(SchemaForm, {
    fields: [{ key: 'id', label: 'ID', field_type: 'hidden', required: false }] satisfies FormField[],
    onsubmit: vi.fn().mockResolvedValue(undefined)
  });

  const hidden = document.querySelector('input[type=hidden]') as HTMLInputElement;
  expect(hidden).not.toBeNull();
  expect(hidden.name).toBe('id');
});

it('renders select field as raw <select> (unchanged — not migrated to primitive)', () => {
  render(SchemaForm, {
    fields: [{
      key: 'env',
      label: 'Environment',
      field_type: 'select',
      required: false,
      options: [{ value: 'prod', label: 'Production' }]
    }] satisfies FormField[],
    onsubmit: vi.fn().mockResolvedValue(undefined)
  });

  // select field uses a raw <select> element — regression guard (not migrated to primitive)
  const sel = document.querySelector('select#env') as HTMLSelectElement;
  expect(sel).not.toBeNull();
  expect(sel.className).toContain('select');
});

it('renders multi_select field as CheckboxList (unchanged — not migrated to primitive)', () => {
  render(SchemaForm, {
    fields: [{
      key: 'tags',
      label: 'Tags',
      field_type: 'multi_select',
      required: false,
      options: [{ value: 'a', label: 'A' }, { value: 'b', label: 'B' }]
    }] satisfies FormField[],
    onsubmit: vi.fn().mockResolvedValue(undefined)
  });

  // multi_select uses CheckboxList — regression guard; verifies the branch was not
  // accidentally replaced by a primitive. These tests pass both before and after
  // migration (the branch is intentionally untouched).
  const checkboxes = document.querySelectorAll('input[type=checkbox]');
  expect(checkboxes.length).toBeGreaterThanOrEqual(2);
});

it('falls back to Input type=text for unknown field_type and warns once', () => {
  const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});

  render(SchemaForm, {
    fields: [
      { key: 'x', label: 'X', field_type: 'unexpected_custom' as unknown as FormField['field_type'], required: false }
    ] satisfies FormField[],
    onsubmit: vi.fn().mockResolvedValue(undefined)
  });

  const input = screen.getByRole('textbox', { name: /X/i });
  expect(input.className).toContain('rounded-[3px]');
  expect(warnSpy).toHaveBeenCalledTimes(1);
  expect(warnSpy.mock.calls[0][0]).toContain('unexpected_custom');

  warnSpy.mockRestore();
});

it('passes error to Input primitive and does not use aria-invalid directly', async () => {
  const onsubmit = vi.fn().mockResolvedValue(undefined);
  render(SchemaForm, {
    fields: [{ key: 'name', label: 'Name', field_type: 'text', required: true }] satisfies FormField[],
    onsubmit
  });

  const btn = screen.getByRole('button', { name: 'Submit' });
  await fireEvent.submit(btn.closest('form')!);

  // Error message comes from Input primitive's error-row render
  const errorMsg = screen.getByText('Name is required.');
  expect(errorMsg).toBeInTheDocument();

  const input = screen.getByRole('textbox', { name: /Name/i });
  // The primitive sets aria-invalid; we do NOT assert absence — just that the error message exists
  expect(input).toHaveAttribute('aria-invalid', 'true');
});

it('submit renders Button primitive with loading={loading || preLoading} and no text-swap', async () => {
  const onsubmit = vi.fn().mockImplementation(() => new Promise(() => {}));
  render(SchemaForm, {
    fields: [{ key: 'name', label: 'Name', field_type: 'text', required: false }] satisfies FormField[],
    onsubmit,
    submitLabel: 'Save Config'
  });

  const btn = screen.getByRole('button', { name: 'Save Config' });
  expect(btn.className).toContain('h-[23px]');
  expect(btn.className).not.toMatch(/preset-filled/);

  await fireEvent.submit(btn.closest('form')!);

  await waitFor(() => {
    expect(btn).toHaveAttribute('aria-busy', 'true');
  });
  expect(screen.queryByText('Processing...')).not.toBeInTheDocument();
  expect(screen.queryByText('Loading...')).not.toBeInTheDocument();
  expect(screen.getByText('Save Config')).toBeInTheDocument();
});
```

- [ ] **Step 2: Run tests to confirm they fail**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
npx vitest run src/lib/components/surfaces/SchemaForm.test.ts
```

Expected: new tests fail on class assertions.

- [ ] **Step 3: Migrate SchemaForm.svelte**

Open `frontend/src/lib/components/surfaces/SchemaForm.svelte`.

a) Add imports (after existing imports at top of `<script>`):

```svelte
import Button from '$lib/components/Button.svelte';
import Input from '$lib/components/Input.svelte';
import Checkbox from '$lib/components/Checkbox.svelte';
import Textarea from '$lib/components/Textarea.svelte';
```

b) Add the unknown-type warning set (after existing non-reactive bookkeeping vars, around line 42):

```svelte
const warnedFieldTypes = new Set<string>();
```

c) Replace the `toggle` field block (around line 423-441). Old:

```svelte
{:else if field.field_type === 'toggle'}
  <FormFieldRow
    label={field.label}
    inputId={field.key}
    required={field.required}
    hint={field.help_text}
    error={fieldErrors[field.key]}
  >
    <input
      type="checkbox"
      id={field.key}
      checked={values[field.key] === 'true'}
      onchange={(e) => {
        values[field.key] = String((e.target as HTMLInputElement).checked);
        clearFieldError(field.key);
      }}
      class="checkbox"
      aria-invalid={fieldErrors[field.key] ? 'true' : undefined}
    />
  </FormFieldRow>
```

New:

```svelte
{:else if field.field_type === 'toggle'}
  <FormFieldRow
    label={field.label}
    inputId={field.key}
    required={field.required}
    hint={field.help_text}
    error={fieldErrors[field.key]}
  >
    <Checkbox
      id={field.key}
      checked={values[field.key] === 'true'}
      disabled={loading}
      onchange={(e) => {
        values[field.key] = (e.target as HTMLInputElement).checked ? 'true' : 'false';
        clearFieldError(field.key);
      }}
    />
  </FormFieldRow>
```

d) Replace the `textarea` field block (lines 334-352). Old:

```svelte
{#if field.field_type === 'textarea'}
  <FormFieldRow
    label={field.label}
    inputId={field.key}
    required={field.required}
    hint={field.help_text}
    error={fieldErrors[field.key]}
  >
    <textarea
      id={field.key}
      bind:value={values[field.key]}
      placeholder={field.placeholder}
      required={field.required}
      class="textarea"
      rows="3"
      aria-invalid={fieldErrors[field.key] ? 'true' : undefined}
      oninput={() => clearFieldError(field.key)}
    ></textarea>
  </FormFieldRow>
```

New:

```svelte
{#if field.field_type === 'textarea'}
  <FormFieldRow
    label={field.label}
    inputId={field.key}
    required={field.required}
    hint={field.help_text}
    error={fieldErrors[field.key]}
  >
    <Textarea
      id={field.key}
      bind:value={values[field.key]}
      placeholder={field.placeholder}
      required={field.required}
      rows={3}
      error={fieldErrors[field.key]}
      oninput={() => clearFieldError(field.key)}
    />
  </FormFieldRow>
```

e) Replace the `ssh_private_key` field block (lines 353-373). Old:

```svelte
{:else if field.field_type === 'ssh_private_key'}
  <FormFieldRow
    label={field.label}
    inputId={field.key}
    required={field.required}
    hint={field.help_text}
    error={fieldErrors[field.key]}
  >
    <textarea
      id={field.key}
      bind:value={values[field.key]}
      placeholder={field.placeholder}
      required={field.required}
      class="textarea font-mono text-xs"
      rows="8"
      spellcheck="false"
      autocomplete="off"
      aria-invalid={fieldErrors[field.key] ? 'true' : undefined}
      oninput={() => clearFieldError(field.key)}
    ></textarea>
  </FormFieldRow>
```

New:

```svelte
{:else if field.field_type === 'ssh_private_key'}
  <FormFieldRow
    label={field.label}
    inputId={field.key}
    required={field.required}
    hint={field.help_text}
    error={fieldErrors[field.key]}
  >
    <Textarea
      id={field.key}
      bind:value={values[field.key]}
      placeholder={field.placeholder}
      required={field.required}
      rows={8}
      variant="mono"
      error={fieldErrors[field.key]}
      oninput={() => clearFieldError(field.key)}
    />
  </FormFieldRow>
```

f) Replace the catch-all text/password/number input block (lines 444-461). Old:

```svelte
{:else}
  <FormFieldRow
    label={field.label}
    inputId={field.key}
    required={field.required}
    hint={field.help_text}
    error={fieldErrors[field.key]}
  >
    <input
      id={field.key}
      type={field.field_type === 'password' ? 'password' : field.field_type === 'number' ? 'number' : 'text'}
      bind:value={values[field.key]}
      placeholder={field.placeholder}
      required={field.required}
      class="input"
      aria-invalid={fieldErrors[field.key] ? 'true' : undefined}
      oninput={() => clearFieldError(field.key)}
    />
  </FormFieldRow>
{/if}
```

New:

```svelte
{:else}
  <FormFieldRow
    label={field.label}
    inputId={field.key}
    required={field.required}
    hint={field.help_text}
    error={fieldErrors[field.key]}
  >
    {#if !['text', 'password', 'number', 'select', 'multi_select', 'toggle', 'hidden', 'textarea', 'ssh_private_key'].includes(field.field_type)}
      {() => {
        if (!warnedFieldTypes.has(field.field_type)) {
          warnedFieldTypes.add(field.field_type);
          console.warn(`[SchemaForm] Unknown field_type "${field.field_type}" — rendering as text input`);
        }
      }}
    {/if}
    <Input
      id={field.key}
      type={field.field_type === 'password' ? 'password' : field.field_type === 'number' ? 'number' : 'text'}
      bind:value={values[field.key]}
      placeholder={field.placeholder}
      required={field.required}
      error={fieldErrors[field.key]}
      oninput={() => clearFieldError(field.key)}
    />
  </FormFieldRow>
{/if}
```

> Note: Svelte 5 does not support arbitrary expression side-effects in templates. Emit the warning using a `$derived`
> side-effect instead. Replace the inline warning snippet above with a derived call in the `<script>` section per
> field. The cleanest approach: in `handleSubmit` before coercion, or in a `$effect` that watches `fields`. Simpler:
> add warning inline as a no-op snippet inside the `{:else}` branch using `{#if}` with a side-effect via a helper
> function. Actual correct pattern:

Replace the catch-all block with:

```svelte
{:else}
  <FormFieldRow
    label={field.label}
    inputId={field.key}
    required={field.required}
    hint={field.help_text}
    error={fieldErrors[field.key]}
  >
    <Input
      id={field.key}
      type={warnUnknownFieldType(field.field_type)}
      bind:value={values[field.key]}
      placeholder={field.placeholder}
      required={field.required}
      error={fieldErrors[field.key]}
      oninput={() => clearFieldError(field.key)}
    />
  </FormFieldRow>
{/if}
```

And add this helper function in the `<script>` section (after the `warnedFieldTypes` set):

```typescript
function warnUnknownFieldType(fieldType: string): 'text' | 'password' | 'number' {
  if (fieldType === 'password') return 'password';
  if (fieldType === 'number') return 'number';
  if (!['text', 'select', 'multi_select', 'toggle', 'hidden', 'textarea', 'ssh_private_key'].includes(fieldType)) {
    if (!warnedFieldTypes.has(fieldType)) {
      warnedFieldTypes.add(fieldType);
      console.warn(`[SchemaForm] Unknown field_type "${fieldType}" — rendering as text input`);
    }
  }
  return 'text';
}
```

g) Replace the submit button (lines 468-478). Old:

```svelte
{#if !hideSubmit}
  <button type="submit" class="btn preset-filled-primary-500" disabled={loading || preLoading}>
    {#if preLoading}
      Loading...
    {:else if loading}
      Processing...
    {:else}
      {submitLabel}
    {/if}
  </button>
{/if}
```

New:

```svelte
{#if !hideSubmit}
  <Button type="submit" variant="primary" loading={loading || preLoading}>
    {submitLabel}
  </Button>
{/if}
```

- [ ] **Step 4: Run tests to confirm they pass**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
npx vitest run src/lib/components/surfaces/SchemaForm.test.ts
```

Expected: all tests pass.

- [ ] **Step 5: Format and type-check**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
npx prettier --write src/lib/components/surfaces/SchemaForm.svelte
npx svelte-check --tsconfig ./tsconfig.json 2>&1 | grep -E "error|Error"
```

Expected: no errors.

- [ ] **Step 6: Commit**

```bash
cd /Users/andreyyantsen/Development/uptrakit
git add frontend/src/lib/components/surfaces/SchemaForm.svelte \
        frontend/src/lib/components/surfaces/SchemaForm.test.ts
git commit -m "feat(frontend): migrate SchemaForm field dispatch + submit to primitives (#4)"
```

---

## Task 5: SurfaceRenderer — modal-trigger button

**Files:**

- Modify: `frontend/src/lib/components/surfaces/SurfaceRenderer.svelte`
- Modify: `frontend/src/lib/components/surfaces/SurfaceRenderer.test.ts`

### Background

`SurfaceRenderer` has exactly one button site (line 186): the `modal_trigger` kind renders a
`<button class="btn preset-tonal-surface" data-ui="modal-trigger">`. This migrates to
`<Button variant="secondary" type="button" data-ui="modal-trigger">`.

- [ ] **Step 1: Write the failing test**

Add to `SurfaceRenderer.test.ts` inside the existing `describe` block:

```typescript
it('modal-trigger renders Button with secondary variant and data-ui preserved', async () => {
  const node: SurfaceNode = {
    kind: 'modal_trigger',
    interaction_id: 'open-modal',
    modal_nodes: []
  };

  const interactions: InteractionDescriptor[] = [
    {
      interaction_id: 'open-modal',
      kind: 'mutation_action',
      label: 'Open Modal',
      transport: { mode: 'controller_local' }
    }
  ];

  const { container } = render(SurfaceRenderer, {
    surfaceId: 'test.surface',
    node,
    interactions,
    dataSources: [],
    dataBySource: {}
  });

  const btn = screen.getByRole('button', { name: 'Open Modal' });
  expect(btn).toHaveAttribute('data-ui', 'modal-trigger');
  // secondary variant uses bg-raised token
  expect(btn.className).toContain('bg-[var(--bg-raised)]');
  expect(btn.className).not.toMatch(/preset-tonal|preset-filled/);
  expect(container.querySelectorAll('button[class*="preset-"]')).toHaveLength(0);
});
```

- [ ] **Step 2: Run test to confirm it fails**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
npx vitest run src/lib/components/surfaces/SurfaceRenderer.test.ts
```

Expected: new test fails on class assertion.

- [ ] **Step 3: Migrate SurfaceRenderer.svelte**

Open `frontend/src/lib/components/surfaces/SurfaceRenderer.svelte`.

a) Add `Button` import in `<script>`:

```svelte
import Button from '$lib/components/Button.svelte';
```

b) Replace the modal-trigger button at line 186:

Old:

```svelte
<button class="btn preset-tonal-surface" data-ui="modal-trigger" type="button" onclick={() => (modalOpen = true)}>
  {interactionLabel(interaction)}
</button>
```

New:

```svelte
<Button variant="secondary" type="button" data-ui="modal-trigger" onclick={() => (modalOpen = true)}>
  {interactionLabel(interaction)}
</Button>
```

- [ ] **Step 4: Run tests to confirm they pass**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
npx vitest run src/lib/components/surfaces/SurfaceRenderer.test.ts
```

Expected: all tests pass.

- [ ] **Step 5: Format and type-check**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
npx prettier --write src/lib/components/surfaces/SurfaceRenderer.svelte
npx svelte-check --tsconfig ./tsconfig.json 2>&1 | grep -E "error|Error"
```

Expected: no errors.

- [ ] **Step 6: Commit**

```bash
cd /Users/andreyyantsen/Development/uptrakit
git add frontend/src/lib/components/surfaces/SurfaceRenderer.svelte \
        frontend/src/lib/components/surfaces/SurfaceRenderer.test.ts
git commit -m "feat(frontend): migrate SurfaceRenderer modal-trigger to Button primitive (#4)"
```

---

## Task 6: SurfaceReadPanel — retry buttons migration

**Files:**

- Modify: `frontend/src/lib/components/surfaces/SurfaceReadPanel.svelte`
- Modify: `frontend/src/lib/components/surfaces/SurfaceReadPanel.test.ts`

### Background

`SurfaceReadPanel` has two retry button sites at lines 318-324 and 343-349. Both share identical inline tailwind
classes (custom danger-tinted outline). Both live inside `<Callout tone="danger">`. Neither uses a `preset-*` class —
this is a bespoke one-off that predates the variant contract. Both migrate to
`<Button variant="danger" size="sm" type="button" onclick={retryHydration}>Try again</Button>`.
The surrounding `<Callout>` is unchanged.

Existing tests at lines 579 and 630 already assert `getByRole('button', { name: 'Try again' })` — they continue to
pass after migration. New assertions verify the `danger` variant class.

- [ ] **Step 1: Write the failing tests**

Add to `SurfaceReadPanel.test.ts` inside the existing `describe` block (after line 635):

```typescript
it('retry button in targeted branch renders Button variant=danger size=sm', async () => {
  // Mock a provider so the targeted branch renders past the "No provider connected" gate.
  vi.mocked(getSurfaceProviders).mockReturnValue([
    {
      provider_id: 'provider-1',
      display_label: 'Provider 1',
      availability: 'available',
      service_id: undefined,
      encryption_metadata: undefined
    }
  ]);
  vi.mocked(invokeSurfaceInteraction).mockRejectedValue(new Error('fail'));

  const read: SurfaceReadResponse = {
    descriptor: {
      surface_id: 'surface.one',
      label: 'Targeted',
      priority: 100,
      slot: 'host_detail.tabs',
      scope: 'tenant',
      targeting: 'targeted',
      provider_kind: 'plugin',
      required_capabilities: [],
      root_node: {
        kind: 'key_value',
        data_source_id: 'data.remote'
      }
    },
    interactions: [
      {
        interaction_id: 'get-info',
        kind: 'data_load',
        label: 'Get Info',
        transport: { mode: 'controller_local' }
      }
    ],
    data_sources: [
      {
        data_source_id: 'data.remote',
        kind: { kind: 'provider_query', operation_id: 'get-info' },
        result_schema: 'object',
        refresh_policy: { type: 'manual' }
      }
    ]
  };

  render(SurfaceReadPanel, {
    surface: makeSurface(),
    read,
    baseParams: { host_id: 'host-001' },
    reloadToken: 0
  });

  const btn = await screen.findByRole('button', { name: 'Try again' });
  // Button primitive's danger variant uses color-error tokens
  expect(btn.className).toContain('color-error');
  // size=sm → h-[19px]
  expect(btn.className).toContain('h-[19px]');
  // No bespoke inline class
  expect(btn.className).not.toContain('border-[var(--color-error-border)]');
});

it('retry button in universal branch renders Button variant=danger size=sm', async () => {
  vi.mocked(invokeSurfaceInteraction).mockRejectedValue(new Error('fail'));

  const read: SurfaceReadResponse = {
    descriptor: {
      surface_id: 'surface.one',
      label: 'Universal',
      priority: 100,
      slot: 'host_detail.tabs',
      scope: 'tenant',
      targeting: 'universal',
      provider_kind: 'plugin',
      required_capabilities: [],
      root_node: {
        kind: 'key_value',
        data_source_id: 'data.remote'
      }
    },
    interactions: [
      {
        interaction_id: 'get-info',
        kind: 'data_load',
        label: 'Get Info',
        transport: { mode: 'controller_local' }
      }
    ],
    data_sources: [
      {
        data_source_id: 'data.remote',
        kind: { kind: 'provider_query', operation_id: 'get-info' },
        result_schema: 'object',
        refresh_policy: { type: 'manual' }
      }
    ]
  };

  render(SurfaceReadPanel, {
    surface: makeSurface(),
    read,
    baseParams: {},
    reloadToken: 0
  });

  const btn = await screen.findByRole('button', { name: 'Try again' });
  expect(btn.className).toContain('color-error');
  expect(btn.className).toContain('h-[19px]');
  expect(btn.className).not.toContain('border-[var(--color-error-border)]');
});
```

> Note: `makeSurface()` returns `targeting: 'universal'`. It can be used as the `surface` prop for both tests — the
> `read.descriptor.targeting` field controls which code branch renders. The targeted branch additionally requires at
> least one available provider; the test above mocks `getSurfaceProviders` accordingly. `getSurfaceProviders` is
> already imported at the top of the test file.

- [ ] **Step 2: Run tests to confirm they fail**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
npx vitest run src/lib/components/surfaces/SurfaceReadPanel.test.ts
```

Expected: new tests fail on class assertions (bespoke inline classes present).

- [ ] **Step 3: Migrate SurfaceReadPanel.svelte**

Open `frontend/src/lib/components/surfaces/SurfaceReadPanel.svelte`.

a) Add `Button` import in `<script>`:

```svelte
import Button from '$lib/components/Button.svelte';
```

b) Replace the first retry button (lines 318-324, inside `{#if descriptor.targeting === 'targeted'}` branch):

Old:

```svelte
<button
  type="button"
  class="mt-1 inline-flex items-center rounded-md border border-[var(--color-error-border)] px-2 py-1 text-xs font-medium text-[var(--color-error)] transition-colors hover:bg-[var(--color-error-bg)]"
  onclick={retryHydration}
>
  Try again
</button>
```

New:

```svelte
<Button variant="danger" size="sm" type="button" onclick={retryHydration}>Try again</Button>
```

c) Replace the second retry button (lines 343-349, inside the `{:else}` / universal branch):

Old:

```svelte
<button
  type="button"
  class="mt-1 inline-flex items-center rounded-md border border-[var(--color-error-border)] px-2 py-1 text-xs font-medium text-[var(--color-error)] transition-colors hover:bg-[var(--color-error-bg)]"
  onclick={retryHydration}
>
  Try again
</button>
```

New:

```svelte
<Button variant="danger" size="sm" type="button" onclick={retryHydration}>Try again</Button>
```

- [ ] **Step 4: Run tests to confirm they pass**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
npx vitest run src/lib/components/surfaces/SurfaceReadPanel.test.ts
```

Expected: all tests pass including pre-existing retry tests.

- [ ] **Step 5: Format and type-check**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
npx prettier --write src/lib/components/surfaces/SurfaceReadPanel.svelte
npx svelte-check --tsconfig ./tsconfig.json 2>&1 | grep -E "error|Error"
```

Expected: no errors.

- [ ] **Step 6: Commit**

```bash
cd /Users/andreyyantsen/Development/uptrakit
git add frontend/src/lib/components/surfaces/SurfaceReadPanel.svelte \
        frontend/src/lib/components/surfaces/SurfaceReadPanel.test.ts
git commit -m "feat(frontend): migrate SurfaceReadPanel retry buttons to Button primitive (#4)"
```

---

## Task 7: E2E — `/dev/surface-preview` Playwright baseline

**Files:**

- Check/create: `frontend/tests/e2e/surface-preview.spec.ts`

### Background

Spec rollout step 9: "Add `/dev/surface-preview` if absent; re-baseline Playwright snapshots."
This route exists in the frontend dev server for visual QA of surface components. Post-migration,
re-baseline the screenshots to capture the new Button/Input/Checkbox/Textarea primitives.

- [ ] **Step 1: Check if `/dev/surface-preview` route exists**

```bash
ls /Users/andreyyantsen/Development/uptrakit/frontend/src/routes/dev/surface-preview/ 2>/dev/null \
  && echo EXISTS || echo MISSING
```

If MISSING: create a minimal `+page.svelte` that renders one of each primitive inside a
`SurfaceInteractionButton` demo. Coordinate with the dev-routes pattern in the codebase
(`frontend/src/routes/dev/`).

- [ ] **Step 2: Re-baseline Playwright snapshots for surface preview**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
npx playwright test tests/e2e/surface-preview.spec.ts --update-snapshots
```

If `surface-preview.spec.ts` does not exist, create a minimal spec:

```typescript
import { test, expect } from '@playwright/test';

test('surface-preview visual baseline', async ({ page }) => {
  await page.goto('/dev/surface-preview');
  await expect(page).toHaveScreenshot('surface-preview.png');
});
```

- [ ] **Step 3: Commit e2e baseline**

```bash
cd /Users/andreyyantsen/Development/uptrakit
git add frontend/tests/e2e/surface-preview.spec.ts \
        frontend/tests/e2e/surface-preview.png-snapshots/ 2>/dev/null || true
git commit -m "test(e2e): baseline /dev/surface-preview after surface-layer-parity migration"
```

---

## Task 8: Source-scan verification + full gate

**Files:**

- Read-only verify: `SurfaceActionBar.svelte`, `SurfaceTable.svelte`, `SurfaceModal.svelte`, `SurfaceKeyValue.svelte`, `SurfaceSlot.svelte`

### Background

The spec requires confirming that the five explicitly-not-migrated files have no `preset-*` `<button>` sites added
since the spec was written. If any are found, add them to the PR.

- [ ] **Step 1: Grep for undiscovered preset buttons**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
grep -n 'preset-filled\|preset-tonal' \
  src/lib/components/surfaces/SurfaceActionBar.svelte \
  src/lib/components/surfaces/SurfaceTable.svelte \
  src/lib/components/surfaces/SurfaceModal.svelte \
  src/lib/components/surfaces/SurfaceKeyValue.svelte \
  src/lib/components/surfaces/SurfaceSlot.svelte
```

Expected: no output (zero matches). If any matches appear, migrate them using the same pattern as Task 1 before continuing.

- [ ] **Step 2: Grep migrated files for any remaining raw preset-* button classes**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
grep -n 'preset-filled\|preset-tonal' \
  src/lib/components/surfaces/SurfaceInteractionButton.svelte \
  src/lib/components/surfaces/SurfaceWorkflow.svelte \
  src/lib/components/surfaces/SurfaceForm.svelte \
  src/lib/components/surfaces/SchemaForm.svelte \
  src/lib/components/surfaces/SurfaceRenderer.svelte \
  src/lib/components/surfaces/SurfaceReadPanel.svelte
```

Expected: no output.

- [ ] **Step 3: Grep for raw `<textarea` (not inside `<Textarea`) in migrated files**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
grep -n '<textarea' \
  src/lib/components/surfaces/SchemaForm.svelte \
  src/lib/components/surfaces/SurfaceForm.svelte
```

Expected: `SurfaceForm.svelte` should still have one `<textarea` (the raw JSON payload textarea in the fallback form
— that one is intentionally NOT migrated; it is not a primitive-scope control). `SchemaForm.svelte` should have zero
matches.

- [ ] **Step 4: Grep for raw `<input type="checkbox"` in migrated files**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
grep -n '<input type="checkbox"' \
  src/lib/components/surfaces/SchemaForm.svelte
```

Expected: no output (the `toggle` field type now uses `<Checkbox>`). Note: `SurfaceWorkflow.svelte` still has an
`<input type="checkbox">` inside the review-state actions list (line ~429) — that is explicitly out of scope per the
spec (it is a skippable-action checkbox, not a form field primitive; it renders inside a card `<label>` and is not
part of the `SchemaForm` field dispatch).

- [ ] **Step 5: Run the full surfaces test suite**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
npx vitest run src/lib/components/surfaces/
```

Expected: all tests pass with no failures.

- [ ] **Step 6: Run full frontend gate**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
npm run lint && npm run format:check && npm run check && npm run test && npm run build
```

Expected: all steps pass.

- [ ] **Step 7: Commit verification pass**

```bash
cd /Users/andreyyantsen/Development/uptrakit
git add --patch  # review; nothing new to stage if all prior tasks committed cleanly
git status       # confirm clean working tree
```

Expected: clean working tree (all changes committed in Tasks 1-6).

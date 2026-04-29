# Select Primitive & Forms Folder Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task.
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `Select.svelte` form primitive, reorganise form components into
`src/lib/components/forms/`, and migrate all eligible inline `<select>` elements across
the codebase.

**Architecture:** Move five existing form primitives (`Input`, `Textarea`, `Checkbox`,
`CheckboxList`, `FormFieldRow`) into `src/lib/components/forms/` with a barrel `index.ts`.
Implement `Select.svelte` following the same pattern as `Input.svelte`. Migrate all eligible
native `<select>` elements to use `<Select>`. Excluded: selects using optgroup, non-string
bound values, non-full-width sizing, per-row conditionally varying options, or
`ProviderSelector`.

**Tech Stack:** Svelte 5, SvelteKit, Tailwind CSS v4, Vitest, `@testing-library/svelte`

---

## File Map

**Created:**

- `src/lib/components/forms/index.ts` — barrel (all form primitives)
- `src/lib/components/forms/Select.svelte` — new primitive
- `src/lib/components/forms/Select.test.ts` — unit tests

**Moved (git mv, no content change except internal import fix):**

- `src/lib/components/Input.svelte` → `src/lib/components/forms/Input.svelte`
- `src/lib/components/Input.test.ts` → `src/lib/components/forms/Input.test.ts`
- `src/lib/components/Textarea.svelte` → `src/lib/components/forms/Textarea.svelte`
- `src/lib/components/Textarea.test.ts` → `src/lib/components/forms/Textarea.test.ts`
- `src/lib/components/Checkbox.svelte` → `src/lib/components/forms/Checkbox.svelte`
- `src/lib/components/Checkbox.test.ts` → `src/lib/components/forms/Checkbox.test.ts`
- `src/lib/components/CheckboxList.svelte` → `src/lib/components/forms/CheckboxList.svelte`
- `src/lib/components/ui/FormFieldRow.svelte` → `src/lib/components/forms/FormFieldRow.svelte`
- `src/lib/components/ui/FormFieldRow.test.ts` → `src/lib/components/forms/FormFieldRow.test.ts`

**Modified (import updates only):**

- `src/lib/components/forms/CheckboxList.svelte` — fix Checkbox import after move
- `src/lib/components/ui/index.ts` — remove FormFieldRow export
- `src/lib/components/ui/SoftwareGroupList.svelte` — update Checkbox import
- `src/routes/public-entry.test.ts` — update Checkbox import
- `src/lib/components/SoftwareMergeWizard.svelte` — update Input import (direct → barrel)
- `src/lib/components/surfaces/SurfaceForm.svelte` — update Textarea import (direct → barrel)
- `src/lib/components/AddSoftwareModal.svelte` — update Input and Checkbox imports (direct → barrel)
- All route/component files that import any of the moved components (~30 files)

**Excluded from migration (stay native `<select>`):**

- `src/routes/software/+page.svelte` line ~1061: `class="select text-sm w-auto"` — non-full-width sizing
- `src/routes/services/+page.svelte` line ~683: `bind:value={mergeTargetId}` with
  `<option value={null}>` — non-string bound value
- `src/lib/components/AssignToHostModal.svelte` line ~348: `execution_site` for multi-role
  table — has conditional option (`{#if role === 'fetch_releases'}` adds Controller)
- `src/lib/components/EditHostAssignmentModal.svelte` lines ~708 and ~1073: use `<optgroup>`
  (Saved/Inline)
- `src/lib/components/ui/ProviderSelector.svelte`: controlled/uncontrolled + per-option disabled

**Modified (select migration):**

- `src/lib/components/surfaces/SchemaForm.svelte`
- `src/routes/audit-logs/+page.svelte`
- `src/lib/components/AssignToHostModal.svelte`
- `src/lib/components/EditHostAssignmentModal.svelte`
- `src/routes/settings/NotificationRulesSettings.svelte`
- `src/routes/settings/PluginConfigsTab.svelte`
- `src/routes/settings/GlobalSettingsTab.svelte`
- `src/routes/settings/RegistrationSettings.svelte`
- `src/routes/history/+page.svelte`
- `src/routes/hosts/[id]/+page.svelte`
- `src/routes/dev/form-primitive-preview/+page.svelte`

---

### Task 1: Move form components to `forms/` and create barrel

**Files:**

- Create: `src/lib/components/forms/` (via first file write)
- Move: 9 files via `git mv`
- Create: `src/lib/components/forms/index.ts`
- Modify: `src/lib/components/forms/CheckboxList.svelte` (fix Checkbox import)

- [ ] **Step 1: Move files with git mv**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
mkdir -p src/lib/components/forms
git mv src/lib/components/Input.svelte src/lib/components/forms/Input.svelte
git mv src/lib/components/Input.test.ts src/lib/components/forms/Input.test.ts
git mv src/lib/components/Textarea.svelte src/lib/components/forms/Textarea.svelte
git mv src/lib/components/Textarea.test.ts src/lib/components/forms/Textarea.test.ts
git mv src/lib/components/Checkbox.svelte src/lib/components/forms/Checkbox.svelte
git mv src/lib/components/Checkbox.test.ts src/lib/components/forms/Checkbox.test.ts
git mv src/lib/components/CheckboxList.svelte src/lib/components/forms/CheckboxList.svelte
git mv src/lib/components/ui/FormFieldRow.svelte src/lib/components/forms/FormFieldRow.svelte
git mv src/lib/components/ui/FormFieldRow.test.ts src/lib/components/forms/FormFieldRow.test.ts
```

- [ ] **Step 2: Fix CheckboxList internal Checkbox import**

In `src/lib/components/forms/CheckboxList.svelte`, line 2 currently reads:

```ts
import Checkbox from '$lib/components/Checkbox.svelte';
```

Change to:

```ts
import Checkbox from './Checkbox.svelte';
```

- [ ] **Step 3: Create forms/index.ts barrel (without Select — added in Task 6)**

Create `src/lib/components/forms/index.ts`:

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
```

- [ ] **Step 4: Commit**

```bash
git add src/lib/components/forms/
git commit -m "refactor(forms): move form primitives into forms/ subfolder with barrel"
```

---

### Task 2: Update `ui/index.ts` and non-route import sites

**Files:**

- Modify: `src/lib/components/ui/index.ts`
- Modify: `src/lib/components/ui/SoftwareGroupList.svelte`
- Modify: `src/routes/public-entry.test.ts`

- [ ] **Step 1: Remove FormFieldRow from `ui/index.ts`**

In `src/lib/components/ui/index.ts`, remove this line:

```ts
export { default as FormFieldRow } from './FormFieldRow.svelte';
```

- [ ] **Step 2: Fix SoftwareGroupList.svelte Checkbox import**

In `src/lib/components/ui/SoftwareGroupList.svelte`, change:

```ts
import Checkbox from '$lib/components/Checkbox.svelte';
```

to:

```ts
import { Checkbox } from '$lib/components/forms';
```

- [ ] **Step 3: Fix public-entry.test.ts Checkbox import**

In `src/routes/public-entry.test.ts`, change:

```ts
import Checkbox from '$lib/components/Checkbox.svelte';
```

to:

```ts
import { Checkbox } from '$lib/components/forms';
```

- [ ] **Step 4: Commit**

```bash
git add src/lib/components/ui/index.ts \
        src/lib/components/ui/SoftwareGroupList.svelte \
        src/routes/public-entry.test.ts
git commit -m "refactor(forms): update ui/index.ts and non-route import sites"
```

---

### Task 3: Update all route and component import sites

**Files:** ~30 files — use grep to find all, apply two import patterns below.

**Pattern A — direct component imports:**

```ts
// Before (any combination):
import Input from '$lib/components/Input.svelte';
import Textarea from '$lib/components/Textarea.svelte';
import Checkbox from '$lib/components/Checkbox.svelte';
import CheckboxList from '$lib/components/CheckboxList.svelte';
import type { InputType } from '$lib/components/Input.svelte';

// After (barrel import, combine all into one line):
import { Input, Textarea, Checkbox, CheckboxList } from '$lib/components/forms';
import type { InputType } from '$lib/components/forms';
```

**Pattern B — FormFieldRow from ui barrel (split import):**

```ts
// Before:
import { PageShell, SectionCard, FormFieldRow, StatusBadge } from '$lib/components/ui';

// After (remove FormFieldRow from ui import, add separate forms import):
import { PageShell, SectionCard, StatusBadge } from '$lib/components/ui';
import { FormFieldRow } from '$lib/components/forms';
```

- [ ] **Step 1: Find all import sites**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
# Find form component imports
grep -rn 'from.*components/Input\|from.*components/Textarea\|from.*components/Checkbox\|from.*components/CheckboxList' src
# Find FormFieldRow imports (11 files import from $lib/components/ui)
grep -rn 'FormFieldRow' src
```

- [ ] **Step 2: Update all matched files using Patterns A and B**

Apply Pattern A to every file that imports from `$lib/components/Input.svelte`,
`Textarea.svelte`, `Checkbox.svelte`, or `CheckboxList.svelte`. Apply Pattern B to every
file that imports `FormFieldRow` from `$lib/components/ui`.

Note: `dev/form-primitive-preview/+page.svelte` also imports `InputType` from
`'$lib/components/Input.svelte'` — move to `'$lib/components/forms'`.

- [ ] **Step 3: Verify no broken imports**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
npm run check
```

Expected: 0 errors. Fix any reported before continuing.

- [ ] **Step 4: Run existing tests to confirm no regressions**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
npm run test
```

Expected: all tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/
git commit -m "refactor(forms): update all import sites to use forms/ barrel"
```

---

### Task 4: Write failing Select tests

**Files:**

- Create: `src/lib/components/forms/Select.test.ts`

- [ ] **Step 1: Create the test file**

Create `src/lib/components/forms/Select.test.ts`:

```ts
import { describe, expect, it, vi } from 'vitest';
import { render, fireEvent } from '@testing-library/svelte';
import Select from './Select.svelte';

type SelectOption = { value: string; label: string };
type SelectProps = {
	id: string;
	value: string;
	options: SelectOption[];
	name?: string;
	placeholder?: string;
	disabled?: boolean;
	required?: boolean;
	error?: string;
	onchange?: (e: Event) => void;
	onblur?: (e: FocusEvent) => void;
	'aria-describedby'?: string;
	'aria-label'?: string;
	class?: string;
};

const OPTS: SelectOption[] = [
	{ value: 'a', label: 'Option A' },
	{ value: 'b', label: 'Option B' },
];

function baseSelect(extra: Partial<SelectProps> = {}): SelectProps {
	return { id: 'test-select', value: 'a', options: OPTS, ...extra };
}

describe('Select primitive', () => {
	it('renders a <select> element', () => {
		const { container } = render(Select, baseSelect());
		expect(container.querySelector('select')).not.toBeNull();
	});

	it('forwards the id prop to the underlying select', () => {
		const { container } = render(Select, baseSelect({ id: 'my-field' }));
		expect(container.querySelector('select')!.getAttribute('id')).toBe('my-field');
	});

	it('renders option elements from options prop', () => {
		const { container } = render(Select, baseSelect());
		const opts = container.querySelectorAll('option');
		expect(opts.length).toBe(2);
		expect(opts[0].value).toBe('a');
		expect(opts[0].textContent).toBe('Option A');
		expect(opts[1].value).toBe('b');
		expect(opts[1].textContent).toBe('Option B');
	});

	it('renders placeholder as first disabled option when provided', () => {
		const { container } = render(Select, baseSelect({ placeholder: 'Pick one' }));
		const opts = container.querySelectorAll('option');
		expect(opts.length).toBe(3);
		expect(opts[0].value).toBe('');
		expect(opts[0].textContent).toBe('Pick one');
		expect(opts[0].disabled).toBe(true);
	});

	it('does not render placeholder option when placeholder is not provided', () => {
		const { container } = render(Select, baseSelect());
		const opts = container.querySelectorAll('option');
		expect(opts.length).toBe(2);
	});

	it('applies base class tokens', () => {
		const { container } = render(Select, baseSelect());
		const cls = container.querySelector('select')!.className;
		expect(cls).toContain('h-8');
		expect(cls).toContain('w-full');
		expect(cls).toContain('bg-[var(--bg-surface)]');
		expect(cls).toContain('border-[var(--border-default)]');
		expect(cls).toContain('text-sm');
		expect(cls).toContain('text-[var(--text-primary)]');
	});

	it('does not include placeholder: pseudo-class token in class string', () => {
		const { container } = render(Select, baseSelect());
		expect(container.querySelector('select')!.className).not.toContain('placeholder:');
	});

	it('applies focus-visible ring class', () => {
		const { container } = render(Select, baseSelect());
		expect(container.querySelector('select')!.className).toContain(
			'focus-visible:shadow-[0_0_0_3px_rgba(var(--accent-rgb),0.25)]'
		);
	});

	it('applies disabled utility classes in class string', () => {
		const { container } = render(Select, baseSelect());
		const cls = container.querySelector('select')!.className;
		expect(cls).toContain('disabled:opacity-40');
		expect(cls).toContain('disabled:cursor-not-allowed');
	});

	it('applies aria-invalid error classes in class string', () => {
		const { container } = render(Select, baseSelect());
		const cls = container.querySelector('select')!.className;
		expect(cls).toContain('aria-[invalid=true]:border-[var(--color-danger-border)]');
		expect(cls).toContain('aria-[invalid=true]:bg-[var(--color-danger-bg)]');
	});

	it('sets aria-invalid="true" when error prop is non-empty', () => {
		const { container } = render(Select, baseSelect({ error: 'Required' }));
		expect(container.querySelector('select')!.getAttribute('aria-invalid')).toBe('true');
	});

	it('omits aria-invalid attribute when error is undefined', () => {
		const { container } = render(Select, baseSelect());
		expect(container.querySelector('select')!.hasAttribute('aria-invalid')).toBe(false);
	});

	it('omits aria-invalid attribute when error is empty string', () => {
		const { container } = render(Select, baseSelect({ error: '' }));
		expect(container.querySelector('select')!.hasAttribute('aria-invalid')).toBe(false);
	});

	it('sets disabled attribute when disabled prop is true', () => {
		const { container } = render(Select, baseSelect({ disabled: true }));
		expect(container.querySelector('select')!.hasAttribute('disabled')).toBe(true);
	});

	it('sets required attribute when required prop is true', () => {
		const { container } = render(Select, baseSelect({ required: true }));
		expect(container.querySelector('select')!.hasAttribute('required')).toBe(true);
	});

	it('forwards name to the select element', () => {
		const { container } = render(Select, baseSelect({ name: 'sort' }));
		expect(container.querySelector('select')!.getAttribute('name')).toBe('sort');
	});

	it('forwards aria-describedby to the select element', () => {
		const { container } = render(Select, baseSelect({ 'aria-describedby': 'hint-id' }));
		expect(container.querySelector('select')!.getAttribute('aria-describedby')).toBe('hint-id');
	});

	it('omits aria-describedby when not provided', () => {
		const { container } = render(Select, baseSelect());
		expect(container.querySelector('select')!.hasAttribute('aria-describedby')).toBe(false);
	});

	it('fires onchange callback on change event', async () => {
		const onchange = vi.fn();
		const { container } = render(Select, baseSelect({ onchange }));
		await fireEvent.change(container.querySelector('select')!);
		expect(onchange).toHaveBeenCalledTimes(1);
	});

	it('fires onblur callback on blur event', async () => {
		const onblur = vi.fn();
		const { container } = render(Select, baseSelect({ onblur }));
		await fireEvent.blur(container.querySelector('select')!);
		expect(onblur).toHaveBeenCalledTimes(1);
	});

	it('concatenates consumer class after internal classes', () => {
		const { container } = render(Select, baseSelect({ class: 'extra-marker' }));
		const cls = container.querySelector('select')!.className;
		expect(cls).toContain('extra-marker');
		expect(cls).toContain('h-8');
	});
});
```

- [ ] **Step 2: Run tests and verify they fail**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
npx vitest run src/lib/components/forms/Select.test.ts
```

Expected: FAIL — `Cannot find module './Select.svelte'`

---

### Task 5: Implement Select.svelte

**Files:**

- Create: `src/lib/components/forms/Select.svelte`

- [ ] **Step 1: Create Select.svelte**

Create `src/lib/components/forms/Select.svelte`:

```svelte
<script lang="ts" module>
	export type SelectOption = { value: string; label: string };

	export type SelectProps = {
		id: string;
		value: string;
		options: SelectOption[];
		name?: string;
		placeholder?: string;
		disabled?: boolean;
		required?: boolean;
		error?: string;
		onchange?: (e: Event) => void;
		onblur?: (e: FocusEvent) => void;
		'aria-describedby'?: string;
		'aria-label'?: string;
		class?: string;
	};
</script>

<script lang="ts">
	import { getContext } from 'svelte';

	const BASE =
		'h-8 w-full px-[10px] rounded-card ' +
		'bg-[var(--bg-surface)] border border-[var(--border-default)] ' +
		'text-sm text-[var(--text-primary)] ' +
		'focus-visible:outline-none ' +
		'focus-visible:shadow-[0_0_0_3px_rgba(var(--accent-rgb),0.25)] ' +
		'disabled:opacity-40 disabled:cursor-not-allowed ' +
		'aria-[invalid=true]:border-[var(--color-danger-border)] ' +
		'aria-[invalid=true]:bg-[var(--color-danger-bg)] ' +
		'transition-[background,border-color] duration-fast';

	let {
		id,
		value = $bindable(''),
		options,
		name,
		placeholder,
		disabled = false,
		required = false,
		error,
		onchange,
		onblur,
		'aria-describedby': ariaDescribedby,
		'aria-label': ariaLabel,
		class: className = ''
	}: SelectProps = $props();

	const rowCtx = getContext<{ id: string | undefined } | undefined>(
		'form-field-row:aria-describedby'
	);
	const computedClass = $derived([BASE, className].filter(Boolean).join(' '));
	const hasError = $derived(!!error);
	const resolvedDescribedBy = $derived(ariaDescribedby ?? rowCtx?.id);
</script>

<select
	{id}
	bind:value
	{name}
	{disabled}
	{required}
	{onchange}
	{onblur}
	aria-invalid={hasError ? 'true' : undefined}
	aria-describedby={resolvedDescribedBy}
	aria-label={ariaLabel}
	class={computedClass}
>
	{#if placeholder !== undefined}
		<option value="" disabled>{placeholder}</option>
	{/if}
	{#each options as opt (opt.value)}
		<option value={opt.value}>{opt.label}</option>
	{/each}
</select>
```

- [ ] **Step 2: Run Select tests and verify they pass**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
npx vitest run src/lib/components/forms/Select.test.ts
```

Expected: all 21 tests PASS.

- [ ] **Step 3: Commit**

```bash
git add src/lib/components/forms/Select.svelte \
        src/lib/components/forms/Select.test.ts
git commit -m "feat(forms): add Select primitive component"
```

---

### Task 6: Add Select to forms/index.ts barrel

**Files:**

- Modify: `src/lib/components/forms/index.ts`

- [ ] **Step 1: Add Select exports to barrel**

Append to the end of `src/lib/components/forms/index.ts`:

```ts
export { default as Select } from './Select.svelte';
export type { SelectProps, SelectOption } from './Select.svelte';
```

- [ ] **Step 2: Run full test suite**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
npm run check && npm run test
```

Expected: all checks and tests pass.

- [ ] **Step 3: Commit**

```bash
git add src/lib/components/forms/index.ts
git commit -m "feat(forms): export Select from forms barrel"
```

---

### Task 7: Migrate SchemaForm.svelte

**Files:**

- Modify: `src/lib/components/surfaces/SchemaForm.svelte`

- [ ] **Step 1: Add Select import**

In `src/lib/components/surfaces/SchemaForm.svelte`, find the existing import of form
components (e.g., `import { Input, ... } from '$lib/components/forms'` — already updated
in Task 3). Add `Select` to the import:

```ts
import { Input, Textarea, CheckboxList, FormFieldRow, Select } from '$lib/components/forms';
```

- [ ] **Step 2: Replace the inline `<select>` block**

Find (lines ~410–422):

```svelte
<select
  id={field.key}
  bind:value={values[field.key]}
  required={field.required}
  class="select"
  aria-invalid={fieldErrors[field.key] ? 'true' : undefined}
  onchange={() => clearFieldError(field.key)}
>
  <option value="">Select...</option>
  {#each resolvedOptions(field) as opt (opt.value)}
    <option value={opt.value}>{opt.label}</option>
  {/each}
</select>
```

Replace with:

```svelte
<Select
  id={field.key}
  bind:value={values[field.key]}
  options={resolvedOptions(field)}
  placeholder="Select..."
  required={field.required}
  error={fieldErrors[field.key]}
  onchange={() => clearFieldError(field.key)}
/>
```

- [ ] **Step 3: Verify type check passes**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
npm run check
```

Expected: 0 errors.

- [ ] **Step 4: Commit**

```bash
git add src/lib/components/surfaces/SchemaForm.svelte
git commit -m "feat(forms): migrate SchemaForm select to Select primitive"
```

---

### Task 8: Migrate audit-logs/+page.svelte

**Files:**

- Modify: `src/routes/audit-logs/+page.svelte`

- [ ] **Step 1: Add Select import**

In `src/routes/audit-logs/+page.svelte`, add `Select` to the existing forms import:

```ts
import { Input, Select } from '$lib/components/forms';
```

- [ ] **Step 2: Replace filter-outcome select**

Find (lines ~228–233):

```svelte
<select id="filter-outcome" class="select" bind:value={filterOutcome}>
  <option value="">All</option>
  {#each OUTCOME_TYPES as outcome (outcome)}
    <option value={outcome}>{outcomeLabel(outcome)}</option>
  {/each}
</select>
```

Replace with:

```svelte
<Select
  id="filter-outcome"
  bind:value={filterOutcome}
  options={[
    { value: '', label: 'All' },
    ...OUTCOME_TYPES.map((o) => ({ value: o, label: outcomeLabel(o) }))
  ]}
/>
```

- [ ] **Step 3: Replace filter-actor-type select**

Find (lines ~240–245):

```svelte
<select id="filter-actor-type" class="select" bind:value={filterActorType}>
  <option value="">All</option>
  {#each ACTOR_TYPES as t (t)}
    <option value={t}>{t}</option>
  {/each}
</select>
```

Replace with:

```svelte
<Select
  id="filter-actor-type"
  bind:value={filterActorType}
  options={[
    { value: '', label: 'All' },
    ...ACTOR_TYPES.map((t) => ({ value: t, label: t }))
  ]}
/>
```

- [ ] **Step 4: Verify and commit**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
npm run check
git add src/routes/audit-logs/+page.svelte
git commit -m "feat(forms): migrate audit-logs filter selects to Select primitive"
```

---

### Task 9: Migrate AssignToHostModal + EditHostAssignmentModal

**Files:**

- Modify: `src/lib/components/AssignToHostModal.svelte`
- Modify: `src/lib/components/EditHostAssignmentModal.svelte`

**Exclusions (leave as native `<select>`):**

- `AssignToHostModal` line ~348: `execution_site` for `fetch_releases`/multi-role — has
  `{#if role === 'fetch_releases'}` conditional option
- `EditHostAssignmentModal` lines ~708 and ~1073: both have `<optgroup>` (Saved/Inline)

#### AssignToHostModal.svelte

- [ ] **Step 1: Add Select import**

Add `Select` to existing forms import in `AssignToHostModal.svelte`:

```ts
import { Input, Checkbox, Select } from '$lib/components/forms';
```

- [ ] **Step 2: Migrate line ~318 — plugin_config_id for multi-role table**

Find:

```svelte
<select
  class="select text-sm"
  bind:value={standardAssignments[role].plugin_config_id}
  disabled={!a.enabled}
  aria-invalid={standardRoleErrors[role] ? 'true' : 'false'}
  onchange={() => {
    clearStandardRoleError(role);
    assignmentError = null;
  }}
>
  <option value="">— none —</option>
  {#each standardConfigsForRole(role) as cfg (cfg.id)}
    <option value={cfg.id}>{cfg.name}</option>
  {/each}
</select>
```

Replace with:

```svelte
<Select
  id="assign-role-{role}-plugin-config"
  class="text-sm"
  bind:value={standardAssignments[role].plugin_config_id}
  options={standardConfigsForRole(role).map((cfg) => ({ value: cfg.id, label: cfg.name }))}
  placeholder="— none —"
  disabled={!a.enabled}
  error={standardRoleErrors[role] || undefined}
  onchange={() => {
    clearStandardRoleError(role);
    assignmentError = null;
  }}
/>
```

- [ ] **Step 3: Migrate line ~382 — pre_update_hook plugin_config_id (flex-1)**

Find:

```svelte
<select
  class="select text-sm flex-1"
  bind:value={entry.plugin_config_id}
  aria-invalid={hookEntryErrors[entry.localKey] ? 'true' : 'false'}
  onchange={() => {
    clearHookEntryError(entry.localKey);
    assignmentError = null;
  }}
>
  <option value="">— select plugin —</option>
  {#each hookConfigs as cfg (cfg.id)}
    <option value={cfg.id}>{cfg.name}</option>
  {/each}
</select>
```

Replace with:

```svelte
<Select
  id="pre-hook-{entry.localKey}-plugin-config"
  class="text-sm flex-1"
  bind:value={entry.plugin_config_id}
  options={hookConfigs.map((cfg) => ({ value: cfg.id, label: cfg.name }))}
  placeholder="— select plugin —"
  error={hookEntryErrors[entry.localKey] || undefined}
  onchange={() => {
    clearHookEntryError(entry.localKey);
    assignmentError = null;
  }}
/>
```

- [ ] **Step 4: Migrate line ~446 — execute_update plugin_config_id**

Find (the execute_update plugin_config_id select — same structure as step 2 but for
execute_update row):

```svelte
<select
  class="select text-sm"
  bind:value={standardAssignments[role].plugin_config_id}
  disabled={!a.enabled}
  aria-invalid={standardRoleErrors[role] ? 'true' : 'false'}
  onchange={() => {
    clearStandardRoleError(role);
    assignmentError = null;
  }}
>
  <option value="">— none —</option>
  {#each standardConfigsForRole(role) as cfg (cfg.id)}
    <option value={cfg.id}>{cfg.name}</option>
  {/each}
</select>
```

Replace with:

```svelte
<Select
  id="assign-execute-{role}-plugin-config"
  class="text-sm"
  bind:value={standardAssignments[role].plugin_config_id}
  options={standardConfigsForRole(role).map((cfg) => ({ value: cfg.id, label: cfg.name }))}
  placeholder="— none —"
  disabled={!a.enabled}
  error={standardRoleErrors[role] || undefined}
  onchange={() => {
    clearStandardRoleError(role);
    assignmentError = null;
  }}
/>
```

- [ ] **Step 5: Migrate line ~476 — execute_update execution_site (static, no conditional)**

Find:

```svelte
<select
  class="select text-sm"
  bind:value={standardAssignments[role].execution_site}
  disabled={!a.enabled}
>
  <option value="auto">Auto</option>
  <option value="agent">Agent</option>
</select>
```

Replace with:

```svelte
<Select
  id="assign-role-{role}-execution-site"
  class="text-sm"
  bind:value={standardAssignments[role].execution_site}
  options={[
    { value: 'auto', label: 'Auto' },
    { value: 'agent', label: 'Agent' },
  ]}
  disabled={!a.enabled}
/>
```

- [ ] **Step 6: Migrate line ~507 — post_update_hook plugin_config_id (flex-1)**

Find the post_update_hook `<select class="select text-sm flex-1">` (same structure as
pre_update_hook in step 3 but with `hookRole === 'post_update_hook'`). Replace with:

```svelte
<Select
  id="post-hook-{entry.localKey}-plugin-config"
  class="text-sm flex-1"
  bind:value={entry.plugin_config_id}
  options={hookConfigs.map((cfg) => ({ value: cfg.id, label: cfg.name }))}
  placeholder="— select plugin —"
  error={hookEntryErrors[entry.localKey] || undefined}
  onchange={() => {
    clearHookEntryError(entry.localKey);
    assignmentError = null;
  }}
/>
```

#### EditHostAssignmentModal.svelte

- [ ] **Step 7: Add Select import**

Add `Select` to existing forms import in `EditHostAssignmentModal.svelte`:

```ts
import { Input, Textarea, Checkbox, Select } from '$lib/components/forms';
```

- [ ] **Step 8: Migrate line ~752 — execution_site for fetch_releases (flat static options)**

Find:

```svelte
<select id="site-{role}" class="select text-sm" bind:value={standardStates[role].execution_site}>
  <option value="auto">Auto (recommended)</option>
  <option value="agent">Agent</option>
  <option value="controller">Controller</option>
</select>
```

Replace with:

```svelte
<Select
  id="site-{role}"
  class="text-sm"
  bind:value={standardStates[role].execution_site}
  options={[
    { value: 'auto', label: 'Auto (recommended)' },
    { value: 'agent', label: 'Agent' },
    { value: 'controller', label: 'Controller' },
  ]}
/>
```

- [ ] **Step 9: Migrate line ~780 — resolvedOptions select in standard override form**

Find (inside `{:else if field.field_type === 'select'}` branch of standard override form,
`class="select text-xs w-full"`):

```svelte
<select
  id="ovr-{role}-{field.key}"
  bind:value={standardStates[role].overrideFormValues[field.key]}
  class="select text-xs w-full"
>
  <option value=""></option>
  {#each resolvedOptions(field) as opt (opt.value)}
    <option value={opt.value}>{opt.label}</option>
  {/each}
</select>
```

Replace with:

```svelte
<Select
  id="ovr-{role}-{field.key}"
  class="text-xs"
  bind:value={standardStates[role].overrideFormValues[field.key]}
  options={resolvedOptions(field)}
  placeholder=""
/>
```

Note: `placeholder=""` renders an empty disabled first option (matching the original
`<option value=""></option>`) because the component uses `{#if placeholder !== undefined}`,
not `{#if placeholder}`.

- [ ] **Step 10: Migrate line ~918 — resolvedOptions select in JSON-override form**

Find (the second `{:else if field.field_type === 'select'}` branch of standard override
form):

```svelte
<select
  id="ovr-{role}-{field.key}"
  bind:value={standardStates[role].overrideFormValues[field.key]}
  class="select text-xs w-full"
>
  <option value="">— keep base config —</option>
  {#each resolvedOptions(field) as opt (opt.value)}
    <option value={opt.value}>{opt.label}</option>
  {/each}
</select>
```

Replace with:

```svelte
<Select
  id="ovr-{role}-{field.key}"
  class="text-xs"
  bind:value={standardStates[role].overrideFormValues[field.key]}
  options={resolvedOptions(field)}
  placeholder="— keep base config —"
/>
```

- [ ] **Step 11: Migrate line ~1123 — resolvedOptions select in hook inline form**

Find (inside hook inline form `{:else if field.field_type === 'select'}` branch):

```svelte
<select
  id="hook-ovr-{entry.localKey}-{field.key}"
  bind:value={entry.overrideFormValues[field.key]}
  class="select text-xs w-full"
>
  <option value=""></option>
  {#each resolvedOptions(field) as opt (opt.value)}
    <option value={opt.value}>{opt.label}</option>
  {/each}
</select>
```

Replace with:

```svelte
<Select
  id="hook-ovr-{entry.localKey}-{field.key}"
  class="text-xs"
  bind:value={entry.overrideFormValues[field.key]}
  options={resolvedOptions(field)}
  placeholder=""
/>
```

- [ ] **Step 12: Migrate line ~1262 — resolvedOptions select in hook JSON-override form**

Find (the second hook inline form `{:else if field.field_type === 'select'}` branch):

```svelte
<select
  id="hook-ovr-{entry.localKey}-{field.key}"
  bind:value={entry.overrideFormValues[field.key]}
  class="select text-xs w-full"
>
  <option value="">— keep base config —</option>
  {#each resolvedOptions(field) as opt (opt.value)}
    <option value={opt.value}>{opt.label}</option>
  {/each}
</select>
```

Replace with:

```svelte
<Select
  id="hook-ovr-{entry.localKey}-{field.key}"
  class="text-xs"
  bind:value={entry.overrideFormValues[field.key]}
  options={resolvedOptions(field)}
  placeholder="— keep base config —"
/>
```

- [ ] **Step 13: Verify type check and commit**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
npm run check
git add src/lib/components/AssignToHostModal.svelte \
        src/lib/components/EditHostAssignmentModal.svelte
git commit -m "feat(forms): migrate modal selects to Select primitive"
```

---

### Task 10: Migrate remaining selects

**Files:**

- `src/routes/settings/NotificationRulesSettings.svelte`
- `src/routes/settings/PluginConfigsTab.svelte`
- `src/routes/settings/GlobalSettingsTab.svelte`
- `src/routes/settings/RegistrationSettings.svelte`
- `src/routes/history/+page.svelte`
- `src/routes/hosts/[id]/+page.svelte`

Add `Select` to each file's existing forms import, then replace each native `<select>` as
shown below.

- [ ] **Step 1: Migrate NotificationRulesSettings.svelte — rule-channel-id**

Find:

```svelte
<select id="rule-channel-id" bind:value={form.channel_id} required class="select">
  {#each channels as ch (ch.id)}
    <option value={ch.id}>{ch.name} ({ch.channel_type})</option>
  {/each}
</select>
```

Replace with:

```svelte
<Select
  id="rule-channel-id"
  bind:value={form.channel_id}
  options={channels.map((ch) => ({ value: ch.id, label: `${ch.name} (${ch.channel_type})` }))}
  required
/>
```

- [ ] **Step 2: Migrate NotificationRulesSettings.svelte — rule-event-type**

Find:

```svelte
<select id="rule-event-type" bind:value={form.event_type} required class="select">
  {#each Object.entries(EVENT_TYPE_LABELS) as [value, label] (value)}
    <option {value}>{label}</option>
  {/each}
</select>
```

Replace with:

```svelte
<Select
  id="rule-event-type"
  bind:value={form.event_type}
  options={Object.entries(EVENT_TYPE_LABELS).map(([value, label]) => ({ value, label }))}
  required
/>
```

- [ ] **Step 3: Migrate PluginConfigsTab.svelte — plugin-config-plugin-type (line ~1156)**

Find:

```svelte
<select
  id="plugin-config-plugin-type"
  class="select"
  bind:value={configForm.plugin_type}
  aria-invalid={configPluginTypeError ? 'true' : undefined}
  onchange={() => {
    configPluginTypeError = '';
    configForm.config = sampleConfigJson(configForm.plugin_type);
    formValues = flattenConfig({}, getFormFields(configForm.plugin_type));
    configFieldErrors = {};
    showJsonEditor = false;
  }}
>
  {#each configurablePluginTypes as t (t.plugin_type)}
    <option value={t.plugin_type}>{t.display_name}</option>
  {/each}
</select>
```

Replace with:

```svelte
<Select
  id="plugin-config-plugin-type"
  bind:value={configForm.plugin_type}
  options={configurablePluginTypes.map((t) => ({ value: t.plugin_type, label: t.display_name }))}
  error={configPluginTypeError || undefined}
  onchange={() => {
    configPluginTypeError = '';
    configForm.config = sampleConfigJson(configForm.plugin_type);
    formValues = flattenConfig({}, getFormFields(configForm.plugin_type));
    configFieldErrors = {};
    showJsonEditor = false;
  }}
/>
```

- [ ] **Step 4: Migrate PluginConfigsTab.svelte — resolvedOptions select in config form (line ~1198)**

Find:

```svelte
<select
  id="cfg-{field.key}"
  bind:value={formValues[field.key]}
  required={field.required}
  aria-invalid={configFieldErrors[field.key] ? 'true' : undefined}
  onchange={() => clearConfigFieldError(field.key)}
  class="select w-full"
>
  <option value="">— select —</option>
  {#each resolvedOptions(field) as opt (opt.value)}
    <option value={opt.value}>{opt.label}</option>
  {/each}
</select>
```

Replace with:

```svelte
<Select
  id="cfg-{field.key}"
  bind:value={formValues[field.key]}
  options={resolvedOptions(field)}
  placeholder="— select —"
  required={field.required}
  error={configFieldErrors[field.key] || undefined}
  onchange={() => clearConfigFieldError(field.key)}
/>
```

- [ ] **Step 5: Migrate PluginConfigsTab.svelte — allowlist-plugin-type (line ~1369)**

Find:

```svelte
<select
  id="allowlist-plugin-type"
  class="select"
  bind:value={allowlistForm.plugin_type}
  aria-invalid={allowlistPluginTypeError ? 'true' : undefined}
  onchange={() => {
    allowlistPluginTypeError = '';
  }}
>
  <option value="">— select —</option>
  {#each discoveryPluginTypes as t (t.plugin_type)}
    <option value={t.plugin_type}>{t.display_name}</option>
  {/each}
</select>
```

Replace with:

```svelte
<Select
  id="allowlist-plugin-type"
  bind:value={allowlistForm.plugin_type}
  options={discoveryPluginTypes.map((t) => ({ value: t.plugin_type, label: t.display_name }))}
  placeholder="— select —"
  error={allowlistPluginTypeError || undefined}
  onchange={() => {
    allowlistPluginTypeError = '';
  }}
/>
```

- [ ] **Step 6: Migrate PluginConfigsTab.svelte — resolvedOptions select in type settings form (line ~1432)**

Find:

```svelte
<select
  id="ts-{field.key}"
  bind:value={typeSettingsFormValues[field.key]}
  required={field.required}
  aria-invalid={typeSettingsFieldErrors[field.key] ? 'true' : undefined}
  onchange={() => clearTypeSettingsFieldError(field.key)}
  class="select w-full"
>
  <option value="">— select —</option>
  {#each resolvedOptions(field) as opt (opt.value)}
    <option value={opt.value}>{opt.label}</option>
  {/each}
</select>
```

Replace with:

```svelte
<Select
  id="ts-{field.key}"
  bind:value={typeSettingsFormValues[field.key]}
  options={resolvedOptions(field)}
  placeholder="— select —"
  required={field.required}
  error={typeSettingsFieldErrors[field.key] || undefined}
  onchange={() => clearTypeSettingsFieldError(field.key)}
/>
```

- [ ] **Step 7: Migrate GlobalSettingsTab.svelte — global-real-ip-header**

Find:

```svelte
<select id="global-real-ip-header" class="select" bind:value={realIpHeader}>
  <option value="X-Forwarded-For">X-Forwarded-For</option>
  <option value="Forwarded">Forwarded (RFC 7239)</option>
  <option value="X-Real-Ip">X-Real-Ip</option>
  <option value="CF-Connecting-IP">CF-Connecting-IP</option>
  <option value="True-Client-IP">True-Client-IP</option>
</select>
```

Replace with:

```svelte
<Select
  id="global-real-ip-header"
  bind:value={realIpHeader}
  options={[
    { value: 'X-Forwarded-For', label: 'X-Forwarded-For' },
    { value: 'Forwarded', label: 'Forwarded (RFC 7239)' },
    { value: 'X-Real-Ip', label: 'X-Real-Ip' },
    { value: 'CF-Connecting-IP', label: 'CF-Connecting-IP' },
    { value: 'True-Client-IP', label: 'True-Client-IP' },
  ]}
/>
```

- [ ] **Step 8: Migrate RegistrationSettings.svelte — registration-mode**

Find:

```svelte
<select id="registration-mode" class="select" bind:value={regMode}>
  <option value="open">Open</option>
  <option value="invite">Invite Only</option>
  <option value="closed">Closed</option>
</select>
```

Replace with:

```svelte
<Select
  id="registration-mode"
  bind:value={regMode}
  options={[
    { value: 'open', label: 'Open' },
    { value: 'invite', label: 'Invite Only' },
    { value: 'closed', label: 'Closed' },
  ]}
/>
```

- [ ] **Step 9: Migrate history/+page.svelte — trigger-software-item**

Find:

```svelte
<select id="trigger-software-item" class="select" bind:value={selectedItemId}>
  <option value="">— select —</option>
  {#each softwareItems as si (si.id)}
    <option value={si.id}>{si.name}</option>
  {/each}
</select>
```

Replace with:

```svelte
<Select
  id="trigger-software-item"
  bind:value={selectedItemId}
  options={softwareItems.map((si) => ({ value: si.id, label: si.name }))}
  placeholder="— select —"
/>
```

- [ ] **Step 10: Migrate history/+page.svelte — trigger-host**

Find:

```svelte
<select id="trigger-host" class="select" bind:value={selectedHostId}>
  <option value="">— select —</option>
  {#each selectedItemHosts as host (host.host_id)}
    <option value={host.host_id}>{host.label}</option>
  {/each}
</select>
```

Replace with:

```svelte
<Select
  id="trigger-host"
  bind:value={selectedHostId}
  options={selectedItemHosts.map((host) => ({ value: host.host_id, label: host.label }))}
  placeholder="— select —"
/>
```

- [ ] **Step 11: Migrate hosts/[id]/+page.svelte — allowlist-plugin-type**

Find:

```svelte
<select id="allowlist-plugin-type" class="select" bind:value={allowlistForm.plugin_type}>
  {#each discoveryPluginTypes as t (t.plugin_type)}
    <option value={t.plugin_type}>{t.display_name}</option>
  {/each}
</select>
```

Replace with:

```svelte
<Select
  id="allowlist-plugin-type"
  bind:value={allowlistForm.plugin_type}
  options={discoveryPluginTypes.map((t) => ({ value: t.plugin_type, label: t.display_name }))}
/>
```

- [ ] **Step 12: Verify type check and commit**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
npm run check
git add src/routes/settings/NotificationRulesSettings.svelte \
        src/routes/settings/PluginConfigsTab.svelte \
        src/routes/settings/GlobalSettingsTab.svelte \
        src/routes/settings/RegistrationSettings.svelte \
        src/routes/history/+page.svelte \
        src/routes/hosts/\[id\]/+page.svelte
git commit -m "feat(forms): migrate settings and route selects to Select primitive"
```

---

### Task 11: Update dev/form-primitive-preview and final verification

**Files:**

- Modify: `src/routes/dev/form-primitive-preview/+page.svelte`

- [ ] **Step 1: Add Select import and demo state**

In `src/routes/dev/form-primitive-preview/+page.svelte`, add `Select` to the existing
forms import:

```ts
import { Input, Checkbox, Textarea, Select } from '$lib/components/forms';
```

Add demo state variables in the `<script>` block (near existing state vars):

```ts
let selectVal = $state('b');
let selectValEmpty = $state('');
```

- [ ] **Step 2: Add Select demo section to template**

After the existing Textarea section, add:

```svelte
<section class="space-y-4 p-6">
  <h2 class="mb-3 text-sm font-bold uppercase tracking-wide">Select — states</h2>
  <div class="grid grid-cols-2 gap-4">
    <div>
      <p class="mb-1 text-xs text-[var(--text-muted)]">Default</p>
      <Select
        id="preview-select-default"
        bind:value={selectVal}
        options={[
          { value: 'a', label: 'Option A' },
          { value: 'b', label: 'Option B' },
          { value: 'c', label: 'Option C' },
        ]}
      />
    </div>
    <div>
      <p class="mb-1 text-xs text-[var(--text-muted)]">With placeholder</p>
      <Select
        id="preview-select-placeholder"
        bind:value={selectValEmpty}
        options={[
          { value: 'x', label: 'Choice X' },
          { value: 'y', label: 'Choice Y' },
        ]}
        placeholder="Select an option"
      />
    </div>
    <div>
      <p class="mb-1 text-xs text-[var(--text-muted)]">Error</p>
      <Select
        id="preview-select-error"
        bind:value={selectVal}
        options={[
          { value: 'a', label: 'Option A' },
          { value: 'b', label: 'Option B' },
        ]}
        error="This field is required"
      />
    </div>
    <div>
      <p class="mb-1 text-xs text-[var(--text-muted)]">Disabled</p>
      <Select
        id="preview-select-disabled"
        value="a"
        options={[
          { value: 'a', label: 'Option A' },
          { value: 'b', label: 'Option B' },
        ]}
        disabled
      />
    </div>
  </div>
</section>
```

- [ ] **Step 3: Run full quality gate**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend
npm run lint && npm run format:check && npm run check && npm run test && npm run build
```

Expected: all checks pass, build succeeds.

- [ ] **Step 4: Final commit**

```bash
git add src/routes/dev/form-primitive-preview/+page.svelte
git commit -m "feat(forms): add Select demo to form-primitive-preview"
```

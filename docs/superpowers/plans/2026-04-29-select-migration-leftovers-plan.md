# Select Migration — Leftover `<select>` Elements Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement
this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Migrate the six remaining raw `<select>` elements in `frontend/src/` to the `Select` form primitive, extending the primitive with
`<optgroup>` support, per-option `disabled`, a `width: 'full' | 'auto'` variant, and a `data-ui="select"` marker. Clean dead code in
`ProviderSelector.svelte` along the way and rewrite the stale regression-guard in `SchemaForm.test.ts`.

**Architecture:** Land the primitive extension as commit 1 (every site below depends on the new API). Migrate sites one-per-commit so each diff stays
reviewable. Sites 4+5 share a helper and land in one commit. ProviderSelector cleanup and its `SurfaceReadPanel` caller updates land together because
the new required `id` prop forces both to move at once.

**Tech Stack:** Svelte 5 (runes: `$state`, `$derived`, `$bindable`, `$props`), TypeScript, Vitest + `@testing-library/svelte`, Tailwind CSS (JIT),
pnpm workspace under `frontend/`.

**Spec:** `docs/superpowers/specs/2026-04-29-select-migration-leftovers-design.md`

---

## File Structure

**Created:**

- *(none — plan only modifies existing files)*

**Modified:**

- `frontend/src/lib/components/forms/Select.svelte` — primitive extension (types, width prop, optgroup render, data-ui marker)
- `frontend/src/lib/components/forms/Select.test.ts` — 7 new test cases
- `frontend/src/lib/components/forms/index.ts` — barrel re-exports `SelectGroup`, `SelectItem`
- `frontend/src/routes/dev/form-primitive-preview/+page.svelte` — grouped + disabled-option demo
- `frontend/src/lib/components/surfaces/SchemaForm.test.ts` — rewrite stale regression-guard
- `frontend/src/routes/services/+page.svelte` — Site 1 migration
- `frontend/src/routes/software/+page.svelte` — Site 2 migration
- `frontend/src/lib/components/AssignToHostModal.svelte` — Site 3 (both tables) + helper
- `frontend/src/lib/components/EditHostAssignmentModal.svelte` — Sites 4+5 + helper
- `frontend/src/lib/components/ui/ProviderSelector.svelte` — Site 6 dead-code removal + migration
- `frontend/src/lib/components/ui/ProviderSelector.test.ts` — drop revert-hack test, add `id` prop to renders
- `frontend/src/lib/components/surfaces/SurfaceReadPanel.svelte` — pass `id` to both ProviderSelector callers

---

## Task 1: Extend Select primitive

**Files:**

- Modify: `frontend/src/lib/components/forms/Select.svelte`
- Modify: `frontend/src/lib/components/forms/Select.test.ts`
- Modify: `frontend/src/lib/components/forms/index.ts`
- Modify: `frontend/src/routes/dev/form-primitive-preview/+page.svelte`

This task adds: (a) `SelectGroup` discriminated union with optgroup render, (b) per-option `disabled` flag, (c) `width: 'full' | 'auto'` prop with
default `'full'`, (d) `data-ui="select"` marker on the inner `<select>`, (e) updated barrel re-exports, (f) preview demo entry.

- [ ] **Step 1.1: Write failing test for `<optgroup>` rendering**

Append to `frontend/src/lib/components/forms/Select.test.ts` (do not touch existing tests; they must keep passing):

```ts
it('renders <optgroup> with label and nested options', () => {
    const { container } = render(Select, baseSelect({
        options: [
            { label: 'Group X', options: [
                { value: 'x1', label: 'X One' },
                { value: 'x2', label: 'X Two' }
            ]}
        ]
    }));
    const groups = container.querySelectorAll('optgroup');
    expect(groups.length).toBe(1);
    expect(groups[0].getAttribute('label')).toBe('Group X');
    const opts = groups[0].querySelectorAll('option');
    expect(opts.length).toBe(2);
    expect(opts[0].value).toBe('x1');
    expect(opts[1].textContent).toBe('X Two');
});
```

Update the local `SelectOption` / `SelectProps` types at the top of `Select.test.ts` to match the new public shape (these are inline copies — no
import):

```ts
type SelectOption = { value: string; label: string; disabled?: boolean };
type SelectGroup = { label: string; options: SelectOption[] };
type SelectItem = SelectOption | SelectGroup;
type SelectProps = {
    id: string;
    value: string;
    options: SelectItem[];
    width?: 'full' | 'auto';
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
```

- [ ] **Step 1.2: Run test, verify it fails**

```bash
cd frontend && npm run test -- Select.test.ts
```

Expected: new optgroup test fails (no `<optgroup>` rendered yet); existing tests pass.

- [ ] **Step 1.3: Write failing test for per-option `disabled`**

Append to `Select.test.ts`:

```ts
it('renders disabled option with disabled attribute', () => {
    const { container } = render(Select, baseSelect({
        options: [
            { value: 'a', label: 'A' },
            { value: 'b', label: 'B', disabled: true }
        ]
    }));
    const opts = container.querySelectorAll('option');
    expect(opts[0].disabled).toBe(false);
    expect(opts[1].disabled).toBe(true);
});
```

- [ ] **Step 1.4: Write failing test for mixed flat + grouped rendering**

Append:

```ts
it('renders mixed flat and grouped options', () => {
    const { container } = render(Select, baseSelect({
        options: [
            { value: '', label: 'Placeholder-ish' },
            { label: 'Group A', options: [{ value: 'a1', label: 'A1' }] },
            { value: 'flat', label: 'Flat option' }
        ]
    }));
    const direct = container.querySelectorAll('select > option');
    expect(direct.length).toBe(2);
    expect(direct[0].textContent).toBe('Placeholder-ish');
    expect(direct[1].textContent).toBe('Flat option');
    expect(container.querySelectorAll('optgroup').length).toBe(1);
});
```

- [ ] **Step 1.5: Write failing test for placeholder + groups coexistence**

Append:

```ts
it('renders placeholder before optgroups when both are provided', () => {
    const { container } = render(Select, baseSelect({
        placeholder: 'Pick',
        options: [
            { label: 'G', options: [{ value: 'g1', label: 'G1' }] }
        ]
    }));
    const select = container.querySelector('select')!;
    const firstChild = select.children[0] as HTMLOptionElement;
    expect(firstChild.tagName).toBe('OPTION');
    expect(firstChild.value).toBe('');
    expect(firstChild.disabled).toBe(true);
    expect(select.children[1].tagName).toBe('OPTGROUP');
});
```

- [ ] **Step 1.6: Write failing test for `width="auto"` and default `width="full"`**

Append:

```ts
it('applies w-full by default', () => {
    const { container } = render(Select, baseSelect());
    expect(container.querySelector('select')!.className).toContain('w-full');
});

it('applies w-auto when width="auto"', () => {
    const { container } = render(Select, baseSelect({ width: 'auto' }));
    const cls = container.querySelector('select')!.className;
    expect(cls).toContain('w-auto');
    expect(cls).not.toContain('w-full');
});
```

(The pre-existing `'applies base class tokens'` test asserts `w-full` is present — that test still applies under default width and stays valid.)

- [ ] **Step 1.7: Write failing test for `data-ui="select"`**

Append:

```ts
it('sets data-ui="select" on the inner select element', () => {
    const { container } = render(Select, baseSelect());
    expect(container.querySelector('select')!.getAttribute('data-ui')).toBe('select');
});
```

- [ ] **Step 1.8: Write failing test for `bind:value` round-trip through grouped option**

Append:

```ts
it('bind:value round-trips through a grouped option', async () => {
    const onchange = vi.fn();
    const { container } = render(Select, baseSelect({
        value: '',
        placeholder: 'Pick',
        options: [
            { label: 'Saved', options: [
                { value: 'cfg:1', label: 'Production' },
                { value: 'cfg:2', label: 'Staging' }
            ]}
        ],
        onchange
    }));
    const select = container.querySelector('select')! as HTMLSelectElement;
    await fireEvent.change(select, { target: { value: 'cfg:2' } });
    expect(onchange).toHaveBeenCalledTimes(1);
    expect(select.value).toBe('cfg:2');
});
```

- [ ] **Step 1.9: Run all primitive tests, verify the seven new ones fail and existing ones pass**

```bash
cd frontend && npm run test -- Select.test.ts
```

Expected: 7 fails (new tests), all existing tests pass. If any existing test fails, stop and investigate.

- [ ] **Step 1.10: Implement primitive extension**

Replace the entire body of `frontend/src/lib/components/forms/Select.svelte` with:

```svelte
<script lang="ts" module>
	export type SelectOption = { value: string; label: string; disabled?: boolean };
	export type SelectGroup = { label: string; options: SelectOption[] };
	export type SelectItem = SelectOption | SelectGroup;

	export type SelectProps = {
		id: string;
		value?: string;
		options: SelectItem[];
		width?: 'full' | 'auto';
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
		'h-8 py-0 pl-[10px] pr-10 rounded-card ' +
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
		value = $bindable(),
		options,
		width = 'full',
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

	const rowCtx = getContext<{ id: string | undefined } | undefined>('form-field-row:aria-describedby');
	const widthClass = $derived(width === 'auto' ? 'w-auto' : 'w-full');
	const computedClass = $derived([BASE, widthClass, className].filter(Boolean).join(' '));
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
	data-ui="select"
	class={computedClass}
>
	{#if placeholder !== undefined}
		<option value="" disabled>{placeholder}</option>
	{/if}
	{#each options as item}
		{#if 'options' in item}
			<optgroup label={item.label}>
				{#each item.options as opt (opt.value)}
					<option value={opt.value} disabled={opt.disabled}>{opt.label}</option>
				{/each}
			</optgroup>
		{:else}
			<option value={item.value} disabled={item.disabled}>{item.label}</option>
		{/if}
	{/each}
</select>
```

- [ ] **Step 1.11: Run primitive tests, verify all pass**

```bash
cd frontend && npm run test -- Select.test.ts
```

Expected: every test (existing + 7 new) passes.

- [ ] **Step 1.12: Update barrel re-exports**

Replace line 16 of `frontend/src/lib/components/forms/index.ts`:

Old:

```ts
export type { SelectProps, SelectOption } from './Select.svelte';
```

New:

```ts
export type { SelectProps, SelectOption, SelectGroup, SelectItem } from './Select.svelte';
```

- [ ] **Step 1.13: Add preview demo for grouped + disabled**

In `frontend/src/routes/dev/form-primitive-preview/+page.svelte`, find the existing Select demo section (search for `<Select` near the existing
`placeholder="Select an option"` block at line ~153) and append a sibling block after it:

```svelte
<div>
	<Select
		id="demo-grouped"
		placeholder="Select config..."
		options={[
			{ label: 'Saved', options: [
				{ value: 'cfg:1', label: 'Production' },
				{ value: 'cfg:2', label: 'Staging' }
			]},
			{ label: 'Inline', options: [
				{ value: 'type:apt', label: 'APT (deprecated)', disabled: true },
				{ value: 'type:docker', label: 'Docker' }
			]}
		]}
	/>
</div>
```

(Match the surrounding indentation and wrapper-`<div>` style. If the existing demos are inside a `<section>` with a heading,
add a small `<h3>Grouped + disabled</h3>` style label in the same flavour as siblings.)

- [ ] **Step 1.14: Verify svelte-check + lint + format clean**

```bash
cd frontend && npm run check && npm run lint && npm run format:check
```

Expected: zero errors / warnings.

- [ ] **Step 1.15: Run full frontend test suite**

```bash
cd frontend && npm run test
```

Expected: all green. Existing call sites of `Select` (Settings, SchemaForm, etc.) compile and pass without source changes (per spec
backwards-compatibility note).

- [ ] **Step 1.16: Commit**

```bash
git add frontend/src/lib/components/forms/Select.svelte \
        frontend/src/lib/components/forms/Select.test.ts \
        frontend/src/lib/components/forms/index.ts \
        frontend/src/routes/dev/form-primitive-preview/+page.svelte
git commit -m "feat(forms): extend Select with optgroup, width variant, data-ui marker

Adds SelectGroup discriminated union, per-option disabled flag, width:
'full' | 'auto' prop, and data-ui=\"select\" marker. Re-exports SelectGroup
and SelectItem through the forms barrel. Preview demo gains a grouped +
disabled-option entry. Existing call sites unchanged."
```

---

## Task 2: Rewrite stale `SchemaForm.test.ts:333` regression-guard

**Files:**

- Modify: `frontend/src/lib/components/surfaces/SchemaForm.test.ts:333`

The existing block asserts the field renders a raw `<select>` ("not migrated — regression guard"). `SchemaForm` migrated to the primitive in commit
`5964fdca`; the test still passes incidentally because the primitive renders a native `<select>`. Replace it with a positive `[data-ui="select"]`
assertion.

- [ ] **Step 2.1: Read the current test block**

```bash
sed -n '325,360p' frontend/src/lib/components/surfaces/SchemaForm.test.ts
```

Record the exact `it(...)` block at line 333 (and any leading comment) so the replacement preserves indentation.

- [ ] **Step 2.2: Replace the test block**

Replace the entire `it('select field renders raw <select> ...', ...)` block (and any stale "regression guard" comment immediately above it) with:

```ts
it('select field renders Select primitive', async () => {
    const loadInitialValues = vi.fn().mockResolvedValue({});
    vi.mocked(apiGet).mockResolvedValue([]);
    const { container } = render(SchemaForm, {
        fields: [
            {
                key: 'region',
                label: 'Region',
                field_type: 'select',
                required: false,
                options: [{ value: 'eu', label: 'EU' }]
            }
        ] satisfies FormField[],
        loadInitialValues,
        onsubmit: vi.fn().mockResolvedValue(undefined)
    });
    await waitFor(() => expect(loadInitialValues).toHaveBeenCalled());
    expect(container.querySelector('[data-ui="select"]')).toBeInTheDocument();
});
```

(`FormField`, `apiGet`, `vi`, `render`, `waitFor`, `expect` are already imported at the top of the file — no new imports.)

- [ ] **Step 2.3: Run the test**

```bash
cd frontend && npm run test -- SchemaForm.test.ts
```

Expected: pass. The primitive's new `data-ui="select"` (Task 1) is what the assertion relies on.

- [ ] **Step 2.4: Verify svelte-check + lint clean**

```bash
cd frontend && npm run check && npm run lint
```

- [ ] **Step 2.5: Commit**

```bash
git add frontend/src/lib/components/surfaces/SchemaForm.test.ts
git commit -m "test(schema-form): replace stale regression-guard with positive primitive check

SchemaForm migrated to the Select primitive in 5964fdca; the
\"renders raw <select>\" guard passed incidentally because the primitive
renders a native <select>. Switch to a [data-ui=\"select\"] assertion that
actually verifies the primitive path."
```

---

## Task 3: Site 1 — `routes/services/+page.svelte` merge target

**Files:**

- Modify: `frontend/src/routes/services/+page.svelte` (lines ~53, 221, 226, 237, 681)

The placeholder option uses `value={null}` today. Drop `null` from `mergeTargetId` (default `''`), update the three reset sites, swap the raw
`<select>` for `<Select>` with `placeholder` + options derived from approved discovery-capable services.

- [ ] **Step 3.1: Confirm `Select` import already present**

```bash
grep -n "import.*Select" frontend/src/routes/services/+page.svelte
```

If `Select` is not yet imported, add an import next to the existing form-primitive imports (or at the bottom of the import block):

```ts
import { Select } from '$lib/components/forms';
```

- [ ] **Step 3.2: Change `mergeTargetId` declaration**

Replace at line 53:

Old:

```ts
let mergeTargetId: string | null = $state(null);
```

New:

```ts
let mergeTargetId = $state('');
```

- [ ] **Step 3.3: Update reset sites**

Replace at lines 221, 226, 237 — each `mergeTargetId = null;` becomes `mergeTargetId = '';`. Use grep to enumerate first:

```bash
grep -n "mergeTargetId = null" frontend/src/routes/services/+page.svelte
```

For every match returned, change `null` to `''`. After editing, re-run the grep — expected output: zero matches.

- [ ] **Step 3.4: Add derived options array**

Find the `<script>` block where `mergeSource` and `services` are declared. After those declarations (anywhere before the `</script>` close), add:

```ts
const mergeTargetOptions = $derived(
    services
        .filter(
            (s) =>
                s.status === 'approved' &&
                s.capabilities.includes('software_discovery') &&
                s.id !== mergeSource?.id
        )
        .map((t) => ({
            value: t.id,
            label: `${t.friendly_name} (${t.hostname})`
        }))
);
```

Verify the field names against the existing inline `<select>` template — match whatever properties the current `<option>` rendering uses (e.g. if it
uses `t.name` instead of `t.friendly_name`, copy that field name verbatim).

- [ ] **Step 3.5: Replace the `<select>` markup at line 681**

Locate the raw `<select>` block (likely 6–15 lines including `<option>` children). Replace it with:

```svelte
<Select
    id="merge-target"
    bind:value={mergeTargetId}
    placeholder="-- Select a service --"
    options={mergeTargetOptions}
/>
```

Preserve the surrounding wrapper element (`<div>`, `<label>`, etc.) verbatim. The truthy check at the next line (`disabled={!mergeTargetId}` on the
Merge button) keeps working because `''` is falsy.

- [ ] **Step 3.6: Run page-level test if one exists, then full suite**

```bash
cd frontend && npm run test -- services
cd frontend && npm run test
```

Expected: green. If a test asserts on dropped class strings or DOM structure (e.g. `querySelector('select.select')`), update it minimally — the
primitive renders a native `<select>` so most queries still pass.

- [ ] **Step 3.7: Verify svelte-check + lint + format clean**

```bash
cd frontend && npm run check && npm run lint && npm run format:check
```

- [ ] **Step 3.8: Manual eyeball check**

```bash
cd frontend && npm run dev
```

Open the services page, click Merge on an approved service, confirm the new `<Select>` shows the placeholder, lists eligible targets, excludes the
source service, and the Merge button activates only after a real target is chosen. Stop the dev server.

- [ ] **Step 3.9: Verify no `<select>` remains at the migrated site**

```bash
grep -n "<select" frontend/src/routes/services/+page.svelte
```

Expected: zero matches.

- [ ] **Step 3.10: Commit**

```bash
git add frontend/src/routes/services/+page.svelte
git commit -m "refactor(services): migrate merge-target dropdown to Select primitive

Drops nullable mergeTargetId in favour of '' default and derives the
target list through the new placeholder-aware Select primitive."
```

---

## Task 4: Site 2 — `routes/software/+page.svelte` plugin filter

**Files:**

- Modify: `frontend/src/routes/software/+page.svelte` (line ~1059)

Toolbar plugin-type filter uses raw `<select class="select text-sm w-auto">` today. Migrate to `<Select width="auto">` so the dropdown sizes to its
content inside the `flex-wrap` toolbar.

- [ ] **Step 4.1: Confirm `Select` import present**

```bash
grep -n "import.*Select" frontend/src/routes/software/+page.svelte
```

Add `import { Select } from '$lib/components/forms';` if missing.

- [ ] **Step 4.2: Replace the raw `<select>` block at line 1059**

Locate the block (look for the `pluginTypeOptions.length > 0` guard and the `pluginTypeFilter` binding). Replace the whole `<select>` element (and its
`<option>` children) with:

```svelte
{#if pluginTypeOptions.length > 0}
    <Select
        id="software-plugin-filter"
        width="auto"
        bind:value={pluginTypeFilter}
        aria-label="Filter by plugin"
        options={[
            { value: '', label: 'All plugins' },
            ...pluginTypeOptions.map((opt) => ({
                value: opt.plugin_type,
                label: opt.display_name
            }))
        ]}
        onchange={() => {
            currentPage = 1;
            loadAll(1);
        }}
    />
{/if}
```

Preserve the surrounding `{#if pluginTypeOptions.length > 0}` guard if it already exists; otherwise the snippet above keeps the same gating behaviour
as the original.

- [ ] **Step 4.3: Run software-page test if any, then full suite**

```bash
cd frontend && npm run test -- software
cd frontend && npm run test
```

- [ ] **Step 4.4: Verify svelte-check + lint + format**

```bash
cd frontend && npm run check && npm run lint && npm run format:check
```

- [ ] **Step 4.5: Manual eyeball check**

```bash
cd frontend && npm run dev
```

Open the software page, confirm the plugin filter shows in the toolbar with `w-auto` sizing (does not stretch to fill the row), still resets the page
to 1 and reloads on change. Stop the dev server.

- [ ] **Step 4.6: Verify no `<select>` remains in this file**

```bash
grep -n "<select" frontend/src/routes/software/+page.svelte
```

Expected: zero matches.

- [ ] **Step 4.7: Commit**

```bash
git add frontend/src/routes/software/+page.svelte
git commit -m "refactor(software): migrate plugin-type filter to Select primitive

Uses width=\"auto\" so the toolbar filter sizes to content instead of
stretching across the flex-wrap row."
```

---

## Task 5: Site 3 — `AssignToHostModal.svelte` (both tables)

**Files:**

- Modify: `frontend/src/lib/components/AssignToHostModal.svelte` (lines ~32, 344, 468)

Table 1 has a raw `<select>` at line 344 with conditional `controller` option for `fetch_releases`. Table 2 (line 468) is already `<Select>` with
hardcoded `[auto, agent]`. Both consume a shared `executionSiteOptions(role)` helper after migration, eliminating drift.

- [ ] **Step 5.1: Confirm `Select` and `SelectOption` imports**

```bash
grep -n "import.*Select" frontend/src/lib/components/AssignToHostModal.svelte
```

Ensure the file imports `Select` and `SelectOption` from the barrel:

```ts
import { Select, type SelectOption } from '$lib/components/forms';
```

(If only `Select` is imported today, extend the existing import; do not add a duplicate line.)

- [ ] **Step 5.2: Add `executionSiteOptions` helper**

In the `<script>` block (anywhere after the `StandardRoleKey` type declaration on line 32 and before the markup), add:

```ts
function executionSiteOptions(role: StandardRoleKey): SelectOption[] {
    return [
        { value: 'auto', label: 'Auto' },
        { value: 'agent', label: 'Agent' },
        ...(role === 'fetch_releases'
            ? [{ value: 'controller', label: 'Controller' }]
            : [])
    ];
}
```

- [ ] **Step 5.3: Replace raw `<select>` at line 344 (Table 1, Site 3a)**

Locate the raw `<select>` inside Table 1 (the `{#each ['detect_version', 'fetch_releases']}` iteration). Replace the whole `<select>...</select>`
block with:

```svelte
<Select
    id="assign-role-{role}-execution-site"
    bind:value={standardAssignments[role].execution_site}
    disabled={!a.enabled}
    options={executionSiteOptions(role)}
/>
```

Match indentation; preserve any surrounding `<td>`/`<div>` wrappers verbatim.

- [ ] **Step 5.4: Update Table 2 (Site 3b, line 468) to use the helper**

Locate the existing `<Select>` block at line 468 with the hardcoded `options={[{ value: 'auto', ... }, { value: 'agent', ... }]}`. Replace only the
`options=` attribute:

Old:

```svelte
options={[{ value: 'auto', label: 'Auto' }, { value: 'agent', label: 'Agent' }]}
```

New:

```svelte
options={executionSiteOptions(role)}
```

For `execute_update` the helper returns `[auto, agent]` — same list as before, so no behavioural change. Pure DRY refactor.

- [ ] **Step 5.5: Run tests**

```bash
cd frontend && npm run test -- AssignToHostModal
cd frontend && npm run test
```

- [ ] **Step 5.6: svelte-check + lint + format**

```bash
cd frontend && npm run check && npm run lint && npm run format:check
```

- [ ] **Step 5.7: Manual eyeball check**

```bash
cd frontend && npm run dev
```

Open the host detail / assignment page, open the Assign-to-host modal, confirm:

- Table 1: the `detect_version` row shows only Auto + Agent; the `fetch_releases` row shows Auto + Agent + Controller.
- Table 2: the `execute_update` row still shows Auto + Agent.
- Disabling a row via the enable toggle disables its execution-site dropdown.

Stop the dev server.

- [ ] **Step 5.8: Verify no `<select>` remains in this file**

```bash
grep -n "<select" frontend/src/lib/components/AssignToHostModal.svelte
```

Expected: zero matches.

- [ ] **Step 5.9: Commit**

```bash
git add frontend/src/lib/components/AssignToHostModal.svelte
git commit -m "refactor(assign-modal): migrate execution-site dropdown to Select primitive

Adds a shared executionSiteOptions(role) helper and uses it in both
role-iteration tables, removing the duplicated hardcoded array in Table 2
and the raw <select> in Table 1."
```

---

## Task 6: Sites 4 + 5 — `EditHostAssignmentModal.svelte` plugin pickers

**Files:**

- Modify: `frontend/src/lib/components/EditHostAssignmentModal.svelte` (lines ~706, ~1070, plus helper)

Both pickers use raw `<select>` with `<optgroup>` for "Saved" / "Inline" branches. Add a shared `pluginConfigItems(saved, types)` helper that returns
a `SelectItem[]`, then replace both call sites with `<Select>`.

- [ ] **Step 6.1: Confirm imports**

```bash
grep -n "import.*from '\$lib/components/forms'" frontend/src/lib/components/EditHostAssignmentModal.svelte
```

Ensure the file imports `Select`, `SelectItem`, and `SelectOption` from the barrel:

```ts
import { Select, type SelectItem, type SelectOption } from '$lib/components/forms';
```

(Combine into the existing forms import — do not add a new line.)

- [ ] **Step 6.2: Identify `PluginConfig` and `PluginType` local types**

```bash
grep -n "PluginConfig\|PluginType" frontend/src/lib/components/EditHostAssignmentModal.svelte | head -20
```

Note the exact type names used for `pluginConfigs` (line 111) and `pluginTypes` (line 112) — likely `PluginConfigResponse` and `PluginTypeInfo`. Use
those in the helper signature below.

- [ ] **Step 6.3: Add `pluginConfigItems` helper**

In the `<script>` block, after `pluginSelection(...)` (line 277) and before `applySelection(...)` (line 283), add:

```ts
function pluginConfigItems(
    savedOpts: PluginConfigResponse[],
    typeOpts: PluginTypeInfo[]
): SelectItem[] {
    const placeholder: SelectOption = { value: '', label: '— not configured —' };
    const inlineOpts = typeOpts.map((pt) => ({
        value: `type:${pt.plugin_type}`,
        label: pt.display_name
    }));
    if (savedOpts.length === 0) {
        return [placeholder, ...inlineOpts];
    }
    const items: SelectItem[] = [
        placeholder,
        {
            label: 'Saved',
            options: savedOpts.map((cfg) => ({
                value: `cfg:${cfg.id}`,
                label: cfg.name
            }))
        }
    ];
    if (inlineOpts.length > 0) {
        items.push({ label: 'Inline', options: inlineOpts });
    }
    return items;
}
```

(Use the type names noted in Step 6.2. If they differ, substitute them directly here and in Step 6.4 / 6.5 below.)

- [ ] **Step 6.4: Replace raw `<select>` at line 706 (Site 4)**

Locate the raw `<select>` inside the `{@const s = standardStates[role]}` block (line 692 area). Replace the whole `<select>...</select>` (including
its inline `<optgroup>` / `<option>` children) with:

```svelte
<Select
    id="cfg-{role}"
    value={pluginSelection(s)}
    options={pluginConfigItems(savedRoleOpts, typeOpts)}
    onchange={(e) => applySelection(standardStates[role], (e.target as HTMLSelectElement).value)}
/>
```

Preserve the surrounding wrapper element (e.g. `<label>`, `<div>`) and any nearby labels verbatim. Confirm `savedRoleOpts` and `typeOpts` already
exist as local `$derived` / `const` bindings near this site (they're the same lists feeding the original `<optgroup>`s); if not, copy the inline
expressions used in the old `<optgroup>` blocks directly into the helper call.

- [ ] **Step 6.5: Replace raw `<select>` at line 1070 (Site 5)**

Locate the second raw `<select>` (inside the `{#each hookLists[role] as entry}` iteration). Replace with:

```svelte
<Select
    id="hook-cfg-{entry.localKey}"
    value={pluginSelection(entry)}
    options={pluginConfigItems(savedHookOpts, hookTypeOpts)}
    onchange={(e) => applySelection(entry, (e.target as HTMLSelectElement).value)}
/>
```

Same caveat as Step 6.4 — copy whatever local binding name fed the original `<optgroup>` for the saved/inline lists.

- [ ] **Step 6.6: Run tests**

```bash
cd frontend && npm run test -- EditHostAssignmentModal
cd frontend && npm run test
```

- [ ] **Step 6.7: svelte-check + lint + format**

```bash
cd frontend && npm run check && npm run lint && npm run format:check
```

- [ ] **Step 6.8: Manual eyeball check**

```bash
cd frontend && npm run dev
```

Open Edit-host-assignment for a host. For each standard role and each hook entry, verify:

- Placeholder `— not configured —` shows when nothing is selected.
- `Saved` optgroup appears only when there is at least one saved config for that plugin type.
- `Inline` optgroup appears only when there is at least one available plugin type.
- Selecting a saved config populates plugin_config_id; selecting an inline type clears it and seeds form fields; selecting placeholder clears both.

Stop the dev server.

- [ ] **Step 6.9: Verify `standardStates` and `hookLists` are still `$state`**

```bash
grep -n "let standardStates" frontend/src/lib/components/EditHostAssignmentModal.svelte
grep -n "let hookLists" frontend/src/lib/components/EditHostAssignmentModal.svelte
```

Expected: both lines contain `= $state(`. The pass-through binding pattern depends on this; if either declaration ever loses the `$state` wrapper, the
dropdown silently desyncs from caller state (per spec line 410-422).

- [ ] **Step 6.10: Verify no `<select>` remains in this file**

```bash
grep -n "<select" frontend/src/lib/components/EditHostAssignmentModal.svelte
```

Expected: zero matches.

- [ ] **Step 6.11: Commit**

```bash
git add frontend/src/lib/components/EditHostAssignmentModal.svelte
git commit -m "refactor(edit-assignment): migrate plugin-config pickers to Select primitive

Extracts a shared pluginConfigItems helper that returns a SelectItem[]
mixing a placeholder option with Saved + Inline optgroups, and replaces
both raw <select> + <optgroup> blocks (cfg picker and hook cfg picker)
with the new primitive. Empty Inline group is filtered to avoid a bare
optgroup label."
```

---

## Task 7: Site 6 — `ProviderSelector.svelte` cleanup + caller updates

**Files:**

- Modify: `frontend/src/lib/components/ui/ProviderSelector.svelte`
- Modify: `frontend/src/lib/components/ui/ProviderSelector.test.ts`
- Modify: `frontend/src/lib/components/surfaces/SurfaceReadPanel.svelte` (lines ~358, ~395)

Drop `emptyMessage` prop, the `{#if providers.length === 0}` branch, the `disabled={providers.length === 0}` attribute, and the manual `select.value =
currentId` revert hack. Add a required `id: string` prop. Update both `SurfaceReadPanel` callers to pass `id`. Slim the test file: drop the
controlled-revert test (it depended on the removed hack), drop class-string assertions, add the `id` prop to every `render()` call.

- [ ] **Step 7.1: Update `ProviderSelector.svelte`**

Replace the entire body of `frontend/src/lib/components/ui/ProviderSelector.svelte` with:

```svelte
<script lang="ts">
	import { Select } from '$lib/components/forms';

	export type ProviderOption = {
		id: string;
		label: string;
		description?: string;
		disabled?: boolean;
	};

	let {
		id,
		label = 'Provider',
		providers = [],
		selectedId,
		onSelect
	}: {
		id: string;
		label?: string;
		providers: ProviderOption[];
		selectedId?: string;
		onSelect?: (id: string) => void;
	} = $props();

	let uncontrolledId = $state('');

	const fallbackId = $derived(providers.find((provider) => !provider.disabled)?.id ?? '');
	const isControlled = $derived(selectedId !== undefined);
	const currentId = $derived(isControlled ? (selectedId ?? fallbackId) : uncontrolledId || fallbackId);
	const selectedProvider = $derived(providers.find((provider) => provider.id === currentId));

	$effect(() => {
		if (isControlled) {
			return;
		}
		if (!providers.some((provider) => provider.id === uncontrolledId && !provider.disabled)) {
			uncontrolledId = fallbackId;
		}
	});

	function handleChange(event: Event): void {
		const nextId = (event.currentTarget as HTMLSelectElement).value;
		if (!isControlled) {
			uncontrolledId = nextId;
		}
		onSelect?.(nextId);
	}
</script>

<div class="space-y-2" data-ui="provider-selector">
	<label for={id} class="block space-y-2">
		<span class="text-sm font-medium text-[var(--text-primary)]">{label}</span>
	</label>
	<Select
		{id}
		value={currentId}
		options={providers.map((p) => ({ value: p.id, label: p.label, disabled: p.disabled }))}
		onchange={handleChange}
	/>
	{#if selectedProvider?.description}
		<p class="text-sm text-[var(--text-secondary)]">{selectedProvider.description}</p>
	{/if}
</div>
```

- [ ] **Step 7.2: Update `ProviderSelector.test.ts`**

Replace the entire body of `frontend/src/lib/components/ui/ProviderSelector.test.ts` with:

```ts
import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import ProviderSelector from './ProviderSelector.svelte';

afterEach(() => {
    cleanup();
    vi.clearAllMocks();
});

describe('ProviderSelector', () => {
    it('renders providers with semantic labels and reports selection changes', async () => {
        const onSelect = vi.fn();

        const view = render(ProviderSelector, {
            id: 'test-provider',
            label: 'Provider',
            selectedId: 'provider-a',
            providers: [
                { id: 'provider-a', label: 'Provider A', description: 'Connected locally' },
                { id: 'provider-b', label: 'Provider B', description: 'Connected remotely' }
            ],
            onSelect
        });

        const select = screen.getByLabelText('Provider');
        expect(screen.getByText('Connected locally')).toBeInTheDocument();

        await fireEvent.change(select, { target: { value: 'provider-b' } });

        expect(onSelect).toHaveBeenCalledWith('provider-b');
        await view.rerender({
            id: 'test-provider',
            label: 'Provider',
            selectedId: 'provider-b',
            providers: [
                { id: 'provider-a', label: 'Provider A', description: 'Connected locally' },
                { id: 'provider-b', label: 'Provider B', description: 'Connected remotely' }
            ],
            onSelect
        });
        expect(screen.getByText('Connected remotely')).toBeInTheDocument();
    });

    it('renders disabled providers with the disabled attribute on their option', () => {
        render(ProviderSelector, {
            id: 'test-provider',
            label: 'Provider',
            selectedId: 'provider-a',
            providers: [
                { id: 'provider-a', label: 'Provider A' },
                { id: 'provider-b', label: 'Provider B', disabled: true }
            ]
        });
        const opts = (screen.getByLabelText('Provider') as HTMLSelectElement).options;
        expect(opts[0].disabled).toBe(false);
        expect(opts[1].disabled).toBe(true);
    });

    it('supports uncontrolled selection changes when selectedId is omitted', async () => {
        render(ProviderSelector, {
            id: 'test-provider',
            label: 'Provider',
            providers: [
                { id: 'provider-a', label: 'Provider A', description: 'Connected locally' },
                { id: 'provider-b', label: 'Provider B', description: 'Connected remotely' }
            ]
        });

        const select = screen.getByLabelText('Provider') as HTMLSelectElement;
        expect(select.value).toBe('provider-a');
        expect(screen.getByText('Connected locally')).toBeInTheDocument();

        await fireEvent.change(select, { target: { value: 'provider-b' } });

        expect(select.value).toBe('provider-b');
        expect(screen.getByText('Connected remotely')).toBeInTheDocument();
    });
});
```

The previous "treats selectedId as authoritative when the parent rerenders with the same selection" test depended on the removed `select.value =
currentId` revert hack and is dropped intentionally — both real callers in `SurfaceReadPanel.svelte` accept `onSelect` updates unconditionally, so the
rejection scenario the test guarded does not exist.

- [ ] **Step 7.3: Update `SurfaceReadPanel.svelte` callers**

In `frontend/src/lib/components/surfaces/SurfaceReadPanel.svelte`, find the first `<ProviderSelector>` invocation (line ~358) and add an
`id="surface-provider"` attribute:

```svelte
<ProviderSelector
    id="surface-provider"
    label="Provider"
    selectedId={selectedProviderId}
    providers={availableProviders.map((provider) => ({
        id: provider.provider_id,
        label: provider.display_label,
        description: undefined
    }))}
    onSelect={(providerId) => {
        selectedProviderId = providerId;
    }}
/>
```

Find the second invocation (line ~395, inside the `{#if contextSelector}` block) and add `id="surface-context-{contextSelector.param_key}"`:

```svelte
<ProviderSelector
    id="surface-context-{contextSelector.param_key}"
    label={contextSelector.label}
    providers={[{ id: '', label: contextSelector.all_option_label }, ...selectorOptions]}
    selectedId={selectedContextValue}
    onSelect={(id) => {
        selectedContextValue = id;
    }}
/>
```

- [ ] **Step 7.4: Run ProviderSelector + SurfaceReadPanel tests**

```bash
cd frontend && npm run test -- ProviderSelector
cd frontend && npm run test -- SurfaceReadPanel
```

Expected: green. If a `SurfaceReadPanel` test fails because it asserted on the wrapper's old internals, update it minimally — most queries against the
inner `<select>` continue to work.

- [ ] **Step 7.5: svelte-check + lint + format**

```bash
cd frontend && npm run check && npm run lint && npm run format:check
```

Expected: zero errors. If `svelte-check` flags missing `id` props on any other `ProviderSelector` caller, add the `id` prop with a stable per-site
name. (Spec lists only the two callers in `SurfaceReadPanel.svelte`; verify no others slipped in via newer commits with `grep -rn "<ProviderSelector"
frontend/src`.)

- [ ] **Step 7.6: Run full test suite**

```bash
cd frontend && npm run test
```

- [ ] **Step 7.7: Manual eyeball check**

```bash
cd frontend && npm run dev
```

Open a surface page that uses `ProviderSelector` (any targeted surface or any `context_selector`-driven surface). Verify:

- Provider dropdown labels are linked to the inner `<select>` (clicking the label focuses the select).
- Switching provider updates the surface body.
- No console errors.

Stop the dev server.

- [ ] **Step 7.8: Verify no `<select>` remains in `ProviderSelector.svelte`**

```bash
grep -n "<select" frontend/src/lib/components/ui/ProviderSelector.svelte
```

Expected: zero matches (the inner select now lives inside the `Select` primitive).

- [ ] **Step 7.9: Commit**

```bash
git add frontend/src/lib/components/ui/ProviderSelector.svelte \
        frontend/src/lib/components/ui/ProviderSelector.test.ts \
        frontend/src/lib/components/surfaces/SurfaceReadPanel.svelte
git commit -m "refactor(provider-selector): migrate to Select primitive and drop dead code

Removes the emptyMessage prop, the {#if providers.length === 0} branch,
the disabled-on-empty attribute, and the manual select.value = currentId
revert hack — all unreachable from real callers in SurfaceReadPanel.
Adds a required id: string prop and threads it through both
SurfaceReadPanel call sites. The controlled-revert test is dropped along
with the hack it was guarding."
```

---

## Task 8: Repo-wide acceptance verification

**Files:** *(verification only — no edits expected)*

- [ ] **Step 8.1: Confirm no raw `<select>` outside the primitive**

```bash
cd frontend && grep -rEn "<select[\s>]" src --include="*.svelte" | grep -v "src/lib/components/forms/Select.svelte"
```

Expected: zero matches.

- [ ] **Step 8.2: Confirm `data-ui="select"` lands on the primitive**

```bash
cd frontend && grep -rn "data-ui=\"select\"" src
```

Expected: exactly one match in `src/lib/components/forms/Select.svelte`.

- [ ] **Step 8.3: Run all frontend gates**

```bash
cd frontend && npm run check && npm run lint && npm run format:check && npm run test && npm run build
```

Expected: every command exits zero.

- [ ] **Step 8.4: Manual eyeball regression sweep**

```bash
cd frontend && npm run dev
```

Visit each migrated surface in turn:

1. Services → Merge service modal (Site 1)
2. Software list toolbar (Site 2)
3. Hosts → Assign to host modal (Site 3, both tables)
4. Hosts → Edit host assignments (Sites 4 + 5, standard + hook pickers)
5. Any surface read panel using ProviderSelector (Site 6, both targeted + context-selector flavours)
6. `/dev/form-primitive-preview` (grouped + disabled-option demo)

Confirm each looks visually identical to the pre-migration baseline (modulo the deliberate Site 6 cleanup) and that disabled/placeholder/optgroup
states render correctly.

Stop the dev server.

- [ ] **Step 8.5: No commit needed** — verification step only.

---

## Self-Review Notes

- **Spec coverage**: Every spec section maps to a task — primitive extension (Task 1), `SchemaForm` test rewrite (Task 2), Sites 1–6 (Tasks 3–7),
  repo-wide acceptance (Task 8). Out-of-scope follow-ups (uncontrolled-mode and `description` slot removal, broader `data-ui` rollout, `width` on
  other primitives) remain explicitly out of scope.
- **Ordering**: Task 1 lands first because every later task imports `SelectGroup` / `SelectItem` or relies on the new `width` / `data-ui` API. Task 2
  lands immediately after because the new `data-ui="select"` is the assertion target. Tasks 3–7 are independent across sites but share Task 1's
  primitive — order between them is flexible.
- **TDD coverage**: Primitive extension (Task 1) is fully TDD'd — seven new failing tests before implementation. Migration tasks (3–7) are pure
  refactors of UI markup; the existing page-level / component-level tests act as regression nets, augmented by the manual eyeball steps.
- **Caveman-mode commit messages**: `chore`/`feat`/`refactor`/`test` prefixes match the repo's recent commit style (`feat(forms): ...`, `fix(forms):
  ...`).

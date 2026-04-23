# Form Primitive Adoption Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task.
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace every raw `<input>`, `<input type="checkbox">`, and `<textarea>`
(where spec primitives exist) with `<Input>`, `<Checkbox>`, and `<Textarea>`
across 24 call sites in 16 files.

**Architecture:** Drop-in substitution. No logic changes, no layout changes, no validation changes.
Existing `bind:value`/`bind:checked` bindings are preserved — all three primitives use Svelte 5
`$bindable()` props. The only new requirement is that every primitive requires a non-optional
`id: string` prop; raw elements that lacked `id` must be assigned one.

**Tech Stack:** SvelteKit, Svelte 5, Vitest

---

## Primitive prop reference

```text
Input:    id (required), type, value (bindable), name, placeholder, autocomplete, disabled, required, error, oninput, onblur, aria-describedby, class
Checkbox: id (required), checked (bindable), name, disabled, onchange, class
Textarea: id (required), value (bindable), name, placeholder, rows, disabled, required, error, variant ('default'|'mono'), oninput, onblur, aria-describedby, class
```

All three forward unknown HTML attributes via `{...rest}`, so `min`, `max`, `aria-label`,
`onkeydown`, `indeterminate` are passed through to the underlying element even if not declared
in the TypeScript interface.

---

## Migration pattern reference

```svelte
<!-- Before: raw input -->
<input class="input w-full" type="text" bind:value={name} placeholder="Foo" id="my-name" />
<!-- After -->
<Input id="my-name" bind:value={name} placeholder="Foo" class="w-full" />

<!-- Before: raw checkbox with bind -->
<input class="checkbox" type="checkbox" bind:checked={enabled} id="toggle" />
<!-- After -->
<Checkbox id="toggle" bind:checked={enabled} />

<!-- Before: raw checkbox controlled (no bind) -->
<input type="checkbox" class="checkbox" checked={selected.has(x)} onchange={() => toggle(x)} />
<!-- After: note checked= not bind:checked= when using controlled pattern -->
<Checkbox id="cb-{x}" checked={selected.has(x)} onchange={() => toggle(x)} />

<!-- Before: raw textarea for monospace -->
<textarea class="textarea font-mono text-xs" bind:value={text} rows="6"></textarea>
<!-- After: use variant="mono" (applies font-mono text-[13px] internally) -->
<Textarea id="my-id" bind:value={text} rows={6} variant="mono" />
```

---

## Files modified

`SurfaceForm.svelte`, `AuthenticationSettings.svelte`, `RegistrationSettings.svelte`,
`AgentCertificateSettings.svelte`, `EnrollmentTokenSettings.svelte`,
`NotificationRulesSettings.svelte`, `OidcProvidersSettings.svelte`, `AddSoftwareModal.svelte`,
`CheckboxList.svelte`, `AssignToHostModal.svelte`, `SoftwareMergeWizard.svelte`,
`software/+page.svelte`, `software/[id]/+page.svelte`, `IgnoreRulesTab.svelte`,
`services/+page.svelte`, `system-services/+page.svelte`

---

## Task 1: Add missing props to Checkbox and Input primitives

Neither `Checkbox.svelte` nor `Input.svelte` has rest prop spread. Both need explicit prop
additions before the downstream migration tasks that use them. This task is required, not
optional — skip it and Tasks 4, 8, 9, 10 will produce type errors.

- [ ] **Step 1: Add `indeterminate` prop to Checkbox.svelte**

`Checkbox.svelte` currently declares: `id, checked, name, disabled, onchange, class`.
No `indeterminate` prop exists. Tasks 9 (software/+page.svelte:940) and 10
(IgnoreRulesTab:162) use `indeterminate={...}` on the migrated `<Checkbox>`.

In `frontend/src/lib/components/Checkbox.svelte`, update `CheckboxProps`:

```svelte
export type CheckboxProps = {
	id: string;
	checked?: boolean;
	indeterminate?: boolean;
	name?: string;
	disabled?: boolean;
	onchange?: (e: Event) => void;
	class?: string;
};
```

And update the `$props()` destructure and template:

```svelte
let {
	id,
	checked = $bindable(false),
	indeterminate = $bindable(false),
	name,
	disabled = false,
	onchange,
	class: className = ''
}: CheckboxProps = $props();
```

```svelte
<input type="checkbox" {id} bind:checked bind:indeterminate {name} {disabled} {onchange}
	class={computedClass} />
```

- [ ] **Step 2: Add `onkeydown`, `min`, `max`, `aria-label` to Input.svelte**

`Input.svelte` has no rest prop spread. Task 8 (SoftwareMergeWizard search input) passes
`onkeydown`. Task 4 (AgentCertificateSettings) passes `min`/`max` on number inputs.

In `frontend/src/lib/components/Input.svelte`, extend `InputProps`:

```svelte
export type InputProps = {
	id: string;
	type?: InputType;
	value?: string;
	name?: string;
	placeholder?: string;
	autocomplete?: string;
	disabled?: boolean;
	required?: boolean;
	error?: string;
	min?: number | string;
	max?: number | string;
	oninput?: (e: Event) => void;
	onblur?: (e: FocusEvent) => void;
	onkeydown?: (e: KeyboardEvent) => void;
	'aria-label'?: string;
	'aria-describedby'?: string;
	class?: string;
};
```

Add destructure and forwarding to match (add `min`, `max`, `onkeydown`, `aria-label` beside
existing props):

```svelte
let {
	...,
	min,
	max,
	onkeydown,
	'aria-label': ariaLabel,
	...
}: InputProps = $props();
```

And on the `<input>` element: `{min} {max} {onkeydown} aria-label={ariaLabel}`

- [ ] **Step 3: Run type check and Vitest baseline**

```bash
cd frontend && npm run check 2>&1 | tail -5 && npm run test -- --run 2>&1 | tail -5
```

Expected: zero type errors, all tests pass.

- [ ] **Step 3: Run Vitest baseline (confirm 0 failures before any changes)**

```bash
cd frontend && npm run test -- --run 2>&1 | tail -5
```

Expected: all tests pass.

---

## Task 2: SurfaceForm textarea → Textarea variant="mono"

**Files:**

- Modify: `frontend/src/lib/components/surfaces/SurfaceForm.svelte:~137`

- [ ] **Step 1: Confirm Textarea is not already imported**

```bash
grep -n "^import.*Textarea" frontend/src/lib/components/surfaces/SurfaceForm.svelte
```

If not present, add the import in the `<script>` block:

```svelte
import Textarea from '$lib/components/Textarea.svelte';
```

- [ ] **Step 2: Replace the raw textarea**

Find:

```svelte
<textarea class="textarea font-mono text-xs" bind:value={payloadText} rows="6"></textarea>
```

Replace with (`id` must be added — it's required):

```svelte
<Textarea id="surface-form-payload" bind:value={payloadText} rows={6} variant="mono" />
```

Note: `rows={6}` (numeric, not string) matches the Textarea prop type. `variant="mono"` applies
`font-mono text-[13px]` internally (correct design-token size; do not use
`class="font-mono text-xs"` which would be 12px).

- [ ] **Step 3: Run type check**

```bash
cd frontend && npm run check 2>&1 | tail -10
```

Expected: zero errors.

- [ ] **Step 4: Run Vitest**

```bash
cd frontend && npm run test -- --run src/lib/components/surfaces/SurfaceForm.test.ts --reporter=verbose
```

Expected: all tests pass.

- [ ] **Step 5: Commit**

```bash
cd frontend && git add src/lib/components/surfaces/SurfaceForm.svelte
git commit -m "fix(frontend): SurfaceForm raw textarea → Textarea variant=mono"
```

---

## Task 3: Simple settings checkboxes — AuthenticationSettings, RegistrationSettings

**Files:**

- Modify: `frontend/src/routes/settings/AuthenticationSettings.svelte:~50`
- Modify: `frontend/src/routes/settings/RegistrationSettings.svelte:~90`

Both have raw checkboxes with `id` already present. Simplest migration.

- [ ] **Step 1: Check existing imports in both files**

```bash
grep -n "^import.*Checkbox" \
  frontend/src/routes/settings/AuthenticationSettings.svelte \
  frontend/src/routes/settings/RegistrationSettings.svelte
```

Add `import Checkbox from '$lib/components/Checkbox.svelte';` to each file that doesn't have it.

- [ ] **Step 2: Fix AuthenticationSettings.svelte**

Find (around line 50):

```svelte
<input id="password-auth-enabled" class="checkbox" type="checkbox" bind:checked={passwordAuthEnabled} />
```

Replace with:

```svelte
<Checkbox id="password-auth-enabled" bind:checked={passwordAuthEnabled} />
```

- [ ] **Step 3: Fix RegistrationSettings.svelte**

Find (around line 90):

```svelte
<input
	id="registration-require-oidc-token"
	class="checkbox"
	type="checkbox"
	bind:checked={regRequireTokenForOidc}
/>
```

Replace with:

```svelte
<Checkbox id="registration-require-oidc-token" bind:checked={regRequireTokenForOidc} />
```

- [ ] **Step 4: Run type check + Vitest for these files**

```bash
cd frontend && npm run check 2>&1 | tail -5 && npm run test -- --run src/routes/settings/AuthenticationSettings.test.ts src/routes/settings/RegistrationSettings.test.ts --reporter=verbose
```

Expected: zero type errors, all tests pass.

- [ ] **Step 5: Commit**

```bash
cd frontend && git add src/routes/settings/AuthenticationSettings.svelte src/routes/settings/RegistrationSettings.svelte
git commit -m "fix(frontend): settings raw checkboxes → Checkbox in AuthenticationSettings and RegistrationSettings"
```

---

## Task 4: Settings with id derivation — AgentCertificateSettings, EnrollmentTokenSettings, NotificationRulesSettings, OidcProvidersSettings

**Files:**

- Modify: `frontend/src/routes/settings/AgentCertificateSettings.svelte:69,77`
- Modify: `frontend/src/routes/settings/EnrollmentTokenSettings.svelte:246`
- Modify: `frontend/src/routes/settings/NotificationRulesSettings.svelte:284`
- Modify: `frontend/src/routes/settings/OidcProvidersSettings.svelte:336,348`

These files have raw checkboxes or inputs that are missing `id` attributes. Each must be given a derived id.

- [ ] **Step 1: Check existing imports in all four files**

```bash
grep -n "^import.*Checkbox\|^import.*Input" \
  frontend/src/routes/settings/AgentCertificateSettings.svelte \
  frontend/src/routes/settings/EnrollmentTokenSettings.svelte \
  frontend/src/routes/settings/NotificationRulesSettings.svelte \
  frontend/src/routes/settings/OidcProvidersSettings.svelte
```

Add missing imports to each file's `<script>` block.

- [ ] **Step 2: Fix AgentCertificateSettings.svelte**

Line 69 — checkbox has id already:

```svelte
<input id="agent-cert-auto-renew" type="checkbox" class="checkbox" bind:checked={useAutoRenewal} />
```

→

```svelte
<Checkbox id="agent-cert-auto-renew" bind:checked={useAutoRenewal} />
```

Line 77 — first input (id already present):
Find:

```svelte
<input id="agent-cert-lifetime" class="input" type="number" min="1" max="730" bind:value={certLifetimeDays} />
```

→

```svelte
<Input id="agent-cert-lifetime" type="number" min="1" max="730" bind:value={certLifetimeDays} />
```

Line ~79 — second input (no id — derive one):
Find:

```svelte
<input class="input" type="number" min="1" bind:value={certRenewalWindowHours} />
```

→

```svelte
<Input id="agent-cert-renewal-window-hours" type="number" min="1" bind:value={certRenewalWindowHours} />
```

- [ ] **Step 3: Fix EnrollmentTokenSettings.svelte**

Line 246 has a checkbox. Find it:

```bash
grep -n 'type="checkbox"' frontend/src/routes/settings/EnrollmentTokenSettings.svelte
```

The raw checkbox may lack an `id`. If so, derive one from context. Typical form:

```svelte
<input type="checkbox" class="checkbox" bind:checked={someVar} />
```

→

```svelte
<Checkbox id="enrollment-token-<field-name>" bind:checked={someVar} />
```

Where `<field-name>` is derived from the surrounding label text or variable name.

- [ ] **Step 4: Fix NotificationRulesSettings.svelte**

Line 284 has a checkbox with no id:

```svelte
<input type="checkbox" bind:checked={form.enabled} class="checkbox" />
```

→

```svelte
<Checkbox id="notification-rule-enabled" bind:checked={form.enabled} />
```

- [ ] **Step 5: Fix OidcProvidersSettings.svelte**

Line 336:

```svelte
<input class="checkbox" type="checkbox" bind:checked={oidcForm.auto_create_users} />
```

→

```svelte
<Checkbox id="oidc-auto-create-users" bind:checked={oidcForm.auto_create_users} />
```

Line 348 (has extra `mt-1` class):

```svelte
<input class="checkbox mt-1" type="checkbox" bind:checked={oidcForm.allow_private_network_issuers} />
```

→

```svelte
<Checkbox id="oidc-allow-private-network-issuers" bind:checked={oidcForm.allow_private_network_issuers} class="mt-1" />
```

- [ ] **Step 6: Run type check + Vitest**

```bash
cd frontend && npm run check 2>&1 | tail -5 && npm run test -- --run \
  src/routes/settings/AgentCertificateSettings.test.ts \
  src/routes/settings/EnrollmentTokenSettings.test.ts \
  src/routes/settings/NotificationRulesSettings.test.ts \
  src/routes/settings/OidcProvidersSettings.test.ts \
  --reporter=verbose
```

Expected: zero type errors, all tests pass.

- [ ] **Step 7: Commit**

```bash
cd frontend && git add \
  src/routes/settings/AgentCertificateSettings.svelte \
  src/routes/settings/EnrollmentTokenSettings.svelte \
  src/routes/settings/NotificationRulesSettings.svelte \
  src/routes/settings/OidcProvidersSettings.svelte
git commit -m "fix(frontend): settings raw checkboxes/inputs → Checkbox/Input with derived ids"
```

---

## Task 5: AddSoftwareModal — inputs and checkbox

**Files:**

- Modify: `frontend/src/lib/components/AddSoftwareModal.svelte:67,86,99`

Two `<input class="input w-full">` elements and one `<input type="checkbox" class="checkbox">`. All have `id` attributes already.

- [ ] **Step 1: Check existing imports**

```bash
grep -n "^import.*Input\|^import.*Checkbox" frontend/src/lib/components/AddSoftwareModal.svelte
```

Add missing: `import Input from '$lib/components/Input.svelte';` and/or `import Checkbox from '$lib/components/Checkbox.svelte';`

- [ ] **Step 2: Replace the two text inputs (lines 67, 86)**

Line 67:

```svelte
<input
	id="add-software-name"
	class="input w-full"
	bind:value={name}
	placeholder="Firefox"
	aria-invalid={nameError ? 'true' : 'false'}
	oninput={() => {
		if (nameError && name.trim()) nameError = null;
	}}
/>
```

→

```svelte
<Input
	id="add-software-name"
	bind:value={name}
	placeholder="Firefox"
	aria-invalid={nameError ? 'true' : 'false'}
	oninput={() => {
		if (nameError && name.trim()) nameError = null;
	}}
	class="w-full"
/>
```

Line 86 (icon URL input — same pattern, different id/value):

```svelte
<input
	id="add-software-icon-url"
	class="input w-full"
	bind:value={iconUrl}
	placeholder="https://example.com/icon.png"
	aria-invalid={iconUrlError ? 'true' : 'false'}
	oninput={() => {
		const trimmed = iconUrl.trim();
		iconUrlError = trimmed.length > 0 && !isValidLogoUrl(trimmed) ? 'Icon URL must be a valid HTTPS URL.' : null;
	}}
/>
```

→

```svelte
<Input
	id="add-software-icon-url"
	bind:value={iconUrl}
	placeholder="https://example.com/icon.png"
	aria-invalid={iconUrlError ? 'true' : 'false'}
	oninput={() => {
		const trimmed = iconUrl.trim();
		iconUrlError = trimmed.length > 0 && !isValidLogoUrl(trimmed) ? 'Icon URL must be a valid HTTPS URL.' : null;
	}}
	class="w-full"
/>
```

- [ ] **Step 3: Replace the checkbox (line 99)**

```svelte
<input id="add-software-featured" class="checkbox" type="checkbox" bind:checked={featured} />
```

→

```svelte
<Checkbox id="add-software-featured" bind:checked={featured} />
```

- [ ] **Step 4: Run type check + Vitest**

```bash
cd frontend && npm run check 2>&1 | tail -5 && npm run test -- --run src/lib/components/AddSoftwareModal.test.ts src/lib/components/add-software-modal.test.ts --reporter=verbose
```

Expected: zero errors, all tests pass.

- [ ] **Step 5: Commit**

```bash
cd frontend && git add src/lib/components/AddSoftwareModal.svelte
git commit -m "fix(frontend): AddSoftwareModal raw inputs/checkbox → Input/Checkbox primitives"
```

---

## Task 6: CheckboxList — controlled checkbox with dynamic id

**Files:**

- Modify: `frontend/src/lib/components/CheckboxList.svelte:40-47`

The checkbox is inside a `{#each}` loop — id must be derived dynamically from `item.value`.

- [ ] **Step 1: Add Checkbox import**

```bash
grep -n "^import.*Checkbox" frontend/src/lib/components/CheckboxList.svelte
```

If not present, add `import Checkbox from '$lib/components/Checkbox.svelte';`

- [ ] **Step 2: Replace the raw checkbox**

Current:

```svelte
			<input
				type="checkbox"
				class="checkbox"
				checked={selected.has(item.value)}
				disabled={item.disabled}
				onchange={() => toggle(item)}
			/>
```

Replace with (controlled pattern — no `bind:checked` since it uses `onchange` callback):

```svelte
			<Checkbox
				id="checklist-item-{item.value}"
				checked={selected.has(item.value)}
				disabled={item.disabled}
				onchange={() => toggle(item)}
			/>
```

Note: `checked=` (not `bind:checked=`) is correct here — this is a controlled checkbox driven by `selected.has(item.value)`, not bound to a reactive variable.

- [ ] **Step 3: Run type check + Vitest for CheckboxList**

```bash
cd frontend && npm run check 2>&1 | tail -5 && npm run test -- --run src/lib/components/*.test.ts --reporter=verbose 2>&1 | grep -E 'FAIL|PASS|Error' | head -20
```

Expected: zero errors, all tests pass.

- [ ] **Step 4: Commit**

```bash
cd frontend && git add src/lib/components/CheckboxList.svelte
git commit -m "fix(frontend): CheckboxList raw checkbox → Checkbox with dynamic id"
```

---

## Task 7: AssignToHostModal — inputs and checkboxes

**Files:**

- Modify: `frontend/src/lib/components/AssignToHostModal.svelte:302,339,465`

Two occurrences of `<input class="input text-sm">` (lines 339 and 465) and one
`<input type="checkbox" class="checkbox">` (line 302). The checkboxes in the role assignment
table likely lack `id` — derive from role context.

- [ ] **Step 1: Audit all raw form elements**

```bash
grep -n '<input' frontend/src/lib/components/AssignToHostModal.svelte
```

Note the exact lines and whether each has an `id`.

- [ ] **Step 2: Check existing imports**

```bash
grep -n "^import.*Input\|^import.*Checkbox" frontend/src/lib/components/AssignToHostModal.svelte
```

Add missing imports.

- [ ] **Step 3: Replace the checkbox (line ~302)**

The checkbox enables/disables a role assignment. The `role` variable is in scope (from the surrounding template loop):

Current:

```svelte
<input
	class="checkbox"
	type="checkbox"
	bind:checked={standardAssignments[role].enabled}
	onchange={() => {
		if (!standardAssignments[role].enabled) {
			standardAssignments[role].plugin_config_id = '';
		}
		clearStandardRoleError(role);
		assignmentError = null;
	}}
/>
```

Replace with (`role` is a string in scope, use it for the id):

```svelte
<Checkbox
	id="assign-role-{role}-enabled"
	bind:checked={standardAssignments[role].enabled}
	onchange={() => {
		if (!standardAssignments[role].enabled) {
			standardAssignments[role].plugin_config_id = '';
		}
		clearStandardRoleError(role);
		assignmentError = null;
	}}
/>
```

- [ ] **Step 4: Replace the text inputs (lines ~339, ~465)**

Both are package identifier inputs in role rows. Current:

```svelte
<input
	class="input text-sm"
	type="text"
	placeholder="e.g. owner/repo"
	bind:value={standardAssignments[role].package_identifier}
	disabled={!a.enabled}
/>
```

Replace with (derive id from `role`):

```svelte
<Input
	id="assign-role-{role}-pkg-id"
	type="text"
	placeholder="e.g. owner/repo"
	bind:value={standardAssignments[role].package_identifier}
	disabled={!a.enabled}
	class="text-sm"
/>
```

There are two identical patterns (one for standard roles, one for hooks). Ensure both are migrated.

- [ ] **Step 5: Run type check + Vitest**

```bash
cd frontend && npm run check 2>&1 | tail -5 && npm run test -- --run src/lib/components/AssignToHostModal.test.ts --reporter=verbose
```

Expected: zero errors, all tests pass.

- [ ] **Step 6: Commit**

```bash
cd frontend && git add src/lib/components/AssignToHostModal.svelte
git commit -m "fix(frontend): AssignToHostModal raw inputs/checkbox → Input/Checkbox with role-derived ids"
```

---

## Task 8: SoftwareMergeWizard search input

**Files:**

- Modify: `frontend/src/lib/components/SoftwareMergeWizard.svelte:261`

One `<input class="input" type="search">` with `onkeydown` and `aria-label`. Both are forwarded via rest props (verified in Task 1).

- [ ] **Step 1: Add Input import if not already present**

```bash
grep -n "^import.*Input" frontend/src/lib/components/SoftwareMergeWizard.svelte
```

Add `import Input from '$lib/components/Input.svelte';` if missing.

- [ ] **Step 2: Replace the search input**

Current:

```svelte
<input
	class="input"
	type="search"
	bind:value={searchQuery}
	placeholder="Search software items"
	aria-label="Search software items"
	disabled={loading}
	onkeydown={(event) => {
		if (event.key !== 'Enter') return;
		event.preventDefault();
		void runSearch();
	}}
/>
```

Replace with (`id` required — derive from context):

```svelte
<Input
	id="software-merge-search"
	type="search"
	bind:value={searchQuery}
	placeholder="Search software items"
	aria-label="Search software items"
	disabled={loading}
	onkeydown={(event) => {
		if (event.key !== 'Enter') return;
		event.preventDefault();
		void runSearch();
	}}
/>
```

- [ ] **Step 3: Run type check + Vitest**

```bash
cd frontend && npm run check 2>&1 | tail -5 && npm run test -- --run src/lib/components/SoftwareMergeWizard.test.ts src/lib/components/software-merge-wizard.test.ts --reporter=verbose
```

Expected: zero errors, all tests pass.

- [ ] **Step 4: Commit**

```bash
cd frontend && git add src/lib/components/SoftwareMergeWizard.svelte
git commit -m "fix(frontend): SoftwareMergeWizard raw search input → Input"
```

---

## Task 9: software/+page.svelte — checkboxes and inputs

**Files:**

- Modify: `frontend/src/routes/software/+page.svelte:889,940,972,1487,1499`

4 checkboxes (one at line 940 uses `indeterminate`) + 2 text inputs (lines 1487, 1492).

- [ ] **Step 1: Audit all raw form elements in software/+page.svelte**

```bash
grep -n '<input' frontend/src/routes/software/+page.svelte
```

Identify lines 889, 940, 972, 1487, 1492, 1499 (and confirm 940 has `indeterminate`).

- [ ] **Step 2: Check existing imports**

```bash
grep -n "^import.*Input\|^import.*Checkbox" frontend/src/routes/software/+page.svelte
```

Add missing imports.

- [ ] **Step 3: Fix checkbox at line ~889 — show-updatable-only filter**

```svelte
<input
	class="checkbox"
	type="checkbox"
	bind:checked={showUpdatableOnly}
	onchange={() => {
		currentPage = 1;
		loadAll(1);
	}}
/>
```

→

```svelte
<Checkbox
	id="software-filter-updatable-only"
	bind:checked={showUpdatableOnly}
	onchange={() => {
		currentPage = 1;
		loadAll(1);
	}}
/>
```

- [ ] **Step 4: Fix checkbox at line ~940 — batch select-all (has `indeterminate`)**

```svelte
<input
	type="checkbox"
	class="checkbox"
	checked={allBatchPageSelected}
	indeterminate={!allBatchPageSelected && batchSelectedIds.size > 0}
	onchange={toggleBatchSelectAll}
	aria-label="Select all"
/>
```

→

```svelte
<Checkbox
	id="software-batch-select-all"
	checked={allBatchPageSelected}
	indeterminate={!allBatchPageSelected && batchSelectedIds.size > 0}
	onchange={toggleBatchSelectAll}
	aria-label="Select all"
/>
```

Note: `indeterminate` is forwarded via rest props (verified in Task 1). If `npm run check`
reports a type error for `indeterminate`, add it to the Checkbox component's TypeScript
interface as `indeterminate?: boolean` before proceeding.

- [ ] **Step 5: Fix per-row checkbox at line ~972**

```svelte
<input
	type="checkbox"
	class="checkbox"
	checked={batchSelectedIds.has(item.id)}
	onchange={() => toggleBatchSelect(item.id)}
	aria-label={'Select ' + item.name}
/>
```

→

```svelte
<Checkbox
	id="software-row-{item.id}"
	checked={batchSelectedIds.has(item.id)}
	onchange={() => toggleBatchSelect(item.id)}
	aria-label={'Select ' + item.name}
/>
```

- [ ] **Step 6: Fix text inputs in the edit form (lines ~1487, ~1492)**

Line ~1487:

```svelte
<input class="input" type="text" bind:value={editForm.name} />
```

→

```svelte
<Input id="software-edit-name" bind:value={editForm.name} />
```

Line ~1492 (icon URL):

```svelte
<input class="input" type="text" bind:value={editForm.icon_url} placeholder="https://example.com/icon.png" />
```

→

```svelte
<Input id="software-edit-icon-url" bind:value={editForm.icon_url} placeholder="https://example.com/icon.png" />
```

- [ ] **Step 7: Fix checkbox in edit form at line ~1499 (featured toggle)**

```svelte
<input class="checkbox" type="checkbox" bind:checked={editForm.featured} />
```

→

```svelte
<Checkbox id="software-edit-featured" bind:checked={editForm.featured} />
```

- [ ] **Step 8: Run type check + Vitest**

```bash
cd frontend && npm run check 2>&1 | tail -5 && npm run test -- --run "src/routes/software/" --reporter=verbose 2>&1 | tail -20
```

Expected: zero errors, all tests pass.

- [ ] **Step 9: Commit**

```bash
cd frontend && git add "src/routes/software/+page.svelte"
git commit -m "fix(frontend): software list page raw checkboxes/inputs → Checkbox/Input primitives"
```

---

## Task 10: software/[id]/+page.svelte and IgnoreRulesTab.svelte

**Files:**

- Modify: `frontend/src/routes/software/[id]/+page.svelte:1119,1129`
- Modify: `frontend/src/routes/software/IgnoreRulesTab.svelte:162,184,249`

- [ ] **Step 1: Fix software/[id]/+page.svelte**

Check imports:

```bash
grep -n "^import.*Input\|^import.*Checkbox" "frontend/src/routes/software/[id]/+page.svelte"
```

Line ~1119 (name input):

```svelte
<input class="input" type="text" bind:value={editForm.name} />
```

→

```svelte
<Input id="software-detail-edit-name" bind:value={editForm.name} />
```

Line ~1123 (icon URL input — also in scope, check if present):

```svelte
<input class="input" type="text" bind:value={editForm.icon_url} placeholder="https://example.com/icon.png" />
```

→

```svelte
<Input id="software-detail-edit-icon-url" bind:value={editForm.icon_url} placeholder="https://example.com/icon.png" />
```

Line ~1129 (featured checkbox):

```svelte
<input class="checkbox" type="checkbox" bind:checked={editForm.featured} />
```

→

```svelte
<Checkbox id="software-detail-edit-featured" bind:checked={editForm.featured} />
```

- [ ] **Step 2: Fix IgnoreRulesTab.svelte**

Check imports:

```bash
grep -n "^import.*Input\|^import.*Checkbox" frontend/src/routes/software/IgnoreRulesTab.svelte
```

Line ~162 (select-all checkbox — has `indeterminate`):

```svelte
<input
	type="checkbox"
	class="checkbox"
	checked={ignores.length > 0 && ignoreSelectedIds.size === ignores.length}
	indeterminate={ignoreSelectedIds.size > 0 && ignoreSelectedIds.size < ignores.length}
	onchange={toggleIgnoreSelectAll}
	aria-label="Select all"
/>
```

→

```svelte
<Checkbox
	id="ignore-rules-select-all"
	checked={ignores.length > 0 && ignoreSelectedIds.size === ignores.length}
	indeterminate={ignoreSelectedIds.size > 0 && ignoreSelectedIds.size < ignores.length}
	onchange={toggleIgnoreSelectAll}
	aria-label="Select all"
/>
```

Line ~184 (per-row checkbox in `{#each}` — derive id from `ignore.id`):

```svelte
<input
	type="checkbox"
	class="checkbox"
	checked={ignoreSelectedIds.has(ignore.id)}
	onchange={() => toggleIgnoreSelect(ignore.id)}
	aria-label="Select {ignore.name}"
/>
```

→

```svelte
<Checkbox
	id="ignore-rule-{ignore.id}"
	checked={ignoreSelectedIds.has(ignore.id)}
	onchange={() => toggleIgnoreSelect(ignore.id)}
	aria-label="Select {ignore.name}"
/>
```

Line ~249 (text input for create form):

```svelte
<input class="input" type="text" placeholder="e.g. FreshRSS or Plex Media Server" bind:value={ignoreForm.name} />
```

→

```svelte
<Input id="ignore-rule-create-name" bind:value={ignoreForm.name} placeholder="e.g. FreshRSS or Plex Media Server" />
```

- [ ] **Step 3: Run type check + Vitest**

```bash
cd frontend && npm run check 2>&1 | tail -5 && npm run test -- --run \
  "src/routes/software/[id]/software-detail.test.ts" \
  src/routes/software/ignore-rules-tab.test.ts \
  --reporter=verbose
```

Expected: zero errors, all tests pass.

- [ ] **Step 4: Commit**

```bash
cd frontend && git add "src/routes/software/[id]/+page.svelte" src/routes/software/IgnoreRulesTab.svelte
git commit -m "fix(frontend): software detail and IgnoreRulesTab raw inputs/checkboxes → Input/Checkbox"
```

---

## Task 11: services/+page.svelte and system-services/+page.svelte

**Files:**

- Modify: `frontend/src/routes/services/+page.svelte:688`
- Modify: `frontend/src/routes/system-services/+page.svelte:661`

Only the text input at the specified line in each file is in scope (per audit). The batch-select checkboxes in these files are NOT in scope for this spec.

- [ ] **Step 1: Find the exact inputs at lines 688 and 661**

```bash
sed -n '685,695p' frontend/src/routes/services/+page.svelte
sed -n '658,668p' frontend/src/routes/system-services/+page.svelte
```

Expected: both show `<input class="input w-full" ...>` — a text/search filter input.

- [ ] **Step 2: Check imports**

```bash
grep -n "^import.*Input" frontend/src/routes/services/+page.svelte frontend/src/routes/system-services/+page.svelte
```

Add `import Input from '$lib/components/Input.svelte';` to files missing it.

- [ ] **Step 3: Fix services/+page.svelte (line ~688)**

Find:

```svelte
<input class="input w-full" ... bind:value={filterText} ... />
```

The exact pattern will vary; keep all existing attributes and add `id`, remove `class="input w-full"`, move `w-full` to `class` prop:

```svelte
<Input id="services-filter" bind:value={filterText} class="w-full" ... />
```

Preserve all other attributes (`placeholder`, `oninput`, etc.) unchanged.

- [ ] **Step 4: Fix system-services/+page.svelte (line ~661)**

Same pattern as above:

```svelte
<Input id="system-services-filter" bind:value={filterText} class="w-full" ... />
```

- [ ] **Step 5: Run type check + Vitest**

```bash
cd frontend && npm run check 2>&1 | tail -5 && npm run test -- --run \
  src/routes/services/services.test.ts \
  src/routes/system-services/system-services.test.ts \
  --reporter=verbose
```

Expected: zero errors, all tests pass.

- [ ] **Step 6: Commit**

```bash
cd frontend && git add src/routes/services/+page.svelte src/routes/system-services/+page.svelte
git commit -m "fix(frontend): services and system-services filter input → Input primitive"
```

---

## EditHostAssignmentModal — Note

The raw `<textarea class="textarea font-mono text-xs">` elements in `EditHostAssignmentModal.svelte`
(lines 770, 891, 1090, 1220) and raw `<input>` elements at lines 738 and 803 are **also in scope**
for this spec. They were NOT included in the tasks above because the token migration plan
(Task 4 in `2026-04-23-token-migration.md`) touches the same file. Migrate these form elements
as part of the token migration task for that file to avoid double-editing, OR as a separate
commit after the token migration is complete.

Three change types in EditHostAssignmentModal:

**Textareas at lines 770, 891, 1090, 1220** (`<textarea class="textarea font-mono text-xs">`):

```svelte
<!-- After — derive id from surrounding context (role, entry.localKey, etc.) -->
<Textarea id="edit-host-std-{role}-config" bind:value={...} rows={4} variant="mono" />
```

**Inputs at lines 738, 803** (`<input class="input text-sm">`):

```svelte
<!-- After -->
<Input id="edit-host-std-{role}-version" class="text-sm" bind:value={...} />
```

**Checkboxes at lines 789, 915, 1106** (`<input type="checkbox" class="checkbox">`):

```svelte
<!-- After — derive id from role or entry.localKey -->
<Checkbox id="edit-host-std-{role}-use-json" bind:checked={...} />
```

---

## Final verification

- [ ] **Confirm all 24 call sites are migrated**

```bash
cd frontend && grep -rn 'class="input\|class="checkbox\|class="textarea\|class="input ' src/lib/components/ src/routes/ --include="*.svelte" | grep -v '.test.ts'
```

Expected: no remaining `class="input"`, `class="checkbox"`, or `class="textarea"` on raw HTML
elements. (The primitives themselves will have these classes internally — those are fine.)

- [ ] **Run full type check**

```bash
cd frontend && npm run check 2>&1 | tail -10
```

Expected: zero type errors.

- [ ] **Run full Vitest suite**

```bash
cd frontend && npm run test -- --run 2>&1 | tail -10
```

Expected: all 811+ Vitest tests pass. If any test asserts `class="checkbox"` on a rendered
element and fails, update it to query by `[data-ui="checkbox"]` or the inner input role instead.

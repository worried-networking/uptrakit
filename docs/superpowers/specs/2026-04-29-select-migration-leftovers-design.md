# Select Migration — Leftover `<select>` Elements

**Date:** 2026-04-29
**Status:** Approved

## Overview

Migrate the 6 remaining raw `<select>` elements in `frontend/src/` to the `Select` form
primitive (`frontend/src/lib/components/forms/Select.svelte`). Extend the primitive with
the minimum API surface the leftover sites require: option groups, per-option `disabled`,
and a `width: 'full' | 'auto'` variant. Clean a stale regression-guard test in
`SchemaForm.test.ts` that was orphaned when `SchemaForm` migrated in commit `5964fdca`.

This is a follow-up to `2026-04-29-select-primitive-design.md`, which introduced the
primitive and migrated the eligible call sites. The 6 sites covered here were excluded
because the primitive lacked the API needed (optgroups, per-option disabled, non-full
width), or because the migration touched non-trivial state (services merge target's
`null` placeholder, ProviderSelector wrapper).

## Goals

- All `<select>` elements outside `forms/Select.svelte` migrated.
- `Select` primitive gains support for `<optgroup>`, per-option `disabled`, and
  `width="auto"`.
- ProviderSelector dead-code (empty branch, manual DOM revert hack) removed in the
  course of its migration — surfaced by the contrarian review as status-quo bias.
- No behavioural regressions at any of the 6 migration sites.

## Non-Goals

- Other form primitives (`Input`, `Textarea`, `Checkbox`) gaining `width` prop or
  `data-ui` markers. Separate cleanup if/when needed.
- Removing ProviderSelector's uncontrolled mode or `description` slot. Both are
  unused by current callers, but unlike the `emptyMessage` prop and revert hack
  (which actively interfered with the migration — the empty branch had to go for
  the `<Select>` swap, and the revert hack contradicted the documented controlled-
  mode contract), `description` and uncontrolled mode are passive surface area:
  they neither block this migration nor mislead readers about behaviour. Deferred
  as a separate cleanup with no migration coupling.
- Adding `tailwind-merge`. The `width` prop avoids the class-conflict scenario without
  a new dependency.
- Visual regression tests. Existing unit + manual eyeball check is sufficient for the
  primitive's complexity level.

## Inventory

| # | Site | Blocker today | Migration shape |
| --- | --- | --- | --- |
| 1 | `routes/services/+page.svelte:681` (merge target) | Placeholder uses `value={null}` | Drop `null`, default to `''` |
| 2 | `routes/software/+page.svelte:1059` (plugin filter) | Needs `w-auto` (toolbar context) | `width="auto"` |
| 3 | `lib/components/AssignToHostModal.svelte:344` (execution_site, table 1: detect_version + fetch_releases) | Conditional `controller` option for `fetch_releases` only | `$derived` options array via shared helper |
| 4 | `lib/components/EditHostAssignmentModal.svelte:706` (cfg picker) | Uses `<optgroup>` | `SelectGroup` discriminated union |
| 5 | `lib/components/EditHostAssignmentModal.svelte:1070` (hook cfg) | Uses `<optgroup>` | `SelectGroup` discriminated union |
| 6 | `lib/components/ui/ProviderSelector.svelte:55` | Per-option `disabled` + dead empty branch | Migrate inner select, drop dead code |

`SchemaForm.svelte` is already migrated (commit `5964fdca`) but `SchemaForm.test.ts:333`
still asserts a stale "not migrated — regression guard". Test passes incidentally
because `Select` renders a native `<select>` internally. Rewrite as a positive
`[data-ui="select"]` assertion.

## Primitive API Extension

**File:** `frontend/src/lib/components/forms/Select.svelte`

### Type definitions

```ts
export type SelectOption = {
    value: string;
    label: string;
    disabled?: boolean;
};

export type SelectGroup = {
    label: string;
    options: SelectOption[];
};

export type SelectItem = SelectOption | SelectGroup;

export type SelectProps = {
    id: string;
    value?: string;
    options: SelectItem[];
    width?: 'full' | 'auto';        // default 'full'
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

### Discriminated rendering

```svelte
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
```

### `$props()` destructure

Updated destructure (changes from current: add `width`, default `'full'`):

```ts
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
```

### Width variant

Keep `BASE` as a plain `const` (no width class). Compose width into `computedClass`:

```ts
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

const widthClass = $derived(width === 'auto' ? 'w-auto' : 'w-full');
const computedClass = $derived([BASE, widthClass, className].filter(Boolean).join(' '));
```

`BASE` stays static so its declaration position relative to `$props()` does not
change. Only the `$derived`s (`widthClass`, `computedClass`) must sit after the
`$props()` destructure.

**Tailwind JIT.** Both `'w-auto'` and `'w-full'` appear as full string literals in the
ternary, so JIT detects them at build time. No safelist required.

`width="full"` is the default and matches all current call sites. `width="auto"` lets
the select size to content — needed when the select sits in a `flex-wrap` toolbar
where stretching to fill the row is wrong.

**`className` does not override `width`.** `computedClass` joins as
`[BASE, widthClass, className]`, so a caller's `class="w-full"` would land after
`widthClass` and win by source order, defeating the prop. Callers must not pass
width utilities (`w-*`) via `class`; use the `width` prop instead. None of the six
migration sites does this today (Site 3b passes `class="text-sm"` — non-conflicting).

### `data-ui` marker

Add `data-ui="select"` to the inner `<select>` element. This is a one-off addition for
SchemaForm test purposes; broader form-primitive `data-ui` rollout (Input, Textarea,
Checkbox) is out of scope.

**Selector scope.** `ProviderSelector.svelte` already sets
`data-ui="provider-selector"` on its outer `<div>`. The new `data-ui="select"` lands
on the inner `<select>` rendered through the migrated primitive. A query for
`[data-ui="select"]` inside a tree containing ProviderSelector will match
ProviderSelector's inner select too. Tests should scope queries to the component
under test (`render(SchemaForm, ...)` does not mount ProviderSelector, so the
SchemaForm regression-guard rewrite is unaffected).

### Backwards compatibility

`SelectOption[]` is structurally assignable to `SelectItem[]` (subtype). All 7 existing
call sites compile unchanged. New `disabled?` field on `SelectOption` is optional.

### Empty group handling

Render-as-given. An empty `<optgroup>` is harmless visually (renders a non-selectable
label). Caller is responsible for filtering empty groups from the options array.
Document on the `SelectGroup` type.

## Per-Site Migration Plans

**Ordering.** Extend the primitive (new types, `width` prop, `data-ui` marker, group
rendering) before migrating any call site — every site below depends on the new API
shape. Land the primitive change as its own commit, then migrate sites one per
commit.

### Site 1 — `routes/services/+page.svelte:681` (merge target)

**State change.** Drop `null` from `mergeTargetId`:

```ts
let mergeTargetId = $state('');
```

Update the three reset sites (lines 221, 226, 237) to assign `''` instead of `null`.
The truthy check at line 690 (`disabled={!mergeTargetId}`) keeps working.

**Markup.**

```svelte
<Select
    id="merge-target"
    bind:value={mergeTargetId}
    placeholder="-- Select a service --"
    options={mergeTargetOptions}
/>
```

**Options derivation.**

```ts
const mergeTargetOptions = $derived(
    services
        .filter((s) =>
            s.status === 'approved'
            && s.capabilities.includes('software_discovery')
            && s.id !== mergeSource?.id)
        .map((t) => ({
            value: t.id,
            label: `${t.friendly_name} (${t.hostname})`
        }))
);
```

### Site 2 — `routes/software/+page.svelte:1059` (plugin filter)

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
        onchange={() => { currentPage = 1; loadAll(1); }}
    />
{/if}
```

### Site 3 — `AssignToHostModal.svelte:344` (execution_site)

**Two-table context.** `AssignToHostModal.svelte` has two role-iteration tables:

- **Table 1** (line 296, `{#each ['detect_version', 'fetch_releases']}`) — line 344 is
  the raw `<select>` to migrate. The `controller` option appears only for
  `fetch_releases`.
- **Table 2** (later, `{#each ['execute_update']}`) — line 468 is already a `<Select>`
  with hardcoded `[{value:'auto'},{value:'agent'}]`.

After migration, both tables should consume the same helper to avoid drift.

**Helper.** The role parameter type is `StandardRoleKey` (already defined locally in
`AssignToHostModal.svelte` at line 32 as `'detect_version' | 'fetch_releases' |
'execute_update'`). Do not introduce a new alias.

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

**Site 3a markup (table 1, line 344 — raw `<select>` migration).**

```svelte
<Select
    id="assign-role-{role}-execution-site"
    bind:value={standardAssignments[role].execution_site}
    disabled={!a.enabled}
    options={executionSiteOptions(role)}
/>
```

**Site 3b markup (table 2, line 468 — replace hardcoded array with helper call).**

```svelte
<Select
    id="assign-role-{role}-execution-site"
    class="text-sm"
    bind:value={standardAssignments[role].execution_site}
    disabled={!a.enabled}
    options={executionSiteOptions(role)}
/>
```

For `execute_update`, helper returns `[auto, agent]` — identical to existing inline
array. No behavioural change at line 468; one-line refactor for DRY.

**Id uniqueness invariant.** Both tables emit `id="assign-role-{role}-execution-site"`.
Table 1 iterates `['detect_version', 'fetch_releases']`, Table 2 iterates
`['execute_update']`. The role keys are disjoint, so no collision. If either iteration
expands to overlap with the other, the ids must be re-prefixed (e.g.,
`assign-table1-role-{role}-...`).

### Sites 4 + 5 — `EditHostAssignmentModal.svelte:706` + `:1070` (plugin config)

**Barrel re-exports.** `frontend/src/lib/components/forms/index.ts` currently
re-exports `SelectProps` + `SelectOption`. The barrel must add `SelectGroup` and
`SelectItem`:

```ts
export type { SelectProps, SelectOption, SelectGroup, SelectItem } from './Select.svelte';
```

**Type imports.** `EditHostAssignmentModal.svelte` (and any other file using the
helper signature) imports `SelectItem` from the forms barrel — but NOT
`SelectOption`, because `EditHostAssignmentModal.svelte` already imports a
different `SelectOption` (the SchemaForm field shape) from `$lib/types`.
Importing both would create a duplicate-identifier collision:

```ts
import { Select, type SelectItem } from '$lib/components/forms';
// existing line stays untouched:
//   import { ..., SelectOption, ... } from '$lib/types';
```

Inside the helper, the placeholder is written as an inline object literal with
no type annotation — TypeScript infers a structurally compatible shape that
satisfies both `SelectGroup.options[number]` and `SelectItem`. Files that do
not have a pre-existing `SelectOption` import from `$lib/types` may import
`SelectOption` from the forms barrel as well; only `EditHostAssignmentModal`
has the collision today.

**Helper (shared by both sites).**

```ts
function pluginConfigItems(
    savedOpts: PluginConfig[],
    typeOpts: PluginType[]
): SelectItem[] {
    // Inline literal — do NOT annotate with `SelectOption`. The `$lib/types`
    // `SelectOption` already imported in this file has the same shape but is a
    // different nominal type than the forms barrel's `SelectOption`. Letting
    // TypeScript infer keeps the placeholder structurally compatible with
    // both `SelectItem` and `SelectGroup.options[number]`.
    const placeholder = { value: '', label: '— not configured —' };
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

Empty `Inline` group is filtered to avoid rendering a bare optgroup label with no
options (visible but unselectable). `Saved` group is gated by `savedOpts.length === 0`
in the early return, so the inverse footgun does not apply.

The `'— not configured —'` entry is rendered as a flat ungrouped option above the
optgroups; the primitive's discriminated render supports this mix.

**Placeholder value invariant.** The placeholder uses `value: ''`. Other options use
the `cfg:<id>` or `type:<plugin_type>` prefix. Plugin config ids and plugin type
names come from the DB (`cfg.id`, `pt.plugin_type`) and are never empty, so no other
option collapses to `''`. `applySelection(state, '')` already treats the empty string
as "not configured" — the placeholder reuses that path intentionally.

**Site 4 markup.**

```svelte
<Select
    id="cfg-{role}"
    value={pluginSelection(s)}
    options={pluginConfigItems(savedRoleOpts, typeOpts)}
    onchange={(e) => applySelection(standardStates[role], (e.target as HTMLSelectElement).value)}
/>
```

**Site 5 markup.**

```svelte
<Select
    id="hook-cfg-{entry.localKey}"
    value={pluginSelection(entry)}
    options={pluginConfigItems(savedHookOpts, hookTypeOpts)}
    onchange={(e) => applySelection(entry, (e.target as HTMLSelectElement).value)}
/>
```

**Pass-through binding contract.** Both sites use `value={pluginSelection(...)}` (no
`bind:value`) because the displayed value is derived from caller state. Correctness
relies on the existing `$state` proxy:

- `standardStates` — `EditHostAssignmentModal.svelte:120`,
  `let standardStates: ... = $state(makeInitialStandardStates())`.
- `entry` — element of `hookLists` at line 123, also `$state`.

`applySelection` mutates the proxy in place; the next render re-evaluates
`pluginSelection(s)` and pushes the new value back through the `value` prop. If the
declaration ever loses the `$state` wrapper, the pass-through pattern silently
desynchronises (DOM holds user pick; `pluginSelection` stays stale). Add an
acceptance check.

**Identity round trip.** Today, `applySelection` is value-preserving:
`pluginSelection(applySelection(target, v))` equals `v` for every input the
primitive can emit (`cfg:<id>`, `type:<pt>`, `''`). It does not reject, normalise,
or remap. If a future change makes `applySelection` non-identity (e.g., maps invalid
ids to `''`), the pass-through pattern will silently desync — the `<select>` DOM
keeps the user's pick while `pluginSelection` returns the normalised value. At that
point either switch to `bind:value` with explicit revert in the parent, or add a
keyed remount.

### Site 6 — `ProviderSelector.svelte`

**Dead-code removal (surfaced by contrarian review).**

Both call sites in `SurfaceReadPanel.svelte` short-circuit empty-providers state
before reaching ProviderSelector:

- Line 354 renders `<EmptyState>` when `availableProviders.length === 0`, never
  rendering ProviderSelector with empty providers.
- Line 397 always prepends a synthetic `{ id: '', label: all_option_label }` entry,
  so providers is never empty.

The `emptyMessage` prop and the `{#if providers.length === 0}` branch are unreachable
in production. The manual `select.value = currentId` revert hack defends against a
parent that conditionally rejects changes, but both callers accept changes
unconditionally — the hack is defensive code for a use case that does not exist.

Actions:

- Remove `emptyMessage` prop (lines 14, 20).
- Remove `{#if providers.length === 0}` branch (lines 61-62).
- Remove manual DOM revert (`select.value = currentId` in `handleChange`).
- Remove `disabled={providers.length === 0}` from inner `<select>` (line 58). The
  empty branch is gone and both `SurfaceReadPanel.svelte` callers always pass
  non-empty providers (line 354 short-circuits with `<EmptyState>`; line 397
  prepends a synthetic `all` entry). Carrying the attribute into the `<Select>`
  call would disable the whole control on a state that is now unreachable.
- Add new required `id: string` prop.

**Caller updates in `SurfaceReadPanel.svelte`.**

- Line 358: add `id="surface-provider"`.
- Line 395: add `id="surface-context-{contextSelector.param_key}"`. `param_key` is
  guaranteed present (line 50 reads `descriptor.context_selector`, and `param_key` is
  used at line 54 to key `baseParams`); this gives a stable, unique id per surface.

**Markup after migration.**

```svelte
<div class="space-y-2" data-ui="provider-selector">
    <label for={id} class="block space-y-2">
        <span class="text-sm font-medium text-[var(--text-primary)]">{label}</span>
    </label>
    <Select
        {id}
        value={currentId}
        options={providers.map((p) => ({
            value: p.id,
            label: p.label,
            disabled: p.disabled
        }))}
        onchange={handleChange}
    />
    {#if selectedProvider?.description}
        <p class="text-sm text-[var(--text-secondary)]">{selectedProvider.description}</p>
    {/if}
</div>
```

**`handleChange` simplifies to:**

```ts
function handleChange(event: Event): void {
    const nextId = (event.currentTarget as HTMLSelectElement).value;
    if (!isControlled) {
        uncontrolledId = nextId;
    }
    onSelect?.(nextId);
}
```

**Caller assumption.** Both current `ProviderSelector` callers in
`SurfaceReadPanel.svelte` accept `onSelect` changes unconditionally:

- Line 367: `onSelect={(providerId) => { selectedProviderId = providerId; }}`
- Line 400: `onSelect={(id) => { selectedContextValue = id; }}`

So the manual `select.value = currentId` revert hack defended a controlled-parent
scenario that does not exist in the codebase: a parent that ignores or rejects the
new value while leaving `selectedId` unchanged. If a future caller needs reject
semantics, that caller owns the revert (e.g., by toggling a `key` on
`ProviderSelector` to force remount, or by setting `selectedId` back to the previous
value through its own state) — not the primitive.

This is documented on `ProviderSelector` so the contract is explicit.

## Test Plan

### Primitive tests (`Select.test.ts`)

Add the following cases (existing tests remain valid):

1. **Renders `<optgroup>` with label + nested options.**
2. **Disabled option not selectable** (assert `<option>.disabled === true`).
3. **Mixed flat + grouped renders both correctly.**
4. **Placeholder coexists with groups** — placeholder is first child, optgroups
   follow.
5. **`width="auto"` renders `w-auto` not `w-full`; default renders `w-full`.**
6. **`data-ui="select"` attribute is present** on inner `<select>`.
7. **`bind:value` round-trip through grouped option** — `fireEvent.change` picks an
   option nested inside an `<optgroup>` (e.g. `cfg:2`); assert the bound variable
   updates after the next microtask. Closes the gap between Site 1's `bind:value`
   path and Sites 4/5's pass-through path.

### Migration site tests — hybrid grep audit

**Step 1 (~10 min):** pre-migration grep to catalogue affected tests.

```bash
rg "querySelector.*['\"]select" frontend/src --type ts
rg "toContain.*['\"]w-full['\"]|toContain.*['\"]\\.select['\"]" frontend/src --type ts
rg "HTMLSelectElement" frontend/src --type ts
```

Classify each match: **no-op** (still matches because primitive renders `<select>`),
**update needed** (asserts on dropped class strings or DOM structure that changes),
or **delete** (covers behaviour now owned by primitive).

**Step 2:** fix per-site during each migration commit. No upfront test rewrite.

### `SchemaForm.test.ts:333` rewrite

The existing `it('select field renders raw <select> (not migrated — regression
guard)', ...)` block (line 333) is replaced wholesale by the block below. The stale
regression-guard comment goes with it. `FormField` and `apiGet` imports are already
present in the test file — no new imports needed.

```ts
it('select field renders Select primitive', async () => {
    const loadInitialValues = vi.fn().mockResolvedValue({});
    vi.mocked(apiGet).mockResolvedValue([]);
    const { container } = render(SchemaForm, {
        fields: [{
            key: 'region', label: 'Region', field_type: 'select', required: false,
            options: [{ value: 'eu', label: 'EU' }]
        }] satisfies FormField[],
        loadInitialValues,
        onsubmit: vi.fn().mockResolvedValue(undefined)
    });
    await waitFor(() => expect(loadInitialValues).toHaveBeenCalled());
    expect(container.querySelector('[data-ui="select"]')).toBeInTheDocument();
});
```

### ProviderSelector test (`ProviderSelector.test.ts`)

Existing tests use `screen.getByLabelText('Provider')` to obtain the inner select
(currently works via implicit-label wrapping). After migration, the wrapper renders
`<label for={id}>` separated from `<select id={id}>` — `getByLabelText` still works
**but only if every `render()` call passes the new required `id` prop**. Test plan:

- **Update every `render(ProviderSelector, ...)` call** to include `id: 'test-select'`
  (or similar). Without this, ProviderSelector stops compiling (TS catches it) and
  even if forced, `getByLabelText` returns nothing because `for` points at no element.
- **Drop:** class-string assertions (`w-full`, `rounded-card`, etc.) — primitive owns
  these now.
- **Drop:** any test of empty branch — branch deleted with `emptyMessage` prop.
- **Keep / add:** behavioural assertions — `onSelect` fires with correct id;
  controlled mode reflects `selectedId` prop; disabled option in providers list
  renders `<option disabled>`; uncontrolled mode (until removed in follow-up).

## Preview Demo Update

`frontend/src/routes/dev/form-primitive-preview/+page.svelte` — add one canonical
mixed-mode demo:

```svelte
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
```

Covers placeholder, two groups, and a disabled option in one view.

## Acceptance Criteria

### Primitive extension

- [ ] `SelectOption | SelectGroup` discriminated union compiles and exports.
- [ ] `frontend/src/lib/components/forms/index.ts` re-exports `SelectGroup` +
      `SelectItem` alongside existing `SelectProps` + `SelectOption`.
- [ ] `width: 'full' | 'auto'` prop, default `'full'`. `w-full` ↔ `w-auto` swap
      verified by test.
- [ ] `data-ui="select"` attribute on inner `<select>`.
- [ ] All 7 new primitive test cases pass.
- [ ] Preview demo renders without console errors / type errors.
- [ ] Existing 7 call sites compile and pass tests with no source changes.

### Each migration

- [ ] No `<select>` element at the migrated site (verified by grep).
- [ ] `<Select>` rendered with correct options shape (flat or grouped).
- [ ] Existing happy-path test for the page/component passes (or is updated if it
      asserted on dropped DOM/classes).
- [ ] No new TypeScript errors.
- [ ] No new ESLint warnings.
- [ ] Manual eyeball check: width, placeholder, group separators render correctly
      in dev server.
- [ ] Sites 4 + 5: `EditHostAssignmentModal.svelte` still declares `standardStates`
      (line 120) and `hookLists` (line 123) as `$state`. Pass-through binding
      contract relies on this; document or re-verify if the declarations move.

### ProviderSelector cleanup

- [ ] `emptyMessage` prop removed.
- [ ] `{#if providers.length === 0}` branch removed.
- [ ] Manual `select.value = currentId` revert hack removed.
- [ ] Both `SurfaceReadPanel.svelte` callers pass `id`.
- [ ] `ProviderSelector.test.ts` slimmed (no class-string assertions, no empty-branch
      tests).
- [ ] Behavioural tests pass: `onSelect` fires with correct id, controlled mode
      reflects `selectedId` prop.

### Repo-wide

- [ ] `grep -rn "<select" frontend/src --include="*.svelte" | grep -v "forms/Select.svelte"`
      returns zero matches. Plain substring match is intentional: BSD `grep -E` does
      not expand `\s`, so a `<select[\s>]` regex would silently miss `<select\n<attr>`
      tags on macOS. Plain `<select` is also unambiguous because no `<selection>` /
      `<select-*>` element exists in this Svelte tree.
- [ ] `npm run check` clean.
- [ ] `npm run lint` clean.
- [ ] `npm run format:check` clean.
- [ ] `npm run test` green.
- [ ] `npm run build` green.
- [ ] `SchemaForm.test.ts:333` test asserts `[data-ui="select"]`, stale comment gone.

## Risks and Mitigations

| Risk | Likelihood | Mitigation |
| --- | --- | --- |
| Existing site test asserts on dropped class string (`w-full`, `select`) | Medium | Hybrid grep audit (Step 1) catalogues these before migration |
| ProviderSelector caller missed during `id` prop addition (TypeScript catches it) | Low | New required prop; `tsc` errors at compile time |
| Site #1 reset point for `mergeTargetId` missed (3 sites: 221, 226, 237) | Low | Listed explicitly; grep confirms no other writers |
| Optgroup rendering looks visually wrong (group label spacing, separators) | Low | Native browser rendering; manual eyeball check on dev server |
| `width="auto"` interacts badly with FormFieldRow grid in some site | Low | Site #2 is the only `width="auto"` caller; not used inside FormFieldRow |
| Status-quo bias preserved a behaviour that should be revisited | Mitigated | Contrarian review surfaced ProviderSelector dead branch; bias sweep run on remaining decisions |

## Out-of-Scope Follow-ups

- ProviderSelector uncontrolled mode removal (`uncontrolledId` + `$effect` defending
  against an unused use case).
- ProviderSelector `description` slot removal (dead in current callers; low-cost
  defensive API).
- `data-ui` markers across other forms primitives (`Input`, `Textarea`, `Checkbox`)
  for consistency with the rest of the UI library.
- `width` prop on other forms primitives if non-full-width need arises.

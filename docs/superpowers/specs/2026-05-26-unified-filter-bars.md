# Unified Filter Bars & URL-Reactive Filter State

**Date:** 2026-05-26
**Status:** Spec — ready for planning

---

## Problem

Two related issues across all table pages:

1. **URL reactivity bug.** Filter state is initialised from `page.url.searchParams` at
   script-evaluation time via `$state(page.url.searchParams.get(...))`. When the page is
   already mounted and the URL changes externally — e.g. clicking a nav badge that appends
   `?updatable=true` — the Svelte component script does not re-run. The filter `$state`
   retains its stale value, and the `$effect` that writes state→URL immediately overwrites the
   incoming params, silently discarding them.

2. **Inconsistent filter UX.** Filter controls live in different locations on different pages:
   inline in the table header on `/software` (the intended pattern), in a separate
   `SectionCard` above the table on `/history`, `/services`, `/system-services`, and in a
   separate `SectionCard` on `/host-tags`. Enum filters use accent button groups on some pages
   and `<Select>` on others. The `/software` "All / Featured / Unfeatured" split is implemented
   as tabs above the table rather than as an inline filter control.

---

## Goals

- Fix the URL-reactivity bug on all in-scope pages so external navigation immediately applies
  filters.
- Standardise filter placement: filters are always integrated into the table card header row.
- Extract three reusable shared artifacts (`createUrlParam`, `FilterBar`, `ExpandableSearch`).
- All enum filters use `<Select>`; all text searches use `ExpandableSearch`.

---

## Scope

**In scope:** `/software`, `/host-tags`, `/history`, `/services`, `/system-services`, `/hosts`.

**Deferred:** `/audit-logs` (many specialised filter fields; separate spec).
**Deferred:** Backend extension of `getHosts()` to support hostname name search (API does not
exist yet; frontend wiring is included, backend is follow-up).

---

## URL-as-Source-of-Truth Pattern

`page` from `$app/state` is reactive in Svelte 5. `page.url.searchParams` updates whenever
SvelteKit processes a navigation — including `replaceState` navigations triggered from within
the same mounted component. The fix is to make all filter values `$derived` from
`page.url.searchParams` instead of `$state` copies.

**Before (broken):**

```svelte
<script lang="ts">
  // Initialised once at script evaluation; never updates when URL changes externally.
  let showUpdatableOnly = $state(page.url.searchParams.get('updatable') === 'true');

  $effect(() => {
    // Writes state → URL. When URL changes externally, this immediately overwrites it.
    const parts: string[] = [];
    if (showUpdatableOnly) parts.push('updatable=true');
    goto(`...?${parts.join('&')}`, { replaceState: true, ... });
  });
</script>
```

**After (correct):**

```svelte
<script lang="ts">
  // Derived from URL; updates reactively whenever the URL changes.
  const updatable = createUrlParam('updatable', {
    parse: (raw) => raw === 'true',
    serialize: (v) => (v ? 'true' : null),
  });

  // Data-loading effect reads from derived values directly.
  $effect(() => {
    const _dep = updatable.value; // explicit dependency
    loadAll(1);
  });
</script>
```

User interactions call `updatable.set(true)`, which calls `goto()` with `replaceState: true`.
This updates `page.url`, which updates `updatable.value`, which triggers the data-loading
effect. No separate URL-sync `$effect` is needed.

**Page reset on filter change:** When `createUrlParam.set()` is called, it always omits `page=`
from the new URL (equivalently, writes `page=1`). This resets pagination on every filter
mutation. External navigation that sets a filter param but not `page=` also lands on page 1
because the absent param is parsed as the default.

---

## Shared Artifacts

### 1. `frontend/src/lib/url-params.svelte.ts`

```typescript
import { page } from "$app/state";
import { goto } from "$app/navigation";

export interface UrlParamOptions<T> {
  parse?: (raw: string | null) => T;
  serialize?: (value: T) => string | null;
}

export interface UrlParam<T> {
  readonly value: T;
  set(value: T): void;
}

export function createUrlParam<T = string>(
  key: string,
  options?: UrlParamOptions<T>,
): UrlParam<T> {
  const parse = options?.parse ?? ((raw) => (raw ?? "") as unknown as T);
  const serialize =
    options?.serialize ?? ((v) => (v === "" || v == null ? null : String(v)));

  // $derived is valid here because createUrlParam is always called at component
  // initialisation time (top-level of <script> blocks in .svelte files or
  // top-level of .svelte.ts modules). Do NOT call createUrlParam inside
  // callbacks or non-reactive functions.
  const derived = $derived(parse(page.url.searchParams.get(key)));

  return {
    get value() {
      return derived;
    },
    set(value: T) {
      const next = new URL(page.url.href);
      const serialized = serialize(value);
      if (serialized == null) {
        next.searchParams.delete(key);
      } else {
        next.searchParams.set(key, serialized);
      }
      // Always reset to page 1 on filter change.
      next.searchParams.delete("page");
      void goto(next, { replaceState: true, keepFocus: true, noScroll: true });
    },
  };
}
```

- `derived` is a `$derived` rune backed by `page.url.searchParams`. It re-evaluates whenever
  `page.url` changes (including external navigation). The getter exposes it to consumers.
- `set()` builds a new `URL` from `page.url.href` (string form, avoids URL-to-URL constructor
  ambiguity across environments), mutates only the target key and resets `page=`, then calls
  `goto()` with `replaceState: true`.
- **Constraint:** `createUrlParam` must be called at component initialisation scope only
  (top-level `<script>` or `.svelte.ts` module scope). Calling it inside an event handler,
  conditional, or `{#each}` block will cause a Svelte rune-outside-reactive-context error at
  runtime. This constraint is not compiler-enforced — add an eslint `no-restricted-syntax` rule
  targeting `createUrlParam` calls in non-top-level positions if the project ESLint config
  supports it, or document in the component's JSDoc.
- **`goto` + `load()` interaction:** `goto(replaceState: true)` triggers SvelteKit's router,
  which will re-run any `+page.ts` / `+page.server.ts` `load()` functions that depend on
  `url.searchParams`. All six in-scope pages fetch data client-side via `$effect` (no server
  `load` reading URL params), so this is a no-op for data loading. If a future page has a
  server `load` reading search params, it should pass `invalidate` selectively to avoid full
  re-runs on every keystroke.
- Built-in convenience: boolean params (`parse: (r) => r === 'true'`, `serialize: (v) => v ?
'true' : null`), enum params (`parse: (r) => VALID.includes(r as T) ? r as T : default`).

**Text input debounce pattern** (implemented at the call site in `ExpandableSearch` or
page-level for custom inputs):

```svelte
<script lang="ts">
  const queryParam = createUrlParam('query');
  // Do NOT initialise from queryParam.value here — use $state('') and let the
  // $effect below set the initial value. This avoids a stale-snapshot on mount.
  let localQuery = $state('');
  let debounceTimer: ReturnType<typeof setTimeout> | undefined;

  // Sync from URL (runs on mount and whenever URL changes externally).
  $effect(() => {
    localQuery = queryParam.value;
  });
</script>

<input
  value={localQuery}
  oninput={(e) => {
    localQuery = e.currentTarget.value;
    clearTimeout(debounceTimer);
    debounceTimer = setTimeout(() => queryParam.set(localQuery), 300);
  }}
/>
```

---

### 2. `frontend/src/lib/components/ui/FilterBar.svelte`

Layout shell for the table card header row. Purely presentational — no filter logic.

```typescript
// Props
{
  filters?: Snippet;   // left side: filter controls
  actions?: Snippet;   // right side: primary action buttons
}
```

Renders:

```svelte
<header class="flex flex-col gap-3 border-b border-[var(--border-subtle)] bg-[var(--bg-raised)] card-padding md:flex-row md:items-center md:justify-between">
  <div class="flex flex-wrap items-center gap-3">
    {@render filters?.()}
  </div>
  {#if actions}
    <div class="shrink-0">
      {@render actions()}
    </div>
  {/if}
</header>
```

- `data-ui="filter-bar"` on the root element for test selectors.
- Exported from `$lib/components/ui/index.ts`.

---

### 3. `frontend/src/lib/components/ui/ExpandableSearch.svelte`

Expandable search input widget. Used on `/software`, `/host-tags`, `/hosts`.

```typescript
// Props
{
  id: string;
  value: string;                   // controlled — the committed (URL-synced) value
  onchange: (v: string) => void;   // called after debounce; should call param.set()
  placeholder?: string;            // default: 'Search...'
  debounceMs?: number;             // default: 300
}
```

Behaviour:

- **Collapsed state:** A ghost `<Button size="sm">` with `<Search size={14} aria-hidden />`.
  Always collapsed when `value === ''` and the user has not yet clicked to expand.
  If `value !== ''` (i.e. an active filter), the input is always shown (never auto-collapses
  to icon-only while a filter is applied).
- **Expanded state:** `<Input id={id} type="search" placeholder={placeholder} />` + a ghost
  `<Button size="sm">` with `<X size={14} aria-hidden />` clear button.
  Auto-focuses the `<Input>` on expand via `tick()`.
- **oninput:** Updates local `$state`, clears and restarts the debounce timer. When the timer
  fires, calls `onchange(localValue)`.
- **Escape key on input:** Clears local value, clears debounce, calls `onchange('')`,
  collapses to icon.
- **Clear button click:** Same as Escape.
- **`value` prop change from outside** (e.g. external URL navigation): `$effect` syncs local
  state to the new value.

Implementation note: `ExpandableSearch` owns both the icon button and the expanded input in the
same component. It does NOT use `bind:` on `value` — it is fully controlled via `value` prop +
`onchange` callback.

```svelte
<script lang="ts">
  import { tick } from 'svelte';
  import { Search, X } from 'lucide-svelte';
  import Button from '$lib/components/Button.svelte';
  import { Input } from '$lib/components/forms';

  let { id, value, onchange, placeholder = 'Search...', debounceMs = 300 }: {
    id: string;
    value: string;
    onchange: (v: string) => void;
    placeholder?: string;
    debounceMs?: number;
  } = $props();

  let localValue = $state(value);
  let expanded = $state(value !== '');
  let inputEl: HTMLInputElement | undefined = $state(undefined);
  let timer: ReturnType<typeof setTimeout> | undefined;

  $effect(() => {
    localValue = value;
    // Only expand — never collapse — on external value change.
    // Collapsing is only triggered by clear() and Escape.
    // Do NOT read `expanded` here — that would make it an implicit $effect
    // dependency, causing the effect to re-run on every user expand/collapse.
    if (value !== '') expanded = true;
  });

  async function expand() {
    expanded = true;
    await tick();
    inputEl?.focus();
  }

  function clear() {
    clearTimeout(timer);
    localValue = '';
    expanded = false;
    onchange('');
  }

  function handleInput(e: Event) {
    localValue = (e.currentTarget as HTMLInputElement).value;
    clearTimeout(timer);
    timer = setTimeout(() => onchange(localValue), debounceMs);
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') clear();
  }
</script>

{#if !expanded}
  <Button variant="ghost" size="sm" ariaLabel={placeholder} onclick={expand}>
    {#snippet leadingIcon()}<Search size={14} aria-hidden="true" />{/snippet}
  </Button>
{:else}
  <div class="flex w-full items-center gap-1 md:w-auto">
    <Input
      bind:el={inputEl}
      {id}
      type="search"
      {placeholder}
      aria-label={placeholder}
      value={localValue}
      class="w-full md:w-48"
      oninput={handleInput}
      onkeydown={handleKeydown}
    />
    <Button variant="ghost" size="sm" ariaLabel="Clear search" onclick={clear}>
      {#snippet leadingIcon()}<X size={14} aria-hidden="true" />{/snippet}
    </Button>
  </div>
{/if}
```

Note: `Input` must expose a `bind:el` prop (or equivalent ref mechanism) for focus management.
If the current `Input` component does not expose this, add `bind:this` support to it as part of
this work.

---

## Per-Page Changes

### `/software` (`frontend/src/routes/software/+page.svelte`)

**URL params — before → after:**

| Old                                                 | New                                  | Notes                      |
| --------------------------------------------------- | ------------------------------------ | -------------------------- |
| `tab=all\|featured\|unfeatured\|ignores\|<surface>` | removed                              | tab= replaced by featured= |
| —                                                   | `featured=all\|featured\|unfeatured` | default: `featured`        |
| `updatable=true`                                    | `updatable=true`                     | unchanged                  |
| `plugin_type=<value>`                               | `plugin_type=<value>`                | unchanged                  |
| `query=<value>`                                     | `query=<value>`                      | unchanged                  |
| `page=<n>`                                          | `page=<n>`                           | unchanged                  |

**Filter params using `createUrlParam`:**

```typescript
const featured = createUrlParam<"all" | "featured" | "unfeatured">("featured", {
  parse: (r): "all" | "featured" | "unfeatured" =>
    r === "all" || r === "unfeatured" ? r : "featured",
  serialize: (v) => (v === "featured" ? null : v), // omit when default
});
const updatable = createUrlParam("updatable", {
  parse: (r) => r === "true",
  serialize: (v) => (v ? "true" : null),
});
const pluginType = createUrlParam("plugin_type");
// query= is managed by ExpandableSearch internally
```

**`featuredFilter()` becomes:**

```typescript
function featuredFilter(): boolean | undefined {
  if (featured.value === "featured") return true;
  if (featured.value === "unfeatured") return false;
  return undefined;
}
```

**TabStrip:** Removed from the "All / Featured / Unfeatured" role. Only rendered when
`slotTabSurfaces.length > 0`. When rendered, contains only surface-contributed tabs.

**`isItemsTab`:** Previously derived from `activeTab`. With tabs removed, the items view is
always shown (no tab concept for the main table). Surface tabs render content panels below the
filter bar when a surface tab is active. The `ignores` tab is gone (Ignore Rules is a card).

**`switchTab` function:** Removed. Surface tab selection handled directly.

**Select binding with `createUrlParam`:** Do NOT use `bind:value` on Selects backed by a URL
param — that creates a circular reactivity loop (`bind:` writes → `goto()` → page updates →
derived updates → `bind:` writes → …). Use one-way `value={param.value}` + `onchange` callback:

```svelte
<Select
  id="software-featured-filter"
  value={featured.value}
  options={[...]}
  onchange={(e) => featured.set(e.currentTarget.value)}
/>
```

**FilterBar in header:**

```svelte
<FilterBar>
  {#snippet filters()}
    {#if canManage}
      <Checkbox
        id="software-batch-select-all"
        checked={allBatchPageSelected}
        indeterminate={!allBatchPageSelected && batchSelectedIds.size > 0}
        onchange={toggleBatchSelectAll}
      />
      <label for="software-batch-select-all" class="cursor-pointer select-none text-sm">Select all</label>
      <span class="h-4 w-px bg-[var(--border-subtle)]" aria-hidden="true"></span>
    {/if}
    <Select
      id="software-featured-filter"
      width="auto"
      value={featured.value}
      aria-label="Filter by featured status"
      options={[
        { value: 'all', label: 'All' },
        { value: 'featured', label: 'Featured' },
        { value: 'unfeatured', label: 'Unfeatured' },
      ]}
      onchange={(e) => featured.set(e.currentTarget.value)}
    />
    <Checkbox
      id="software-filter-updatable-only"
      checked={updatable.value}
      onchange={(e) => updatable.set(e.currentTarget.checked)}
    />
    <label for="software-filter-updatable-only" class="cursor-pointer select-none text-sm">
      Updates available
    </label>
    {#if pluginTypeOptions.length > 0}
      <Select
        id="software-plugin-filter"
        width="auto"
        value={pluginType.value}
        aria-label="Filter by plugin"
        options={[{ value: '', label: 'All plugins' }, ...pluginTypeOptions.map(...)]}
        onchange={(e) => pluginType.set(e.currentTarget.value)}
      />
    {/if}
    <ExpandableSearch
      id="software-name-filter"
      value={queryParam.value}
      onchange={(v) => queryParam.set(v)}
      placeholder="Filter by name"
    />
  {/snippet}
  {#snippet actions()}
    {#if canManage}
      <Button variant="primary" size="sm" onclick={() => (showAddModal = true)}>Add Software</Button>
    {/if}
  {/snippet}
</FilterBar>
```

**Batch select checkbox:** The "Select all" checkbox and separator remain in the FilterBar
`filters` snippet, before the featured Select, guarded by `canManage`.

**Ignore Rules:** Collapsible `<details>` element below the main software group list:

```svelte
<details class="rounded-card border border-[var(--border-subtle)] bg-[var(--bg-surface)] shadow-sm">
  <summary class="card-padding cursor-pointer text-sm font-semibold select-none">
    Ignore Rules
  </summary>
  <div class="border-t border-[var(--border-subtle)]">
    <IgnoreRulesTab />
  </div>
</details>
```

**Data-loading `$effect`:** Replaces the old `onMount` + URL-sync `$effect` pair:

```typescript
$effect(() => {
  // All filter values are reactive dependencies.
  const _f = featured.value;
  const _u = updatable.value;
  const _p = pluginType.value;
  const _q = queryParam.value;
  if (canView) loadAll(1);
});
```

(The `currentPage` param still uses `parseUrlPage` for pagination navigation — page increments
via `TableFooterBar` call `goto()` with `page=N` added.)

---

### `/host-tags` (`frontend/src/routes/host-tags/+page.svelte`)

**Removed:** `SectionCard title="Search"` containing the bare `<Input>`.

**Added:** `FilterBar` inside (or wrapping the header of) `SectionCard title="Tags"`:

```svelte
<SectionCard title="Tags">
  {#snippet filterBar()}
    <FilterBar>
      {#snippet filters()}
        <ExpandableSearch
          id="tags-name-filter"
          value={queryParam.value}
          onchange={(v) => queryParam.set(v)}
          placeholder="Filter by name"
        />
      {/snippet}
      {#snippet actions()}
        {#if canManage}
          <Button variant="primary" onclick={openCreateDialog}>Create Tag</Button>
        {/if}
      {/snippet}
    </FilterBar>
  {/snippet}
  <DataTable ...>
    ...
  </DataTable>
</SectionCard>
```

**`searchQuery` state:** Replaced by `createUrlParam('query')`. `getHostTags` call passes
`queryParam.value || undefined`.

**URL params — before → after:**

| Old        | New                     |
| ---------- | ----------------------- |
| `page=<n>` | `page=<n>` (unchanged)  |
| —          | `query=<value>` (added) |

---

### `/history` (`frontend/src/routes/history/+page.svelte`)

**Removed:** `SectionCard title="Filters"`.

**Changed:** `SectionCard title="History Feed"` gains a `filterBar` snippet (see SectionCard
integration section) containing a `FilterBar` with the status Select.

**Status filter Select:**

```svelte
<Select
  id="history-status-filter"
  width="auto"
  aria-label="Filter by status"
  options={[
    { value: 'all',         label: 'All' },
    { value: 'in_progress', label: 'In Progress' },
    { value: 'queued',      label: 'Queued' },
    { value: 'pending',     label: 'Pending' },
    { value: 'failed',      label: 'Failed' },
    { value: 'completed',   label: 'Completed' },
  ]}
  onchange={(e) => statusParam.set(e.currentTarget.value)}
/>
```

The option values match the existing
`STATUS_FILTER_VALUES = ['all', 'queued', 'pending', 'in_progress', 'completed', 'failed']`
in the page; no API change needed.

**URL params:** unchanged (`status=`, `page=`). Bug fix only: `statusFilter` becomes
`$derived`.

---

### `/services` (`frontend/src/routes/services/+page.svelte`)

**Removed:** `SectionCard title="Service Filters"` and accent button group.

**Changed:** `SectionCard title="Registered Services"` gains `FilterBar` in header.

**Capability filter Select:**

```svelte
<Select
  id="services-capability-filter"
  width="auto"
  aria-label="Filter by capability"
  options={[
    { value: 'all',                label: 'All' },
    { value: 'software_discovery', label: 'Software Discovery' },
    { value: 'ssh_remote',         label: 'SSH Remote' },
  ]}
  onchange={(e) => capabilityParam.set(e.currentTarget.value)}
/>
```

**URL params:** unchanged (`capability=`, `page=`). Bug fix only.

---

### `/system-services` (`frontend/src/routes/system-services/+page.svelte`)

**Removed:** `SectionCard title="Status Filters"` and accent button group.

**Changed:** `SectionCard title="Registered System Services"` gains `FilterBar` in header.

**Status filter Select:**

```svelte
<Select
  id="system-services-status-filter"
  width="auto"
  aria-label="Filter by status"
  options={[
    { value: 'all',         label: 'All' },
    { value: 'pending',     label: 'Pending' },
    { value: 'approved',    label: 'Approved' },
    { value: 'rejected',    label: 'Rejected' },
    { value: 'deactivated', label: 'Deactivated' },
  ]}
  onchange={(e) => statusParam.set(e.currentTarget.value)}
/>
```

**URL params:** unchanged (`status=`, `page=`). Bug fix only.

---

### `/hosts` (`frontend/src/routes/hosts/+page.svelte`)

**Changed:** Existing `SectionCard title="Registered Hosts"` gains `FilterBar` in header.

The `getHosts()` API does **not** yet support a `query` param. Wiring `ExpandableSearch` to
the URL while data does not filter would be a false affordance — the search widget would appear
functional but produce no results change. Therefore:

- The `FilterBar` is added to the header (structural unification).
- `ExpandableSearch` is **not** rendered until backend support exists. The `filters` snippet is
  empty for now.
- The `filterBar` snippet prop on `SectionCard` is still wired (so the card renders cleanly
  with an empty filter bar) if there are actions to show; otherwise `FilterBar` may be omitted
  entirely since `/hosts` has no create action on this page.

```svelte
<!-- /hosts: FilterBar present for structural consistency; no filters yet -->
<SectionCard title="Registered Hosts">
  {#snippet filterBar()}
    <!-- ExpandableSearch will be added here once getHosts() supports query param -->
  {/snippet}
  <DataTable ...> ... </DataTable>
</SectionCard>
```

**Follow-up task (out of scope here):**

- Extend `getHosts(page?, perPage?, query?)` in `api.ts` to pass `query` as a URL param.
- Extend the backend `/hosts` endpoint to filter by hostname/friendly name.
- Add `ExpandableSearch` to the hosts `FilterBar` and wire `createUrlParam('query')` to it.

---

## SectionCard + FilterBar Integration

Pages that wrap their table in `SectionCard` need `FilterBar` integrated without double-border
artefacts and without losing the SectionCard title.

`SectionCard` gains an optional **`filterBar?: Snippet`** prop (named `filterBar` to avoid a
snippet naming clash with `FilterBar`'s own `filters` prop). When provided:

- The title row does **not** render its `border-b` class.
- Instead, the `filterBar` snippet is rendered immediately below the title row; the `FilterBar`
  rendered inside it carries the `border-b`, producing exactly one divider between the
  title+filter area and the table body.

```svelte
<!-- SectionCard internal structure when `filterBar` is provided -->
<div class="rounded-card border border-[var(--border-subtle)] bg-[var(--bg-surface)] shadow-sm">
  {#if title}
    <!-- No border-b here when filterBar snippet is present -->
    <div class="card-padding {filterBar ? '' : 'border-b border-[var(--border-subtle)]'} ...">
      <h2>{title}</h2>
    </div>
  {/if}
  {#if filterBar}
    {@render filterBar()}   <!-- FilterBar with border-b goes here -->
  {/if}
  <div>
    {@render children()}
  </div>
</div>
```

Usage on `/host-tags`:

```svelte
<SectionCard title="Tags">
  {#snippet filterBar()}
    <FilterBar>
      {#snippet filters()}
        <ExpandableSearch ... />
      {/snippet}
      {#snippet actions()}
        <Button variant="primary" onclick={openCreateDialog}>Create Tag</Button>
      {/snippet}
    </FilterBar>
  {/snippet}
  <DataTable ...> ... </DataTable>
</SectionCard>
```

The software page's custom `<div class="rounded-card ...">` continues to use `FilterBar`
directly in its `<header>` — no `SectionCard` involved there.

---

## `Input` Component — `bind:el` Ref

`ExpandableSearch` needs to focus the `<Input>` after expanding. The current `Input` component
must expose the underlying `<input>` element via `bind:el` (a `$bindable` prop of type
`HTMLInputElement | undefined`). If this prop does not exist, add it as part of this work.

---

## Tests

### New test files

**`frontend/src/lib/components/ui/filter-bar.test.ts`**

- Renders `{#snippet filters()}` content on the left.
- Renders `{#snippet actions()}` content on the right.
- Has `data-ui="filter-bar"` attribute.
- Filters and actions are in the correct DOM order.

**`frontend/src/lib/components/ui/expandable-search.test.ts`**

- Starts collapsed when `value === ''`.
- Starts expanded when `value !== ''`.
- Clicking the search icon expands and focuses the input.
- Typing debounces and calls `onchange` after `debounceMs`.
- Pressing Escape clears value, collapses, calls `onchange('')`.
- Clicking the clear (X) button clears value, collapses, calls `onchange('')`.
- External `value` prop change syncs local state (simulates browser back/external nav).

**`frontend/src/lib/url-params.svelte.test.ts`**

- `createUrlParam` returns current URL param value reactively.
- `.set()` calls `goto()` with updated URL.
- `.set()` removes `page=` from the URL.
- Boolean parse/serialize round-trip.
- Enum parse falls back to default when value is unknown.

### Updated test files

**`frontend/src/routes/software/software-name-filter.test.ts`**

- Replace `tab=` URL mock with `featured=` param.
- Add case: `?updatable=true` on mount → `getSoftwareItems` called with updatable=true.
- Add case: `?plugin_type=npm` on mount → `getSoftwareItems` called with plugin_type=npm.
- Keep existing: `?query=foo` populates `ExpandableSearch` input with "foo".

**`frontend/src/routes/host-tags/host-tags.test.ts`**

- Remove assertion that `SectionCard title="Search"` exists.
- Add assertion that `ExpandableSearch` (search icon button or search input) is inside the
  table card header, not in a separate card above.
- Add case: `?query=prod` on mount → `getHostTags` called with `query='prod'`.

### Playwright e2e tests (visual parity suite)

Per project rules, visual changes require desktop parity coverage (macOS + Chromium). Add or
extend the following in `frontend/tests/e2e/`:

**`filter-bar-parity.spec.ts`** (new)

- For each in-scope page (`/software`, `/host-tags`, `/history`, `/services`,
  `/system-services`, `/hosts`): verify the separate filter `SectionCard` is **absent** and
  `data-ui="filter-bar"` is present inside the table card.
- `/software`: verify TabStrip with "All/Featured/Unfeatured" tabs is absent; featured
  `<Select>` is present inside `[data-ui="filter-bar"]`.
- `/host-tags`: verify `SectionCard title="Search"` is absent; `ExpandableSearch` icon or
  input is inside `[data-ui="filter-bar"]`.
- URL reactivity smoke test: navigate to `/software`, then navigate to
  `/software?updatable=true` (same tab, no reload); assert updates-available filter is active
  and table reloads.

---

## Documentation Updates

**`docs/development/ui/primitives.md`** — add entries for:

- `FilterBar` — layout shell, props (`filters` snippet, `actions` snippet), visual spec,
  `data-ui` attribute, usage example.
- `ExpandableSearch` — expandable search widget, props, collapsed/expanded states, keyboard
  behaviour, usage example with `createUrlParam`.

---

## Out of Scope / Deferred

- `/audit-logs` filter unification (8+ specialised fields; separate spec).
- Backend hostname/name search on `/hosts` (API extension required).
- Surface-contributed filter controls (plugin surfaces injecting filter widgets).
- URL param persistence for `/hosts` name search actually filtering results (depends on
  backend).
- Any visual design changes beyond moving controls from separate SectionCards into FilterBar.

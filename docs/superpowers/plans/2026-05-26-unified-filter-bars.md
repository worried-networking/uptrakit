# Unified Filter Bars & URL-Reactive Filter State — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended)
> or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax
> for tracking.

**Goal:** Fix URL-reactivity bug on all table pages (filter state updates when URL changes externally),
move filter controls into table card headers, and extract three shared artifacts (`createUrlParam`,
`FilterBar`, `ExpandableSearch`).

**Architecture:** URL is the single source of truth. Filter values AND `currentPage` are both `$derived`
from `page.url.searchParams`, so any external navigation (badge click, browser back, etc.) automatically
updates all derived values and triggers a single data-load `$effect`. There is no separate "Effect 1
writes page= to URL" — `currentPage` is derived from the URL, and pagination handlers write to the URL
directly via `goto`. `createUrlParam.set()` already removes `page=` (resetting pagination on every filter
change). A `FilterBar` layout shell and an `ExpandableSearch` controlled component are shared across all
six in-scope pages. `SectionCard` gains an optional `filterBar` snippet prop to avoid double-border
artefacts.

**Tech Stack:** SvelteKit 2, Svelte 5 runes (`$derived`, `$state`, `$effect`, `$props`, `$bindable`),
TypeScript strict mode, `@testing-library/svelte` + Vitest, Playwright (e2e parity), Lucide-Svelte icons,
Tailwind CSS utility classes + design-system CSS tokens.

---

## File Map

**New files:**

- `frontend/src/lib/url-params.svelte.ts` — `createUrlParam` factory
- `frontend/src/lib/components/ui/FilterBar.svelte` — layout shell
- `frontend/src/lib/components/ui/FilterBar.test.ts` — unit tests
- `frontend/src/lib/components/ui/ExpandableSearch.svelte` — expandable search widget
- `frontend/src/lib/components/ui/ExpandableSearch.test.ts` — unit tests
- `frontend/src/lib/url-params.svelte.test.ts` — `createUrlParam` tests (via test harness component)
- `frontend/tests/e2e/filter-bar-parity.spec.ts` — Playwright parity tests

**Modified files:**

- `frontend/src/lib/components/forms/Input.svelte` — add `el: HTMLInputElement | undefined = $bindable(undefined)` prop
- `frontend/src/lib/components/ui/SectionCard.svelte` — add `filterBar?: Snippet` prop
- `frontend/src/lib/components/ui/SectionCard.test.ts` — cover new `filterBar` prop
- `frontend/src/lib/components/ui/index.ts` — export `FilterBar`, `ExpandableSearch`
- `frontend/src/routes/software/+page.svelte` — full filter migration
- `frontend/src/routes/software/software-name-filter.test.ts` — update for `featured=` param, reactive URL
- `frontend/src/routes/host-tags/+page.svelte` — add FilterBar, URL-persist query
- `frontend/src/routes/host-tags/host-tags.test.ts` — update for new layout
- `frontend/src/routes/history/+page.svelte` — replace filter SectionCard with FilterBar
- `frontend/src/routes/history/history.test.ts` — update filter-chip tests
- `frontend/src/routes/services/+page.svelte` — replace filter SectionCard with FilterBar
- `frontend/src/routes/services/services.test.ts` — update filter-chip tests
- `frontend/src/routes/system-services/+page.svelte` — replace filter SectionCard with FilterBar
- `frontend/src/routes/system-services/system-services.test.ts` — update filter-chip tests
- `frontend/src/routes/hosts/+page.svelte` — minimal: no filter params to migrate
- `docs/development/ui/primitives.md` — document `FilterBar` and `ExpandableSearch`

---

## Task 1: `createUrlParam` factory

**Files:**

- Create: `frontend/src/lib/url-params.svelte.ts`
- Create: `frontend/src/lib/url-params.svelte.test.ts`

- [ ] **Step 1.1: Write the factory**

```typescript
// frontend/src/lib/url-params.svelte.ts
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

/**
 * Creates a URL search-param binding backed by $derived from page.url.searchParams.
 *
 * CONSTRAINT: must be called at component initialisation scope only — top-level
 * <script> in .svelte files, or top-level of .svelte.ts modules. Calling inside
 * a callback or event handler causes a Svelte rune-outside-reactive-context error.
 *
 * set() always removes 'page=' from the URL (resets pagination on filter change).
 * Does NOT re-run SvelteKit load() on pages that fetch data client-side via $effect.
 */
export function createUrlParam<T = string>(
  key: string,
  options?: UrlParamOptions<T>,
): UrlParam<T> {
  const parse = options?.parse ?? ((raw) => (raw ?? "") as unknown as T);
  const serialize =
    options?.serialize ?? ((v) => (v === "" || v == null ? null : String(v)));

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
      next.searchParams.delete("page");
      void goto(next, { replaceState: true, keepFocus: true, noScroll: true });
    },
  };
}
```

- [ ] **Step 1.2: Write tests via a minimal test harness component**

Create `frontend/src/lib/url-params.svelte.test.ts`:

```typescript
// frontend/src/lib/url-params.svelte.test.ts
import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/svelte";
import { page } from "$app/state";
import { goto } from "$app/navigation";

// We test createUrlParam by rendering a minimal Svelte component that calls it.
// Direct calls outside component context would throw a rune-outside-reactive-context error.
// The harness exposes param.value via data-testid="current-value" and param.set() via a button.

vi.mock("$app/navigation", () => ({ goto: vi.fn() }));

// Override page URL for each test by mutating the shared mock object.
function setUrl(url: string) {
  const parsed = new URL(url);
  Object.defineProperty(page, "url", { value: parsed, configurable: true });
}

// Import the test harness component (created in next step).
import UrlParamHarness from "./url-params.test-harness.svelte";

describe("createUrlParam", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    setUrl("http://localhost/");
  });

  it("returns empty string when param absent", () => {
    setUrl("http://localhost/software");
    render(UrlParamHarness, { paramKey: "query" });
    expect(screen.getByTestId("current-value").textContent).toBe('""');
  });

  it("returns param value when present", () => {
    setUrl("http://localhost/software?query=nginx");
    render(UrlParamHarness, { paramKey: "query" });
    expect(screen.getByTestId("current-value").textContent).toBe('"nginx"');
  });

  it("set() calls goto() with updated URL", async () => {
    setUrl("http://localhost/software");
    const { rerender } = render(UrlParamHarness, {
      paramKey: "query",
      testSetValue: "",
    });
    // Provide the value to set, then click the set button.
    await rerender({ paramKey: "query", testSetValue: "nginx" });
    await fireEvent.click(screen.getByTestId("do-set"));
    expect(vi.mocked(goto)).toHaveBeenCalledWith(
      expect.objectContaining({ searchParams: expect.any(URLSearchParams) }),
      { replaceState: true, keepFocus: true, noScroll: true },
    );
    const calledUrl: URL = vi.mocked(goto).mock.calls[0][0] as URL;
    expect(calledUrl.searchParams.get("query")).toBe("nginx");
  });

  it("set() removes page= from URL", async () => {
    setUrl("http://localhost/software?page=3");
    const { rerender } = render(UrlParamHarness, {
      paramKey: "query",
      testSetValue: "",
    });
    await rerender({ paramKey: "query", testSetValue: "nginx" });
    await fireEvent.click(screen.getByTestId("do-set"));
    const calledUrl: URL = vi.mocked(goto).mock.calls[0][0] as URL;
    expect(calledUrl.searchParams.has("page")).toBe(false);
  });

  it("enum param falls back to default for unknown value", () => {
    setUrl("http://localhost/?status=unknown");
    render(UrlParamHarness, {
      paramKey: "status",
      parse: (r: string | null) =>
        r === "pending" || r === "completed" ? r : "all",
    });
    expect(screen.getByTestId("current-value").textContent).toBe('"all"');
  });
});
```

- [ ] **Step 1.3: Create the test harness Svelte component**

Create `frontend/src/lib/url-params.test-harness.svelte`:

```svelte
<script lang="ts">
	import { createUrlParam, type UrlParamOptions } from './url-params.svelte';

	let {
		paramKey,
		parse,
		serialize,
		testSetValue = ''
	}: {
		paramKey: string;
		parse?: (r: string | null) => unknown;
		serialize?: (v: unknown) => string | null;
		testSetValue?: unknown;
	} = $props();

	// eslint-disable-next-line @typescript-eslint/no-explicit-any -- test harness requires generic param type
	const param = createUrlParam(paramKey, { parse, serialize } as UrlParamOptions<any>);
</script>

<!-- Exposes current value as JSON so tests can assert any type (string, boolean, etc.) -->
<span data-testid="current-value">{JSON.stringify(param.value)}</span>
<!-- Calls param.set(testSetValue) when clicked; rerender with new testSetValue before clicking -->
<button data-testid="do-set" onclick={() => param.set(testSetValue)}>set</button>
```

- [ ] **Step 1.4: Run tests**

```bash
cd frontend && npm run test -- --reporter=verbose url-params
```

Expected: all 5 tests pass.

- [ ] **Step 1.5: Commit**

```bash
git add frontend/src/lib/url-params.svelte.ts frontend/src/lib/url-params.svelte.test.ts frontend/src/lib/url-params.test-harness.svelte
git commit -m "feat(frontend): add createUrlParam factory for URL-reactive filter state"
```

---

## Task 2: `Input.svelte` — expose `el` bindable prop

**Files:**

- Modify: `frontend/src/lib/components/forms/Input.svelte`

`ExpandableSearch` needs to focus the underlying `<input>` element after expanding. `Input.svelte` currently has no `el` prop.

- [ ] **Step 2.1: Add `el` to `InputProps` and wire `bind:this`**

In `frontend/src/lib/components/forms/Input.svelte`, add `el` to the module-level type and the component `$props`:

```svelte
<script lang="ts" module>
	import type { FullAutoFill } from 'svelte/elements';

	export type InputType = 'text' | 'email' | 'password' | 'url' | 'number' | 'search' | 'datetime-local';

	export type InputProps = {
		id: string;
		type: InputType;
		value: string | number;
		/** Ref to the underlying <input> element. Use bind:el={myRef} from the parent. */
		el?: HTMLInputElement;
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
		inputmode?: 'none' | 'text' | 'decimal' | 'numeric' | 'tel' | 'search' | 'email' | 'url';
		pattern?: string;
		maxlength?: number;
		'aria-describedby'?: string;
		'aria-label'?: string;
		class?: string;
	};
</script>

<script lang="ts">
	import { getContext } from 'svelte';

	const BASE =
		'h-8 w-full py-0 px-[10px] rounded-card ' +
		'bg-[var(--bg-surface)] border border-[var(--border-default)] ' +
		'text-sm text-[var(--text-primary)] ' +
		'placeholder:text-[var(--text-muted)] ' +
		'focus-visible:outline-none ' +
		'focus-visible:shadow-[0_0_0_3px_rgba(var(--accent-rgb),0.25)] ' +
		'disabled:opacity-40 disabled:cursor-not-allowed ' +
		'aria-[invalid=true]:border-[var(--color-danger-border)] ' +
		'aria-[invalid=true]:bg-[var(--color-danger-bg)] ' +
		'transition-[background,border-color] duration-fast';

	let {
		id,
		type,
		value = $bindable(''),
		el = $bindable(undefined),
		name,
		placeholder,
		autocomplete,
		disabled = false,
		required = false,
		error,
		min,
		max,
		oninput,
		onblur,
		onkeydown,
		inputmode,
		pattern,
		maxlength,
		'aria-describedby': ariaDescribedby,
		'aria-label': ariaLabel,
		class: className = ''
	}: InputProps = $props();

	const rowCtx = getContext<{ id: string | undefined } | undefined>('form-field-row:aria-describedby');
	const computedClass = $derived([BASE, className].filter(Boolean).join(' '));
	const hasError = $derived(!!error);
	const resolvedDescribedBy = $derived(ariaDescribedby ?? rowCtx?.id);
</script>

<input
	{id}
	{type}
	bind:value
	bind:this={el}
	{name}
	{placeholder}
	autocomplete={autocomplete as FullAutoFill | undefined}
	{disabled}
	{required}
	{min}
	{max}
	{inputmode}
	{pattern}
	{maxlength}
	{oninput}
	{onblur}
	{onkeydown}
	aria-invalid={hasError ? 'true' : undefined}
	aria-describedby={resolvedDescribedBy}
	aria-label={ariaLabel}
	data-ui="input"
	class={computedClass}
/>
```

- [ ] **Step 2.2: Run existing Input tests**

```bash
cd frontend && npm run test -- --reporter=verbose Input
```

Expected: all Input tests pass (no regressions).

- [ ] **Step 2.3: Commit**

```bash
git add frontend/src/lib/components/forms/Input.svelte
git commit -m "feat(frontend): add bind:el ref prop to Input component"
```

---

## Task 3: `FilterBar.svelte` component

**Files:**

- Create: `frontend/src/lib/components/ui/FilterBar.svelte`
- Create: `frontend/src/lib/components/ui/FilterBar.test.ts`
- Modify: `frontend/src/lib/components/ui/index.ts`

- [ ] **Step 3.1: Write failing tests**

Create `frontend/src/lib/components/ui/FilterBar.test.ts`:

```typescript
import { cleanup, render, screen } from "@testing-library/svelte";
import { afterEach, describe, expect, it } from "vitest";
import { createRawSnippet } from "svelte";
import FilterBar from "./FilterBar.svelte";

function makeSnippet(html: string) {
  return createRawSnippet(() => ({
    render() {
      return html;
    },
  }));
}

afterEach(() => {
  cleanup();
});

describe("FilterBar", () => {
  it('renders data-ui="filter-bar" attribute', () => {
    const { container } = render(FilterBar, {
      filters: makeSnippet("<span>filter content</span>"),
    });
    expect(
      container.querySelector('[data-ui="filter-bar"]'),
    ).toBeInTheDocument();
  });

  it("renders filters snippet on the left", () => {
    render(FilterBar, {
      filters: makeSnippet('<span data-testid="f">filter</span>'),
    });
    expect(screen.getByTestId("f")).toBeInTheDocument();
  });

  it("renders actions snippet on the right when provided", () => {
    const { container } = render(FilterBar, {
      filters: makeSnippet("<span>f</span>"),
      actions: makeSnippet('<button type="button">Add</button>'),
    });
    expect(screen.getByRole("button", { name: "Add" })).toBeInTheDocument();
    // actions div has shrink-0 class
    expect(container.querySelector(".shrink-0")).toBeInTheDocument();
  });

  it("omits actions container when actions snippet not provided", () => {
    const { container } = render(FilterBar, {
      filters: makeSnippet("<span>f</span>"),
    });
    expect(container.querySelector(".shrink-0")).not.toBeInTheDocument();
  });

  it("filters are in a flex-wrap container before actions", () => {
    const { container } = render(FilterBar, {
      filters: makeSnippet('<span data-testid="f">f</span>'),
      actions: makeSnippet("<button>Add</button>"),
    });
    const header = container.querySelector('[data-ui="filter-bar"]')!;
    const children = Array.from(header.children);
    expect(children[0].querySelector('[data-testid="f"]')).toBeInTheDocument();
    expect(children[1].querySelector("button")).toBeInTheDocument();
  });
});
```

- [ ] **Step 3.2: Run tests to verify they fail (component does not exist yet)**

```bash
cd frontend && npm run test -- --reporter=verbose FilterBar
```

Expected: FAIL — `Cannot find module './FilterBar.svelte'`.

- [ ] **Step 3.3: Create `FilterBar.svelte`**

```svelte
<!-- frontend/src/lib/components/ui/FilterBar.svelte -->
<script lang="ts">
	import type { Snippet } from 'svelte';

	let {
		filters,
		actions
	}: {
		filters?: Snippet;
		actions?: Snippet;
	} = $props();
</script>

<header
	class="flex flex-col gap-3 border-b border-[var(--border-subtle)] bg-[var(--bg-raised)] card-padding md:flex-row md:items-center md:justify-between"
	data-ui="filter-bar"
>
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

- [ ] **Step 3.4: Export from `index.ts`**

Add to `frontend/src/lib/components/ui/index.ts`:

```typescript
export { default as FilterBar } from "./FilterBar.svelte";
```

- [ ] **Step 3.5: Run tests**

```bash
cd frontend && npm run test -- --reporter=verbose FilterBar
```

Expected: all 5 tests pass.

- [ ] **Step 3.6: Commit**

```bash
git add frontend/src/lib/components/ui/FilterBar.svelte frontend/src/lib/components/ui/FilterBar.test.ts frontend/src/lib/components/ui/index.ts
git commit -m "feat(frontend): add FilterBar layout component"
```

---

## Task 4: `ExpandableSearch.svelte` component

**Files:**

- Create: `frontend/src/lib/components/ui/ExpandableSearch.svelte`
- Create: `frontend/src/lib/components/ui/ExpandableSearch.test.ts`
- Modify: `frontend/src/lib/components/ui/index.ts`

- [ ] **Step 4.1: Write failing tests**

Create `frontend/src/lib/components/ui/ExpandableSearch.test.ts`:

```typescript
import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import ExpandableSearch from "./ExpandableSearch.svelte";

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

describe("ExpandableSearch", () => {
  it("starts collapsed when value is empty", () => {
    render(ExpandableSearch, {
      id: "test-search",
      value: "",
      onchange: vi.fn(),
      placeholder: "Search...",
    });
    expect(screen.queryByRole("searchbox")).not.toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Search..." }),
    ).toBeInTheDocument();
  });

  it("starts expanded when value is non-empty", () => {
    render(ExpandableSearch, {
      id: "test-search",
      value: "nginx",
      onchange: vi.fn(),
      placeholder: "Search...",
    });
    const input = screen.getByRole("searchbox") as HTMLInputElement;
    expect(input.value).toBe("nginx");
  });

  it("clicking the search icon button expands to show input", async () => {
    render(ExpandableSearch, {
      id: "test-search",
      value: "",
      onchange: vi.fn(),
      placeholder: "Search...",
    });
    await fireEvent.click(screen.getByRole("button", { name: "Search..." }));
    await waitFor(() =>
      expect(screen.getByRole("searchbox")).toBeInTheDocument(),
    );
  });

  it("calls onchange after debounce when typing", async () => {
    vi.useFakeTimers();
    const onchange = vi.fn();
    render(ExpandableSearch, {
      id: "test-search",
      value: "",
      onchange,
      debounceMs: 100,
    });
    await fireEvent.click(screen.getByRole("button", { name: "Search..." }));
    await waitFor(() =>
      expect(screen.getByRole("searchbox")).toBeInTheDocument(),
    );
    await fireEvent.input(screen.getByRole("searchbox"), {
      target: { value: "nginx" },
    });
    expect(onchange).not.toHaveBeenCalled();
    vi.advanceTimersByTime(100);
    expect(onchange).toHaveBeenCalledWith("nginx");
  });

  it("pressing Escape clears value and collapses", async () => {
    const onchange = vi.fn();
    render(ExpandableSearch, { id: "test-search", value: "nginx", onchange });
    const input = screen.getByRole("searchbox");
    await fireEvent.keydown(input, { key: "Escape" });
    expect(onchange).toHaveBeenCalledWith("");
    await waitFor(() =>
      expect(screen.queryByRole("searchbox")).not.toBeInTheDocument(),
    );
  });

  it("clicking clear button calls onchange with empty string and collapses", async () => {
    const onchange = vi.fn();
    render(ExpandableSearch, { id: "test-search", value: "nginx", onchange });
    const clearBtn = screen.getByRole("button", { name: "Clear search" });
    await fireEvent.click(clearBtn);
    expect(onchange).toHaveBeenCalledWith("");
    await waitFor(() =>
      expect(screen.queryByRole("searchbox")).not.toBeInTheDocument(),
    );
  });

  it("external value prop change syncs local state", async () => {
    const { rerender } = render(ExpandableSearch, {
      id: "test-search",
      value: "",
      onchange: vi.fn(),
    });
    await rerender({ value: "updated" });
    await waitFor(() => {
      const input = screen.getByRole("searchbox") as HTMLInputElement;
      expect(input.value).toBe("updated");
    });
  });
});
```

- [ ] **Step 4.2: Run tests to verify they fail**

```bash
cd frontend && npm run test -- --reporter=verbose ExpandableSearch
```

Expected: FAIL — `Cannot find module './ExpandableSearch.svelte'`.

- [ ] **Step 4.3: Create `ExpandableSearch.svelte`**

```svelte
<!-- frontend/src/lib/components/ui/ExpandableSearch.svelte -->
<script lang="ts">
	import { tick } from 'svelte';
	import { Search, X } from 'lucide-svelte';
	import Button from '$lib/components/Button.svelte';
	import { Input } from '$lib/components/forms';

	let {
		id,
		value,
		onchange,
		placeholder = 'Search...',
		debounceMs = 300
	}: {
		id: string;
		value: string;
		onchange: (v: string) => void;
		placeholder?: string;
		debounceMs?: number;
	} = $props();

	let localValue = $state(value);
	let expanded = $state(value !== '');
	let inputEl: HTMLInputElement | undefined;  // element ref — no reactivity needed
	let timer: ReturnType<typeof setTimeout> | undefined;

	$effect(() => {
		// Cancel pending debounce whenever external value changes (e.g. external nav clears query=).
		clearTimeout(timer);
		localValue = value;
		// Only expand on external value — never collapse (collapse is clear/Escape only).
		// Do NOT read `expanded` here to avoid making it an implicit $effect dependency.
		if (value !== '') expanded = true;
	});

	$effect(() => {
		return () => clearTimeout(timer);
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

- [ ] **Step 4.4: Export from `index.ts`**

Add to `frontend/src/lib/components/ui/index.ts`:

```typescript
export { default as ExpandableSearch } from "./ExpandableSearch.svelte";
```

- [ ] **Step 4.5: Run tests**

```bash
cd frontend && npm run test -- --reporter=verbose ExpandableSearch
```

Expected: all 7 tests pass.

- [ ] **Step 4.6: Commit**

```bash
git add frontend/src/lib/components/ui/ExpandableSearch.svelte frontend/src/lib/components/ui/ExpandableSearch.test.ts frontend/src/lib/components/ui/index.ts
git commit -m "feat(frontend): add ExpandableSearch component with debounce and collapse"
```

---

## Task 5: `SectionCard.svelte` — add `filterBar` snippet prop

**Files:**

- Modify: `frontend/src/lib/components/ui/SectionCard.svelte`
- Modify: `frontend/src/lib/components/ui/SectionCard.test.ts`

- [ ] **Step 5.1: Add failing test to `SectionCard.test.ts`**

Add to the existing describe block in `frontend/src/lib/components/ui/SectionCard.test.ts`:

```typescript
it("renders filterBar snippet below title without border-b on title row", () => {
  const { container } = render(SectionCard, {
    title: "My Table",
    children: makeSnippet("<p>body</p>"),
    filterBar: makeSnippet('<div data-testid="fb">FilterBar</div>'),
  });
  expect(screen.getByTestId("fb")).toBeInTheDocument();
  // Title header should not have border-b when filterBar is present.
  const header = container.querySelector("header");
  expect(header?.className).not.toContain("border-b");
});

it("renders border-b on title row when filterBar not provided", () => {
  const { container } = render(SectionCard, {
    title: "My Table",
    children: makeSnippet("<p>body</p>"),
  });
  const header = container.querySelector("header");
  expect(header?.className).toContain("border-b");
});
```

- [ ] **Step 5.2: Run test to verify it fails**

```bash
cd frontend && npm run test -- --reporter=verbose SectionCard
```

Expected: new tests FAIL (filterBar prop does not exist yet).

- [ ] **Step 5.3: Update `SectionCard.svelte`**

Replace the entire file content of `frontend/src/lib/components/ui/SectionCard.svelte`:

```svelte
<script lang="ts">
	import type { Snippet } from 'svelte';

	let {
		title,
		description,
		actions,
		filterBar,
		children
	}: {
		title?: string;
		description?: string;
		actions?: Snippet;
		/** Optional snippet rendered between title and children. When present,
		 *  the title row loses its border-b (the FilterBar inside carries the divider). */
		filterBar?: Snippet;
		children: Snippet;
	} = $props();
</script>

<section
	class="rounded-card border border-[var(--border-subtle)] bg-[var(--bg-surface)] shadow-sm"
	data-ui="section-card"
>
	{#if title || description || actions}
		<header
			class="flex flex-col gap-3 {filterBar ? '' : 'border-b border-[var(--border-subtle)]'} card-padding md:flex-row md:items-start md:justify-between"
		>
			<div class="space-y-1">
				{#if title}
					<h2 class="text-section-title font-semibold text-[var(--text-primary)]">{title}</h2>
				{/if}
				{#if description}
					<p class="text-sm text-[var(--text-secondary)]">{description}</p>
				{/if}
			</div>
			{#if actions}
				<div class="flex shrink-0 flex-wrap items-center gap-2">
					{@render actions()}
				</div>
			{/if}
		</header>
	{/if}

	{#if filterBar}
		{@render filterBar()}
	{/if}

	<div class="card-padding">
		{@render children()}
	</div>
</section>
```

- [ ] **Step 5.4: Run all SectionCard tests**

```bash
cd frontend && npm run test -- --reporter=verbose SectionCard
```

Expected: all tests pass including new ones.

- [ ] **Step 5.5: Commit**

```bash
git add frontend/src/lib/components/ui/SectionCard.svelte frontend/src/lib/components/ui/SectionCard.test.ts
git commit -m "feat(frontend): add filterBar snippet prop to SectionCard"
```

---

## Task 6: Migrate `/software` page

**Files:**

- Modify: `frontend/src/routes/software/+page.svelte`
- Modify: `frontend/src/routes/software/software-name-filter.test.ts`

The software page has the most complex migration: replaces `tab=` with `featured=`, removes the
TabStrip for featured/unfeatured/all/ignores tabs, moves Ignore Rules to a `<details>` card, and
converts all filter state to `createUrlParam`.

- [ ] **Step 6.1: Update the script block — replace filter $state with createUrlParam**

In `frontend/src/routes/software/+page.svelte`, in the `<script>` block:

Add imports at the top:

```typescript
import { createUrlParam } from "$lib/url-params.svelte";
import { FilterBar, ExpandableSearch } from "$lib/components/ui";
```

Replace lines ~93-98 (the four filter `$state` declarations and nameFilterOpen/nameFilterWrapperEl) with:

```typescript
const featured = createUrlParam<"all" | "featured" | "unfeatured">("featured", {
  parse: (r): "all" | "featured" | "unfeatured" =>
    r === "all" || r === "unfeatured" ? r : "featured",
  serialize: (v) => (v === "featured" ? null : v),
});
const updatable = createUrlParam("updatable", {
  parse: (r) => r === "true",
  serialize: (v) => (v ? "true" : null),
});
const pluginType = createUrlParam("plugin_type");
const queryParam = createUrlParam("query");
let pluginTypeOptions: { plugin_type: string; display_name: string }[] = $state(
  [],
);
```

Remove: `let activeTab`, `let showUpdatableOnly`, `let pluginTypeFilter`, `let nameFilter`, `let nameFilterDebounce`, `let nameFilterOpen`, `let nameFilterWrapperEl`.

- [ ] **Step 6.2: Update derived values that referenced activeTab/showUpdatableOnly/etc.**

Replace `isItemsTab` derived (currently checks `activeTab`):

```typescript
// isItemsTab is always true now (no tabs for featured/unfeatured/all).
// Surface tabs are still conditionally rendered below.
```

Remove: `const tabItems`, `const isItemsTab`.

Update `itemsEmptyState` to use `featured.value` and `updatable.value`:

```typescript
const itemsEmptyState = $derived.by(() => {
  if (updatable.value) {
    return {
      title: "No updates available",
      description: "All software in this view is up to date.",
    };
  }
  if (pluginType.value) {
    return {
      title: "No matching software",
      description: "No items are tracked using the selected plugin.",
    };
  }
  if (featured.value === "featured") {
    return {
      title: "No featured software",
      description: "Feature software items to highlight them on the dashboard.",
    };
  }
  if (featured.value === "unfeatured") {
    return {
      title: "No unfeatured software",
      description: "All software items are currently featured.",
    };
  }
  return {
    title: "No software registered yet",
    description: "Register a package to start tracking.",
  };
});
```

- [ ] **Step 6.3: Update `featuredFilter()` to use `featured.value`**

```typescript
function featuredFilter(): boolean | undefined {
  if (featured.value === "featured") return true;
  if (featured.value === "unfeatured") return false;
  return undefined;
}
```

- [ ] **Step 6.4: Update `loadAll` to use derived filter values**

In `loadAll`, replace the `getSoftwareItems` call arguments — change `showUpdatableOnly`,
`pluginTypeFilter`, `nameFilter` to the new derived values. The function signature, loading/error state
management, and result processing (items, staleDetailIds, itemDetailsById, etc.) are UNCHANGED. Only
the API call arguments and the `currentPage = result.page` line change:

```typescript
// OLD API call (lines ~393-400):
const result = await getSoftwareItems(
  pg,
  undefined,
  featuredFilter(),
  undefined,
  showUpdatableOnly ? true : undefined,
  pluginTypeFilter || undefined,
  nameFilter || undefined,
);

// NEW API call:
const result = await getSoftwareItems(
  pg,
  undefined,
  featuredFilter(),
  undefined,
  updatable.value ? true : undefined,
  pluginType.value || undefined,
  queryParam.value || undefined,
);
```

Also **remove** the line `currentPage = result.page;` (line ~418). With `$derived` currentPage, this
assignment is a compile error. `totalPages` and `totalItems` assignments are unchanged.

Update `selectAllSoftwarePages` — same argument change only:

```typescript
// NEW call inside selectAllSoftwarePages:
const result = await getSoftwareItems(
  p,
  100,
  featuredFilter(),
  undefined,
  updatable.value ? true : undefined,
  pluginType.value || undefined,
  queryParam.value || undefined,
);
```

No other changes to `selectAllSoftwarePages`.

- [ ] **Step 6.5: Replace filter state, monolithic $effect, and pagination with $derived currentPage**

The plan uses **`$derived` currentPage** to avoid a double-goto race between a pagination-writer
`$effect` and `createUrlParam.set()`. With `$derived` currentPage, there is only ONE URL write path
(either `createUrlParam.set()` for filters or the `onPageChange` handler for pagination). A single
data-loading `$effect` reacts to BOTH filter changes and page changes automatically.

**5a. Change `currentPage` from `$state` to `$derived`:**

Find the line:

```typescript
let currentPage: number = $state(parseUrlPage(page.url));
```

Replace with:

```typescript
const currentPage = $derived(parseUrlPage(page.url));
```

**5b. Add `import { goto } from '$app/navigation';` if not already present (it should be).**

**5c. Remove the existing monolithic URL-sync `$effect` (lines ~244-258).** This effect built a URL
string from filter state and called `goto`. Delete it entirely. No replacement needed —
`createUrlParam.set()` and the `onPageChange` handler now handle all URL writes.

**5d. Remove `loadAll(currentPage)` call from `onMount`.** The `$effect` below handles initial load.

**5e. Add the single data-loading `$effect` after the filter `createUrlParam` declarations:**

```typescript
// Single reactive data-load: fires on initial mount AND whenever any filter or page changes.
$effect(() => {
  const _f = featured.value;
  const _u = updatable.value;
  const _p = pluginType.value;
  const _q = queryParam.value;
  const _page = currentPage;
  if (canView) loadAll(currentPage);
});
```

No `untrack()` needed — `currentPage` is `$derived` from URL, not `$state`, so there is no circular dependency.

**5f. Remove `currentPage = result.page` from `loadAll`.** Since `currentPage` is derived from the URL, it cannot be assigned. The lines to remove are:

```typescript
currentPage = result.page;
```

(line ~418 in the current file). `totalPages` and `totalItems` assignments remain.

**5g. Add a `resetToPage1()` helper function** (needed for batch operation callbacks that previously did `currentPage = 1`):

```typescript
function resetToPage1() {
  const next = new URL(page.url.href);
  next.searchParams.delete("page");
  void goto(next, { replaceState: true, keepFocus: true, noScroll: true });
}
```

Replace every `currentPage = 1;` assignment in non-onchange contexts (batch callbacks at lines
~1102, 1122, 1156, 1167 and `switchTab` at line 441 — but `switchTab` is removed in Step 6.6) with
`resetToPage1()`.

**5h. Update `onMount` event subscriptions** — keep them unchanged (`loadAll(currentPage, true)` still
works since `currentPage` is readable as a derived value).

Also remove `clearTimeout(nameFilterDebounce)` from `onDestroy` (ExpandableSearch owns debounce cleanup now).

- [ ] **Step 6.6: Remove `switchTab` function and the tab-validation $effect**

Remove `function switchTab(...)` entirely.
Remove the `$effect(() => { if (isItemsTab || activeTab === 'ignores') ... })` block.

- [ ] **Step 6.7: Update the template — replace TabStrip and Ignore Rules tab**

In the template, remove:

```svelte
<TabStrip items={tabItems} activeId={activeTab} ariaLabel="Software tabs" idBase="software" onSelect={switchTab} />
```

Replace the `{#if isItemsTab}` wrapper with `{#if canView}` (always show since there's no tab concept for main table).

The `{:else if activeTab === 'ignores'}` block (which rendered `<IgnoreRulesTab />`) is removed.
Instead, add a collapsible card AFTER the main software card (outside the existing `{#if}` block):

```svelte
<!-- After the main software group list card -->
<details class="rounded-card border border-[var(--border-subtle)] bg-[var(--bg-surface)] shadow-sm">
	<summary class="card-padding cursor-pointer select-none text-sm font-semibold">
		Ignore Rules
	</summary>
	<div class="border-t border-[var(--border-subtle)]">
		<IgnoreRulesTab />
	</div>
</details>
```

Surface tab panels (`{:else if showSurfaceSoftwareTabs}`) remain, but TabStrip is only rendered when `slotTabSurfaces.length > 0`:

```svelte
{#if showSurfaceSoftwareTabs}
	<TabStrip
		items={slotTabSurfaces.map((s) => ({ id: s.surface_id, label: s.label }))}
		activeId={activeSurfaceTab}
		ariaLabel="Extension tabs"
		idBase="software-surface"
		onSelect={(id) => (activeSurfaceTab = id)}
	/>
	{#each slotTabSurfaces as surface (surface.surface_id)}
		{#if activeSurfaceTab === surface.surface_id}
			<SectionCard title={surface.label}>
				<SurfaceReadPanel {surface} read={slotTabReads[surface.surface_id]} />
			</SectionCard>
		{/if}
	{/each}
{/if}
```

Note: replace `activeTab` references in surface tab rendering with a new `let activeSurfaceTab = $state(...)` scoped only to surface tabs.

- [ ] **Step 6.8: Replace the inline filter header with `FilterBar`**

Replace lines ~1081-1197 (the `<header>` inside the software groups div):

```svelte
<FilterBar>
	{#snippet filters()}
		{#if canManage}
			<div class="flex cursor-pointer select-none items-center gap-2 text-sm">
				<Checkbox
					id="software-batch-select-all"
					checked={allBatchPageSelected}
					indeterminate={!allBatchPageSelected && batchSelectedIds.size > 0}
					onchange={toggleBatchSelectAll}
				/>
				<label for="software-batch-select-all" class="cursor-pointer select-none">Select all</label>
			</div>
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
				{ value: 'unfeatured', label: 'Unfeatured' }
			]}
			onchange={(e) => {
				// No currentPage = 1 needed: createUrlParam.set() removes page= from URL,
				// and $derived currentPage auto-derives to 1 from the updated URL.
				featured.set((e.currentTarget as HTMLSelectElement).value as 'all' | 'featured' | 'unfeatured');
			}}
		/>
		<label class="flex cursor-pointer select-none items-center gap-2 text-sm">
			<Checkbox
				id="software-filter-updatable-only"
				checked={updatable.value}
				onchange={(e) => {
					updatable.set((e.currentTarget as HTMLInputElement).checked);
				}}
			/>
			Updates available
		</label>
		{#if pluginTypeOptions.length > 0}
			<Select
				id="software-plugin-filter"
				width="auto"
				value={pluginType.value}
				aria-label="Filter by plugin"
				options={[
					{ value: '', label: 'All plugins' },
					...pluginTypeOptions.map((opt) => ({ value: opt.plugin_type, label: opt.display_name }))
				]}
				onchange={(e) => {
					pluginType.set((e.currentTarget as HTMLSelectElement).value);
				}}
			/>
		{/if}
		<ExpandableSearch
			id="software-name-filter"
			value={queryParam.value}
			onchange={(v) => {
				queryParam.set(v);
			}}
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

Note: `Select.svelte` uses `value = $bindable()` but we must pass `value={featured.value}` (one-way)
plus `onchange` — not `bind:value`. This avoids a circular reactivity loop.

- [ ] **Step 6.9: Remove `TabStrip` from imports if no longer needed at top level**

Check if `TabStrip` is still used after changes. If only needed for surface tabs, keep the import. If unused, remove it from the import block.

- [ ] **Step 6.10: Update `software-name-filter.test.ts`**

Replace the file content:

```typescript
import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/svelte";
import { Permission } from "$lib/types";

vi.mock("$app/state", () => ({
  page: {
    url: new URL("http://localhost/software?query=foo"),
  },
}));

vi.mock("$app/navigation", () => ({ goto: vi.fn() }));
vi.mock("$lib/auth.svelte", () => ({ getUser: vi.fn(() => null) }));
vi.mock("$lib/api", () => ({
  getSoftwareItems: vi.fn(async () => ({
    items: [],
    page: 1,
    per_page: 50,
    total: 0,
    total_pages: 1,
  })),
  deleteSoftwareItem: vi.fn(async () => undefined),
  checkSoftwareItemVersions: vi.fn(async () => undefined),
  updateSoftwareItem: vi.fn(async () => undefined),
  listPluginTypes: vi.fn(async () => []),
  getSoftwareItem: vi.fn(async () => undefined),
  triggerSoftwareUpdate: vi.fn(async () => undefined),
  batchSoftwareItems: vi.fn(async () => undefined),
  executeBatchChunked: vi.fn(async () => undefined),
  previewSoftwareItemMerge: vi.fn(async () => undefined),
  executeSoftwareItemMerge: vi.fn(async () => undefined),
}));
vi.mock("$lib/stores/events.svelte", () => ({
  subscribeToEvent: vi.fn(() => () => {}),
}));
vi.mock("$lib/surfaces/registry.svelte", () => ({
  getSurfaceReadLoading: vi.fn(() => false),
  getSurfaceReadModel: vi.fn(() => undefined),
  getSurfaceReadRequested: vi.fn(() => false),
  getSurfacesBySlot: vi.fn(() => []),
  loadSurfaceReadModels: vi.fn(async () => {}),
}));
vi.mock("$lib/surfaces/read-model", () => ({
  filterSurfacesByPermission: vi.fn(() => []),
  isSurfaceTabPending: vi.fn(() => false),
}));
vi.mock("$lib/notifications.svelte", () => ({
  showSuccess: vi.fn(),
  showError: vi.fn(),
}));

import SoftwarePage from "./+page.svelte";
import * as auth from "$lib/auth.svelte";
import * as api from "$lib/api";
import { page } from "$app/state";

const viewUser = {
  id: "00000000-0000-0000-0000-000000000001",
  email: "user@example.com",
  first_name: "Test",
  last_name: "User",
  has_pending_email_change: false,
  permissions: [Permission.ViewSoftware],
};

describe("Software page — URL-reactive filter state", () => {
  beforeEach(() => {
    vi.mocked(auth.getUser).mockReturnValue(viewUser);
  });

  it("pre-populates search from ?query= in URL", async () => {
    // page.url is mocked to http://localhost/software?query=foo
    render(SoftwarePage);
    await waitFor(() =>
      expect(
        screen.getByRole("heading", { name: "Software" }),
      ).toBeInTheDocument(),
    );
    const input = screen.getByRole("searchbox") as HTMLInputElement;
    expect(input.value).toBe("foo");
  });

  it("passes query param to getSoftwareItems on mount", async () => {
    const nginxUrl = new URL("http://localhost/software?query=nginx");
    Object.defineProperty(page, "url", { value: nginxUrl, configurable: true });
    render(SoftwarePage);
    await waitFor(() =>
      expect(
        screen.getByRole("heading", { name: "Software" }),
      ).toBeInTheDocument(),
    );
    expect(vi.mocked(api.getSoftwareItems)).toHaveBeenCalledWith(
      expect.anything(),
      undefined,
      expect.anything(),
      undefined,
      undefined,
      undefined,
      "nginx",
    );
  });

  it("reads featured=all from URL and renders All option selected", async () => {
    const url = new URL("http://localhost/software?featured=all");
    Object.defineProperty(page, "url", { value: url, configurable: true });
    render(SoftwarePage);
    await waitFor(() =>
      expect(
        screen.getByRole("heading", { name: "Software" }),
      ).toBeInTheDocument(),
    );
    // featuredFilter() returns undefined when featured=all → getSoftwareItems called with undefined featured
    expect(vi.mocked(api.getSoftwareItems)).toHaveBeenCalledWith(
      expect.anything(),
      undefined,
      undefined,
      undefined,
      undefined,
      undefined,
      undefined,
    );
  });

  it("reads updatable=true from URL and passes to getSoftwareItems", async () => {
    const url = new URL("http://localhost/software?updatable=true");
    Object.defineProperty(page, "url", { value: url, configurable: true });
    render(SoftwarePage);
    await waitFor(() =>
      expect(
        screen.getByRole("heading", { name: "Software" }),
      ).toBeInTheDocument(),
    );
    expect(vi.mocked(api.getSoftwareItems)).toHaveBeenCalledWith(
      expect.anything(),
      undefined,
      expect.anything(),
      undefined,
      true,
      undefined,
      undefined,
    );
  });

  it("reads plugin_type=npm from URL and passes to getSoftwareItems", async () => {
    const url = new URL("http://localhost/software?plugin_type=npm");
    Object.defineProperty(page, "url", { value: url, configurable: true });
    render(SoftwarePage);
    await waitFor(() =>
      expect(
        screen.getByRole("heading", { name: "Software" }),
      ).toBeInTheDocument(),
    );
    expect(vi.mocked(api.getSoftwareItems)).toHaveBeenCalledWith(
      expect.anything(),
      undefined,
      expect.anything(),
      undefined,
      undefined,
      "npm",
      undefined,
    );
  });
});
```

- [ ] **Step 6.11: Run tests**

```bash
cd frontend && npm run test -- --reporter=verbose software
```

Expected: all tests pass.

- [ ] **Step 6.12: Run type-check**

```bash
cd frontend && npm run check 2>&1 | head -50
```

Expected: no TypeScript errors in software page.

- [ ] **Step 6.13: Commit**

```bash
git add frontend/src/routes/software/+page.svelte frontend/src/routes/software/software-name-filter.test.ts
git commit -m "feat(frontend): migrate /software page to URL-reactive filter state and FilterBar"
```

---

## Task 7: Migrate `/host-tags` page

**Files:**

- Modify: `frontend/src/routes/host-tags/+page.svelte`
- Modify: `frontend/src/routes/host-tags/host-tags.test.ts`

- [ ] **Step 7.1: Update imports in `+page.svelte`**

Add to the imports in `<script>`:

```typescript
import { createUrlParam } from "$lib/url-params.svelte";
import { FilterBar, ExpandableSearch } from "$lib/components/ui";
```

- [ ] **Step 7.2: Replace `searchQuery` $state with `createUrlParam`**

Remove:

```typescript
let searchQuery: string = $state("");
let searchTimeout: ReturnType<typeof setTimeout> | undefined;
```

Add:

```typescript
const queryParam = createUrlParam("query");
```

- [ ] **Step 7.3: Switch `currentPage` to `$derived`, remove URL-sync $effect, add single data-loading $effect**

Change:

```typescript
let currentPage: number = $state(parseUrlPage(page.url));
```

To:

```typescript
const currentPage = $derived(parseUrlPage(page.url));
```

Remove the existing `$effect` (lines ~82-88, writes `page=` to URL) — no replacement needed.
Remove `loadTags(currentPage)` call from `onMount` — the effect below handles initial load.

Add after the `queryParam` declaration:

```typescript
// Single reactive data-load: fires on initial mount and whenever query or page changes.
$effect(() => {
  const _q = queryParam.value;
  const _page = currentPage;
  loadTags(currentPage);
});
```

Keep all event subscriptions in `onMount` unchanged.

- [ ] **Step 7.4: Update `loadTags` to use `queryParam.value` and remove `currentPage = result.page`**

```typescript
async function loadTags(pg: number, background = false) {
  try {
    if (!background) error = null;
    const result = await getHostTags(
      pg,
      undefined,
      queryParam.value || undefined,
    );
    tags = result.items;
    // Do NOT add currentPage = result.page — currentPage is $derived from URL.
    totalPages = result.total_pages;
    totalItems = result.total;
    if (background) error = null;
  } catch (e) {
    if (!background) {
      error = e instanceof Error ? e.message : "Failed to load tags";
    }
  }
}
```

- [ ] **Step 7.5: Remove `handleSearchInput` function and `searchTimeout`**

Remove `function handleSearchInput(...)` — `ExpandableSearch` handles debounce internally.
Remove `if (searchTimeout) clearTimeout(searchTimeout)` from `onDestroy`.

Also remove from `selectAllPages` any reference to `searchQuery` — replace with `queryParam.value`.

- [ ] **Step 7.6: Update the template — remove Search SectionCard, add FilterBar**

Remove:

```svelte
<SectionCard title="Search">
	<Input id="search-tags" type="text" placeholder="Search tags..." value={searchQuery} oninput={handleSearchInput} />
</SectionCard>
```

Also remove the "Create Tag" button from the `PageShell`'s `{#snippet actions()}` (it moves to `FilterBar`'s actions slot):

```svelte
<!-- Remove from PageShell actions: -->
{#snippet actions()}
	{#if canManage}
		<Button variant="primary" onclick={openCreateDialog}>Create Tag</Button>
	{/if}
{/snippet}
```

Update the `SectionCard title="Tags"` to use the `filterBar` snippet prop:

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
	<!-- DataTable and pagination are unchanged — keep existing DataTable, EmptyState,
	     loading/error Callout, and TableFooterBar exactly as they are. Only the
	     SectionCard wrapper gains the filterBar snippet above. -->
</SectionCard>
```

- [ ] **Step 7.7: Update `host-tags.test.ts`**

Update the assertion that checks `SectionCard title="Search"` — it no longer exists. Add assertions for the new layout:

In the existing `'renders shared shell primitives...'` test, update or add:

```typescript
// SectionCard title="Search" must be gone.
expect(
  screen.queryByRole("heading", { name: "Search" }),
).not.toBeInTheDocument();

// FilterBar must be inside the Tags section.
expect(document.querySelector('[data-ui="filter-bar"]')).toBeInTheDocument();
```

Add a new test:

```typescript
it("ExpandableSearch is inside the table card header, not a separate SectionCard", async () => {
  render(HostTagsPage);
  await waitFor(() =>
    expect(screen.getByText("Host Tags")).toBeInTheDocument(),
  );
  const filterBar = document.querySelector('[data-ui="filter-bar"]')!;
  // The search icon button (collapsed state) is inside the filter bar.
  expect(filterBar.querySelector("button")).toBeInTheDocument();
  // No separate "Search" SectionCard.
  expect(
    screen.queryByRole("heading", { name: "Search" }),
  ).not.toBeInTheDocument();
});

it("query param is read from URL and passed to getHostTags", async () => {
  vi.mocked(api.getHostTags).mockResolvedValue(makePage([]));
  // Override page URL with query param.
  const { page: mockedPage } = await import("$app/state");
  Object.defineProperty(mockedPage, "url", {
    value: new URL("http://localhost/host-tags?query=prod"),
    configurable: true,
  });
  render(HostTagsPage);
  await waitFor(() =>
    expect(vi.mocked(api.getHostTags)).toHaveBeenCalledWith(
      expect.anything(),
      undefined,
      "prod",
    ),
  );
});
```

- [ ] **Step 7.8: Run tests**

```bash
cd frontend && npm run test -- --reporter=verbose host-tags
```

Expected: all tests pass.

- [ ] **Step 7.9: Commit**

```bash
git add frontend/src/routes/host-tags/+page.svelte frontend/src/routes/host-tags/host-tags.test.ts
git commit -m "feat(frontend): migrate /host-tags page to URL-reactive filter state and FilterBar"
```

---

## Task 8: Migrate `/history` page

**Files:**

- Modify: `frontend/src/routes/history/+page.svelte`
- Modify: `frontend/src/routes/history/history.test.ts`

- [ ] **Step 8.1: Add imports**

```typescript
import { createUrlParam } from "$lib/url-params.svelte";
import { FilterBar } from "$lib/components/ui";
```

- [ ] **Step 8.2: Replace `statusFilter` $state with `createUrlParam`**

Remove:

```typescript
let statusFilter: StatusFilter = $state(
  parseUrlParam(page.url, "status", STATUS_FILTER_VALUES, "all"),
);
```

Add:

```typescript
const statusParam = createUrlParam<StatusFilter>("status", {
  parse: (r): StatusFilter =>
    STATUS_FILTER_VALUES.includes(r as StatusFilter)
      ? (r as StatusFilter)
      : "all",
  serialize: (v) => (v === "all" ? null : v),
});
```

Replace all `statusFilter` references with `statusParam.value` throughout the script. In SSE event
handlers that check `statusFilter === 'all'` etc., replace with `statusParam.value === 'all'`.

Also: `const showSummaryStrip = $derived(statusParam.value === 'all' && currentPage === 1 && !loading && !error);`

- [ ] **Step 8.3: Switch `currentPage` to `$derived`, remove URL-sync $effect, add single data-loading $effect**

Change:

```typescript
let currentPage: number = $state(parseUrlPage(page.url));
```

To:

```typescript
const currentPage = $derived(parseUrlPage(page.url));
```

Remove the existing `$effect` that builds URL string — no replacement needed.
Remove `loadHistory(currentPage)` call from `onMount`.

Add after `statusParam` declaration:

```typescript
$effect(() => {
  const _status = statusParam.value;
  const _page = currentPage;
  if (canView) loadHistory(currentPage);
});
```

Also remove `currentPage = res.page` (line ~184) from inside `loadHistory`. Keep all event subscriptions in `onMount` unchanged.

- [ ] **Step 8.4: Update template — remove Filters SectionCard, add FilterBar to History Feed**

Remove:

```svelte
<SectionCard title="Filters">
	{#snippet actions()}
		{#if canManage}
			<Button variant="primary" size="sm" onclick={openTriggerModal}>Trigger Update</Button>
		{/if}
	{/snippet}
	<div class="flex gap-1 flex-wrap">
		{#each ['all', 'pending', 'in_progress', 'completed', 'failed'] as s (s)}
			<Button variant={statusFilter === s ? 'accent' : 'ghost'} ...>{chipLabel}</Button>
		{/each}
	</div>
</SectionCard>
```

Update `<SectionCard title="History Feed">` to use `filterBar` snippet:

```svelte
<SectionCard title="History Feed">
	{#snippet filterBar()}
		<FilterBar>
			{#snippet filters()}
				<Select
					id="history-status-filter"
					width="auto"
					aria-label="Filter by status"
					value={statusParam.value}
					options={[
						{ value: 'all',         label: 'All' },
						{ value: 'in_progress', label: 'In Progress' },
						{ value: 'queued',      label: 'Queued' },
						{ value: 'pending',     label: 'Pending' },
						{ value: 'failed',      label: 'Failed' },
						{ value: 'completed',   label: 'Completed' }
					]}
					onchange={(e) => {
						statusParam.set((e.currentTarget as HTMLSelectElement).value as StatusFilter);
					}}
				/>
			{/snippet}
			{#snippet actions()}
				{#if canManage}
					<Button variant="primary" size="sm" onclick={openTriggerModal}>Trigger Update</Button>
				{/if}
			{/snippet}
		</FilterBar>
	{/snippet}
	<!-- All content between {/snippet} and </SectionCard> is UNCHANGED:
	     summary strip, loading Callout, error Callout, EmptyState, history feed list,
	     and TableFooterBar. Only the filterBar snippet block is new. -->
</SectionCard>
```

- [ ] **Step 8.5: Update `history.test.ts` — remove button-chip tests, add Select tests**

Find and update the filter chip tests. The button chips are gone; the Select is now the filter control.

Remove tests that assert accent button variant on filter chips.
Replace with:

Add these mocks at the top of the test file (alongside existing `vi.mock` calls):

```typescript
vi.mock("$app/navigation", () => ({ goto: vi.fn() }));
```

Add `goto` to the imports:

```typescript
import { goto } from "$app/navigation";
```

Add `fireEvent` to the `@testing-library/svelte` import if not already present.

```typescript
describe("status filter Select", () => {
  it('Select is present inside [data-ui="filter-bar"]', async () => {
    render(HistoryPage);
    await waitFor(() =>
      expect(screen.getByLabelText("Filter by status")).toBeInTheDocument(),
    );
    const select = screen.getByLabelText(
      "Filter by status",
    ) as HTMLSelectElement;
    expect(select.value).toBe("all");
    expect(
      document.querySelector('[data-ui="filter-bar"]')?.contains(select),
    ).toBe(true);
  });

  it('no separate "Filters" SectionCard', async () => {
    render(HistoryPage);
    await waitFor(() =>
      expect(screen.getByText("History Feed")).toBeInTheDocument(),
    );
    expect(
      screen.queryByRole("heading", { name: "Filters" }),
    ).not.toBeInTheDocument();
  });

  it("changing select calls loadHistory with new status", async () => {
    render(HistoryPage);
    await waitFor(() =>
      expect(screen.getByLabelText("Filter by status")).toBeInTheDocument(),
    );
    // Verify initial call uses no status filter.
    expect(vi.mocked(api.listUpdateHistory)).toHaveBeenCalledWith(
      expect.objectContaining({ status: undefined }),
    );
    vi.mocked(api.listUpdateHistory).mockClear();
    // Change the select to 'failed'.
    const select = screen.getByLabelText(
      "Filter by status",
    ) as HTMLSelectElement;
    await fireEvent.change(select, { target: { value: "failed" } });
    // goto is called (filter param change).
    expect(vi.mocked(goto)).toHaveBeenCalled();
  });
});
```

- [ ] **Step 8.6: Run tests**

```bash
cd frontend && npm run test -- --reporter=verbose history
```

Expected: all tests pass.

- [ ] **Step 8.7: Commit**

```bash
git add frontend/src/routes/history/+page.svelte frontend/src/routes/history/history.test.ts
git commit -m "feat(frontend): migrate /history page to URL-reactive filter state and FilterBar"
```

---

## Task 9: Migrate `/services` page

**Files:**

- Modify: `frontend/src/routes/services/+page.svelte`
- Modify: `frontend/src/routes/services/services.test.ts`

- [ ] **Step 9.1: Add imports and replace `capabilityFilter` $state**

Add imports:

```typescript
import { createUrlParam } from "$lib/url-params.svelte";
import { FilterBar } from "$lib/components/ui";
```

Remove:

```typescript
let capabilityFilter: CapabilityFilter = $state(
  parseUrlParam(page.url, "capability", CAPABILITY_FILTER_VALUES, "all"),
);
```

Add:

```typescript
const capabilityParam = createUrlParam<CapabilityFilter>("capability", {
  parse: (r): CapabilityFilter =>
    CAPABILITY_FILTER_VALUES.includes(r as CapabilityFilter)
      ? (r as CapabilityFilter)
      : "all",
  serialize: (v) => (v === "all" ? null : v),
});
```

Replace all `capabilityFilter` references with `capabilityParam.value`.
Remove `function setFilter(...)`.

- [ ] **Step 9.2: Switch `currentPage` to `$derived`, remove URL-sync $effect, add single data-loading $effect**

Change `let currentPage: number = $state(...)` to:

```typescript
const currentPage = $derived(parseUrlPage(page.url));
```

Remove the existing `$effect` that builds URL string (lines ~130-135) — no replacement.
Remove `loadServices(...)` call from `onMount`.

Add:

```typescript
$effect(() => {
  const _cap = capabilityParam.value;
  const _page = currentPage;
  if (canView) loadServices(currentPage);
});
```

Also remove `currentPage = result.page` (or equivalent) from `loadServices`.

Update `loadServices` to use `capabilityParam.value`:

```typescript
async function loadServices(page: number, background = false, retry = false) {
  // ...
  const result = await getServices({
    page,
    capability:
      capabilityParam.value === "all" ? undefined : capabilityParam.value,
    // ...
  });
}
```

- [ ] **Step 9.3: Update template — remove Service Filters SectionCard, add FilterBar**

Remove `<SectionCard title="Service Filters">` with button group.

Update `<SectionCard title="Registered Services">` to use `filterBar` snippet:

```svelte
<SectionCard title="Registered Services">
	{#snippet filterBar()}
		<FilterBar>
			{#snippet filters()}
				<Select
					id="services-capability-filter"
					width="auto"
					aria-label="Filter by capability"
					value={capabilityParam.value}
					options={[
						{ value: 'all',                label: 'All' },
						{ value: 'software_discovery', label: 'Software Discovery' },
						{ value: 'ssh_remote',         label: 'SSH Remote' }
					]}
					onchange={(e) => {
						capabilityParam.set((e.currentTarget as HTMLSelectElement).value as CapabilityFilter);
					}}
				/>
			{/snippet}
		</FilterBar>
	{/snippet}
	<!-- All content between {/snippet} and </SectionCard> is UNCHANGED:
	     loading/error Callout, EmptyState, DataTable, TableFooterBar. -->
</SectionCard>
```

- [ ] **Step 9.4: Update `services.test.ts` — remove button-chip tests, add Select tests**

Remove the `describe('capability filter chips', ...)` block and the test at line 128 that fires a button click.

Add:

```typescript
describe("capability filter Select", () => {
  beforeEach(() => {
    vi.mocked(api.getServices).mockResolvedValue(makePage([]));
  });

  it('Select is present inside [data-ui="filter-bar"]', async () => {
    render(ServicesPage);
    await waitFor(() =>
      expect(screen.getByLabelText("Filter by capability")).toBeInTheDocument(),
    );
    const select = screen.getByLabelText(
      "Filter by capability",
    ) as HTMLSelectElement;
    expect(select.value).toBe("all");
  });

  it('no separate "Service Filters" SectionCard', async () => {
    render(ServicesPage);
    await waitFor(() =>
      expect(
        screen.getByRole("heading", { name: "Registered Services" }),
      ).toBeInTheDocument(),
    );
    expect(
      screen.queryByRole("heading", { name: "Service Filters" }),
    ).not.toBeInTheDocument();
  });

  it("initial load calls getServices with no capability filter", async () => {
    render(ServicesPage);
    await waitFor(() => expect(vi.mocked(api.getServices)).toHaveBeenCalled());
    expect(vi.mocked(api.getServices)).toHaveBeenCalledWith(
      expect.objectContaining({ capability: undefined }),
    );
  });
});
```

- [ ] **Step 9.5: Run tests**

```bash
cd frontend && npm run test -- --reporter=verbose services
```

Expected: all tests pass.

- [ ] **Step 9.6: Commit**

```bash
git add frontend/src/routes/services/+page.svelte frontend/src/routes/services/services.test.ts
git commit -m "feat(frontend): migrate /services page to URL-reactive filter state and FilterBar"
```

---

## Task 10: Migrate `/system-services` page

**Files:**

- Modify: `frontend/src/routes/system-services/+page.svelte`
- Modify: `frontend/src/routes/system-services/system-services.test.ts`

Same migration pattern as Task 9 but for `statusFilter` on system-services.

- [ ] **Step 10.1: Add imports and replace `statusFilter` $state**

```typescript
import { createUrlParam } from "$lib/url-params.svelte";
import { FilterBar } from "$lib/components/ui";
```

The page has:

```typescript
const STATUS_FILTER_VALUES = [
  "all",
  "pending",
  "approved",
  "rejected",
  "deactivated",
] as const;
type StatusFilter = (typeof STATUS_FILTER_VALUES)[number];
let statusFilter: StatusFilter = $state(
  parseUrlParam(page.url, "status", STATUS_FILTER_VALUES, "all"),
);
```

Replace `statusFilter` $state:

```typescript
const statusParam = createUrlParam<StatusFilter>("status", {
  parse: (r): StatusFilter =>
    STATUS_FILTER_VALUES.includes(r as StatusFilter)
      ? (r as StatusFilter)
      : "all",
  serialize: (v) => (v === "all" ? null : v),
});
```

Replace all `statusFilter` references with `statusParam.value`.
Remove `function setFilter(...)`.

- [ ] **Step 10.2: Switch `currentPage` to `$derived`, remove URL-sync $effect, add single data-loading $effect**

Change `let currentPage: number = $state(...)` to:

```typescript
const currentPage = $derived(parseUrlPage(page.url));
```

Remove the existing URL-sync `$effect` — no replacement. Remove `loadSystemServices(...)` call from `onMount`.

Add:

```typescript
$effect(() => {
  const _status = statusParam.value;
  const _page = currentPage;
  if (canView) loadSystemServices(currentPage);
});
```

Update `loadSystemServices` to use `statusParam.value` and remove `currentPage = result.page`:

```typescript
const result = await getSystemServices({
  page,
  status: statusParam.value === "all" ? undefined : statusParam.value,
  // ... other params unchanged ...
});
// Do NOT add currentPage = result.page.
```

- [ ] **Step 10.3: Update template**

Remove `<SectionCard title="Status Filters">` with button group.

Update `<SectionCard title="Registered System Services">` to use `filterBar` snippet:

```svelte
<SectionCard title="Registered System Services">
	{#snippet filterBar()}
		<FilterBar>
			{#snippet filters()}
				<Select
					id="system-services-status-filter"
					width="auto"
					aria-label="Filter by status"
					value={statusParam.value}
					options={[
						{ value: 'all',         label: 'All' },
						{ value: 'pending',     label: 'Pending' },
						{ value: 'approved',    label: 'Approved' },
						{ value: 'rejected',    label: 'Rejected' },
						{ value: 'deactivated', label: 'Deactivated' }
					]}
					onchange={(e) => {
						statusParam.set((e.currentTarget as HTMLSelectElement).value as StatusFilter);
					}}
				/>
			{/snippet}
		</FilterBar>
	{/snippet}
	<!-- All content between {/snippet} and </SectionCard> is UNCHANGED:
	     loading/error Callout, EmptyState, DataTable, TableFooterBar. -->
</SectionCard>
```

- [ ] **Step 10.4: Update `system-services.test.ts` — remove button-chip tests, add Select tests**

Remove the `describe('status filter chips', ...)` block.

Add:

```typescript
describe("status filter Select", () => {
  beforeEach(() => {
    vi.mocked(api.getSystemServices).mockResolvedValue(makePage([]));
  });

  it('Select is present inside [data-ui="filter-bar"]', async () => {
    render(SystemServicesPage);
    await waitFor(() =>
      expect(screen.getByLabelText("Filter by status")).toBeInTheDocument(),
    );
    const select = screen.getByLabelText(
      "Filter by status",
    ) as HTMLSelectElement;
    expect(select.value).toBe("all");
  });

  it('no separate "Status Filters" SectionCard', async () => {
    render(SystemServicesPage);
    await waitFor(() =>
      expect(
        screen.getByRole("heading", { name: "Registered System Services" }),
      ).toBeInTheDocument(),
    );
    expect(
      screen.queryByRole("heading", { name: "Status Filters" }),
    ).not.toBeInTheDocument();
  });

  it("initial load calls getSystemServices with no status filter", async () => {
    render(SystemServicesPage);
    await waitFor(() =>
      expect(vi.mocked(api.getSystemServices)).toHaveBeenCalled(),
    );
    expect(vi.mocked(api.getSystemServices)).toHaveBeenCalledWith(
      expect.objectContaining({ status: undefined }),
    );
  });
});
```

- [ ] **Step 10.5: Run tests**

```bash
cd frontend && npm run test -- --reporter=verbose system-services
```

Expected: all tests pass.

- [ ] **Step 10.6: Commit**

```bash
git add frontend/src/routes/system-services/+page.svelte frontend/src/routes/system-services/system-services.test.ts
git commit -m "feat(frontend): migrate /system-services page to URL-reactive filter state and FilterBar"
```

---

## Task 11: `/hosts` page — wire empty FilterBar

**Files:**

- Modify: `frontend/src/routes/hosts/+page.svelte`

The hosts page has no filter params yet (`getHosts()` does not accept `query`). However, the spec
requires `[data-ui="filter-bar"]` to be present (for layout consistency and e2e parity test in
Task 12). Add an empty `FilterBar` to the `SectionCard title="Registered Hosts"` header. No
`ExpandableSearch` — adding a search widget that doesn't filter would be a false affordance.

- [ ] **Step 11.1: Add FilterBar import to hosts page**

Add to the `<script>` imports in `frontend/src/routes/hosts/+page.svelte`:

```typescript
import { FilterBar } from "$lib/components/ui";
```

- [ ] **Step 11.2: Add filterBar snippet prop to Registered Hosts SectionCard**

Find `<SectionCard title="Registered Hosts">` (there should be one such card containing the DataTable). Add the `filterBar` snippet as the first child:

```svelte
<SectionCard title="Registered Hosts">
	{#snippet filterBar()}
		<!-- ExpandableSearch will be added here once getHosts() supports the query param (deferred). -->
		<FilterBar />
	{/snippet}
	<!-- Existing DataTable, EmptyState, TableFooterBar unchanged. -->
</SectionCard>
```

Note: `<FilterBar />` with no snippets renders a valid empty header with `data-ui="filter-bar"`. The
`border-b` on the title row is suppressed by the `filterBar` presence, and `FilterBar`'s own
`border-b` provides the single divider.

- [ ] **Step 11.3: Run tests**

```bash
cd frontend && npm run test -- --reporter=verbose hosts
```

Expected: all tests pass (existing tests unchanged; FilterBar renders without errors).

- [ ] **Step 11.4: Commit**

```bash
git add frontend/src/routes/hosts/+page.svelte
git commit -m "feat(frontend): wire empty FilterBar to /hosts page for layout consistency"
```

---

## Task 12: Playwright e2e parity test

**Files:**

- Create: `frontend/tests/e2e/filter-bar-parity.spec.ts`

- [ ] **Step 12.1: Check existing Playwright setup**

```bash
ls frontend/tests/e2e/ | head -10
cat frontend/playwright.config.ts | head -20
```

Confirm base URL and test structure.

- [ ] **Step 12.2: Create parity test file**

```typescript
// frontend/tests/e2e/filter-bar-parity.spec.ts
import { expect, test } from "@playwright/test";

/**
 * Parity tests: verify that separate filter SectionCards are gone and
 * FilterBar is integrated into the table card header on all in-scope pages.
 *
 * These tests require the dev server to be running with a valid session.
 * Run: npm run test:e2e -- filter-bar-parity
 */

const PAGES_WITH_FILTER_BAR = [
  { path: "/software", title: "Software", removedSection: null },
  { path: "/host-tags", title: "Host Tags", removedSection: "Search" },
  { path: "/history", title: "History Feed", removedSection: "Filters" },
  {
    path: "/services",
    title: "Registered Services",
    removedSection: "Service Filters",
  },
  {
    path: "/system-services",
    title: "Registered System Services",
    removedSection: "Status Filters",
  },
  { path: "/hosts", title: "Registered Hosts", removedSection: null },
];

for (const { path, title, removedSection } of PAGES_WITH_FILTER_BAR) {
  test(`${path}: [data-ui="filter-bar"] is present in the table card`, async ({
    page,
  }) => {
    await page.goto(path);
    await expect(page.locator(`h1, h2`).filter({ hasText: title })).toBeVisible(
      { timeout: 10_000 },
    );
    await expect(page.locator('[data-ui="filter-bar"]')).toBeVisible();
  });

  if (removedSection) {
    test(`${path}: separate "${removedSection}" SectionCard is absent`, async ({
      page,
    }) => {
      await page.goto(path);
      await expect(
        page.locator(`h1, h2`).filter({ hasText: title }),
      ).toBeVisible({ timeout: 10_000 });
      await expect(
        page.locator(`h2`).filter({ hasText: removedSection }),
      ).not.toBeVisible();
    });
  }
}

test("/software: TabStrip with All/Featured/Unfeatured tabs is absent", async ({
  page,
}) => {
  await page.goto("/software");
  await expect(
    page.locator("h1, h2").filter({ hasText: "Software" }),
  ).toBeVisible({ timeout: 10_000 });
  // The old TabStrip rendered buttons with these exact labels at top of page.
  await expect(
    page.locator('[role="tablist"] button').filter({ hasText: "Featured" }),
  ).not.toBeVisible();
  await expect(
    page.locator('[role="tablist"] button').filter({ hasText: "Unfeatured" }),
  ).not.toBeVisible();
  // The featured Select is present in the FilterBar instead.
  await expect(
    page.locator(
      '[data-ui="filter-bar"] select, [data-ui="filter-bar"] [aria-label="Filter by featured status"]',
    ),
  ).toBeVisible();
});

test("/software: URL reactivity — navigating to ?updatable=true applies filter", async ({
  page,
}) => {
  // Start on /software without any filter.
  await page.goto("/software");
  await expect(page.locator("h1").filter({ hasText: "Software" })).toBeVisible({
    timeout: 10_000,
  });

  // Navigate to the same page with ?updatable=true (simulates clicking an external badge).
  await page.goto("/software?updatable=true");

  // The updatable checkbox should be checked.
  const checkbox = page.locator("#software-filter-updatable-only");
  await expect(checkbox).toBeChecked({ timeout: 5_000 });
});

test("/host-tags: ExpandableSearch is inside table card, not separate SectionCard", async ({
  page,
}) => {
  await page.goto("/host-tags");
  await expect(page.locator("h1").filter({ hasText: "Host Tags" })).toBeVisible(
    { timeout: 10_000 },
  );
  const filterBar = page.locator('[data-ui="filter-bar"]');
  await expect(filterBar).toBeVisible();
  // The search icon button is inside the filter bar.
  await expect(filterBar.locator("button").first()).toBeVisible();
  // No separate "Search" heading.
  await expect(
    page.locator("h2").filter({ hasText: "Search" }),
  ).not.toBeVisible();
});
```

- [ ] **Step 12.3: Note on running the e2e tests**

These tests require the full app (dev server + database). Run them with:

```bash
cd frontend && npm run test:e2e -- filter-bar-parity
```

They are excluded from unit test runs (`vitest.config.ts` excludes `tests/e2e/**`).

- [ ] **Step 12.4: Commit**

```bash
git add frontend/tests/e2e/filter-bar-parity.spec.ts
git commit -m "test(frontend): add Playwright parity tests for FilterBar integration"
```

---

## Task 13: Update `primitives.md` documentation

**Files:**

- Modify: `docs/development/ui/primitives.md`

- [ ] **Step 13.1: Add `createUrlParam`, `FilterBar`, and `ExpandableSearch` entries**

Find the appropriate section in `docs/development/ui/primitives.md` (after the existing component list). Add:

````markdown
### `createUrlParam`

**File:** `frontend/src/lib/url-params.svelte.ts`
**Import:** `import { createUrlParam } from '$lib/url-params.svelte';`

Factory that creates a URL search-parameter binding backed by `$derived` from `page.url.searchParams`.
Use it for any filter value that should survive page navigation and be shareable via URL.

**Signature:** `createUrlParam<T>(key: string, options?: UrlParamOptions<T>): UrlParam<T>`

**Returns:** `{ readonly value: T; set(v: T): void }` — `value` is reactive (updates on every URL change); `set()` navigates via `goto(replaceState: true)` and **always removes `page=` from the URL** (pagination reset on every filter change).

**Options:**

- `parse?: (raw: string | null) => T` — converts the raw URL string to your type. Default: identity (empty string when absent).
- `serialize?: (v: T) => string | null` — converts value back to string. Return `null` to omit the param. Default: omits when empty string or null.

**CONSTRAINT:** Must be called at component initialisation scope only — top-level `<script>` block in `.svelte` files. Calling inside a callback, event handler, or `$effect` throws a rune-outside-reactive-context error.

**Pagination note:** `currentPage` on all in-scope pages is `$derived(parseUrlPage(page.url))`. A single data-loading `$effect` tracks both filter params and `currentPage`. When `set()` removes `page=`, `currentPage` auto-derives to 1 — no separate `currentPage = 1` assignment needed in `onchange` handlers.

---

### `FilterBar`

**File:** `frontend/src/lib/components/ui/FilterBar.svelte`
**Export:** `$lib/components/ui`

Layout shell for table card header rows. Purely presentational — no filter logic. Provides a standard two-zone flex layout: filters on the left, primary actions on the right.

**Props:**

- `filters?: Snippet` — filter controls (Selects, Checkboxes, ExpandableSearch). Rendered in a flex-wrap row on the left.
- `actions?: Snippet` — primary action buttons (e.g. "Add Software", "Create Tag"). Rendered on the right with `shrink-0`. Omit entirely if no actions.

**Renders:** `<header data-ui="filter-bar">` with `border-b border-[var(--border-subtle)]`, `bg-[var(--bg-raised)]`, `card-padding`. On small screens, stacks vertically.

**Usage:**

```svelte
<FilterBar>
  {#snippet filters()}
    <Select id="..." ... />
    <ExpandableSearch ... />
  {/snippet}
  {#snippet actions()}
    <Button variant="primary">Create</Button>
  {/snippet}
</FilterBar>
```
````

**With SectionCard:** Pass a `FilterBar` inside the `filterBar` snippet prop on `SectionCard`.
The `SectionCard` suppresses its own title row `border-b` when `filterBar` is provided;
the `FilterBar` carries the single divider between title+filter and content.

```svelte
<SectionCard title="Tags">
  {#snippet filterBar()}
    <FilterBar>...</FilterBar>
  {/snippet}
  ...
</SectionCard>
```

**Do NOT use** `bind:value` on any Select or Input inside `FilterBar` when the value is
backed by a URL param (circular reactivity). Use one-way `value={param.value}` + `onchange`.

---

### `ExpandableSearch`

**File:** `frontend/src/lib/components/ui/ExpandableSearch.svelte`
**Export:** `$lib/components/ui`

Expandable search input widget for use inside `FilterBar`. Starts as a ghost icon button;
expands to a full `Input type="search"` on click. Controlled component — parent owns the
committed (URL-synced) value.

**Props:**

- `id: string` — passed to the underlying `<Input>` for accessibility.
- `value: string` — the committed value (from `queryParam.value`). Non-empty → always shown expanded.
- `onchange: (v: string) => void` — called after debounce completes. Should call `queryParam.set(v)`.
- `placeholder?: string` — default `'Search...'`. Used as `aria-label` on the icon button.
- `debounceMs?: number` — default `300`. Delay before `onchange` fires.

**Behaviour:** Escape key and clear button both call `onchange('')` and collapse.
Auto-focuses the input on expand via `tick()`.
External `value` prop changes sync local state via `$effect` (handles browser back/external nav).

**Usage with `createUrlParam`:**

```svelte
<script lang="ts">
  import { createUrlParam } from '$lib/url-params.svelte';
  const queryParam = createUrlParam('query');
</script>

<ExpandableSearch
  id="my-name-filter"
  value={queryParam.value}
  onchange={(v) => queryParam.set(v)}
  placeholder="Filter by name"
/>
```

````markdown

- [ ] **Step 13.2: Run markdownlint**

```bash
npx markdownlint --config .markdownlint.json 'docs/development/ui/primitives.md'
````

Expected: no errors. Fix any line-length violations (limit is 150 chars per `.markdownlint.json`).

- [ ] **Step 13.3: Commit**

```bash
git add docs/development/ui/primitives.md
git commit -m "docs(frontend): document FilterBar and ExpandableSearch components in primitives.md"
```

---

## Task 14: Full QA pass

- [ ] **Step 14.1: Run full frontend test suite**

```bash
cd frontend && npm run test 2>&1 | tail -30
```

Expected: all tests pass (zero failures).

- [ ] **Step 14.2: TypeScript type check**

```bash
cd frontend && npm run check 2>&1 | tail -20
```

Expected: zero TypeScript errors.

- [ ] **Step 14.3: Lint**

```bash
cd frontend && npm run lint 2>&1 | tail -20
```

Expected: zero lint errors.

- [ ] **Step 14.4: Build**

```bash
cd frontend && npm run build 2>&1 | tail -20
```

Expected: build succeeds.

- [ ] **Step 14.5: Final commit if any post-QA fixes were needed**

```bash
git add -p  # stage only QA fix changes
git commit -m "fix(frontend): post-QA cleanup for unified filter bars"
```

---

## Self-Review Checklist

**Spec coverage:**

- [x] `createUrlParam` factory → Task 1
- [x] `Input.svelte` `bind:el` prop → Task 2
- [x] `FilterBar.svelte` → Task 3
- [x] `ExpandableSearch.svelte` → Task 4
- [x] `SectionCard.filterBar` snippet prop → Task 5
- [x] `/software` migration (featured param, TabStrip removal, Ignore Rules collapsible) → Task 6
- [x] `/host-tags` migration (URL-persist query, SectionCard Search removal) → Task 7
- [x] `/history` migration (6 status options including queued) → Task 8
- [x] `/services` migration (Select with Software Discovery / SSH Remote labels) → Task 9
- [x] `/system-services` migration → Task 10
- [x] `/hosts` — no filter changes; ExpandableSearch deferred (false affordance prevention) → Task 11
- [x] Playwright e2e parity tests → Task 12
- [x] `primitives.md` documentation → Task 13
- [x] Full QA → Task 14

**Type consistency check:**

- `createUrlParam` returns `UrlParam<T>` with `{ value: T; set(v: T): void }` — consistent with all page usages.
- `FilterBar` props: `filters?: Snippet`, `actions?: Snippet` — consistent across Tasks 3, 6, 7, 8, 9, 10.
- `ExpandableSearch` props: `id, value, onchange, placeholder?, debounceMs?` — consistent across Tasks 4, 6, 7.
- `SectionCard` new prop: `filterBar?: Snippet` — consistent across Tasks 5, 7, 8, 9, 10.
- `Input` new prop: `el?: HTMLInputElement` (bindable) — used in Task 4 via `bind:el={inputEl}`.

**Idiomatic patterns confirmed:**

- No `bind:value` on URL-backed Select controls (circular reactivity).
- `$derived` in factory function is valid in `.svelte.ts` at module scope.
- `$derived` used for `currentPage` on all pages — same URL-as-source-of-truth pattern as filter
  params. No `untrack()` needed. Single data-loading `$effect` per page tracks filter params +
  `currentPage`.
- `$effect` cleanup via return function for timer in `ExpandableSearch`.

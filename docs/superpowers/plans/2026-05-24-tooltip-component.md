# Tooltip Primitive + RadioCardGroup Migration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a general-purpose `Tooltip` UI primitive (portaled bubble, info-icon trigger, arrow,
opacity fade) and migrate `RadioCardGroup` option descriptions from inline text to this tooltip.

**Architecture:** When `content` is non-empty, `Tooltip.svelte` keeps its bubble in the DOM at all
times (portaled to `<body>` via the existing `use:portal` action), hiding it with CSS only rather
than `{#if visible}` — same pattern as `ContextMenu.svelte`. An empty-string `content` prop renders
nothing (no trigger, no bubble).
`RadioCardGroup` cards change from `<button>` to `<div role="radio">` to allow the nested tooltip
`<button>` trigger without invalid HTML. Show/hide is driven by `mouseenter`/`focus` events with a
100 ms debounce delay (WCAG 1.4.13).

**Tech Stack:** Svelte 5 (runes: `$state`, `$effect`, `$props`), TypeScript strict, Tailwind CSS,
lucide-svelte, Vitest + @testing-library/svelte

---

## File Map

| Action | File                                                       | Reason                                     |
| ------ | ---------------------------------------------------------- | ------------------------------------------ |
| Create | `frontend/src/lib/components/ui/Tooltip.svelte`            | New primitive                              |
| Create | `frontend/src/lib/components/ui/Tooltip.test.ts`           | Unit tests for Tooltip                     |
| Modify | `frontend/src/lib/components/ui/index.ts`                  | Export Tooltip from barrel                 |
| Modify | `frontend/src/app.css`                                     | Add `[data-ui='tooltip']` z-index rule     |
| Modify | `frontend/src/lib/theme/css-contract.test.ts`              | Assert the new z-index rule                |
| Modify | `frontend/src/lib/components/forms/RadioCardGroup.svelte`  | Rename field, change element, wire Tooltip |
| Modify | `frontend/src/lib/components/forms/RadioCardGroup.test.ts` | Update fixtures, add tooltip tests         |
| Modify | `frontend/src/routes/settings/AccessSettings.svelte`       | Rename description→tooltip in modeOptions  |
| Modify | `docs/development/ui/primitives.md`                        | Add Tooltip section, update RadioCardGroup |
| Modify | `docs/development/ui/tokens.md`                            | Update z-index table note for tooltip      |

---

## Task 1: CSS z-index pin + contract test

**Files:**

- Modify: `frontend/src/app.css` (after line 137, after `[data-ui='context-menu-shell']` block)
- Modify: `frontend/src/lib/theme/css-contract.test.ts`

- [ ] **Step 1: Add the failing contract test assertion**

  Open `frontend/src/lib/theme/css-contract.test.ts`. Inside the `'pins the shared layering z-index contract in app.css'`
  test, append this line after the existing `expect` calls:

  ```typescript
  expect(appCss).toMatch(/\[data-ui='tooltip'\][\s\S]*?z-index:\s*100;/);
  ```

  The test block should now look like:

  ```typescript
  it("pins the shared layering z-index contract in app.css", () => {
    expect(appCss).toMatch(
      /\[data-ui='app-shell-header'\][\s\S]*?z-index:\s*10;/,
    );
    expect(appCss).toMatch(
      /\[data-ui='app-shell-sidebar'\][\s\S]*?z-index:\s*20;/,
    );
    expect(appCss).toMatch(
      /\[data-ui='context-menu-shell'\][\s\S]*?z-index:\s*100;/,
    );
    expect(appCss).toMatch(
      /\[data-ui='toast-notifications'\][\s\S]*?z-index:\s*920;/,
    );
    expect(appCss).toMatch(
      /\[data-ui='modal-backdrop'\][\s\S]*?z-index:\s*900;/,
    );
    expect(appCss).toMatch(/\[data-ui='modal-shell'\][\s\S]*?z-index:\s*910;/);
    expect(appCss).toMatch(/\[data-ui='tooltip'\][\s\S]*?z-index:\s*100;/);
  });
  ```

- [ ] **Step 2: Run the test to confirm it fails**

  ```bash
  cd frontend && npm test -- --reporter=verbose src/lib/theme/css-contract.test.ts
  ```

  Expected: FAIL — `expected string to match /\[data-ui='tooltip'\][\s\S]*?z-index:\s*100;/`

- [ ] **Step 3: Add the CSS rule to `app.css`**

  In `frontend/src/app.css`, find the `[data-ui='context-menu-shell']` block and add the tooltip
  rule immediately after it:

  ```css
  [data-ui='context-menu-shell'] {
  	z-index: 100;
  }

  [data-ui='tooltip'] {
  	z-index: 100;
  }
  ```

  Use a tab for indentation (Prettier config: `useTabs: true`). Use single quotes around the
  attribute value — all existing rules in `app.css` use single quotes and the contract test
  regex asserts single-quote form.

- [ ] **Step 4: Run the test to confirm it passes**

  ```bash
  cd frontend && npm test -- --reporter=verbose src/lib/theme/css-contract.test.ts
  ```

  Expected: PASS

- [ ] **Step 5: Commit**

  ```bash
  git add frontend/src/app.css frontend/src/lib/theme/css-contract.test.ts
  git commit -m "feat(ui): pin tooltip z-index in app.css and css-contract test"
  ```

---

## Task 2: Tooltip — static structure + ARIA + content guard

Create the component shell with correct ARIA, the portal bubble always in the DOM, the
`content=""` guard, and the module-level ID counter. No show/hide logic yet — the bubble renders
with `invisible opacity-0` permanently at this point.

**Files:**

- Create: `frontend/src/lib/components/ui/Tooltip.svelte`
- Create: `frontend/src/lib/components/ui/Tooltip.test.ts`

- [ ] **Step 1: Write failing tests**

  Create `frontend/src/lib/components/ui/Tooltip.test.ts`:

  ```typescript
  import { describe, expect, it } from "vitest";
  import { render, screen } from "@testing-library/svelte";
  import Tooltip from "./Tooltip.svelte";

  describe("Tooltip", () => {
    it("renders nothing when content is empty string", () => {
      const { container } = render(Tooltip, { content: "" });
      expect(container.querySelector("button")).toBeNull();
      // use:portal appends to document.body — verify no bubble portaled either
      expect(document.body.querySelector('[role="tooltip"]')).toBeNull();
    });

    it("renders trigger button when content is provided", () => {
      render(Tooltip, { content: "Hello world" });
      expect(
        screen.getByRole("button", { name: "More information" }),
      ).toBeTruthy();
    });

    it("trigger aria-describedby matches tooltip id", () => {
      render(Tooltip, { content: "Hello world" });
      const trigger = screen.getByRole("button", { name: "More information" });
      const tooltipId = trigger.getAttribute("aria-describedby");
      expect(tooltipId).toBeTruthy();
      const tooltipEl = document.getElementById(tooltipId!);
      expect(tooltipEl).toBeTruthy();
      expect(tooltipEl?.getAttribute("role")).toBe("tooltip");
    });

    it("tooltip bubble always in DOM (not hidden with {#if visible})", () => {
      render(Tooltip, { content: "Hello world" });
      const tooltip = document.querySelector('[role="tooltip"]');
      // bubble present in DOM even when not triggered (CSS-only hiding)
      expect(tooltip).toBeTruthy();
      expect(tooltip?.classList.contains("invisible")).toBe(true);
    });

    it("accepts explicit id prop", () => {
      render(Tooltip, { content: "Hello", id: "my-tip" });
      expect(document.getElementById("my-tip")).toBeTruthy();
      expect(document.getElementById("my-tip")?.getAttribute("role")).toBe(
        "tooltip",
      );
    });
  });
  ```

- [ ] **Step 2: Run to confirm tests fail**

  ```bash
  cd frontend && npm test -- --reporter=verbose src/lib/components/ui/Tooltip.test.ts
  ```

  Expected: FAIL — `Cannot find module './Tooltip.svelte'`

- [ ] **Step 3: Create `Tooltip.svelte`**

  Create `frontend/src/lib/components/ui/Tooltip.svelte`:

  ```svelte
  <script lang="ts" module>
  	let _nextId = 0;
  </script>

  <script lang="ts">
  	import { Info } from 'lucide-svelte';
  	import { portal } from '$lib/actions/portal';

  	let {
  		content,
  		id
  	}: {
  		content: string;
  		id?: string;
  	} = $props();

  	const tooltipId = id ?? `tooltip-${++_nextId}`;

  	let triggerEl: HTMLButtonElement | undefined = $state(undefined);
  	let tooltipEl: HTMLDivElement | undefined = $state(undefined);
  	let visible = $state(false);
  	let placement: 'top' | 'bottom' = $state('top');
  	let tooltipTop = $state(0);
  	let tooltipLeft = $state(0);
  	let arrowLeft = $state(0);
  </script>

  {#if content}
  	<button
  		type="button"
  		bind:this={triggerEl}
  		aria-label="More information"
  		aria-describedby={tooltipId}
  		class="inline-flex cursor-default items-center text-[var(--text-muted)] hover:text-[var(--text-secondary)] focus-visible:outline-none focus-visible:shadow-[0_0_0_3px_rgba(var(--accent-rgb),0.25)]"
  		onclick={(e) => e.stopPropagation()}
  	>
  		<Info size={14} aria-hidden="true" />
  	</button>

  	<div
  		bind:this={tooltipEl}
  		use:portal
  		id={tooltipId}
  		role="tooltip"
  		data-ui="tooltip"
  		class="invisible fixed z-[100] max-w-[220px] rounded-panel border border-[var(--border-default)] bg-[var(--bg-raised)] px-3 py-2 text-xs whitespace-pre-line text-[var(--text-primary)] opacity-0 transition-[opacity] duration-fast"
  		style="top: {tooltipTop}px; left: {tooltipLeft}px;"
  	>
  		{content}
  		<div
  			class="absolute h-1.5 w-1.5 rotate-45 border-[var(--border-default)] bg-[var(--bg-raised)]"
  			class:border-b={placement === 'top'}
  			class:border-r={placement === 'top'}
  			class:border-t={placement === 'bottom'}
  			class:border-l={placement === 'bottom'}
  			style="left: {arrowLeft - 3}px; {placement === 'top' ? 'bottom: -3px' : 'top: -3px'}"
  		></div>
  	</div>
  {/if}
  ```

- [ ] **Step 4: Run tests to confirm they pass**

  ```bash
  cd frontend && npm test -- --reporter=verbose src/lib/components/ui/Tooltip.test.ts
  ```

  Expected: PASS (5 tests)

- [ ] **Step 5: Commit**

  ```bash
  git add frontend/src/lib/components/ui/Tooltip.svelte frontend/src/lib/components/ui/Tooltip.test.ts
  git commit -m "feat(ui): add Tooltip component static structure and ARIA"
  ```

---

## Task 3: Tooltip — show/hide behavior + debounce

Wire the `mouseenter`/`focus`/`mouseleave`/`blur`/`Escape` handlers with 100 ms debounce.
Toggle `invisible` + `opacity-0` classes via `class:` bindings driven by `visible` state.
Clean up the timer on destroy.

**Files:**

- Modify: `frontend/src/lib/components/ui/Tooltip.svelte`
- Modify: `frontend/src/lib/components/ui/Tooltip.test.ts`

- [ ] **Step 1: Add failing tests**

  Append these tests to the `describe('Tooltip')` block in `Tooltip.test.ts`:

  ```typescript
  import { describe, expect, it, vi, afterEach } from "vitest";
  import { tick } from "svelte";
  import { render, screen, fireEvent } from "@testing-library/svelte";
  ```

  Replace the import lines at the top with the above, then add inside `describe('Tooltip')`:

  ```typescript
  describe("show/hide behavior", () => {
    afterEach(() => {
      vi.useRealTimers();
    });

    it("bubble is invisible when trigger is not hovered or focused", () => {
      render(Tooltip, { content: "Hello" });
      const bubble = document.querySelector('[role="tooltip"]')!;
      expect(bubble.classList.contains("invisible")).toBe(true);
    });

    it("bubble becomes visible on trigger mouseenter", async () => {
      vi.useFakeTimers();
      render(Tooltip, { content: "Hello" });
      const trigger = screen.getByRole("button", { name: "More information" });
      await fireEvent.mouseEnter(trigger);
      const bubble = document.querySelector('[role="tooltip"]')!;
      expect(bubble.classList.contains("invisible")).toBe(false);
    });

    it("bubble becomes visible on trigger focus", async () => {
      vi.useFakeTimers();
      render(Tooltip, { content: "Hello" });
      const trigger = screen.getByRole("button", { name: "More information" });
      await fireEvent.focus(trigger);
      const bubble = document.querySelector('[role="tooltip"]')!;
      expect(bubble.classList.contains("invisible")).toBe(false);
    });

    it("bubble hides after mouseleave debounce (100ms)", async () => {
      vi.useFakeTimers();
      render(Tooltip, { content: "Hello" });
      const trigger = screen.getByRole("button", { name: "More information" });
      await fireEvent.mouseEnter(trigger);
      await fireEvent.mouseLeave(trigger);
      // Still visible immediately after mouseleave
      const bubble = document.querySelector('[role="tooltip"]')!;
      expect(bubble.classList.contains("invisible")).toBe(false);
      // Hidden after debounce expires — advance timer then flush Svelte microtask queue
      vi.advanceTimersByTime(150);
      await tick();
      expect(bubble.classList.contains("invisible")).toBe(true);
    });

    it("bubble hides after blur debounce (100ms)", async () => {
      vi.useFakeTimers();
      render(Tooltip, { content: "Hello" });
      const trigger = screen.getByRole("button", { name: "More information" });
      await fireEvent.focus(trigger);
      await fireEvent.blur(trigger);
      vi.advanceTimersByTime(150);
      await tick();
      const bubble = document.querySelector('[role="tooltip"]')!;
      expect(bubble.classList.contains("invisible")).toBe(true);
    });

    it("hover-bridge: mouseleave trigger + mouseenter bubble keeps tooltip visible", async () => {
      vi.useFakeTimers();
      render(Tooltip, { content: "Hello" });
      const trigger = screen.getByRole("button", { name: "More information" });
      const bubble = document.querySelector('[role="tooltip"]')!;
      await fireEvent.mouseEnter(trigger);
      await fireEvent.mouseLeave(trigger);
      await fireEvent.mouseEnter(bubble);
      vi.advanceTimersByTime(150);
      await tick();
      expect(bubble.classList.contains("invisible")).toBe(false);
    });

    it("Escape hides the tooltip immediately without debounce", async () => {
      vi.useFakeTimers();
      render(Tooltip, { content: "Hello" });
      const trigger = screen.getByRole("button", { name: "More information" });
      await fireEvent.mouseEnter(trigger);
      await fireEvent.keyDown(trigger, { key: "Escape" });
      const bubble = document.querySelector('[role="tooltip"]')!;
      expect(bubble.classList.contains("invisible")).toBe(true);
    });
  });
  ```

- [ ] **Step 2: Run to confirm tests fail**

  ```bash
  cd frontend && npm test -- --reporter=verbose src/lib/components/ui/Tooltip.test.ts
  ```

  Expected: FAIL — show/hide tests fail because bubble always has `invisible` class.

- [ ] **Step 3: Add show/hide logic to `Tooltip.svelte`**

  In the `<script lang="ts">` block, after the `$state` declarations, add:

  ```typescript
  let hideTimeout: ReturnType<typeof setTimeout> | undefined;

  function show() {
    clearTimeout(hideTimeout);
    visible = true;
  }

  function scheduleHide() {
    hideTimeout = setTimeout(() => {
      visible = false;
    }, 100);
  }

  $effect(() => {
    return () => clearTimeout(hideTimeout);
  });
  ```

  Update the `<button>` trigger to add the event handlers:

  ```svelte
  	<button
  		type="button"
  		bind:this={triggerEl}
  		aria-label="More information"
  		aria-describedby={tooltipId}
  		class="inline-flex cursor-default items-center text-[var(--text-muted)] hover:text-[var(--text-secondary)] focus-visible:outline-none focus-visible:shadow-[0_0_0_3px_rgba(var(--accent-rgb),0.25)]"
  		onclick={(e) => e.stopPropagation()}
  		onmouseenter={show}
  		onmouseleave={scheduleHide}
  		onfocus={show}
  		onblur={scheduleHide}
  		onkeydown={(e) => {
  			if (e.key === 'Escape' && visible) {
  				visible = false;
  			}
  		}}
  	>
  		<Info size={14} aria-hidden="true" />
  	</button>
  ```

  Update the bubble `<div>` to add hover-bridge handlers and toggle visibility classes:

  ```svelte
  	<div
  		bind:this={tooltipEl}
  		use:portal
  		id={tooltipId}
  		role="tooltip"
  		data-ui="tooltip"
  		class="fixed z-[100] max-w-[220px] rounded-panel border border-[var(--border-default)] bg-[var(--bg-raised)] px-3 py-2 text-xs whitespace-pre-line text-[var(--text-primary)] transition-[opacity] duration-fast"
  		class:invisible={!visible}
  		class:opacity-0={!visible}
  		style="top: {tooltipTop}px; left: {tooltipLeft}px;"
  		onmouseenter={show}
  		onmouseleave={scheduleHide}
  	>
  ```

  Note: `invisible` (Tailwind `visibility: hidden`) is toggled; `opacity-0` drives the fade.
  Remove the hardcoded `invisible` and `opacity-0` from the static class string — they are now
  dynamic via `class:`.

- [ ] **Step 4: Run tests to confirm they pass**

  ```bash
  cd frontend && npm test -- --reporter=verbose src/lib/components/ui/Tooltip.test.ts
  ```

  Expected: PASS (all tests)

- [ ] **Step 5: Commit**

  ```bash
  git add frontend/src/lib/components/ui/Tooltip.svelte frontend/src/lib/components/ui/Tooltip.test.ts
  git commit -m "feat(ui): add Tooltip show/hide behavior with debounce and hover-bridge"
  ```

---

## Task 4: Tooltip — positioning logic

Add the `$effect` that computes `tooltipTop`, `tooltipLeft`, `arrowLeft`, and `placement` when
`visible` becomes `true`. Unit tests cannot verify exact pixel values (JSDOM returns 0 for
`getBoundingClientRect`) — test only that the effect does not crash and that the bubble is still
visible after the effect runs.

**Files:**

- Modify: `frontend/src/lib/components/ui/Tooltip.svelte`
- Modify: `frontend/src/lib/components/ui/Tooltip.test.ts`

- [ ] **Step 1: Add a smoke test for positioning**

  Append inside the `describe('show/hide behavior')` block in `Tooltip.test.ts`:

  ```typescript
  it("bubble remains visible after positioning effect runs", async () => {
    vi.useFakeTimers();
    render(Tooltip, { content: "Hello" });
    const trigger = screen.getByRole("button", { name: "More information" });
    await fireEvent.mouseEnter(trigger);
    const bubble = document.querySelector('[role="tooltip"]')!;
    // getBoundingClientRect returns zeros in JSDOM — effect must not crash
    expect(bubble.classList.contains("invisible")).toBe(false);
  });
  ```

- [ ] **Step 2: Run to confirm test passes already** (show logic handles null rects gracefully)

  ```bash
  cd frontend && npm test -- --reporter=verbose src/lib/components/ui/Tooltip.test.ts
  ```

  Expected: PASS

- [ ] **Step 3: Add positioning `$effect` to `Tooltip.svelte`**

  In the `<script lang="ts">` block, after the `scheduleHide` function, add:

  ```typescript
  $effect(() => {
    if (!visible || !tooltipEl || !triggerEl) return;

    const triggerRect = triggerEl.getBoundingClientRect();
    const tooltipRect = tooltipEl.getBoundingClientRect();

    let top = triggerRect.top - tooltipRect.height - 8;
    let left = triggerRect.left + triggerRect.width / 2 - tooltipRect.width / 2;
    let currentPlacement: "top" | "bottom" = "top";

    if (top < 8) {
      top = triggerRect.bottom + 8;
      currentPlacement = "bottom";
    }

    left = Math.max(
      8,
      Math.min(left, window.innerWidth - tooltipRect.width - 8),
    );

    const arrowX = Math.max(
      8,
      Math.min(
        triggerRect.left + triggerRect.width / 2 - left,
        tooltipRect.width - 14,
      ),
    );

    tooltipTop = top;
    tooltipLeft = left;
    arrowLeft = arrowX;
    placement = currentPlacement;
  });
  ```

- [ ] **Step 4: Run tests to confirm they still pass**

  ```bash
  cd frontend && npm test -- --reporter=verbose src/lib/components/ui/Tooltip.test.ts
  ```

  Expected: PASS

- [ ] **Step 5: Commit**

  ```bash
  git add frontend/src/lib/components/ui/Tooltip.svelte frontend/src/lib/components/ui/Tooltip.test.ts
  git commit -m "feat(ui): add Tooltip positioning logic (top/bottom flip, horizontal clamp)"
  ```

---

## Task 5: Export Tooltip from the UI barrel

**Files:**

- Modify: `frontend/src/lib/components/ui/index.ts`

- [ ] **Step 1: Add export**

  Open `frontend/src/lib/components/ui/index.ts` and append this line at the end:

  ```typescript
  export { default as Tooltip } from "./Tooltip.svelte";
  ```

- [ ] **Step 2: Verify TypeScript is happy**

  ```bash
  cd frontend && npm run check
  ```

  Expected: no errors

- [ ] **Step 3: Commit**

  ```bash
  git add frontend/src/lib/components/ui/index.ts
  git commit -m "feat(ui): export Tooltip from UI barrel"
  ```

---

## Task 6: RadioCardGroup migration

Rename `description?` → `tooltip?` on `RadioCardOption`, change the card element from
`<button type="button" role="radio">` to `<div role="radio" tabindex>`, and wire the `Tooltip`
component. The container `<div role="radiogroup">` already exists at line 45 — no change needed
there. Update tests.

**Files:**

- Modify: `frontend/src/lib/components/forms/RadioCardGroup.svelte`
- Modify: `frontend/src/lib/components/forms/RadioCardGroup.test.ts`

- [ ] **Step 1: Update the test file**

  Replace the contents of `frontend/src/lib/components/forms/RadioCardGroup.test.ts` entirely:

  ```typescript
  import { describe, expect, it, vi } from "vitest";
  import { render, screen, fireEvent } from "@testing-library/svelte";
  import RadioCardGroup from "./RadioCardGroup.svelte";

  const options = [
    { value: "open", label: "Open", tooltip: "Anyone can create an account." },
    { value: "invite", label: "Invite Only", tooltip: "Token required." },
    { value: "closed", label: "Closed", tooltip: "No new accounts." },
  ];

  describe("RadioCardGroup", () => {
    it("renders all options", () => {
      render(RadioCardGroup, { name: "mode", value: "open", options });
      expect(screen.getByText("Open")).toBeTruthy();
      expect(screen.getByText("Invite Only")).toBeTruthy();
      expect(screen.getByText("Closed")).toBeTruthy();
    });

    it("selected card has aria-checked=true", () => {
      render(RadioCardGroup, { name: "mode", value: "invite", options });
      const inviteCard = screen.getByRole("radio", { name: /invite only/i });
      expect(inviteCard).toHaveAttribute("aria-checked", "true");
    });

    it("unselected cards have aria-checked=false", () => {
      render(RadioCardGroup, { name: "mode", value: "open", options });
      const inviteCard = screen.getByRole("radio", { name: /invite only/i });
      expect(inviteCard).toHaveAttribute("aria-checked", "false");
    });

    it("container has role=radiogroup", () => {
      render(RadioCardGroup, { name: "mode", value: "open", options });
      expect(screen.getByRole("radiogroup")).toBeTruthy();
    });

    it("calls onchange when a card is clicked", async () => {
      const onchange = vi.fn();
      render(RadioCardGroup, {
        name: "mode",
        value: "open",
        options,
        onchange,
      });
      await fireEvent.click(screen.getByRole("radio", { name: /closed/i }));
      expect(onchange).toHaveBeenCalledWith("closed");
    });

    it("disabled cards do not fire onchange", async () => {
      const onchange = vi.fn();
      render(RadioCardGroup, {
        name: "mode",
        value: "open",
        options,
        onchange,
        disabled: true,
      });
      await fireEvent.click(screen.getByRole("radio", { name: /closed/i }));
      expect(onchange).not.toHaveBeenCalled();
    });

    it("renders info icon button when tooltip is set", () => {
      render(RadioCardGroup, { name: "mode", value: "open", options });
      const infoButtons = screen.getAllByRole("button", {
        name: "More information",
      });
      expect(infoButtons.length).toBe(3);
    });

    it("renders no info icon button when tooltip is absent", () => {
      const optionsNoTooltip = [
        { value: "open", label: "Open" },
        { value: "closed", label: "Closed" },
      ];
      render(RadioCardGroup, {
        name: "mode",
        value: "open",
        options: optionsNoTooltip,
      });
      expect(
        screen.queryByRole("button", { name: "More information" }),
      ).toBeNull();
    });

    it("clicking info icon does not select the card", async () => {
      const onchange = vi.fn();
      render(RadioCardGroup, {
        name: "mode",
        value: "open",
        options,
        onchange,
      });
      const infoButton = screen.getAllByRole("button", {
        name: "More information",
      })[2]; // closed card
      await fireEvent.click(infoButton);
      expect(onchange).not.toHaveBeenCalled();
    });

    it("Enter key on focused card fires onchange", async () => {
      const onchange = vi.fn();
      render(RadioCardGroup, { name: "mode", value: "open", options, onchange });
      const closedCard = screen.getByRole("radio", { name: /closed/i });
      await fireEvent.keyDown(closedCard, { key: "Enter" });
      expect(onchange).toHaveBeenCalledWith("closed");
    });

    it("Space key on focused card fires onchange", async () => {
      const onchange = vi.fn();
      render(RadioCardGroup, { name: "mode", value: "open", options, onchange });
      const closedCard = screen.getByRole("radio", { name: /closed/i });
      await fireEvent.keyDown(closedCard, { key: " " });
      expect(onchange).toHaveBeenCalledWith("closed");
    });
  });
  ```

- [ ] **Step 2: Run to confirm tests fail**

  ```bash
  cd frontend && npm test -- --reporter=verbose src/lib/components/forms/RadioCardGroup.test.ts
  ```

  Expected: FAIL — `description` field still exists; no `Tooltip` buttons; card is still a `<button>`
  so clicking the info button triggers radio selection.

- [ ] **Step 3: Update `RadioCardGroup.svelte`**

  Replace the entire file contents with:

  ```svelte
  <script lang="ts" module>
  	export interface RadioCardOption<T extends string = string> {
  		value: T;
  		label: string;
  		tooltip?: string;
  	}
  </script>

  <script lang="ts" generics="T extends string">
  	import { Tooltip } from '$lib/components/ui';

  	let {
  		name,
  		value,
  		options,
  		onchange,
  		disabled = false
  	}: {
  		name: string;
  		value: T;
  		options: RadioCardOption<T>[];
  		onchange?: (value: T) => void;
  		disabled?: boolean;
  	} = $props();

  	function select(v: T) {
  		if (!disabled) onchange?.(v);
  	}

  	function handleKeydown(e: KeyboardEvent, idx: number) {
  		if (disabled) return;
  		// Enter/Space activate the focused card — native <button> click fires on these keys
  		// automatically but <div role="radio"> does not, so we must handle them explicitly.
  		if (e.key === 'Enter' || e.key === ' ') {
  			e.preventDefault();
  			select(options[idx].value);
  			return;
  		}
  		let next = idx;
  		if (e.key === 'ArrowRight' || e.key === 'ArrowDown') {
  			e.preventDefault();
  			next = (idx + 1) % options.length;
  		} else if (e.key === 'ArrowLeft' || e.key === 'ArrowUp') {
  			e.preventDefault();
  			next = (idx - 1 + options.length) % options.length;
  		}
  		if (next !== idx) {
  			onchange?.(options[next].value);
  		}
  	}
  </script>

  <div
  	role="radiogroup"
  	aria-label={name}
  	style="display: grid; grid-template-columns: repeat({options.length}, 1fr); gap: 0.5rem;"
  >
  	{#each options as option, i (option.value)}
  		{@const selected = option.value === value}
  		<div
  			role="radio"
  			tabindex={disabled ? -1 : 0}
  			aria-checked={selected}
  			aria-disabled={disabled}
  			aria-label={option.label}
  			onclick={() => select(option.value)}
  			onkeydown={(e) => handleKeydown(e, i)}
  			class="
  				rounded px-3 py-3 text-left transition-[background,border-color,color]
  				duration-fast cursor-pointer
  				{selected
  				? 'border-2 border-[rgba(var(--accent-rgb,6,182,212),0.6)] bg-[rgba(var(--accent-rgb,6,182,212),0.07)] text-[var(--accent-bright)]'
  				: 'border border-[var(--border-subtle)] bg-transparent text-[var(--text-secondary)]'}
  				{disabled ? 'cursor-not-allowed opacity-40' : ''}
  			"
  		>
  			<div class="flex items-center gap-1">
  				<span class="text-sm font-semibold">{option.label}</span>
  				{#if option.tooltip}
  					<Tooltip content={option.tooltip} />
  				{/if}
  			</div>
  		</div>
  	{/each}
  </div>
  ```

- [ ] **Step 4: Run tests to confirm they pass**

  ```bash
  cd frontend && npm test -- --reporter=verbose src/lib/components/forms/RadioCardGroup.test.ts
  ```

  Expected: PASS (all 11 tests)

- [ ] **Step 5: Run full type check**

  ```bash
  cd frontend && npm run check
  ```

  Expected: no errors

- [ ] **Step 6: Commit**

  ```bash
  git add frontend/src/lib/components/forms/RadioCardGroup.svelte frontend/src/lib/components/forms/RadioCardGroup.test.ts
  git commit -m "feat(ui): migrate RadioCardGroup descriptions to Tooltip; card button→div"
  ```

---

## Task 7: AccessSettings — rename description → tooltip

**Files:**

- Modify: `frontend/src/routes/settings/AccessSettings.svelte`

- [ ] **Step 1: Update `modeOptions`**

  In `frontend/src/routes/settings/AccessSettings.svelte`, find the `modeOptions` constant
  (around line 46–50) and change the three `description` keys to `tooltip`:

  ```typescript
  const modeOptions = [
    {
      value: "open" as const,
      label: "Open",
      tooltip: "Anyone can create an account.",
    },
    {
      value: "invite" as const,
      label: "Invite Only",
      tooltip: "Token required to register.",
    },
    {
      value: "closed" as const,
      label: "Closed",
      tooltip: "No new accounts allowed.",
    },
  ];
  ```

- [ ] **Step 2: Run type check**

  ```bash
  cd frontend && npm run check
  ```

  Expected: no errors — TypeScript will catch any remaining `description` property usages.

- [ ] **Step 3: Run all tests**

  ```bash
  cd frontend && npm test
  ```

  Expected: PASS

- [ ] **Step 4: Commit**

  ```bash
  git add frontend/src/routes/settings/AccessSettings.svelte
  git commit -m "feat(settings): rename RadioCardGroup description→tooltip in modeOptions"
  ```

---

## Task 8: Quality gate pass

- [ ] **Step 1: Full quality gate**

  ```bash
  cd frontend && npm run lint && npm run format:check && npm run check && npm run test && npm run build
  ```

  Expected: all pass. Fix any lint/format issues before proceeding:
  - Lint errors: run `npm run lint -- --fix` for auto-fixable issues.
  - Format errors: run `npx prettier --write src/` then re-check.

- [ ] **Step 2: Commit if any auto-fixes were needed**

  Only commit if step 1 required changes. If all passed cleanly, skip this step.

  ```bash
  git add -p
  git commit -m "chore(ui): apply lint/format fixes for Tooltip feature"
  ```

---

## Task 9: Documentation — `primitives.md`

**Files:**

- Modify: `docs/development/ui/primitives.md`

- [ ] **Step 1: Add `Tooltip` section under Feedback Primitives**

  In `docs/development/ui/primitives.md`, find the `## Feedback Primitives` heading. Insert a
  new `### Tooltip` section **after** the `### Callout` section and **before** the
  `### EmptyState` section:

  ````markdown
  ---
  
  ### Tooltip
  
  Inline info icon that reveals a styled tooltip bubble on hover or focus. Use for supplemental
  option descriptions that don't need to be permanently visible.
  
  **Location:** `frontend/src/lib/components/ui/Tooltip.svelte`
  **Import:** `import { Tooltip } from '$lib/components/ui';`
  
  ```typescript
  // frontend/src/lib/components/ui/Tooltip.svelte
  {
    content: string;  // tooltip text; supports multiline via \n (rendered with white-space: pre-line)
    id?: string;      // explicit tooltip element id; auto-generated if omitted
  }
  ```
  
  Usage:
  
  ```svelte
  <Tooltip content="Anyone can create an account." />
  
  <Tooltip content={"Line one.\nLine two."} />
  ```
  
  Passing `content=""` renders nothing (no icon, no bubble) — treat empty string as absent.
  
  **Visual rules:**
  
  - Trigger: `<button type="button">` with `<Info size={14} />` icon from lucide-svelte.
  - Icon color: `--text-muted` at rest, `--text-secondary` on hover/focus.
  - Focus ring: `focus-visible:shadow-[0_0_0_3px_rgba(var(--accent-rgb),0.25)]`.
  - Bubble: `bg-[var(--bg-raised)]`, `border-[var(--border-default)]`, `rounded-panel` (4px),
    `max-w-[220px]`, `text-xs text-[var(--text-primary)]`.
  - Arrow: 6×6px rotated square at the near-trigger edge of the bubble.
  - Animation: `transition-[opacity] duration-fast` (120ms); `invisible + opacity-0` when hidden.
  - Z-index: `100` (same tier as `ContextMenu`; `[data-ui="tooltip"]` rule in `app.css`).
  
  **Positioning:** centers above the trigger; flips to below if top would clip the viewport;
  horizontal clamping keeps 8px clearance from viewport edges.
  
  **Accessibility:**
  
  - Trigger has `aria-label="More information"` and `aria-describedby={tooltipId}`.
  - Bubble has `role="tooltip"` and matching `id`.
  - Bubble is **always in the DOM** (CSS-only show/hide) so `aria-describedby` always resolves.
  - `Escape` dismisses without moving focus and without `stopPropagation` (modal-close propagates
    normally).
  - Touch: reveal via focus (tap icon → focus → show; tap elsewhere → blur → hide).
  
  **Deferred:** `SurfaceActionButton` `title=` → `Tooltip` migration is out of scope for this
  feature; tracked separately.
  
  ---
  ````

- [ ] **Step 2: Update the `RadioCardGroup` section**

  In the same file, find the `### RadioCardGroup` section. Update the type block and example:

  Change:

  ```typescript
  export type RadioCardOption = {
    value: string;
    label: string;
    description?: string;
  };
  ```

  To:

  ```typescript
  export type RadioCardOption = {
    value: string;
    label: string;
    tooltip?: string; // shown in a Tooltip bubble; omit to render the card without an info icon
  };
  ```

  Update the example options to use `tooltip:` instead of `description:`:

  ```svelte
  <RadioCardGroup
    name="registration-mode"
    value={form.draft.mode}
    options={[
      { value: 'open', label: 'Open', tooltip: 'Anyone can register.' },
      { value: 'invite', label: 'Invite Only', tooltip: 'Token required.' },
      { value: 'closed', label: 'Closed', tooltip: 'No new accounts.' }
    ]}
    onchange={(v) => form.update('mode', v)}
  />
  ```

  Also note that the card element changed from `<button>` to `<div role="radio">` (allows nesting
  the tooltip `<button>` inside without invalid HTML). No API or ARIA behaviour change.

- [ ] **Step 3: Run markdownlint**

  ```bash
  npx markdownlint --config .markdownlint.json 'docs/**/*.md'
  ```

  Expected: no errors. Fix any line-length violations (max 150 chars; tables and code blocks are
  exempt).

- [ ] **Step 4: Commit**

  ```bash
  git add docs/development/ui/primitives.md
  git commit -m "docs(ui): add Tooltip section and update RadioCardGroup docs in primitives.md"
  ```

---

## Task 10: Documentation — `tokens.md`

**Files:**

- Modify: `docs/development/ui/tokens.md`

- [ ] **Step 1: Update the Z-Index Scale table**

  In `docs/development/ui/tokens.md`, find the `## Z-Index Scale` table. The table currently has
  a `Dropdown / tooltip` row with value `100` and use `Inline popovers`. Update that row's Use
  column to reference the CSS contract explicitly:

  Find:

  ```markdown
  | Dropdown / tooltip | `100` | Inline popovers |
  ```

  Replace with:

  ```markdown
  | Dropdown / tooltip | `100` | Inline popovers (`[data-ui="context-menu-shell"]`, `[data-ui="tooltip"]`) |
  ```

- [ ] **Step 2: Run markdownlint**

  ```bash
  npx markdownlint --config .markdownlint.json 'docs/**/*.md'
  ```

  Expected: no errors.

- [ ] **Step 3: Commit**

  ```bash
  git add docs/development/ui/tokens.md
  git commit -m "docs(ui): update z-index scale table to list tooltip data-ui selector"
  ```

---

## Self-Review

**Spec coverage check:**

| Spec requirement                                         | Task      |
| -------------------------------------------------------- | --------- |
| New `Tooltip.svelte` in `$lib/components/ui/`            | Task 2–4  |
| Export from `index.ts`                                   | Task 5    |
| `RadioCardGroup`: rename `description?` → `tooltip?`     | Task 6    |
| `RadioCardGroup`: `<button>` → `<div role="radio">`      | Task 6    |
| `RadioCardGroup`: wire `<Tooltip>`                       | Task 6    |
| `AccessSettings`: rename in `modeOptions`                | Task 7    |
| `RadioCardGroup.test.ts`: update fixtures + new tests    | Task 6    |
| `Tooltip.test.ts`: all specified test cases              | Tasks 2–4 |
| `app.css`: `[data-ui='tooltip']` rule                    | Task 1    |
| `css-contract.test.ts`: assertion                        | Task 1    |
| `primitives.md`: Tooltip section + RadioCardGroup update | Task 9    |
| `tokens.md`: z-index table note                          | Task 10   |

All spec requirements covered. ✓

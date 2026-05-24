# Spec: Tooltip Primitive + RadioCardGroup Migration

**Date:** 2026-05-24
**Status:** Draft

---

## Overview

Introduce a general-purpose `Tooltip` UI primitive that renders an info icon (`ℹ`) button
inline with content. Hovering or focusing the button reveals a styled tooltip bubble with an
arrow. Migrate `RadioCardGroup` option descriptions from inline text to this tooltip pattern.

---

## Scope

### In scope

- New `Tooltip.svelte` in `frontend/src/lib/components/ui/`
- Export `Tooltip` from `frontend/src/lib/components/ui/index.ts` (add named export alongside existing primitives)
- `RadioCardGroup.svelte`: rename `description?` → `tooltip?` on `RadioCardOption`; replace
  inline description `<div>` with inline `<Tooltip content={option.tooltip} />`
- `AccessSettings.svelte`: rename `description` → `tooltip` in the three `modeOptions` entries
- `RadioCardGroup.test.ts`: update fixtures for the renamed field
- `frontend/src/app.css`: add `[data-ui='tooltip']` z-index rule
- `frontend/src/lib/theme/css-contract.test.ts`: add assertion for the new rule
- `docs/development/ui/primitives.md`: new `Tooltip` section; update `RadioCardGroup` section
- `docs/development/ui/tokens.md`: confirm z-index table row for tooltip (value `100`, same tier
  as context menu shell)

### Out of scope

- `SurfaceActionButton` `title=` → `Tooltip` migration (separate ticket)
- Rich/interactive tooltip content (popover pattern, links inside tooltips)
- 4-way smart auto-placement (top/bottom suffices)

---

## New Component: `Tooltip`

**File:** `frontend/src/lib/components/ui/Tooltip.svelte`

### API

```typescript
// frontend/src/lib/components/ui/Tooltip.svelte
{
  content: string;  // tooltip text; rendered with white-space: pre-line for multiline support
  id?: string;      // explicit id for the tooltip element; auto-generated if omitted
}
```

If `content` is an empty string the component renders nothing (no icon, no bubble). Callers that
gate on `tooltip?` optional naturally satisfy this — an empty string is treated identically to an
absent field. Passing `content=""` is a programming error and produces a no-op.

**ID generation:** when `id` is omitted, use a module-level counter:

```typescript
let _nextId = 0;
// inside component: const tooltipId = id ?? `tooltip-${++_nextId}`;
```

This is deterministic in tests, SSR-safe, and avoids `crypto.randomUUID()` browser API
requirements.

The component renders its own trigger — a small `<button>` containing `<Info size={14} />` from
`lucide-svelte`. Callers do not supply a trigger snippet. The trigger button calls
`stopPropagation()` on its `click` event so it does not activate any ancestor interactive
element (e.g., a radio card `<div role="radio">` wrapping it).

Usage:

```svelte
<Tooltip content="Anyone can create an account." />

<Tooltip content={"First line.\nSecond line."} />
```

### Visual design

**Trigger button:**

- `<button type="button">` with `aria-label="More information"` and `aria-describedby={tooltipId}`
- Icon: `<Info size={14} aria-hidden="true" />` at `text-[var(--text-muted)]`
- Hover/focus icon color: `text-[var(--text-secondary)]`
- Focus ring: `focus-visible:outline-none focus-visible:shadow-[0_0_0_3px_rgba(var(--accent-rgb),0.25)]`
- Transition: do not add a `transition-*` class; the global `app.css` rule already applies
  `transition: background 0.12s, border-color 0.12s, color 0.12s` to all `<button>` elements.
  Adding `transition-[color]` would override the global shorthand and drop `background` and
  `border-color` from the transition triplet.
- No background, no border — icon-only button

**Tooltip bubble (portaled, always in DOM):**

The bubble is **always in the DOM** (portaled to `<body>`), never gated by `{#if visible}`.
Hiding is done via CSS only (`visibility: hidden` when not visible, `opacity-0` when transitioning
out). This keeps `aria-describedby` on the trigger permanently linked to a real DOM element,
which is required by WCAG — a dangling `aria-describedby` reference produces no accessible
description.

- Background: `bg-[var(--bg-raised)]`
- Border: `border border-[var(--border-default)]`
- Border radius: `rounded-panel` (4px)
- Text: `text-xs text-[var(--text-primary)]` with `whitespace-pre-line`
- Padding: `px-3 py-2`
- Max width: `max-w-[220px]`
- `position: fixed`; carries `z-[100]` Tailwind class on the element (same convention as
  `ContextMenu` which uses `z-[100]` directly) AND is covered by `data-ui="tooltip"` in `app.css`
  as a contract pin
- Animation: `transition-[opacity] duration-fast`; `opacity-0` + `invisible` when hidden,
  `opacity-100` + `visible` when shown (both `visibility` and `opacity` toggled together to
  prevent pointer-event interception while hidden)

**Arrow:**

- A `6×6px` rotated square (`rotate-45`) placed at the near-trigger edge of the bubble
- Shares `bg-[var(--bg-raised)]` fill and `border-[var(--border-default)]` on the two exposed sides
- When tooltip is above trigger: arrow sits at the bottom edge, pointing down; exposed sides are
  `border-b border-r`
- When tooltip is below trigger (flipped): arrow sits at the top edge, pointing up; exposed sides
  are `border-t border-l`
- Arrow is horizontally centered on the trigger icon, clamped to stay within the bubble bounds

### Show / hide behavior

| Event                           | Action                                       |
| ------------------------------- | -------------------------------------------- |
| `mouseenter` on trigger         | show                                         |
| `focus` on trigger              | show                                         |
| `mouseleave` on trigger         | schedule hide (100 ms delay)                 |
| `blur` on trigger               | schedule hide (100 ms delay)                 |
| `mouseenter` on tooltip bubble  | cancel scheduled hide                        |
| `mouseleave` on tooltip bubble  | schedule hide (100 ms delay)                 |
| `Escape` keydown (when visible) | hide immediately; do NOT `stopPropagation()` |

Do not call `stopPropagation()` on Escape. The tooltip closes, then the event propagates
normally to close any containing modal — which is the correct compound behavior.

The 100 ms delay satisfies WCAG 1.4.13 (tooltip must be hoverable — mouse can move from trigger
to tooltip without it dismissing).

Use `mouseenter`/`mouseleave` (not `pointerenter`/`pointerleave`) intentionally. Pointer events
fire on touch tap+release in sequence, causing the tooltip to appear and immediately dismiss.
Touch users get tooltip reveal via `focus` (tap → focus → show; tap elsewhere → blur → hide),
which was the accepted design decision.

**Timer cleanup:** the pending `setTimeout` handle must be cleared when the component is
destroyed. Clear it in the `$effect` return callback or in `onDestroy` to prevent post-unmount
state mutation.

### Positioning algorithm

The tooltip bubble is always in the DOM (never `{#if}` gated) so `tooltipEl` is always bound.
Positioning runs inside a `$effect` that reads `visible`. The effect pattern matches `ContextMenu`
exactly: element is always mounted, `$effect` computes position and reveals it. Guard the effect
with `if (!tooltipEl || !triggerEl) return` as a safety net.

Because the bubble is always in the DOM, its dimensions are available for measurement at any
time. When `visible` transitions to `true`, compute the position before revealing: the element
is already `invisible opacity-0` (from the hidden state), which hides it visually while keeping
it laid out. After setting the `top`/`left` style, transition to `visible opacity-100` to
trigger the fade.

1. Guard: `if (!tooltipEl || !triggerEl) return`
2. Read `triggerRect = triggerEl.getBoundingClientRect()`
3. Read `tooltipRect = tooltipEl.getBoundingClientRect()` (element in DOM, `visibility: hidden`)
4. Default position: above trigger, centered horizontally on trigger midpoint

   ```text
   left = triggerRect.left + triggerRect.width / 2 - tooltipRect.width / 2
   top  = triggerRect.top - tooltipRect.height - 8  // 8px gap including arrow
   ```

5. **Vertical flip:** if `top < 8`, tooltip would clip viewport top → render below instead:

   ```text
   top = triggerRect.bottom + 8
   placement = 'bottom'
   ```

6. **Horizontal clamp:** keep `8px` clearance from viewport edges:

   ```text
   left = clamp(left, 8, window.innerWidth - tooltipRect.width - 8)
   ```

7. Arrow horizontal position: trigger center relative to tooltip left edge, clamped so the arrow
   stays within the bubble with 8px padding from each edge. The arrow element is 6px wide:

   ```text
   arrowX = clamp(triggerRect.left + triggerRect.width / 2 - left, 8, tooltipRect.width - 14)
   // upper bound: tooltipRect.width - arrowWidth(6) - edgePad(8) = tooltipRect.width - 14
   ```

8. Set `style` on the portaled element: `position: fixed; top: ${top}px; left: ${left}px`.

9. Set arrow `style`: `left: ${arrowX}px`; set `placement` variable to `'top'` or `'bottom'`
   to control which sides of the rotated square receive a border.

**Known limitation:** position is computed once when `visible` becomes true. If the user scrolls
or resizes the viewport while the tooltip is open, the bubble drifts from its trigger. This is
acceptable given the 100 ms hide-on-mouseleave timer; add `scroll`/`resize` repositioning only
if reported as a real user problem.

### Accessibility

- Trigger `<button>` has `aria-describedby={tooltipId}` linking to the tooltip `<div>`.
- Tooltip `<div>` has `role="tooltip"` and matching `id`.
- Trigger `aria-label="More information"` (no visible label text).
- Tooltip content is plain text only; no interactive elements inside `role="tooltip"`.
- `Escape` dismisses the tooltip without moving focus (WCAG 1.4.13).
- Tooltip appears on `:focus-visible` trigger (keyboard accessible).

### Portal

Use existing `use:portal` action (`$lib/actions/portal.ts`) — same pattern as `ContextMenu` and
`ModalBackdrop`. The tooltip `<div>` is portaled to `document.body` so `position: fixed`
resolves against the viewport regardless of any ancestor containing block. The portal action's
`destroy()` callback calls `node.parentNode?.removeChild(node)`, so portaled nodes are cleaned
up when the Tooltip component unmounts — no DOM leaks.

---

## Modified: `RadioCardGroup`

**File:** `frontend/src/lib/components/forms/RadioCardGroup.svelte`

### Type change

```typescript
// Before
export interface RadioCardOption<T extends string = string> {
  value: T;
  label: string;
  description?: string;
}

// After
export interface RadioCardOption<T extends string = string> {
  value: T;
  label: string;
  tooltip?: string;
}
```

### Card element change (required for valid HTML)

The current card element is `<button type="button" role="radio">`. A `<Tooltip>` component
renders its own `<button>` trigger — nesting `<button>` inside `<button>` is invalid HTML and
causes inconsistent browser behavior (click events on the inner button bubble up and incorrectly
trigger radio card selection).

**Fix:** Change the card element from `<button>` to `<div role="radio" tabindex={disabled ? -1 : 0}>`.
The container `<div>` already carries `role="radiogroup"` (confirmed at RadioCardGroup.svelte:45),
so the ARIA parent→child ownership contract (`radiogroup` owns `radio`) is preserved.
The existing `handleKeydown` handler already manages all keyboard navigation; the only additional
change is that `onclick` replaces the implicit button click. No ARIA roles or keyboard semantics
change.

The `<Tooltip>` trigger's `click` event must call `stopPropagation()` to prevent the click from
reaching the card `<div>` and inadvertently selecting the radio option.

### Render change

Replace the inline description block with an inline `Tooltip`. The label row becomes a flex
container:

```svelte
<!-- Before: <button type="button" role="radio" ...> -->
<div class="text-sm font-semibold">{option.label}</div>
{#if option.description}
  <div class="mt-1 text-xs leading-relaxed opacity-70">{option.description}</div>
{/if}
<!-- </button> -->

<!-- After: <div role="radio" tabindex={disabled ? -1 : 0} ...> -->
<div class="flex items-center gap-1">
  <span class="text-sm font-semibold">{option.label}</span>
  {#if option.tooltip}
    <Tooltip content={option.tooltip} />
  {/if}
</div>
<!-- </div> -->
```

The card body no longer needs a description row. No other layout changes.

---

## Modified: `AccessSettings`

**File:** `frontend/src/routes/settings/AccessSettings.svelte`

Rename the three `description` keys to `tooltip`:

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

---

## Infrastructure changes

### `frontend/src/app.css`

Add alongside the existing `[data-ui='context-menu-shell']` rule:

```css
[data-ui="tooltip"] {
  z-index: 100;
}
```

### `frontend/src/lib/theme/css-contract.test.ts`

Add assertion to the z-index contract test:

```typescript
expect(appCss).toMatch(/\[data-ui='tooltip'\][\s\S]*?z-index:\s*100;/);
```

---

## Documentation deliverables

1. **`docs/development/ui/primitives.md`**
   - Add `Tooltip` section under Feedback Primitives (after Callout, before EmptyState) covering:
     API table, usage example, visual rules, accessibility contract, and the note that
     `SurfaceActionButton`'s `title=` migration is deferred.
   - Update `RadioCardGroup` section: change `description?` to `tooltip?` in the type block and
     usage example; note that the field now renders as an inline tooltip, not inline text.

2. **`docs/development/ui/tokens.md`**
   - Z-Index Scale table already lists `Dropdown / tooltip | 100`. Add `[data-ui="tooltip"]` to
     the "Use" column note so the CSS contract is explicit. No new row needed.

---

## Tests

### Unit tests (`RadioCardGroup.test.ts`)

- Update any fixture passing `description:` to pass `tooltip:` instead.
- Add: when `tooltip` is set, an `<Info>` icon button is rendered inside the card.
- Add: when `tooltip` is absent, no icon button is rendered.

### Unit tests (`Tooltip.test.ts`, new file)

Use `vi.useFakeTimers()` for all timing-dependent assertions to avoid real 100 ms waits.

The tooltip bubble is **always in the DOM** (always portaled to `<body>`). Tests must not assert
DOM absence. Instead assert CSS-visibility state (`invisible` class or `visibility: hidden` style)
for the hidden state.

**JSDOM note:** `getBoundingClientRect()` returns zero dimensions in JSDOM. Tests that verify
positioning math must either mock `getBoundingClientRect` on both `triggerEl` and `tooltipEl` or
skip positioning assertions (test visibility state only; positioning is a browser integration
concern verified via Playwright).

- Trigger not hovered/focused → bubble has `visibility: hidden` (not visible to user).
- Trigger `mouseenter` → bubble becomes visible (no longer `visibility: hidden`).
- Trigger `focus` → bubble appears with `role="tooltip"`.
- Trigger `mouseleave` → bubble still visible immediately; advances fake timer past 100 ms →
  bubble gone (debounce behavior).
- Trigger `blur` → same debounce behavior as `mouseleave`.
- Trigger `mouseleave` → tooltip `mouseenter` → advances past 100 ms → bubble still visible
  (hover-bridge: mouse moved from trigger to bubble).
- `Escape` keydown while visible → bubble gone immediately.
- `aria-describedby` on trigger matches `id` on tooltip bubble.
- `content=""` → component renders nothing (no trigger button in DOM).

---

## Quality gates

Standard frontend gates apply:

```sh
cd frontend && npm run lint && npm run format:check && npm run check && npm run test && npm run build
```

No Rust or backend changes — backend quality gates not required.

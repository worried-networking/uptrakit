# UI Design Language Foundation And Authoring Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement
> this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the frontend token adapter, shared UI primitives, and parity fixtures that make it straightforward to build new built-in and
surface-backed pages in one consistent visual language.

**Architecture:** Introduce a checked-in semantic-token adapter first, then build a small shared primitive layer under
`frontend/src/lib/components/ui/`, then refactor the Surfaces renderer to consume that same primitive layer. End by adding deterministic fixture
coverage so new page work can reuse the same patterns instead of cloning route-local CSS and Skeleton presets.

**Tech Stack:** SvelteKit, Svelte 5, Tailwind/Skeleton, Vitest, Testing Library, existing frontend route/component structure.

**Execution Context:** Run commands from the repository root. On a clean machine, run `cd frontend && npm ci` once before Task 1.

---

## File Map

| File                                                                                                                                                              | Change                                                                                                                                                               |
| ----------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `frontend/src/theme/adapter-manifest.json`                                                                                                                        | New semantic-token adapter manifest required by the design-language spec                                                                                             |
| `frontend/src/app.css`                                                                                                                                            | Publish CSS custom properties and map them to the active theme runtime                                                                                               |
| `frontend/src/lib/components/ui/`                                                                                                                                 | New shared primitive directory for shell, card, tab, callout, empty-state, badge, field-row, provider-selector, table, and shared modal/menu shell patterns          |
| `frontend/src/lib/components/ui/*.test.ts`                                                                                                                        | Unit/rendering tests for the new shared primitives                                                                                                                   |
| `frontend/src/lib/components/Modal.svelte`                                                                                                                        | Existing shared modal shell aligned with the design-language primitive contract                                                                                      |
| `frontend/src/lib/components/ContextMenu.svelte`                                                                                                                  | Existing shared menu shell aligned with the design-language primitive contract                                                                                       |
| `frontend/src/lib/components/surfaces/SurfaceRenderer.svelte`                                                                                                     | Swap bespoke node rendering for shared primitives                                                                                                                    |
| `frontend/src/lib/components/surfaces/SurfaceTable.svelte`                                                                                                        | Reuse the shared data-table primitive                                                                                                                                |
| `frontend/src/lib/components/surfaces/SurfaceModal.svelte`                                                                                                        | Reuse the shared modal shell and callout/empty-state primitives                                                                                                      |
| `frontend/src/lib/components/surfaces/SurfaceWorkflow.svelte`                                                                                                     | Reuse the shared workflow/step shell                                                                                                                                 |
| `frontend/src/lib/components/surfaces/SurfaceActionBar.svelte`                                                                                                    | Align surface action rows with the shared button/callout shell treatment                                                                                             |
| `frontend/src/lib/components/surfaces/SurfaceReadPanel.svelte`                                                                                                    | Standardize provider-selector and runtime-state handling                                                                                                             |
| `frontend/src/lib/components/surfaces/SurfaceForm.svelte`, `SurfaceKeyValue.svelte`, `SurfaceSlot.svelte`, `SurfaceInteractionButton.svelte`, `SchemaForm.svelte` | Audit remaining surface building blocks and refactor any bespoke styling into shared primitives or explicitly keep them unchanged because they are already compliant |
| `frontend/src/lib/components/surfaces/*.test.ts`                                                                                                                  | Parity-oriented tests proving surfaces consume the shared primitives                                                                                                 |
| `frontend/src/lib/surfaces/registry.svelte.ts`                                                                                                                    | Preserve canonical ordering semantics in one place                                                                                                                   |
| `frontend/src/lib/test-fixtures/ui-parity.ts`                                                                                                                     | Deterministic fixture helpers for parity coverage                                                                                                                    |

---

### Task 1: Add The Semantic Token Adapter Contract

**Files:**

- Create: `frontend/src/theme/adapter-manifest.json`
- Modify: `frontend/src/app.css`
- Create: `frontend/src/lib/theme/adapter-manifest.test.ts`

- [ ] **Step 1: Write the failing adapter-manifest completeness test**

Create `frontend/src/lib/theme/adapter-manifest.test.ts` with a test skeleton like:

```ts
import manifest from "../../theme/adapter-manifest.json";
import { describe, expect, it } from "vitest";

const requiredTokens = [
  "--bg-base",
  "--bg-surface",
  "--bg-raised",
  "--border-subtle",
  "--border-default",
  "--text-inverted",
  "--text-primary",
  "--text-secondary",
  "--text-muted",
  "--accent",
  "--accent-rgb",
  "--accent-bright",
  "--accent-dark",
  "--accent-deep",
  "--color-success",
  "--color-success-bg",
  "--color-success-border",
  "--color-warning",
  "--color-warning-bg",
  "--color-warning-border",
  "--color-error",
  "--color-error-bg",
  "--color-error-border",
  "--color-info",
  "--color-info-bg",
  "--color-info-border",
];

describe("adapter manifest", () => {
  it("covers all required semantic tokens in dark and light themes", () => {
    for (const token of requiredTokens) {
      expect(manifest).toContainEqual(expect.objectContaining({ token, theme: "dark" }));
      expect(manifest).toContainEqual(expect.objectContaining({ token, theme: "light" }));
    }
  });
});
```

Run:

```bash
cd frontend && npm run test -- src/lib/theme/adapter-manifest.test.ts
```

Expected: FAIL because the manifest file does not exist yet.

- [ ] **Step 2: Create the manifest and wire the semantic CSS variables**

Create `frontend/src/theme/adapter-manifest.json` as an array of `{ token, theme, maps_to }` objects and update `frontend/src/app.css` so both themes
expose the spec tokens as CSS custom properties.

Use a manifest shape like:

```json
[
  { "token": "--bg-base", "theme": "dark", "maps_to": "--color-surface-950" },
  { "token": "--bg-base", "theme": "light", "maps_to": "--color-surface-50" },
  { "token": "--accent", "theme": "dark", "maps_to": "--color-primary-500" }
]
```

Use `maps_to` to point at the source theme token or CSS variable name that feeds the semantic adapter, not at Tailwind utility classes or final hex
literals.

Add CSS custom-property blocks in `app.css` shaped like:

```css
:root {
  --bg-base: #f8fafc;
  --bg-surface: #ffffff;
  --accent-rgb: 59 130 246;
}

.dark {
  --bg-base: #09090b;
  --bg-surface: #111113;
  --accent-rgb: 96 165 250;
}
```

Run:

```bash
cd frontend && npm run test -- src/lib/theme/adapter-manifest.test.ts
```

Expected: PASS with both themes covered by the manifest.

- [ ] **Step 3: Verify the frontend still type-checks**

Run:

```bash
cd frontend && npm run check
```

Expected: the frontend type-check passes after importing the JSON manifest and updating the stylesheet.

- [ ] **Step 4: Commit**

```bash
git add frontend/src/theme/adapter-manifest.json frontend/src/app.css frontend/src/lib/theme/adapter-manifest.test.ts
git commit -m "feat: add ui design token adapter"
```

---

### Task 2: Build The Shared UI Primitive Layer

**Files:**

- Create: `frontend/src/lib/components/ui/PageShell.svelte`
- Create: `frontend/src/lib/components/ui/SectionCard.svelte`
- Create: `frontend/src/lib/components/ui/TabStrip.svelte`
- Create: `frontend/src/lib/components/ui/Callout.svelte`
- Create: `frontend/src/lib/components/ui/EmptyState.svelte`
- Create: `frontend/src/lib/components/ui/StatusBadge.svelte`
- Create: `frontend/src/lib/components/ui/FormFieldRow.svelte`
- Create: `frontend/src/lib/components/ui/ProviderSelector.svelte`
- Create: `frontend/src/lib/components/ui/DataTable.svelte`
- Create: `frontend/src/lib/components/ui/index.ts`
- Modify: `frontend/src/lib/components/Modal.svelte`
- Modify: `frontend/src/lib/components/ContextMenu.svelte`
- Create: `frontend/src/lib/components/ui/PageShell.test.ts`
- Create: `frontend/src/lib/components/ui/SectionCard.test.ts`
- Create: `frontend/src/lib/components/ui/TabStrip.test.ts`
- Create: `frontend/src/lib/components/ui/Callout.test.ts`
- Create: `frontend/src/lib/components/ui/EmptyState.test.ts`
- Create: `frontend/src/lib/components/ui/StatusBadge.test.ts`
- Create: `frontend/src/lib/components/ui/FormFieldRow.test.ts`
- Create: `frontend/src/lib/components/ui/ProviderSelector.test.ts`
- Create: `frontend/src/lib/components/ui/DataTable.test.ts`
- Modify: `frontend/src/lib/components/Modal.test.ts`
- Modify: `frontend/src/lib/components/ContextMenu.test.ts`

- [ ] **Step 1: Write the first shared-primitive test**

Start with `frontend/src/lib/components/ui/TabStrip.test.ts` and prove the active/hover API you want to reuse across built-in and surface tabs.

Use a test structure like:

```ts
render(TabStrip, {
  items: [
    { id: "general", label: "General" },
    { id: "plugin-configs", label: "Plugin Configs" },
  ],
  activeId: "general",
});

expect(screen.getByRole("tab", { name: "General" })).toHaveAttribute("data-state", "active");
expect(screen.getByRole("tab", { name: "Plugin Configs" })).toHaveAttribute("data-state", "inactive");
```

Run:

```bash
cd frontend && npm run test -- src/lib/components/ui/TabStrip.test.ts
```

Expected: FAIL because the new primitive does not exist yet.

- [ ] **Step 2: Implement the base primitives**

Create the new `frontend/src/lib/components/ui/` directory and add:

- `PageShell.svelte` for route title + actions + content container
- `SectionCard.svelte` for card/panel chrome
- `TabStrip.svelte` for shared tabs
- `Callout.svelte` and `EmptyState.svelte` for runtime/error/empty handling
- `StatusBadge.svelte` for semantic status variants
- `FormFieldRow.svelte` and `ProviderSelector.svelte` for standard field layout
- `DataTable.svelte` for shared table shell
- integrate the existing `frontend/src/lib/components/Modal.svelte` and `frontend/src/lib/components/ContextMenu.svelte` into this same shared
  primitive layer rather than treating them as surface-only or route-local shells
- write the remaining primitive tests in this task as well: `PageShell.test.ts`, `SectionCard.test.ts`, `Callout.test.ts`, `EmptyState.test.ts`,
  `StatusBadge.test.ts`, `FormFieldRow.test.ts`, `ProviderSelector.test.ts`, and `DataTable.test.ts`

This task owns the shared component contract for `Modal.svelte` and `ContextMenu.svelte`: shared props, exports, and primitive compatibility.
Migration Task 1 owns route-level consumption and any final shell styling convergence in callers.

Each primitive should expose semantic props, not route-specific styling flags. For example:

```ts
export type TabItem = {
  id: string;
  label: string;
  disabled?: boolean;
};
```

Run:

```bash
cd frontend && npm run test -- src/lib/components/ui/PageShell.test.ts src/lib/components/ui/SectionCard.test.ts src/lib/components/ui/TabStrip.test.ts src/lib/components/ui/Callout.test.ts src/lib/components/ui/EmptyState.test.ts src/lib/components/ui/StatusBadge.test.ts src/lib/components/ui/FormFieldRow.test.ts src/lib/components/ui/ProviderSelector.test.ts src/lib/components/ui/DataTable.test.ts src/lib/components/Modal.test.ts src/lib/components/ContextMenu.test.ts
```

Expected: PASS with the shared primitive suite green and the existing modal/menu shells proven compatible with the shared design language.

- [ ] **Step 3: Export the primitives through one stable entry point**

Add `frontend/src/lib/components/ui/index.ts` with exports shaped like:

```ts
export { default as PageShell } from "./PageShell.svelte";
export { default as SectionCard } from "./SectionCard.svelte";
export { default as TabStrip } from "./TabStrip.svelte";
export { default as Callout } from "./Callout.svelte";
export { default as EmptyState } from "./EmptyState.svelte";
export { default as StatusBadge } from "./StatusBadge.svelte";
export { default as FormFieldRow } from "./FormFieldRow.svelte";
export { default as ProviderSelector } from "./ProviderSelector.svelte";
export { default as DataTable } from "./DataTable.svelte";
export { default as ModalShell } from "../Modal.svelte";
export { default as ContextMenuShell } from "../ContextMenu.svelte";
```

Run:

```bash
cd frontend && npm run check
```

Expected: the new primitive entry point is type-safe and importable.

- [ ] **Step 4: Commit**

```bash
git add frontend/src/lib/components/ui frontend/src/lib/components/Modal.svelte frontend/src/lib/components/Modal.test.ts frontend/src/lib/components/ContextMenu.svelte frontend/src/lib/components/ContextMenu.test.ts
git commit -m "feat: add shared ui design language primitives"
```

---

### Task 3: Refactor The Surfaces Renderer To Use The Shared Primitives

**Files:**

- Modify: `frontend/src/lib/components/surfaces/SurfaceRenderer.svelte`
- Modify: `frontend/src/lib/components/surfaces/SurfaceTable.svelte`
- Modify: `frontend/src/lib/components/surfaces/SurfaceModal.svelte`
- Modify: `frontend/src/lib/components/surfaces/SurfaceWorkflow.svelte`
- Modify: `frontend/src/lib/components/surfaces/SurfaceActionBar.svelte`
- Modify: `frontend/src/lib/components/surfaces/SurfaceReadPanel.svelte`
- Modify or explicitly scope out: `frontend/src/lib/components/surfaces/SurfaceForm.svelte`
- Modify or explicitly scope out: `frontend/src/lib/components/surfaces/SurfaceKeyValue.svelte`
- Modify or explicitly scope out: `frontend/src/lib/components/surfaces/SurfaceSlot.svelte`
- Modify or explicitly scope out: `frontend/src/lib/components/surfaces/SurfaceInteractionButton.svelte`
- Modify or explicitly scope out: `frontend/src/lib/components/surfaces/SchemaForm.svelte`
- Create: `frontend/src/lib/components/surfaces/SurfaceModal.test.ts`
- Modify: `frontend/src/lib/components/surfaces/SurfaceRenderer.test.ts`
- Modify: `frontend/src/lib/components/surfaces/SurfaceTable.test.ts`
- Modify: `frontend/src/lib/components/surfaces/SurfaceActionBar.test.ts`
- Modify or create as needed: `frontend/src/lib/components/surfaces/SurfaceForm.test.ts`
- Modify or create as needed: `frontend/src/lib/components/surfaces/SurfaceInteractionButton.test.ts`
- Modify or create as needed: `frontend/src/lib/components/surfaces/SurfaceSlot.test.ts`
- Modify: `frontend/src/lib/components/surfaces/SurfaceReadPanel.test.ts`
- Modify: `frontend/src/lib/components/surfaces/SurfaceWorkflow.test.ts`

- [ ] **Step 1: Write a parity-oriented regression test**

Extend `frontend/src/lib/components/surfaces/SurfaceRenderer.test.ts` and create `frontend/src/lib/components/surfaces/SurfaceModal.test.ts` first so
the refactor starts with failing coverage. At minimum, prove callouts and empty states no longer leak raw contract IDs and that surface modal chrome
matches the shared modal shell.

Use a test structure like:

```ts
expect(screen.queryByText(/interaction_id/i)).not.toBeInTheDocument();
expect(screen.getByText("Action unavailable")).toBeInTheDocument();
```

Run:

```bash
cd frontend && npm run test -- src/lib/components/surfaces/SurfaceRenderer.test.ts src/lib/components/surfaces/SurfaceModal.test.ts
```

Expected: FAIL if the renderer still emits raw contract internals.

- [ ] **Step 2: Swap renderer branches to shared primitives**

Update the surface components so:

- `Tabs` reuse `TabStrip`
- `Table` reuses `DataTable`
- `Callout` and `EmptyState` reuse the new shared components
- `ModalTrigger` and `WorkflowTrigger` render through the shared modal/workflow shell
- `SurfaceReadPanel` owns provider selector placement through `ProviderSelector`
- `SurfaceForm`, `SurfaceKeyValue`, `SurfaceSlot`, `SurfaceInteractionButton`, and `SchemaForm` are either refactored to use the shared primitives in
  this task or explicitly left unchanged because they already inherit compliant styling without route-local chrome

The renderer should prefer human copy such as `Action unavailable` rather than raw contract identifiers.

Run:

```bash
cd frontend && npm run test -- src/lib/components/surfaces/SurfaceRenderer.test.ts src/lib/components/surfaces/SurfaceTable.test.ts src/lib/components/surfaces/SurfaceWorkflow.test.ts src/lib/components/surfaces/SurfaceActionBar.test.ts src/lib/components/surfaces/SurfaceReadPanel.test.ts src/lib/components/surfaces/SurfaceModal.test.ts
```

Expected: PASS with surface components rendering through the shared primitive layer.

Update `SurfaceReadPanel.test.ts` assertions to prove the new `ProviderSelector`-based rendering explicitly. Do not loosen the runtime-state coverage
just to make the refactor pass. If you also touched `SurfaceForm`, `SurfaceInteractionButton`, or `SurfaceSlot`, run their corresponding tests in the
same verification step before committing.

- [ ] **Step 3: Verify the Surfaces registry ordering contract still holds**

Add or update `frontend/src/lib/surfaces/registry.test.ts` so it locks the canonical ordering rules now captured by the spec.

Use an assertion pattern like:

```ts
expect(resolveSurfacePageNavItems([...], true).map((item) => item.id)).toEqual([
  'surface-a',
  'surface-b',
  'surface-c'
]);
```

Run:

```bash
cd frontend && npm run test -- src/lib/surfaces/registry.test.ts
```

Expected: PASS with stable ordering and dedupe behavior preserved.

- [ ] **Step 4: Commit**

```bash
git add frontend/src/lib/components/surfaces/SurfaceRenderer.svelte frontend/src/lib/components/surfaces/SurfaceTable.svelte frontend/src/lib/components/surfaces/SurfaceModal.svelte frontend/src/lib/components/surfaces/SurfaceWorkflow.svelte frontend/src/lib/components/surfaces/SurfaceActionBar.svelte frontend/src/lib/components/surfaces/SurfaceReadPanel.svelte frontend/src/lib/components/surfaces/SurfaceRenderer.test.ts frontend/src/lib/components/surfaces/SurfaceTable.test.ts frontend/src/lib/components/surfaces/SurfaceActionBar.test.ts frontend/src/lib/components/surfaces/SurfaceReadPanel.test.ts frontend/src/lib/components/surfaces/SurfaceWorkflow.test.ts frontend/src/lib/components/surfaces/SurfaceModal.test.ts frontend/src/lib/surfaces/registry.svelte.ts frontend/src/lib/surfaces/registry.test.ts
git commit -m "refactor: reuse shared ui primitives in surfaces renderer"
```

If this task also touched `SurfaceForm`, `SurfaceKeyValue`, `SurfaceSlot`, `SurfaceInteractionButton`, or `SchemaForm`, stage those files explicitly
in the same commit rather than using a directory-wide `git add`.

---

### Task 4: Add Deterministic Parity Fixtures For Future Page Work

**Files:**

- Create: `frontend/src/lib/test-fixtures/ui-parity.ts`
- Create: `frontend/src/lib/test-fixtures/ui-parity.test.ts`
- Modify: `frontend/src/routes/settings/surface-tabs.test.ts`
- Modify: `frontend/src/routes/software/surface-tabs.test.ts`
- Modify: `frontend/src/routes/surfaces/surfaces-page.test.ts`

- [ ] **Step 1: Add the fixture helper**

Create `frontend/src/lib/test-fixtures/ui-parity.ts` and `frontend/src/lib/test-fixtures/ui-parity.test.ts` together so the helper and its first
contract test land in the same step. The helper should build deterministic built-in and surface-backed render scenarios using fixed labels, counts,
and provider availability.

Include helpers shaped like:

```ts
export function buildParitySurfaceTab(id: string, label: string) {
  return { surface_id: id, label, priority: 100, slot: "settings.tabs", targeting: "universal" };
}
```

Run:

```bash
cd frontend && npm run test -- src/lib/test-fixtures/ui-parity.test.ts
```

Expected: PASS with deterministic fixture helpers available for later route and Playwright work.

- [ ] **Step 2: Reuse the fixture helper in existing route tests**

Update the settings/software/surfaces route tests so they use the deterministic fixture helper instead of ad hoc inline surface payloads.

Run:

```bash
cd frontend && npm run test -- src/routes/settings/surface-tabs.test.ts src/routes/software/surface-tabs.test.ts src/routes/surfaces/surfaces-page.test.ts
```

Expected: PASS with route parity fixtures centralized.

- [ ] **Step 3: Final frontend foundation verification**

Run:

```bash
cd frontend && npm run test && npm run check && npm run lint && npm run format:check && npm run build
```

Expected: the shared foundation work passes the existing unit, type-check, and lint gates before any route-migration work starts.

- [ ] **Step 4: Commit**

```bash
git add frontend/src/lib/test-fixtures frontend/src/routes/settings/surface-tabs.test.ts frontend/src/routes/software/surface-tabs.test.ts frontend/src/routes/surfaces/surfaces-page.test.ts
git commit -m "test: add ui parity fixtures for shared frontend patterns"
```

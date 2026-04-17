<!-- markdownlint-disable MD013 -->

# UI Design Language Shared Visual Alignment Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Bring the shared theme tokens, badges, menus, and table/footer primitives into exact alignment with the approved UI design-language spec so page work can build on a correct visual contract.

**Architecture:** Fix the semantic token adapter first so the runtime exposes the exact dark-teal and light-blue accent contract from the spec. Then tighten the shared primitives that still drift visually (`StatusBadge`, `ContextMenu`, `DataTable`, `Pagination`) and add the missing reusable pieces (`ContextMenuItem`, clickable badge, shared table footer). Finish by documenting the shared contract and extending parity coverage so route work can consume a stable base instead of recreating route-local styling.

**Tech Stack:** SvelteKit, Svelte 5, CSS custom properties in `frontend/src/app.css`, Tailwind/Skeleton runtime, Vitest, Testing Library, Playwright, Markdown docs.

**Execution Context:** Run commands from the repository root. On a clean machine, run `cd frontend && npm ci && npx svelte-kit sync` once before Task 1.

---

## File Map

<!-- markdownlint-disable MD060 -->

| File                                                       | Change                                                                                                                                          |
| ---------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------- |
| `frontend/src/app.css`                                     | Correct light/dark semantic token values to match the approved spec exactly                                                                     |
| `frontend/src/theme/adapter-manifest.json`                 | Update semantic token mappings so accent/info families map to the right runtime tokens per theme                                                |
| `frontend/src/lib/theme/adapter-manifest.test.ts`          | Tighten token mapping assertions for dark-teal/light-blue accent parity                                                                         |
| `frontend/src/lib/theme/design-token-values.test.ts`       | New raw-CSS guard test for the exact accent/info values from the spec                                                                           |
| `frontend/src/lib/components/ui/StatusBadge.svelte`        | Align badge size, typography, radius, and semantic variants with Section 4.1                                                                    |
| `frontend/src/lib/components/ui/ClickableBadge.svelte`     | New shared clickable-badge primitive for hover-swap status actions                                                                              |
| `frontend/src/lib/components/ui/UpdateAllBadge.svelte`     | New shared grouped-update badge primitive for the Software route’s spec-defined bulk action affordance                                          |
| `frontend/src/lib/components/ui/PillBadge.svelte`          | New shared compact pill primitive for plugin/type labels that must match the same visual language as status badges                              |
| `frontend/src/lib/components/TagBadge.svelte`              | Align generic tag pills to the approved compact pill metrics instead of route-local color treatment                                             |
| `frontend/src/lib/components/ui/ContextMenuItem.svelte`    | New shared menu-item row primitive for route and surface menus                                                                                  |
| `frontend/src/lib/components/ContextMenu.svelte`           | Align shell radius, border, width, padding rhythm, and default item spacing                                                                     |
| `frontend/src/lib/components/ui/DataTable.svelte`          | Add shared footer slot and tighten header/body metrics to Section 4.12                                                                          |
| `frontend/src/lib/components/ui/TableFooterBar.svelte`     | New shared footer row that keeps totals and pagination aligned to the table shell                                                               |
| `frontend/src/lib/components/Pagination.svelte`            | Align pagination sizing, spacing, and totals relationship so it visually locks to the shared footer bar and the table above it                  |
| `frontend/src/lib/components/ui/index.ts`                  | Export the new shared visual primitives                                                                                                         |
| `frontend/src/lib/components/ui/*.test.ts`                 | Add/expand unit coverage for badge, menu item, table footer, and pagination contract                                                            |
| `frontend/src/lib/components/ContextMenu.test.ts`          | Assert shell classes and menu-item composition points                                                                                           |
| `frontend/src/lib/components/Pagination.test.ts`           | Assert footer-friendly rendering and totals alignment hooks                                                                                     |
| `frontend/src/lib/components/surfaces/SurfaceTable.svelte` | Migrate shared surface table footer to the canonical table-footer primitive                                                                     |
| `frontend/src/lib/components/BatchActionBar.svelte`        | Migrate overflow menu rows to the canonical menu-item primitive                                                                                 |
| `docs/development/frontend-components.md`                  | Document `ClickableBadge`, `UpdateAllBadge`, `PillBadge`, `ContextMenuItem`, and `TableFooterBar` as part of the shared design-language surface |
| `frontend/tests/e2e/ui-parity.test.ts`                     | Add deterministic desktop parity coverage for badges, context menus, and table footer alignment                                                 |

<!-- markdownlint-enable MD060 -->

---

### Task 1: Align Semantic Tokens With The Approved Color Contract

**Files:**

- Modify: `frontend/src/app.css`
- Modify: `frontend/src/theme/adapter-manifest.json`
- Modify: `frontend/src/lib/theme/adapter-manifest.test.ts`
- Create: `frontend/src/lib/theme/design-token-values.test.ts`

- [ ] **Step 1: Write the failing token-value regression**

Create `frontend/src/lib/theme/design-token-values.test.ts`:

```ts
import appCss from "../../app.css?raw";
import { describe, expect, it } from "vitest";

describe("design token CSS values", () => {
  const rootBlock = appCss.match(/:root\\s*\\{([\\s\\S]*?)\\}/)?.[1] ?? "";
  const darkBlock = appCss.match(/\\.dark\\s*\\{([\\s\\S]*?)\\}/)?.[1] ?? "";

  it("pins the approved light-theme semantic color values inside :root", () => {
    expect(rootBlock).toContain("--bg-base: #f8fafc;");
    expect(rootBlock).toContain("--bg-surface: #ffffff;");
    expect(rootBlock).toContain("--bg-raised: #f1f5f9;");
    expect(rootBlock).toContain("--border-subtle: #e2e8f0;");
    expect(rootBlock).toContain("--border-default: #cbd5e1;");
    expect(rootBlock).toContain("--text-primary: #0f172a;");
    expect(rootBlock).toContain("--text-secondary: #64748b;");
    expect(rootBlock).toContain("--text-muted: #94a3b8;");
    expect(rootBlock).toContain("--text-inverted: #f8fafc;");
    expect(rootBlock).toContain("--theme-accent: #2563eb;");
    expect(rootBlock).toContain("--theme-accent-rgb: 37 99 235;");
    expect(rootBlock).toContain("--theme-accent-bright: #3b82f6;");
    expect(rootBlock).toContain("--theme-accent-dark: #1d4ed8;");
    expect(rootBlock).toContain("--theme-accent-deep: #1e40af;");
    expect(rootBlock).toContain("--color-success: #16a34a;");
    expect(rootBlock).toContain("--color-success-bg: rgba(22, 163, 74, 0.08);");
    expect(rootBlock).toContain(
      "--color-success-border: rgba(22, 163, 74, 0.2);",
    );
    expect(rootBlock).toContain("--color-warning: #d97706;");
    expect(rootBlock).toContain("--color-warning-bg: rgba(217, 119, 6, 0.1);");
    expect(rootBlock).toContain(
      "--color-warning-border: rgba(217, 119, 6, 0.22);",
    );
    expect(rootBlock).toContain("--color-error: #dc2626;");
    expect(rootBlock).toContain("--color-error-bg: rgba(220, 38, 38, 0.08);");
    expect(rootBlock).toContain(
      "--color-error-border: rgba(220, 38, 38, 0.2);",
    );
    expect(rootBlock).toContain("--theme-info: #0891b2;");
    expect(rootBlock).toContain("--theme-info-bg: rgba(8, 145, 178, 0.08);");
    expect(rootBlock).toContain(
      "--theme-info-border: rgba(8, 145, 178, 0.22);",
    );
  });

  it("pins the approved dark-theme semantic color values inside .dark", () => {
    expect(darkBlock).toContain("--bg-base: #09090b;");
    expect(darkBlock).toContain("--bg-surface: #111113;");
    expect(darkBlock).toContain("--bg-raised: #18181b;");
    expect(darkBlock).toContain("--border-subtle: #1c1c1f;");
    expect(darkBlock).toContain("--border-default: #27272a;");
    expect(darkBlock).toContain("--text-primary: #e4e4e7;");
    expect(darkBlock).toContain("--text-secondary: #a1a1aa;");
    expect(darkBlock).toContain("--text-muted: #52525b;");
    expect(darkBlock).toContain("--text-inverted: #09090b;");
    expect(darkBlock).toContain("--theme-accent: #06b6d4;");
    expect(darkBlock).toContain("--theme-accent-rgb: 6 182 212;");
    expect(darkBlock).toContain("--theme-accent-bright: #22d3ee;");
    expect(darkBlock).toContain("--theme-accent-dark: #0891b2;");
    expect(darkBlock).toContain("--theme-accent-deep: #0e7490;");
    expect(darkBlock).toContain("--color-success: #4ade80;");
    expect(darkBlock).toContain(
      "--color-success-bg: rgba(74, 222, 128, 0.14);",
    );
    expect(darkBlock).toContain(
      "--color-success-border: rgba(74, 222, 128, 0.22);",
    );
    expect(darkBlock).toContain("--color-warning: #fbbf24;");
    expect(darkBlock).toContain(
      "--color-warning-bg: rgba(251, 191, 36, 0.14);",
    );
    expect(darkBlock).toContain(
      "--color-warning-border: rgba(251, 191, 36, 0.24);",
    );
    expect(darkBlock).toContain("--color-error: #fdba74;");
    expect(darkBlock).toContain("--color-error-bg: rgba(253, 186, 116, 0.14);");
    expect(darkBlock).toContain(
      "--color-error-border: rgba(253, 186, 116, 0.22);",
    );
    expect(darkBlock).toContain("--theme-info: #67e8f9;");
    expect(darkBlock).toContain("--theme-info-bg: rgba(6, 182, 212, 0.1);");
    expect(darkBlock).toContain(
      "--theme-info-border: rgba(6, 182, 212, 0.22);",
    );
  });

  it("keeps info tokens distinct from accent tokens in both theme blocks", () => {
    expect(rootBlock).toContain("--theme-info: #0891b2;");
    expect(rootBlock).not.toContain("--theme-info: #2563eb;");
    expect(darkBlock).toContain("--theme-info: #67e8f9;");
    expect(darkBlock).not.toContain("--theme-info: #06b6d4;");
  });
});
```

Run:

```bash
cd frontend && npm run test -- src/lib/theme/design-token-values.test.ts src/lib/theme/adapter-manifest.test.ts
```

Expected: FAIL because `app.css` and `adapter-manifest.json` still map both themes to the generic `primary-*` accent family and do not pin the exact spec values.

- [ ] **Step 2: Correct the CSS token values and manifest mappings**

Update `frontend/src/app.css` so the semantic custom properties use the exact spec values instead of runtime `primary-*` aliases for accent and info. Introduce theme-source variables and point the semantic contract at those variables so the manifest continues to map to named runtime tokens rather than raw literals. The target shape is:

```css
:root {
  --theme-accent: #2563eb;
  --theme-accent-rgb: 37 99 235;
  --theme-accent-bright: #3b82f6;
  --theme-accent-dark: #1d4ed8;
  --theme-accent-deep: #1e40af;
  --theme-info: #0891b2;
  --theme-info-bg: rgba(8, 145, 178, 0.08);
  --theme-info-border: rgba(8, 145, 178, 0.22);
  --accent: var(--theme-accent);
  --accent-rgb: var(--theme-accent-rgb);
  --accent-bright: var(--theme-accent-bright);
  --accent-dark: var(--theme-accent-dark);
  --accent-deep: var(--theme-accent-deep);
  --color-info: var(--theme-info);
  --color-info-bg: var(--theme-info-bg);
  --color-info-border: var(--theme-info-border);
}

.dark {
  --theme-accent: #06b6d4;
  --theme-accent-rgb: 6 182 212;
  --theme-accent-bright: #22d3ee;
  --theme-accent-dark: #0891b2;
  --theme-accent-deep: #0e7490;
  --theme-info: #67e8f9;
  --theme-info-bg: rgba(6, 182, 212, 0.1);
  --theme-info-border: rgba(6, 182, 212, 0.22);
}
```

Update `frontend/src/theme/adapter-manifest.json` and `frontend/src/lib/theme/adapter-manifest.test.ts` so the mapping contract is no longer ambiguous about where theme-specific accent values come from. The semantic tokens may still point at the same theme-source variable names, but the plan must make clear that those variables resolve to different dark/light values in `app.css`. Use exact records like:

```json
{ "token": "--accent", "theme": "dark", "maps_to": "--theme-accent" },
{ "token": "--accent", "theme": "light", "maps_to": "--theme-accent" },
{ "token": "--accent-rgb", "theme": "dark", "maps_to": "--theme-accent-rgb" },
{ "token": "--accent-rgb", "theme": "light", "maps_to": "--theme-accent-rgb" }
```

Run:

```bash
cd frontend && npm run test -- src/lib/theme/design-token-values.test.ts src/lib/theme/adapter-manifest.test.ts
```

Expected: PASS with the raw CSS and manifest both pinned to the spec’s exact accent contract.

- [ ] **Step 3: Verify the shared theme entrypoint still builds**

Run:

```bash
cd frontend && npm run check && npm run build
```

Expected: PASS with no CSS or asset pipeline regressions.

- [ ] **Step 4: Commit**

```bash
git add frontend/src/app.css frontend/src/theme/adapter-manifest.json frontend/src/lib/theme/adapter-manifest.test.ts frontend/src/lib/theme/design-token-values.test.ts
git commit -m "fix: align ui design tokens with spec"
```

---

### Task 2: Add The Remaining Shared Visual Primitives

**Files:**

- Modify: `frontend/src/lib/components/ui/StatusBadge.svelte`
- Create: `frontend/src/lib/components/ui/ClickableBadge.svelte`
- Create: `frontend/src/lib/components/ui/UpdateAllBadge.svelte`
- Create: `frontend/src/lib/components/ui/PillBadge.svelte`
- Modify: `frontend/src/lib/components/TagBadge.svelte`
- Create: `frontend/src/lib/components/ui/ContextMenuItem.svelte`
- Modify: `frontend/src/lib/components/ContextMenu.svelte`
- Modify: `frontend/src/lib/components/ui/DataTable.svelte`
- Create: `frontend/src/lib/components/ui/TableFooterBar.svelte`
- Modify: `frontend/src/lib/components/Pagination.svelte`
- Modify: `frontend/src/lib/components/ui/index.ts`
- Create: `frontend/src/lib/components/ui/ClickableBadge.test.ts`
- Create: `frontend/src/lib/components/ui/UpdateAllBadge.test.ts`
- Create: `frontend/src/lib/components/ui/PillBadge.test.ts`
- Create: `frontend/src/lib/components/ui/ContextMenuItem.test.ts`
- Create: `frontend/src/lib/components/ui/TableFooterBar.test.ts`
- Modify: `frontend/src/lib/components/ui/StatusBadge.test.ts`
- Modify: `frontend/src/lib/components/ui/DataTable.test.ts`
- Modify: `frontend/src/lib/components/ContextMenu.test.ts`
- Modify: `frontend/src/lib/components/Pagination.test.ts`

- [ ] **Step 1: Write the failing primitive tests**

Create/extend the shared tests with assertions that match the spec:

`frontend/src/lib/components/ui/ClickableBadge.test.ts`

```ts
render(ClickableBadge, {
  tone: "info",
  idleLabel: "2 updates",
  hoverLabel: "→ Software",
});

const badge = screen.getByRole("button", { name: "2 updates" });
expect(badge).toHaveAttribute("data-ui", "clickable-badge");
expect(badge).toHaveAttribute("data-tone", "info");
```

`frontend/src/lib/components/ui/ContextMenuItem.test.ts`

```ts
render(ContextMenuItem, { label: "Delete", destructive: true });
const item = screen.getByRole("menuitem", { name: "Delete" });
expect(item.className).toContain("min-h-8");
expect(item.className).toContain("text-[10px]");
```

`frontend/src/lib/components/ui/TableFooterBar.test.ts`

```ts
render(TableFooterBar, {
  total: 42,
  currentPage: 2,
  totalPages: 4,
  onPageChange: vi.fn(),
});

expect(screen.getByText("42 total")).toBeInTheDocument();
expect(
  screen.getByRole("navigation", { name: /pagination/i }),
).toBeInTheDocument();
```

Run:

```bash
cd frontend && npm run test -- src/lib/components/ui/ClickableBadge.test.ts src/lib/components/ui/ContextMenuItem.test.ts src/lib/components/ui/TableFooterBar.test.ts src/lib/components/ui/StatusBadge.test.ts src/lib/components/ui/DataTable.test.ts src/lib/components/ContextMenu.test.ts src/lib/components/Pagination.test.ts
```

Expected: FAIL because the new primitives do not exist and the current badge/menu/table contract does not satisfy the spec metrics.

- [ ] **Step 2: Implement the shared badge, menu, and table-footer contract**

Create the missing primitives and tighten the existing ones.

Target `StatusBadge.svelte` shape:

```svelte
<span
    class={`inline-flex min-h-[14px] items-center justify-center rounded-[2px] border px-1.5 text-[7.5px] font-bold uppercase tracking-[0.04em] ${toneClasses[tone]}`}
    data-ui="status-badge"
    data-tone={tone}
>
    {label}
</span>
```

Target `ClickableBadge.svelte` structure:

```svelte
<button class={`inline-flex min-w-max items-center justify-center rounded-[2px] border px-1.5 text-[7.5px] font-bold uppercase tracking-[0.04em] ${toneClasses[tone]}`} data-ui="clickable-badge">
    <span class="idle">{idleLabel}</span>
    <span class="hov">{hoverLabel}</span>
</button>
```

Target `UpdateAllBadge.svelte` structure:

```svelte
<button
    class={`inline-flex min-h-[16px] min-w-max items-center justify-center rounded-[2px] border px-1.5 text-[7.5px] font-bold uppercase tracking-[0.04em] ${stateClasses[disabled ? 'disabled' : 'accent']}`}
    data-ui="update-all-badge"
>
    <span class="idle">{idleLabel}</span>
    <span class="hov">{hoverLabel}</span>
</button>
```

Target `PillBadge.svelte` structure:

```svelte
<span
    class="inline-flex min-h-[14px] items-center rounded-full border border-[var(--border-default)] bg-[var(--bg-raised)] px-2 text-[7.5px] font-semibold uppercase tracking-[0.08em] text-[var(--text-secondary)]"
    data-ui="pill-badge"
>
    {label}
</span>
```

Target `ContextMenuItem.svelte` structure:

```svelte
<button
    role="menuitem"
    tabindex="-1"
    class={`flex min-h-8 w-full items-center rounded-[4px] px-3 text-left text-[10px] text-[var(--text-primary)] hover:bg-[var(--bg-raised)] ${destructive ? 'text-[var(--color-error)]' : ''}`}
>
    {label}
</button>
```

Target `TableFooterBar.svelte` structure:

```svelte
<div class="flex items-center justify-between border-t border-[var(--border-subtle)] bg-[var(--bg-surface)] px-4 py-3" data-ui="table-footer-bar">
    <span class="text-[10px] text-[var(--text-secondary)]">{total} total</span>
    <Pagination {currentPage} {totalPages} onPageChange={onPageChange} />
</div>
```

Also:

- add a `footer` snippet to `DataTable.svelte`
- remove the implicit top margin from `Pagination.svelte` so it composes cleanly inside `TableFooterBar`
- align pagination button height, gap rhythm, and vertical centering with the totals label so the footer reads as one row instead of two unrelated blocks
- align `TagBadge.svelte` and any plugin/type pills to `PillBadge` instead of keeping a separate route-local pill language
- export `UpdateAllBadge` and `PillBadge` from `frontend/src/lib/components/ui/index.ts`
- update `ContextMenu.svelte` to the spec shell: `--bg-surface`, `--border-default`, `4px` radius, route-neutral internal spacing

Run:

```bash
cd frontend && npm run test -- src/lib/components/ui/ClickableBadge.test.ts src/lib/components/ui/UpdateAllBadge.test.ts src/lib/components/ui/PillBadge.test.ts src/lib/components/ui/ContextMenuItem.test.ts src/lib/components/ui/TableFooterBar.test.ts src/lib/components/ui/StatusBadge.test.ts src/lib/components/ui/DataTable.test.ts src/lib/components/ContextMenu.test.ts src/lib/components/Pagination.test.ts
```

Expected: PASS with the shared visual primitives now matching the spec metrics.

- [ ] **Step 3: Migrate the shared consumers and docs**

Update `frontend/src/lib/components/BatchActionBar.svelte` and `frontend/src/lib/components/surfaces/SurfaceTable.svelte` to consume `ContextMenuItem` and `TableFooterBar` instead of route-local text-size classes and free-floating pagination blocks.

As part of this step, extend `frontend/src/lib/components/Pagination.test.ts` and `frontend/src/lib/components/ui/TableFooterBar.test.ts` with one explicit pagination-alignment assertion each:

```ts
expect(screen.getByRole("navigation", { name: /pagination/i })).not.toHaveClass(
  expect.stringContaining("mt-4"),
);
expect(container.querySelector('[data-ui="table-footer-bar"]')).toHaveClass(
  "items-center",
  "justify-between",
);
```

Document the additions in `docs/development/frontend-components.md` with a section shaped like:

```md
| `ClickableBadge` | Hover-swap status actions such as `N updates` → `→ Software` | Use for spec-defined interactive badges only; do not restyle route-local buttons into fake badges. |
| `UpdateAllBadge` | Group-level Software bulk-update affordance | Reserved for the spec-defined `Update all` treatment; do not replace with generic accent buttons. |
| `PillBadge` | Compact neutral pills for plugin/type labels | Use for plugin labels and other neutral taxonomy chips that must visually align with status badges. |
| `ContextMenuItem` | Standard menu rows inside `ContextMenuShell` | Owns row height, text size, hover fill, and destructive color treatment. |
| `TableFooterBar` | Totals + pagination row aligned to the table shell | Pair with `DataTable` `footer` snippet; do not place raw pagination blocks outside the wrapper. |
```

Run:

```bash
cd frontend && npm run test -- src/lib/components/ui/ClickableBadge.test.ts src/lib/components/ui/UpdateAllBadge.test.ts src/lib/components/ui/PillBadge.test.ts src/lib/components/ui/ContextMenuItem.test.ts src/lib/components/ui/TableFooterBar.test.ts src/lib/components/surfaces/SurfaceTable.test.ts
cd frontend && npm run check
markdownlint --config .markdownlint.json docs/development/frontend-components.md
```

Expected: PASS with the shared consumers and developer docs aligned to the new primitives. If `BatchActionBar` needs direct regression coverage in this task, create `frontend/src/lib/components/BatchActionBar.test.ts` first and then add it to the run command.

- [ ] **Step 4: Commit**

```bash
git add frontend/src/lib/components/ui/StatusBadge.svelte frontend/src/lib/components/ui/ClickableBadge.svelte frontend/src/lib/components/ui/UpdateAllBadge.svelte frontend/src/lib/components/ui/PillBadge.svelte frontend/src/lib/components/TagBadge.svelte frontend/src/lib/components/ui/ContextMenuItem.svelte frontend/src/lib/components/ui/DataTable.svelte frontend/src/lib/components/ui/TableFooterBar.svelte frontend/src/lib/components/Pagination.svelte frontend/src/lib/components/ContextMenu.svelte frontend/src/lib/components/ui/index.ts
git add frontend/src/lib/components/ui/ClickableBadge.test.ts frontend/src/lib/components/ui/UpdateAllBadge.test.ts frontend/src/lib/components/ui/PillBadge.test.ts frontend/src/lib/components/ui/ContextMenuItem.test.ts frontend/src/lib/components/ui/TableFooterBar.test.ts frontend/src/lib/components/ui/StatusBadge.test.ts frontend/src/lib/components/ui/DataTable.test.ts frontend/src/lib/components/ContextMenu.test.ts frontend/src/lib/components/Pagination.test.ts frontend/src/lib/components/surfaces/SurfaceTable.svelte frontend/src/lib/components/BatchActionBar.svelte docs/development/frontend-components.md
git commit -m "feat: add shared ui alignment primitives"
```

---

### Task 3: Lock The Shared Contract With Visual Parity Coverage

**Files:**

- Modify: `frontend/tests/e2e/ui-parity.test.ts`
- Modify: `frontend/src/lib/test-fixtures/ui-parity.ts`
- Modify if needed for deterministic fixture support: `frontend/tests/e2e/ui-parity-responsive.test.ts`

- [ ] **Step 1: Add failing desktop parity expectations**

Extend the parity fixtures and screenshot assertions so the shared visual contract is frozen before route-level redesign work.

Add screenshot slices for:

- context menu row shell (`ContextMenuShell` + `ContextMenuItem`)
- clickable badge idle/hover states
- table footer totals + pagination alignment

Add Playwright assertions shaped like:

```ts
await expect(page.getByTestId("parity-context-menu")).toHaveScreenshot(
  "ui-parity-context-menu-shell.png",
);
await expect(page.getByTestId("parity-clickable-badge")).toHaveScreenshot(
  "ui-parity-clickable-badge.png",
);
await expect(page.getByTestId("parity-table-footer")).toHaveScreenshot(
  "ui-parity-table-footer.png",
);
```

Run:

```bash
cd frontend && npm run test:e2e -- --grep "ui parity"
```

Expected: FAIL because the new parity fixtures and baselines do not exist yet.

- [ ] **Step 2: Wire deterministic fixtures and capture baselines**

Update `frontend/src/lib/test-fixtures/ui-parity.ts` and `frontend/tests/e2e/ui-parity.test.ts` so each new primitive is rendered in a deterministic fixture route using the corrected token values.

Use stable fixture labels and file names:

```ts
"ui-parity-context-menu-shell.png";
"ui-parity-clickable-badge.png";
"ui-parity-table-footer.png";
```

Run:

```bash
cd frontend && npm run test:e2e -- --grep "ui parity"
```

Expected: PASS with new baselines captured and deterministic fixture coverage in place.

- [ ] **Step 3: Commit**

```bash
git add frontend/tests/e2e/ui-parity.test.ts frontend/src/lib/test-fixtures/ui-parity.ts frontend/tests/e2e/ui-parity.test.ts-snapshots
git add frontend/tests/e2e/ui-parity-responsive.test.ts frontend/tests/e2e/ui-parity-responsive.test.ts-snapshots # only if this task actually changed responsive coverage
git commit -m "test: add shared visual parity coverage"
```

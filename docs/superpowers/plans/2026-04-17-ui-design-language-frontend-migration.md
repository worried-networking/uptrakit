# UI Design Language Frontend Migration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Migrate the current frontend onto the approved UI design language,
using the new shared primitive layer so built-in and surface-backed routes
share one visual language instead of route-local styling.

**Architecture:** Migrate in three passes. First, update the shared shell and
global feedback surfaces that every route depends on. Second, migrate the
parity-sensitive route families where built-in and surface-backed UI meet
(`settings`, `software`, `host detail`, `surface.page`, host-context menu).
Third, migrate the remaining pages and then promote the still-`Target`
shell/responsive rules only after the implemented/transitional slices are
stable.

**Tech Stack:** SvelteKit routes, Svelte shared components, Tailwind/Skeleton theme runtime, Vitest, Playwright, existing route-level tests.

**Execution Context:** Run commands from the repository root. On a clean
machine, run `cd frontend && npm ci` before Task 1. Before Playwright work on
clean machines, run `cd frontend && npx playwright install --with-deps
chromium` once.

---

## Task 0: Verify Foundation Preconditions Before Migration

**Files:**

- Verify: `frontend/src/theme/adapter-manifest.json`
- Verify: `frontend/src/lib/components/ui/`
- Verify: `frontend/src/lib/components/ui/index.ts`
- Verify: `frontend/src/lib/test-fixtures/ui-parity.ts`

- [ ] **Step 1: Confirm the shared foundation artifacts exist**

Before changing route or shell styling, verify the foundation plan has already
landed the shared inputs this migration depends on:

- `frontend/src/theme/adapter-manifest.json`
- `frontend/src/lib/components/ui/`
- `frontend/src/lib/components/ui/index.ts` exporting the foundation-owned
  `ModalShell` and `ContextMenuShell` bridge
- `frontend/src/lib/test-fixtures/ui-parity.ts`

Run:

```bash
test -f frontend/src/theme/adapter-manifest.json && test -d frontend/src/lib/components/ui && test -f frontend/src/lib/components/ui/index.ts && rg -q "ModalShell" frontend/src/lib/components/ui/index.ts && rg -q "ContextMenuShell" frontend/src/lib/components/ui/index.ts && test -f frontend/src/lib/test-fixtures/ui-parity.ts
```

Expected: PASS. If any check fails, stop and finish the foundation plan first
rather than re-creating those artifacts ad hoc inside the migration work.

---

## File Map

| File | Change |
| --- | --- |
| `frontend/src/routes/+layout.svelte` | Shell migration for nav ordering, shared chrome, alerts, and toast mount point |
| `frontend/src/lib/components/ToastNotifications.svelte` | Toast layout and behavior alignment |
| `frontend/src/lib/components/TerminalOutput.svelte` | Transitional terminal-shell alignment before final responsive promotion |
| `frontend/src/lib/components/Modal.svelte` | Shared modal shell styling alignment |
| `frontend/src/lib/components/ConfirmDialog.svelte` | Align confirmation dialog with the new shared dialog styles |
| `frontend/src/lib/components/ContextMenu.svelte` | Align menu shell with design-language menu rules |
| `frontend/src/routes/settings/+page.svelte` and settings leaf components | Shared tab strip, section-card, and form-row migration |
| `frontend/src/routes/software/+page.svelte` and `frontend/src/routes/software/[id]/+page.svelte` | Shared tab strip, data table, host-context menu, and terminal/status alignment |
| `frontend/src/routes/hosts/+page.svelte` and `frontend/src/routes/hosts/[id]/+page.svelte` | List/detail card migration and host detail slot parity |
| `frontend/src/routes/history/+page.svelte` | Transitional terminal/history styling alignment |
| `frontend/src/routes/surfaces/[id]/+page.svelte` | Shared page-shell parity for `surface.page` |
| `frontend/src/routes/services/+page.svelte`, `frontend/src/routes/system-services/+page.svelte`, `frontend/src/routes/host-tags/+page.svelte`, `frontend/src/routes/audit-logs/+page.svelte`, `frontend/src/routes/+page.svelte` | Remaining built-in page migration onto shared shell primitives |
| `frontend/src/routes/**/*.test.ts` | Route-level regression coverage for migrated pages |
| `frontend/tests/e2e/ui-parity.test.ts` | Desktop parity coverage once the route slices are stable |
| `frontend/tests/e2e/ui-parity-responsive.test.ts` | Responsive parity coverage for the target shell rollout |

---

### Task 1: Migrate The Shared Shell And Global Feedback Components

**Files:**

- Modify: `frontend/src/routes/+layout.svelte`
- Modify: `frontend/src/lib/components/ToastNotifications.svelte`
- Modify: `frontend/src/lib/components/TerminalOutput.svelte`
- Modify: `frontend/src/lib/components/ConfirmDialog.svelte`
- Verify and only patch if foundation left a blocking gap: `frontend/src/lib/components/Modal.svelte`
- Verify and only patch if foundation left a blocking gap: `frontend/src/lib/components/ContextMenu.svelte`
- Create: `frontend/src/lib/components/TerminalOutput.test.ts`
- Test: `frontend/src/lib/components/Modal.test.ts`
- Test: `frontend/src/lib/components/ConfirmDialog.test.ts`
- Test: `frontend/src/lib/components/ContextMenu.test.ts`
- Test: `frontend/src/routes/surface-migration.test.ts`

- [ ] **Step 1: Add the first shell regression**

Extend `frontend/src/routes/surface-migration.test.ts` or add a new
shell-focused route test so the merged navigation order is locked before
changing the layout.

Use an assertion shaped like:

```ts
expect(screen.getAllByRole('link').map((node) => node.textContent)).toContain('Software');
expect(screen.getAllByRole('link').map((node) => node.textContent)).toContain('Settings');
```

Add one explicit order assertion covering a built-in item and a `surface.page`
item with equal labels but different stable IDs.

Run:

```bash
cd frontend && npm run test -- src/routes/surface-migration.test.ts
```

Expected: the new ordering assertion fails until Step 2 completes. The simple
link-presence assertions are only sanity checks and may already pass.

- [ ] **Step 2: Migrate the shared shell**

Update `frontend/src/routes/+layout.svelte` so it consumes the new shell/page primitives and the canonical nav comparator from the shared foundation plan.

This task must:

- replace route-local nav-item styling with the shared shell treatment
- keep the current deployed shell metrics where the spec is still `Target`
- preserve the unified `surface.page` merge path
- leave the future tablet/mobile shell rollout for Task 5 in this plan

Run:

```bash
cd frontend && npm run test -- src/routes/surface-migration.test.ts
```

Expected: PASS with the shell using shared primitives and stable ordering.

- [ ] **Step 3: Migrate global feedback primitives**

Update `ToastNotifications.svelte`, `TerminalOutput.svelte`, and
`ConfirmDialog.svelte` to consume the shared `Callout`, `StatusBadge`,
`SectionCard`, and modal/menu shell primitives rather than route-local
presets. Treat `Modal.svelte` and `ContextMenu.svelte` as foundation-owned
shared primitives: only patch them here if the migration uncovers a blocking
integration gap that foundation Task 2 did not already cover.

Run:

```bash
cd frontend && npm run test -- src/lib/components/TerminalOutput.test.ts src/lib/components/Modal.test.ts src/lib/components/ConfirmDialog.test.ts src/lib/components/ContextMenu.test.ts src/routes/surface-migration.test.ts
```

Expected: PASS with shared modal/menu behavior preserved under the new styling.

- [ ] **Step 4: Commit**

```bash
git add frontend/src/routes/+layout.svelte frontend/src/lib/components/ToastNotifications.svelte frontend/src/lib/components/TerminalOutput.svelte frontend/src/lib/components/TerminalOutput.test.ts frontend/src/lib/components/ConfirmDialog.svelte frontend/src/lib/components/ConfirmDialog.test.ts frontend/src/routes/surface-migration.test.ts
git add --update frontend/src/lib/components/Modal.svelte frontend/src/lib/components/Modal.test.ts frontend/src/lib/components/ContextMenu.svelte frontend/src/lib/components/ContextMenu.test.ts
git commit -m "refactor: migrate shared frontend shell to design language"
```

---

### Task 2: Migrate The Parity-Sensitive Route Families

**Files:**

- Modify: `frontend/src/routes/settings/+page.svelte`
- Modify: `frontend/src/routes/settings/GlobalSettingsTab.svelte`
- Modify: `frontend/src/routes/settings/*.svelte`
- Modify: `frontend/src/routes/software/+page.svelte`
- Modify: `frontend/src/routes/software/[id]/+page.svelte`
- Modify: `frontend/src/routes/hosts/[id]/+page.svelte`
- Modify: `frontend/src/routes/surfaces/[id]/+page.svelte`
- Test: `frontend/src/routes/settings/surface-tabs.test.ts`
- Test: `frontend/src/routes/software/surface-tabs.test.ts`
- Test: `frontend/src/routes/surfaces/surfaces-page.test.ts`
- Test: `frontend/src/routes/hosts/[id]/host-detail.test.ts`

- [ ] **Step 1: Migrate the Settings route**

Refactor `frontend/src/routes/settings/+page.svelte` and its leaf panels so
the top-level settings shell uses `PageShell`, `TabStrip`, `SectionCard`, and
`FormFieldRow`.

Keep the existing URL-based tab state and permission logic intact. Do not redesign the data flow in this step.

Run:

```bash
cd frontend && npm run test -- src/routes/settings/surface-tabs.test.ts
```

Expected: PASS with built-in tabs and surface-backed tabs sharing the same visual container.

- [ ] **Step 2: Migrate the Software route family**

Refactor `frontend/src/routes/software/+page.svelte` and
`frontend/src/routes/software/[id]/+page.svelte` so they use the shared
`TabStrip`, `DataTable`, `StatusBadge`, `Callout`, and menu/modal shells.

This step must explicitly align:

- built-in software tabs with `software.tabs`
- host-context launcher rows with `software_item.host_context_menu`
- update/status affordances with the design-language badge rules

Run:

```bash
cd frontend && npm run test -- src/routes/software/surface-tabs.test.ts src/routes/surfaces/surfaces-page.test.ts
```

Expected: PASS with the route family using shared primitives and surface-backed tabs still functioning.

- [ ] **Step 3: Migrate host detail and surface pages**

Refactor `frontend/src/routes/hosts/[id]/+page.svelte` and
`frontend/src/routes/surfaces/[id]/+page.svelte` so the host detail slot
container and `surface.page` route both use the shared card/page shell.

Keep `host_detail.tabs` as an inline card stack until the spec promotes a different host-detail structure.

Run:

```bash
cd frontend && npm run test -- "src/routes/hosts/[id]/host-detail.test.ts" src/routes/surfaces/surfaces-page.test.ts
```

Expected: PASS with host-detail and surface-page parity slices stabilized.

- [ ] **Step 4: Commit**

```bash
git add frontend/src/routes/settings/+page.svelte frontend/src/routes/settings/GlobalSettingsTab.svelte frontend/src/routes/settings/AuthenticationSettings.svelte frontend/src/routes/settings/AgentCertificateSettings.svelte frontend/src/routes/settings/DangerZone.svelte frontend/src/routes/settings/EnrollmentTokenSettings.svelte frontend/src/routes/settings/NotificationLogView.svelte frontend/src/routes/settings/NotificationRulesSettings.svelte frontend/src/routes/settings/OidcProvidersSettings.svelte frontend/src/routes/settings/PluginConfigsTab.svelte frontend/src/routes/settings/RegistrationSettings.svelte frontend/src/routes/settings/SchedulerTab.svelte frontend/src/routes/settings/SystemServicesSettings.svelte frontend/src/routes/software/+page.svelte frontend/src/routes/software/IgnoreRulesTab.svelte "frontend/src/routes/software/[id]/+page.svelte" "frontend/src/routes/hosts/[id]/+page.svelte" "frontend/src/routes/surfaces/[id]/+page.svelte" frontend/src/routes/settings/surface-tabs.test.ts frontend/src/routes/software/surface-tabs.test.ts frontend/src/routes/surfaces/surfaces-page.test.ts "frontend/src/routes/hosts/[id]/host-detail.test.ts"
git commit -m "refactor: migrate parity-sensitive routes to design language"
```

---

### Task 3: Migrate The Remaining Built-In Routes

**Files:**

- Modify: `frontend/src/routes/+page.svelte`
- Modify: `frontend/src/routes/hosts/+page.svelte`
- Modify: `frontend/src/routes/history/+page.svelte`
- Modify: `frontend/src/routes/services/+page.svelte`
- Modify: `frontend/src/routes/system-services/+page.svelte`
- Modify: `frontend/src/routes/host-tags/+page.svelte`
- Modify: `frontend/src/routes/audit-logs/+page.svelte`
- Modify: `frontend/src/routes/profile/+page.svelte`
- Modify or explicitly defer with rationale: `frontend/src/routes/device/+page.svelte`
- Modify or explicitly defer with rationale: `frontend/src/routes/login/+page.svelte`
- Modify or explicitly defer with rationale: `frontend/src/routes/register/+page.svelte`
- Create: `frontend/src/routes/history/history.test.ts`
- Create: `frontend/src/routes/system-services/system-services.test.ts`
- Create: `frontend/src/routes/host-tags/host-tags.test.ts`
- Create: `frontend/src/routes/audit-logs/audit-logs.test.ts`
- Create: `frontend/src/routes/home.test.ts`
- Create: `frontend/src/routes/profile/profile.test.ts`
- Create or update if those routes are migrated now: `frontend/src/routes/device/device.test.ts`, `frontend/src/routes/login/login.test.ts`, `frontend/src/routes/register/register.test.ts`
- Test: `frontend/src/routes/hosts/hosts.test.ts`
- Test: `frontend/src/routes/services/services.test.ts`

- [ ] **Step 1: Migrate list and dashboard pages to shared cards/tables**

Refactor the built-in list pages so they use `PageShell`, `SectionCard`,
`DataTable`, `EmptyState`, `Callout`, and `StatusBadge` rather than route-local
wrappers.

Prioritize:

1. `hosts/+page.svelte`
2. `services/+page.svelte`
3. `system-services/+page.svelte`
4. `host-tags/+page.svelte`
5. `audit-logs/+page.svelte`
6. home/dashboard `+page.svelte`
7. `profile/+page.svelte`
8. `device/+page.svelte`, `login/+page.svelte`, and `register/+page.svelte`
   unless they are explicitly deferred with rationale because they require a
   separate auth/device-flow shell

Run:

```bash
cd frontend && npm run test -- src/routes/hosts/hosts.test.ts src/routes/services/services.test.ts src/routes/system-services/system-services.test.ts src/routes/host-tags/host-tags.test.ts src/routes/audit-logs/audit-logs.test.ts src/routes/home.test.ts src/routes/profile/profile.test.ts
```

Expected: PASS with each touched management route covered by a route-level
regression instead of relying on only two legacy tests. If `device`, `login`,
or `register` are migrated in this task, run their route tests here too.

- [ ] **Step 2: Migrate History onto the transitional terminal language**

Update `frontend/src/routes/history/+page.svelte` so its inline output, badges, and metadata use the same transitional terminal shell primitives as `TerminalOutput.svelte`.

This step should not yet introduce the final mobile/full-screen shell; it
should only make current history output visually consistent with the shared
terminal language.

Run:

```bash
cd frontend && npm run test -- src/lib/components/TerminalOutput.test.ts src/routes/history/history.test.ts
```

Expected: PASS with the shared terminal component still covering the history-aligned output path.

- [ ] **Step 3: Commit**

```bash
git add frontend/src/routes/+page.svelte frontend/src/routes/home.test.ts frontend/src/routes/hosts/+page.svelte frontend/src/routes/hosts/hosts.test.ts frontend/src/routes/history/+page.svelte frontend/src/routes/history/history.test.ts frontend/src/routes/services/+page.svelte frontend/src/routes/services/services.test.ts frontend/src/routes/system-services/+page.svelte frontend/src/routes/system-services/system-services.test.ts frontend/src/routes/host-tags/+page.svelte frontend/src/routes/host-tags/host-tags.test.ts frontend/src/routes/audit-logs/+page.svelte frontend/src/routes/audit-logs/audit-logs.test.ts frontend/src/routes/profile/+page.svelte frontend/src/routes/profile/profile.test.ts
git commit -m "refactor: migrate remaining built-in routes to design language"
```

If this task also migrates `device/+page.svelte`, `login/+page.svelte`, or
`register/+page.svelte`, stage those route and test files explicitly in the
same commit instead of leaving them for a follow-up patch by accident.

---

### Task 4: Add Route-Level Visual Parity Coverage

**Files:**

- Modify: existing Vitest route tests under `frontend/src/routes/`
- Create: `frontend/tests/e2e/ui-parity.test.ts`
- Modify: `docs/superpowers/ui-parity-waivers.json` only if a temporary exception is truly unavoidable

- [ ] **Step 1: Add the first Playwright parity slice**

Introduce one Playwright test file that compares:

- built-in settings tab vs `settings.tabs`
- built-in software tab vs `software.tabs`
- built-in nav item vs `surface.page`

Use deterministic fixtures from the foundation plan and freeze theme/viewport
inputs per the design-language spec. Reuse the existing
`frontend/playwright.config.ts` `webServer` setup instead of inventing a new
preview/start command for this plan. npm forwards arguments after `--` to
Playwright in the documented `npm run test:e2e -- --grep ...` form. Implement
these parity checks with `expect(page).toHaveScreenshot(...)` so the baseline
capture step has a concrete artifact model.

Run:

```bash
cd frontend && npm run test:e2e -- --grep "ui parity"
cd frontend && npx playwright test --update-snapshots --grep "ui parity"
```

Expected: the first command fails until the new parity assertions exist, and
the second command establishes the initial snapshot baseline once the capture
rules are intentional. Commit the generated Playwright snapshot artifacts under
the default `frontend/tests/e2e/*-snapshots/` directories in the same task.

- [ ] **Step 2: Expand parity coverage to the other required route pairs**

Add the rest of the now-required desktop parity slices:

- `settings.below.global` panel
- `host_detail.tabs` slot container
- `software_item.host_context_menu` launcher and opened modal
- `surface.page` loaded and runtime-state shells

Keep all desktop parity coverage in
`frontend/tests/e2e/ui-parity.test.ts` so the commit and snapshot staging stay
deterministic. Keep the test titles tagged with `ui parity` so the documented
grep command remains valid.

Only add a waiver entry if a temporary exception is explicitly justified and
dated. If a waiver is required and `docs/superpowers/ui-parity-waivers.json`
does not exist yet, land the governance plan's waiver-file step first or
create the file as `[]` before appending the documented schema entry.

Run:

```bash
cd frontend && npm run test:e2e -- --grep "ui parity"
```

Expected: PASS on the desktop parity matrix, with any approved waivers checked in.

- [ ] **Step 3: Commit**

```bash
git add frontend/tests/e2e/ui-parity.test.ts frontend/tests/e2e/ui-parity.test.ts-snapshots
test ! -f docs/superpowers/ui-parity-waivers.json || git add docs/superpowers/ui-parity-waivers.json
git add frontend/src/routes/settings/surface-tabs.test.ts frontend/src/routes/software/surface-tabs.test.ts frontend/src/routes/surfaces/surfaces-page.test.ts "frontend/src/routes/hosts/[id]/host-detail.test.ts"
git commit -m "test: add route-level ui parity coverage"
```

---

### Task 5: Promote The Still-Target Shell And Responsive Rules

**Files:**

- Modify: `frontend/src/routes/+layout.svelte`
- Modify: `frontend/src/lib/components/ToastNotifications.svelte`
- Modify: `frontend/src/lib/components/TerminalOutput.svelte`
- Create or modify: `frontend/tests/e2e/ui-parity-responsive.test.ts`
- Test: Playwright parity coverage for tablet/mobile shell states

- [ ] **Step 1: Migrate the shell metrics to the target values**

Promote the shared shell from the currently deployed metrics to the target spec values:

- sidebar width `180px`
- top bar `40px`
- content padding `12px 14px`
- target nav item sizing and spacing

Do this only after Tasks 1 through 4 are green.

Run:

```bash
cd frontend && npm run test && npm run check && npm run lint && npm run format:check
```

Expected: the frontend still passes all existing unit/type/lint checks after the shell-metric promotion.

- [ ] **Step 2: Implement the responsive shell target**

Update the layout and shared feedback components to implement the still-`Target` responsive rules:

- tablet overlay sidebar
- mobile bottom navigation / overflow sheet
- bottom-center mobile toasts
- mobile terminal full-screen behavior

Create or update responsive Playwright specs under `frontend/tests/e2e/` and
tag the relevant cases with `responsive ui parity` so the documented grep
command maps to concrete coverage instead of an implied future suite.

Run:

```bash
cd frontend && npm run test:e2e -- --grep "responsive ui parity"
```

Expected: PASS on tablet/mobile parity coverage once the responsive shell exists.

- [ ] **Step 3: Final frontend verification**

Run:

```bash
cd frontend && npm run test && npm run check && npm run lint && npm run format:check && npm run build && npm run test:e2e
```

Expected: the full frontend suite passes after the migration and responsive promotion are complete.

- [ ] **Step 4: Commit**

```bash
git add frontend/src/routes/+layout.svelte frontend/src/lib/components/ToastNotifications.svelte frontend/src/lib/components/TerminalOutput.svelte frontend/tests/e2e/ui-parity-responsive.test.ts frontend/tests/e2e/ui-parity-responsive.test.ts-snapshots
git commit -m "feat: complete ui design language frontend migration"
```

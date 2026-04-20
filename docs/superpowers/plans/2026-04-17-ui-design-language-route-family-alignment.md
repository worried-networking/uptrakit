<!-- markdownlint-disable MD013 -->

# UI Design Language Route Family Alignment Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement
> this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close every remaining spec-versus-reality gap across the built-in management and data routes, including all route-owned tables, cards,
badges, pills, buttons, toggles, empty and loading states, menus, footers, dialogs, and terminal interactions, so no undocumented divergence remains
in this route scope.

**Architecture:** Work route-family by route-family rather than scattering cosmetic edits across the app. Start with the shared list/index patterns
(`Dashboard`, `Services`, `System Services`, `Hosts`, `Host Tags`, `Audit Logs`, `Profile`), then tackle the denser interaction routes (`Software`,
`Software Detail`, `History`). Keep all route work on the shared primitives and shared token contract, and refresh route-level parity coverage as each
family lands. This plan owns the remaining built-in-route gaps for spec Sections 4.1 through 4.14, Sections 5.1 through 5.3, and route-level
interaction conventions from Section 8. Shared terminal-shell implementation and the built-in Software modal or wizard components are owned by
`2026-04-17-ui-design-language-interaction-contract-and-modal-convergence.md`; this plan only consumes those shared pieces at the route integration
layer.

**Tech Stack:** SvelteKit routes, shared UI primitives in `frontend/src/lib/components/ui`, route-owned dialogs in `frontend/src/lib/components`,
Vitest, Testing Library, Playwright, Markdown docs.

**Execution Context:** Run commands from the repository root. This plan assumes the shared visual/token foundation has already landed. Use
`docs/development/web-ui-inventory.md`, `docs/development/ui-design-language.md`, and the approved spec as the source of truth during implementation.
Do not run any `--update-snapshots` step in this plan until `2026-04-17-ui-design-language-interaction-contract-and-modal-convergence.md` Task 5 has
landed, because that plan owns the parity-gate mechanics. Serialize any task that edits `frontend/src/routes/home.test.ts`,
`frontend/tests/e2e/ui-parity.test.ts`, or its snapshots; do not run Task 1 in parallel with `2026-04-17-ui-design-language-shell-and-entry-flows.md`,
and do not run snapshot work in parallel with the settings/surfaces plan because they share the same parity fixture file. Do not run Task 3 in
parallel with `2026-04-17-ui-design-language-interaction-contract-and-modal-convergence.md`, because both plans touch
`frontend/src/routes/history/+page.svelte`, `frontend/src/routes/software/[id]/+page.svelte`, and related route tests. When work needs shared
transition, focus, z-index, modal-shell, or terminal-shell changes, execute the companion interaction-contract plan first or in a stacked sequence so
route work consumes the shared behavior rather than reintroducing route-local styling.

---

## File Map

| File                                                 | Change                                                                                                  |
| ---------------------------------------------------- | ------------------------------------------------------------------------------------------------------- |
| `frontend/src/routes/+page.svelte`                   | Recheck dashboard summary, attention cards, and recent-updates table against the design language        |
| `frontend/src/routes/services/+page.svelte`          | Align service list table, status stack, row actions, dialogs, and footer placement                      |
| `frontend/src/routes/system-services/+page.svelte`   | Align system-service list table, status stack, row actions, dialogs, and footer placement               |
| `frontend/src/routes/hosts/+page.svelte`             | Align host list stat rhythm, status badges, row actions, and footer placement                           |
| `frontend/src/routes/hosts/[id]/+page.svelte`        | Align host-detail cards, inline tables, and detail dialogs                                              |
| `frontend/src/routes/hosts/[id]/host-detail.test.ts` | Preserve host-detail card rhythm, inline table treatment, and `host_detail.tabs` parity                 |
| `frontend/src/routes/host-tags/+page.svelte`         | Align host-tag search/list/dialog surfaces                                                              |
| `frontend/src/routes/software/+page.svelte`          | Continue route-level cleanup of grouped software list, route dialogs, menus, and batch flows            |
| `frontend/src/routes/software/IgnoreRulesTab.svelte` | Align ignore-rules tab shell, table rhythm, and footer behavior with the shared Software route language |
| `frontend/src/routes/software/[id]/+page.svelte`     | Align software-detail table, live terminal affordances, release notes, and host-context interactions    |
| `frontend/src/routes/history/+page.svelte`           | Align history feed, trigger-update overlay, and embedded output treatment                               |
| `frontend/src/routes/audit-logs/+page.svelte`        | Align scope/filter/list layout and footer behavior                                                      |
| `frontend/src/routes/profile/+page.svelte`           | Align account/token sections and token modals                                                           |
| `frontend/src/lib/components/ConfirmDialog.svelte`   | Recheck route confirm-dialog content density and semantic treatment if route work exposes drift         |
| `frontend/src/lib/components/BatchActionBar.svelte`  | Recheck floating batch toolbar metrics against route-family usage                                       |
| `frontend/src/routes/**/*.test.ts`                   | Extend route-level regressions for layout, badges, menus, dialogs, and footer alignment                 |
| `frontend/tests/e2e/ui-parity.test.ts`               | Add or refresh built-in route parity fixtures for changed route families                                |

---

### Task 0: Record The Remaining Built-In Route Gap Checklist

**Files:**

- Verify against: `docs/development/web-ui-inventory.md`
- Verify against: `docs/development/ui-design-language.md`
- Verify against: `docs/superpowers/specs/2026-04-16-ui-design-language-design.md`

- [ ] **Step 1: Enumerate every in-scope route discrepancy before implementation**

Build a checklist for every remaining route gap in this scope, including:

- badges, pills, buttons, toggles, stat cards, loading states, and empty states
- confirmation dialogs, form validation, tab strips, data tables, context menus, and workflow shells
- page-pattern drift on Software, Hosts, and History
- route-owned interaction conventions such as disabled actions, hover states, selection, and confirm ordering
- terminal-launch and route integration convergence on History and Software Detail after the shared terminal shell lands

Expected: every known built-in route discrepancy maps to one of the tasks below before implementation starts; nothing in this scope is silently
deferred.

---

### Task 1: Align List And Summary Routes

**Files:**

- Modify: `frontend/src/routes/+page.svelte`
- Modify: `frontend/src/routes/services/+page.svelte`
- Modify: `frontend/src/routes/system-services/+page.svelte`
- Modify: `frontend/src/routes/hosts/+page.svelte`
- Modify: `frontend/src/routes/host-tags/+page.svelte`
- Modify: `frontend/src/routes/audit-logs/+page.svelte`
- Modify: `frontend/src/routes/profile/+page.svelte`
- Modify: `frontend/src/routes/home.test.ts`
- Modify: `frontend/src/routes/services/services.test.ts`
- Modify: `frontend/src/routes/system-services/system-services.test.ts`
- Modify: `frontend/src/routes/hosts/hosts.test.ts`
- Modify: `frontend/src/routes/host-tags/host-tags.test.ts`
- Modify: `frontend/src/routes/audit-logs/audit-logs.test.ts`
- Modify: `frontend/src/routes/profile/profile.test.ts`
- Modify if needed: `frontend/tests/e2e/ui-parity.test.ts`

- [ ] **Step 1: Write the failing route-family regressions**

Extend route tests so they cover:

- table header/body/footer alignment
- menu-row and action-badge treatment
- section-card spacing consistency
- dialog launcher behavior where present

Run:

```bash
(cd frontend && npm run test -- src/routes/home.test.ts src/routes/services/services.test.ts src/routes/system-services/system-services.test.ts src/routes/hosts/hosts.test.ts src/routes/host-tags/host-tags.test.ts src/routes/audit-logs/audit-logs.test.ts src/routes/profile/profile.test.ts)
```

Expected: FAIL where pages still drift from the design-language contract.

- [ ] **Step 2: Implement the list/summary alignment**

Refactor the listed routes to ensure:

- all list and index routes in this task use the same `SectionCard` and `DataTable` rhythm
- pagination/totals stay inside the shared footer contract
- row action launchers and context menus use consistent placement and text sizing
- route-level dialogs use the same modal density and button ordering
- dashboard summary sections and attention blocks match the same card/badge rhythm as other routes

Run:

```bash
(cd frontend && npm run test -- src/routes/home.test.ts src/routes/services/services.test.ts src/routes/system-services/system-services.test.ts src/routes/hosts/hosts.test.ts src/routes/host-tags/host-tags.test.ts src/routes/audit-logs/audit-logs.test.ts src/routes/profile/profile.test.ts)
(cd frontend && npm run check)
(cd frontend && npm run lint)
```

Expected: PASS for the whole list/summary route family.

- [ ] **Step 3: Refresh parity coverage for the list/summary family**

Only run this step after `2026-04-17-ui-design-language-interaction-contract-and-modal-convergence.md` Task 5 has landed in the same branch or is
already merged below this work.

Run:

```bash
(cd frontend && npm run test:e2e -- --list --grep "dashboard|services|system services|hosts|host tags|audit logs|profile")
(cd frontend && npm run test:e2e -- --grep "dashboard|services|system services|hosts|host tags|audit logs|profile")
(cd frontend && npm run test:e2e -- --grep "dashboard|services|system services|hosts|host tags|audit logs|profile" --update-snapshots)
(cd frontend && npm run test:e2e -- --grep "dashboard|services|system services|hosts|host tags|audit logs|profile")
```

Expected: PASS with updated route snapshots.

- [ ] **Step 4: Commit**

```bash
git add frontend/src/routes/+page.svelte frontend/src/routes/services/+page.svelte frontend/src/routes/system-services/+page.svelte frontend/src/routes/hosts/+page.svelte frontend/src/routes/host-tags/+page.svelte frontend/src/routes/audit-logs/+page.svelte frontend/src/routes/profile/+page.svelte frontend/src/routes/home.test.ts frontend/src/routes/services/services.test.ts frontend/src/routes/system-services/system-services.test.ts frontend/src/routes/hosts/hosts.test.ts frontend/src/routes/host-tags/host-tags.test.ts frontend/src/routes/audit-logs/audit-logs.test.ts frontend/src/routes/profile/profile.test.ts frontend/tests/e2e/ui-parity.test.ts
if [ -d frontend/tests/e2e/ui-parity.test.ts-snapshots ]; then git add frontend/tests/e2e/ui-parity.test.ts-snapshots; fi
git commit -m "refactor: align list routes with design language"
```

---

### Task 2: Align Host Detail Route

**Files:**

- Modify: `frontend/src/routes/hosts/[id]/+page.svelte`
- Modify: `frontend/src/routes/hosts/[id]/host-detail.test.ts`
- Modify if needed: `frontend/tests/e2e/ui-parity.test.ts`

- [ ] **Step 1: Write the failing host-detail regressions**

Add or extend tests so they cover:

- host-detail hero/stat-card rhythm and section spacing
- inline table and footer alignment inside the detail route
- detail dialogs and context actions using the shared modal/menu hierarchy
- `host_detail.tabs` slot-state coverage for loaded, `permission_denied`, targeted `no_compatible_provider`, `contract_mismatch`,
  `hydration_action_failure`, and omitted-state
- route-owned host chrome versus `host_detail.tabs` surface card-shell parity

Run:

```bash
(cd frontend && npm run test -- 'src/routes/hosts/[id]/host-detail.test.ts')
```

Expected: FAIL where host detail still drifts from the approved card, table, or slot-container treatment.

- [ ] **Step 2: Implement the host-detail alignment**

Bring the host-detail route into the shared design language so:

- detail cards, stat blocks, and inline sections follow the same `SectionCard` and spacing rhythm as other built-in routes
- inline lists and tables use the shared footer/pagination contract
- route-owned dialogs, menus, and launcher rows match the same density and ordering as the rest of the app
- the built-in host detail body exposes the stable container and `data-parity-region` markers needed for `host_detail.tabs` parity checks

Run:

```bash
(cd frontend && npm run test -- 'src/routes/hosts/[id]/host-detail.test.ts')
(cd frontend && npm run check)
(cd frontend && npm run lint)
```

Expected: PASS with host detail aligned and ready for slot-backed parity checks.

- [ ] **Step 3: Refresh host-detail parity**

Only run this step after `2026-04-17-ui-design-language-interaction-contract-and-modal-convergence.md` Task 5 has landed in the same branch or is
already merged below this work.

Run:

```bash
(cd frontend && npm run test:e2e -- --list --grep "host detail|host_detail\\.tabs|ui parity")
(cd frontend && npm run test:e2e -- --grep "host detail|host_detail\\.tabs|ui parity")
(cd frontend && npm run test:e2e -- --grep "host detail|host_detail\\.tabs|ui parity" --update-snapshots)
(cd frontend && npm run test:e2e -- --grep "host detail|host_detail\\.tabs|ui parity")
```

Expected: PASS with refreshed built-in host-detail versus `host_detail.tabs` parity baselines in both themes.

- [ ] **Step 4: Commit**

```bash
git add 'frontend/src/routes/hosts/[id]/+page.svelte' 'frontend/src/routes/hosts/[id]/host-detail.test.ts' frontend/tests/e2e/ui-parity.test.ts
if [ -d frontend/tests/e2e/ui-parity.test.ts-snapshots ]; then git add frontend/tests/e2e/ui-parity.test.ts-snapshots; fi
git commit -m "refactor: align host detail with design language"
```

---

### Task 3: Align Dense Interaction Routes

**Files:**

- Modify: `frontend/src/routes/software/+page.svelte`
- Modify: `frontend/src/routes/software/IgnoreRulesTab.svelte`
- Modify: `frontend/src/routes/software/[id]/+page.svelte`
- Modify: `frontend/src/routes/history/+page.svelte`
- Modify associated `software*.test.ts` and `history*.test.ts` files
- Modify if needed: `frontend/src/lib/components/ConfirmDialog.svelte`
- Modify if needed: `frontend/src/lib/components/BatchActionBar.svelte`

- [ ] **Step 1: Write the failing dense-route regressions**

Add or extend route tests to cover:

- grouped software list behavior, folding, overflow expansion, and update affordances
- software-detail modals and host-row actions
- route integration behavior for assign-host, edit-assignment, and merge-wizard flows on Software routes after the shared modal or workflow
  convergence lands
- `software.tabs` slot-state coverage for loaded, `permission_denied`, targeted `no_compatible_provider`, `contract_mismatch`, and structural
  `no_surface_content` host-chrome checks
- built-in software tab versus `software.tabs` parity and tab-container ownership
- `software_item.host_context_menu` launcher row, opened modal, `permission_denied` or `contract_mismatch` fallback, and omitted-state coverage
- built-in host context launcher/menu item versus `software_item.host_context_menu` launcher and opened modal treatment
- history feed card rhythm, trigger-update overlay, and terminal-launcher treatment
- dense route dialogs using consistent confirm/cancel ordering and shared semantics
- final terminal-shell convergence criteria from spec Section 6

Run:

```bash
(cd frontend && npm run test -- src/routes/software/software-trigger-status.test.ts src/routes/software/surface-tabs.test.ts 'src/routes/software/[id]/software-detail-update-trigger.test.ts' 'src/routes/software/[id]/software-detail.test.ts' src/routes/history/history.test.ts src/routes/history/history-trigger-status.test.ts)
```

Expected: FAIL where dense routes still use route-local exceptions that are not spec-compliant.

- [ ] **Step 2: Implement the dense-route alignment**

Normalize the route-owned UI on:

- grouped list and detail layouts
- release/update/terminal dialogs
- assign-host, edit-assignment, and merge-wizard entry points consuming the shared modal, form-validation, loading, and workflow contracts owned by
  the interaction-convergence plan
- context-menu copy density and action ordering
- history trigger flow and terminal-launch flow
- any remaining route-local modal or dialog treatment that diverges from the shared design language once shared overlays have landed
- the route integration points for the canonical shared terminal-shell component in History and Software Detail

Run:

```bash
(cd frontend && npm run test -- src/routes/software/software-trigger-status.test.ts src/routes/software/surface-tabs.test.ts 'src/routes/software/[id]/software-detail-update-trigger.test.ts' 'src/routes/software/[id]/software-detail.test.ts' src/routes/history/history.test.ts src/routes/history/history-trigger-status.test.ts)
(cd frontend && npm run check)
(cd frontend && npm run lint)
```

Expected: PASS for Software and History route families, with both routes consuming the same canonical shared terminal shell and no route-local visual
drift around those integration points.

- [ ] **Step 3: Refresh dense-route parity**

Only run this step after `2026-04-17-ui-design-language-interaction-contract-and-modal-convergence.md` Task 3 and Task 5 have landed in the same
branch or are already merged below this work.

Run:

```bash
(cd frontend && npm run test:e2e -- --grep "software|history|software\\.tabs|host context menu|ui parity")
(cd frontend && npm run test:e2e -- --grep "software|history|software\\.tabs|host context menu|ui parity" --update-snapshots)
(cd frontend && npm run test:e2e -- --grep "software|history|software\\.tabs|host context menu|ui parity")
```

Expected: PASS with refreshed dense-route visual baselines plus `software.tabs` and `software_item.host_context_menu` parity snapshots in both themes,
and route-level terminal-launch parity coverage for Software Detail and History.

- [ ] **Step 4: Commit**

```bash
git add frontend/src/routes/software/+page.svelte frontend/src/routes/software/IgnoreRulesTab.svelte 'frontend/src/routes/software/[id]/+page.svelte' frontend/src/routes/history/+page.svelte frontend/src/routes/software/software-trigger-status.test.ts frontend/src/routes/software/surface-tabs.test.ts 'frontend/src/routes/software/[id]/software-detail-update-trigger.test.ts' 'frontend/src/routes/software/[id]/software-detail.test.ts' frontend/src/routes/history/history.test.ts frontend/src/routes/history/history-trigger-status.test.ts frontend/tests/e2e/ui-parity.test.ts
if ! git diff --quiet -- frontend/src/lib/components/ConfirmDialog.svelte; then git add frontend/src/lib/components/ConfirmDialog.svelte; fi
if ! git diff --quiet -- frontend/src/lib/components/BatchActionBar.svelte; then git add frontend/src/lib/components/BatchActionBar.svelte; fi
if [ -d frontend/tests/e2e/ui-parity.test.ts-snapshots ]; then git add frontend/tests/e2e/ui-parity.test.ts-snapshots; fi
git commit -m "refactor: align dense routes with design language"
```

---

### Task 4: Update Route Inventory Docs

**Files:**

- Modify: `docs/development/web-ui-inventory.md`

- [ ] **Step 1: Update route and dialog inventory**

Reflect final route ownership, dialog names, and any removed/added route-local UI after the alignment work lands.

- [ ] **Step 2: Verify docs**

Run:

```bash
markdownlint --config .markdownlint.json docs/development/web-ui-inventory.md
```

Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add docs/development/web-ui-inventory.md
git commit -m "docs: refresh route inventory after UI alignment"
```

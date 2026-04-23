<!-- markdownlint-disable MD013 -->

# UI Design Language Settings And Surfaces Alignment Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement
> this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close every remaining spec-versus-reality gap in the settings experience, canonical surface pages, and shared surface runtime so built-in
and surface-backed UI remain visually indistinguishable with no undocumented divergence left in this scope.

**Architecture:** Treat settings and surfaces as one parity subsystem. First bring the settings tab shell and each built-in settings panel onto one
consistent spacing, form-validation, loading, empty, badge, table, and dialog rhythm. Then align the canonical `/surfaces/[id]` route plus shared
surface runtime components (`SurfaceReadPanel`, `SurfaceRenderer`, workflows, modals, tables`) with the same primitives and runtime-state vocabulary
documented in the design-language guide and redesign spec. Finish by updating parity coverage and governance docs. This plan owns the remaining gaps
for spec Sections 4.10 through 4.16, Section 5.4, Section 5.5, and settings/surface parts of Section 8.

**Tech Stack:** SvelteKit routes, shared UI primitives, shared Surfaces runtime, Vitest, Playwright, Markdown docs, waiver/governance docs if needed.

**Execution Context:** Run commands from the repository root. Use `docs/development/web-ui-inventory.md`, `docs/development/ui-design-language.md`,
and `docs/superpowers/specs/2026-04-16-ui-design-language-design.md` as the implementation reference set. Complete Task 1 through Step 3 before Task 2
begins; both tasks depend on the same settings-panel regression surface and should not be dispatched in parallel. Do not run any `--update-snapshots`
step in this plan until `2026-04-17-ui-design-language-interaction-contract-and-modal-convergence.md` Task 5 has landed, because that plan owns the
parity-gate mechanics. Serialize any task that edits `frontend/src/routes/surfaces/surfaces-page.test.ts`, `frontend/tests/e2e/ui-parity.test.ts`,
`docs/development/frontend-components.md`, or their snapshots with the other active UI plans. When work needs shared transition, focus, z-index, or
modal-shell changes, execute the companion interaction-contract plan first or in a stacked sequence so settings and surfaces consume the shared
behavior rather than forking their own styling.

---

## File Map

| File                                                                    | Change                                                                                                                              |
| ----------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------- |
| `frontend/src/routes/settings/+page.svelte`                             | Align settings tab strip, tab-body framing, and built-in/surface tab parity                                                         |
| `frontend/src/routes/settings/global/+page.svelte`                      | Verify the global-settings redirect shim remains UI-less or route it onto the shared settings shell if any visible UI is introduced |
| `frontend/src/routes/settings/*.svelte`                                 | Align settings section cards, form rows, inline tables, and dialogs across all built-in settings panels                             |
| `frontend/src/routes/surfaces/[id]/+page.svelte`                        | Align canonical standalone surface-page shell and fallback states                                                                   |
| `frontend/src/lib/components/surfaces/SurfaceReadPanel.svelte`          | Recheck runtime-state rendering, provider selection, and error/no-provider presentation                                             |
| `frontend/src/lib/components/surfaces/SurfaceRenderer.svelte`           | Recheck node rendering against the approved primitive mapping                                                                       |
| `frontend/src/lib/components/surfaces/SchemaForm.svelte`                | Recheck schema-driven field rendering, validation, and spacing against the shared form contract                                     |
| `frontend/src/lib/components/surfaces/SurfaceKeyValue.svelte`           | Recheck read-only key/value presentation against the approved primitive mapping                                                     |
| `frontend/src/lib/components/surfaces/SurfaceModal.svelte`              | Recheck modal presentation for surface-triggered interactions                                                                       |
| `frontend/src/lib/components/surfaces/SurfaceWorkflow.svelte`           | Recheck workflow-step shell, review state, and security-impact presentation                                                         |
| `frontend/src/lib/components/surfaces/SurfaceActionBar.svelte`          | Recheck surface action-bar treatment against shared action rows and batch controls                                                  |
| `frontend/src/lib/components/surfaces/SurfaceSlot.svelte`               | Recheck slot-container framing and parity-region marker ownership                                                                   |
| `frontend/src/lib/components/surfaces/SurfaceTable.svelte`              | Recheck table/footer parity between built-in and surface-backed tables                                                              |
| `frontend/src/routes/settings/surface-tabs.test.ts`                     | Preserve built-in vs `settings.tabs` parity                                                                                         |
| `frontend/src/routes/settings/settings-panels.test.ts`                  | Preserve built-in settings panel form, dialog, badge, and inline-table alignment                                                    |
| `frontend/src/routes/surfaces/surfaces-page.test.ts`                    | Preserve canonical surface page behavior and fallback states                                                                        |
| `frontend/src/lib/components/surfaces/SchemaForm.test.ts`               | Preserve schema-driven form rendering and validation treatment                                                                      |
| `frontend/src/lib/components/surfaces/SurfaceKeyValue.test.ts`          | Create or extend read-only key/value regressions if coverage is missing                                                             |
| `frontend/src/lib/components/surfaces/SurfaceForm.test.ts`              | Preserve shared surface form validation and spacing treatment                                                                       |
| `frontend/src/lib/components/surfaces/SurfaceInteractionButton.test.ts` | Preserve shared surface interaction-button density and modal/workflow launch treatment                                              |
| `frontend/src/lib/components/surfaces/*.test.ts`                        | Add runtime-state and primitive-mapping regressions where needed                                                                    |
| `frontend/tests/e2e/ui-parity.test.ts`                                  | Add or refresh settings/surface parity coverage                                                                                     |
| `docs/development/frontend-components.md`                               | Refresh shared-surface component documentation if responsibilities change                                                           |
| `docs/development/ui-design-language.md`                                | Refresh implementation guidance if the current alignment closes transitional gaps                                                   |
| `frontend/tests/e2e/ui-parity-waivers.json`                             | Update only if a temporary parity exception is unavoidable and explicitly justified                                                 |

---

### Task 0: Record The Remaining Settings And Surface Gap Checklist

**Files:**

- Verify against: `docs/development/web-ui-inventory.md`
- Verify against: `docs/development/ui-design-language.md`
- Verify against: `docs/superpowers/specs/2026-04-16-ui-design-language-design.md`

- [ ] **Step 1: Enumerate every in-scope settings and surface discrepancy before implementation**

Build a checklist for every remaining settings and surface gap in this scope, including:

- settings form validation, inline errors, loading states, empty states, toggles, tables, badges, dialogs, and destructive flows
- settings-page pattern ownership and tab behavior
- slot-backed detail panel treatment
- surface runtime-state treatment and targeted-provider selection
- shared-surface primitive mapping and workflow or modal parity
- settings and surface interaction-convention drift from Section 8

Expected: every remaining settings or surface discrepancy maps to an owning task before implementation starts; nothing in this scope is left implicit.

---

### Task 1: Align Built-In Settings Panels

**Files:**

- Modify: `frontend/src/routes/settings/+page.svelte`
- Modify: `frontend/src/routes/settings/AgentCertificateSettings.svelte`
- Modify: `frontend/src/routes/settings/AuthenticationSettings.svelte`
- Modify: `frontend/src/routes/settings/DangerZone.svelte`
- Modify: `frontend/src/routes/settings/EnrollmentTokenSettings.svelte`
- Modify: `frontend/src/routes/settings/GlobalSettingsTab.svelte`
- Modify: `frontend/src/routes/settings/NotificationLogView.svelte`
- Modify: `frontend/src/routes/settings/NotificationRulesSettings.svelte`
- Modify: `frontend/src/routes/settings/OidcProvidersSettings.svelte`
- Modify: `frontend/src/routes/settings/PluginConfigsTab.svelte`
- Modify: `frontend/src/routes/settings/RegistrationSettings.svelte`
- Modify: `frontend/src/routes/settings/SchedulerTab.svelte`
- Modify: `frontend/src/routes/settings/SystemServicesSettings.svelte`
- Modify: `frontend/src/routes/settings/surface-tabs.test.ts`
- Create or modify: `frontend/src/routes/settings/settings-panels.test.ts`

- [ ] **Step 1: Write the failing settings regressions**

Add or extend tests so they cover:

- shared tab-strip framing for built-in and surface settings tabs
- consistent `FormFieldRow` rhythm across leaf panels
- form validation, inline error copy, and loading or empty states inside settings panels
- settings modal/confirm dialog consistency
- settings inline tables and badges using the same shared patterns as built-in routes

Run:

```bash
(cd frontend && npm run test -- src/routes/settings/surface-tabs.test.ts src/routes/settings/settings-panels.test.ts)
```

Expected: FAIL or missing coverage for the settings alignment contract.

- [ ] **Step 2: Implement built-in settings alignment**

Refactor the settings route and child panels so:

- the tab strip, tab body, and section spacing match the approved settings treatment
- settings forms all use the same field-row, button, and callout rhythm
- settings form validation, inline loading treatment, and empty-state handling match the approved spec conventions
- built-in settings tables and token lists match the same `DataTable`/badge/footer language as the rest of the app
- settings dialogs and confirms use the shared `ModalShell` contract exported from `$lib/components/ui`, with
  `frontend/src/lib/components/Modal.svelte` treated as the underlying shared shell owned by the interaction-contract plan rather than left as an
  implicit legacy wrapper

Run:

```bash
(cd frontend && npm run test -- src/routes/settings/surface-tabs.test.ts src/routes/settings/settings-panels.test.ts)
(cd frontend && npm run check)
(cd frontend && npm run lint)
```

Expected: PASS with the built-in settings experience visually unified.

- [ ] **Step 3: Commit**

```bash
git add frontend/src/routes/settings/+page.svelte frontend/src/routes/settings/AgentCertificateSettings.svelte frontend/src/routes/settings/AuthenticationSettings.svelte frontend/src/routes/settings/DangerZone.svelte frontend/src/routes/settings/EnrollmentTokenSettings.svelte frontend/src/routes/settings/GlobalSettingsTab.svelte frontend/src/routes/settings/NotificationLogView.svelte frontend/src/routes/settings/NotificationRulesSettings.svelte frontend/src/routes/settings/OidcProvidersSettings.svelte frontend/src/routes/settings/PluginConfigsTab.svelte frontend/src/routes/settings/RegistrationSettings.svelte frontend/src/routes/settings/SchedulerTab.svelte frontend/src/routes/settings/SystemServicesSettings.svelte frontend/src/routes/settings/surface-tabs.test.ts frontend/src/routes/settings/settings-panels.test.ts
git commit -m "refactor: align settings panels with design language"
```

---

### Task 2: Align Canonical Surface Pages And Shared Surface Runtime

**Files:**

- Modify: `frontend/src/routes/surfaces/[id]/+page.svelte`
- Modify: `frontend/src/lib/components/surfaces/SurfaceReadPanel.svelte`
- Modify: `frontend/src/lib/components/surfaces/SurfaceRenderer.svelte`
- Modify: `frontend/src/lib/components/surfaces/SchemaForm.svelte`
- Modify: `frontend/src/lib/components/surfaces/SurfaceKeyValue.svelte`
- Modify: `frontend/src/lib/components/surfaces/SurfaceModal.svelte`
- Modify: `frontend/src/lib/components/surfaces/SurfaceWorkflow.svelte`
- Modify: `frontend/src/lib/components/surfaces/SurfaceActionBar.svelte`
- Modify: `frontend/src/lib/components/surfaces/SurfaceSlot.svelte`
- Modify: `frontend/src/lib/components/surfaces/SurfaceTable.svelte`
- Modify associated surface tests
- Modify: `frontend/src/lib/components/surfaces/SchemaForm.test.ts`
- Create or modify if needed: `frontend/src/lib/components/surfaces/SurfaceKeyValue.test.ts`

- [ ] **Step 1: Write the failing surface-runtime regressions**

Add or extend tests to cover:

- canonical `/surfaces/[id]` page shell and fallback states
- `surface.page` parity for loaded, `permission_denied`, targeted `no_compatible_provider`, `contract_mismatch`, and `hydration_action_failure` states
- `settings.tabs` parity for loaded, `permission_denied`, targeted `no_compatible_provider`, `contract_mismatch`, and `no_surface_content` states
- built-in inline settings card versus `settings.below.global` surface panel parity for loaded, `permission_denied`, targeted
  `no_compatible_provider`, `contract_mismatch`, `hydration_action_failure`, and omitted-state
- deterministic multi-provider ordering for `settings.tabs` and `settings.below.global` by `priority`, then `label`, then `surface_id`
- standard form field row versus targeted-surface provider selector parity
- targeted-surface provider selector ownership, ensuring the host route or slot wrapper renders exactly one selector and surface nodes do not render a
  duplicate nested selector
- key/value read rows, schema-form field rendering, and interaction-button launchers using the same approved spacing and tone hierarchy as other
  shared primitives
- surface modal and workflow parity with built-in modal/workflow treatment
- surface table/footer parity with built-in `DataTable`
- shared-primitive parity coverage for `Table` header, body, empty, and row-action treatment
- shared-primitive parity coverage for `Callout` info, warning, and danger variants
- shared-primitive parity coverage for `ModalTrigger` trigger/opened-shell states and `WorkflowTrigger` trigger/opened-shell/step-indicator states
- dual-theme parity coverage for every required built-in versus surface-backed pair in this plan scope

Run:

```bash
(cd frontend && npm run test -- src/routes/settings/surface-tabs.test.ts src/routes/settings/settings-panels.test.ts src/routes/surfaces/surfaces-page.test.ts src/lib/components/surfaces/SurfaceReadPanel.test.ts src/lib/components/surfaces/SurfaceRenderer.test.ts src/lib/components/surfaces/SchemaForm.test.ts src/lib/components/surfaces/SurfaceKeyValue.test.ts src/lib/components/surfaces/SurfaceForm.test.ts src/lib/components/surfaces/SurfaceInteractionButton.test.ts src/lib/components/surfaces/SurfaceModal.test.ts src/lib/components/surfaces/SurfaceWorkflow.test.ts src/lib/components/surfaces/SurfaceTable.test.ts src/lib/components/surfaces/SurfaceActionBar.test.ts src/lib/components/surfaces/SurfaceSlot.test.ts)
```

Expected: FAIL where shared surface runtime still exposes visual or state-treatment drift.

- [ ] **Step 2: Implement surface parity alignment**

Bring the surface runtime fully into the shared design language:

- canonical surface pages use the same page and section shell language as built-in pages
- all runtime states use the documented shared empty/callout/provider-selector treatment
- surface-triggered modals and workflows match built-in modal/workflow density and action rhythm
- surface tables and action bars remain visually indistinguishable from built-in list/table components
- built-in settings-card and surface-panel boundaries expose stable `data-parity-region` markers for `settings.below.global` and `surface.page`
  capture regions
- parity fixtures and state helpers use the documented slot/state names exactly, with no surface-only aliases

Run:

```bash
(cd frontend && npm run test -- src/routes/settings/surface-tabs.test.ts src/routes/settings/settings-panels.test.ts src/routes/surfaces/surfaces-page.test.ts src/lib/components/surfaces/SurfaceReadPanel.test.ts src/lib/components/surfaces/SurfaceRenderer.test.ts src/lib/components/surfaces/SchemaForm.test.ts src/lib/components/surfaces/SurfaceKeyValue.test.ts src/lib/components/surfaces/SurfaceForm.test.ts src/lib/components/surfaces/SurfaceInteractionButton.test.ts src/lib/components/surfaces/SurfaceModal.test.ts src/lib/components/surfaces/SurfaceWorkflow.test.ts src/lib/components/surfaces/SurfaceTable.test.ts src/lib/components/surfaces/SurfaceActionBar.test.ts src/lib/components/surfaces/SurfaceSlot.test.ts)
(cd frontend && npm run check)
(cd frontend && npm run lint)
```

Expected: PASS with shared-surface runtime states and interaction surfaces aligned to the same visual language as built-in UI.

- [ ] **Step 3: Refresh settings/surfaces parity**

Only run this step after `2026-04-17-ui-design-language-interaction-contract-and-modal-convergence.md` Task 5 has landed in the same branch or is
already merged below this work.

Run:

```bash
(cd frontend && npm run test:e2e -- --grep "settings|surface\\.page|settings\\.tabs|settings\\.below\\.global|provider selector|ui parity")
(cd frontend && npm run test:e2e -- --grep "settings|surface\\.page|settings\\.tabs|settings\\.below\\.global|provider selector|ui parity" --update-snapshots)
(cd frontend && npm run test:e2e -- --grep "settings|surface\\.page|settings\\.tabs|settings\\.below\\.global|provider selector|ui parity")
```

Expected: PASS with updated settings-tab and surface-page parity baselines in both themes for built-in settings tab versus `settings.tabs`, built-in
inline settings card versus `settings.below.global`, standard form field row versus targeted-surface provider selector, built-in top-level nav item
versus `surface.page` nav item, and built-in page shell/body versus `surface.page` shell/body across the required runtime states.

- [ ] **Step 4: Commit**

```bash
git add 'frontend/src/routes/surfaces/[id]/+page.svelte' frontend/src/lib/components/surfaces/SurfaceReadPanel.svelte frontend/src/lib/components/surfaces/SurfaceRenderer.svelte frontend/src/lib/components/surfaces/SchemaForm.svelte frontend/src/lib/components/surfaces/SurfaceKeyValue.svelte frontend/src/lib/components/surfaces/SurfaceModal.svelte frontend/src/lib/components/surfaces/SurfaceWorkflow.svelte frontend/src/lib/components/surfaces/SurfaceActionBar.svelte frontend/src/lib/components/surfaces/SurfaceSlot.svelte frontend/src/lib/components/surfaces/SurfaceTable.svelte frontend/src/routes/settings/surface-tabs.test.ts frontend/src/routes/settings/settings-panels.test.ts frontend/src/routes/surfaces/surfaces-page.test.ts frontend/src/lib/components/surfaces/SurfaceReadPanel.test.ts frontend/src/lib/components/surfaces/SurfaceRenderer.test.ts frontend/src/lib/components/surfaces/SchemaForm.test.ts frontend/src/lib/components/surfaces/SurfaceKeyValue.test.ts frontend/src/lib/components/surfaces/SurfaceForm.test.ts frontend/src/lib/components/surfaces/SurfaceInteractionButton.test.ts frontend/src/lib/components/surfaces/SurfaceModal.test.ts frontend/src/lib/components/surfaces/SurfaceWorkflow.test.ts frontend/src/lib/components/surfaces/SurfaceTable.test.ts frontend/src/lib/components/surfaces/SurfaceActionBar.test.ts frontend/src/lib/components/surfaces/SurfaceSlot.test.ts frontend/tests/e2e/ui-parity.test.ts
if [ -d frontend/tests/e2e/ui-parity.test.ts-snapshots ]; then git add frontend/tests/e2e/ui-parity.test.ts-snapshots; fi
git commit -m "refactor: align settings surfaces with design language"
```

---

### Task 3: Refresh Design-Language Governance Docs

**Files:**

- Modify: `docs/development/frontend-components.md`
- Modify: `docs/development/ui-design-language.md`
- Modify if needed: `frontend/tests/e2e/ui-parity-waivers.json`

- [ ] **Step 1: Update docs to match the new settings/surface contract**

Document:

- final built-in settings layout rules
- final surface runtime-state treatment
- any remaining intentional parity waivers, with explicit `scope`, `owner`, `expiry_date`, `capture_region`, `justification`, and `review_ref`

- [ ] **Step 2: Verify docs**

Run:

```bash
if [ -f frontend/tests/e2e/ui-parity-waivers.json ]; then \
  jq empty frontend/tests/e2e/ui-parity-waivers.json && \
  jq -e 'type == "array" and all(.[]; (.scope | type == "string") and (.owner | type == "string") and (.expiry_date | type == "string" and test("^[0-9]{4}-[0-9]{2}-[0-9]{2}$")) and (.capture_region | type == "string") and (.justification | type == "string") and (.review_ref | type == "string"))' frontend/tests/e2e/ui-parity-waivers.json >/dev/null; \
fi && \
markdownlint --config .markdownlint.json docs/development/frontend-components.md docs/development/ui-design-language.md docs/development/README.md docs/README.md
```

Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add docs/development/frontend-components.md docs/development/ui-design-language.md
if [ -f frontend/tests/e2e/ui-parity-waivers.json ]; then git add frontend/tests/e2e/ui-parity-waivers.json; fi
git commit -m "docs: refresh settings and surfaces design guidance"
```

<!-- markdownlint-disable MD013 -->

# UI Design Language Interaction Contract And Modal Convergence Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement
> this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the remaining cross-cutting spec-versus-reality gaps that are not cleanly owned by route-family or settings-only work: transition
rules, focus-visible behavior, z-index scale, route-owned software modals and wizard flows, and the full canonical terminal modal shell from
Section 6.

**Architecture:** Treat these gaps as one convergence layer. First normalize the shared interaction contract in CSS and shared components so hover,
focus, layering, and disabled behavior are spec-compliant across the app. Then align the built-in Software route modal and wizard stack so it uses the
same modal, form-validation, loading, destructive-action, and workflow conventions as the rest of the design language. Finish by converging
`TerminalOutput.svelte` and its History/Software Detail integrations onto the canonical Section 6 modal shell, with explicit chrome and responsive
behavior rather than route-local approximations. This plan is the explicit owner for shared terminal-shell implementation and the built-in Software
modal or wizard components; route-family and shell plans should depend on these outcomes instead of editing the same shared files in parallel.

**Tech Stack:** SvelteKit, shared UI components, app-level CSS tokens, Xterm.js, Vitest, Playwright, Markdown docs.

**Execution Context:** Run commands from the repository root. Use `docs/development/web-ui-inventory.md`, `docs/development/ui-design-language.md`,
and `docs/superpowers/specs/2026-04-16-ui-design-language-design.md` as the source of truth. Serialize any task that edits
`frontend/tests/e2e/ui-parity.test.ts`, `frontend/tests/e2e/ui-parity-responsive.test.ts`, `docs/development/frontend-components.md`, or their
snapshots with the other active UI plans. Land Task 1 before `2026-04-17-ui-design-language-shell-and-entry-flows.md` Task 1 if both plans need to
touch `frontend/src/routes/+layout.svelte`. Do not run Task 2 or Task 3 in parallel with `2026-04-17-ui-design-language-route-family-alignment.md`,
because both plans touch `frontend/src/routes/history/+page.svelte`, `frontend/src/routes/software/[id]/+page.svelte`, and related route tests. Land
shared interaction-contract work before route-specific consumers whenever possible.

---

## File Map

| File                                                          | Change                                                                                                                                |
| ------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------- |
| `frontend/src/app.css`                                        | Normalize global focus-visible behavior, transition allowances, and shared layering tokens                                            |
| `frontend/src/theme/adapter-manifest.json`                    | Verify or update canonical token-to-runtime mappings if shared token names or mappings change                                         |
| `frontend/src/routes/+layout.svelte`                          | Align shell stacking and hover-motion usage with the shared interaction contract if shared CSS alone is insufficient                  |
| `frontend/src/lib/components/Modal.svelte`                    | Align the shared `ModalShell` wrapper dimensions, density, and footer treatment to the design-language modal contract                 |
| `frontend/src/lib/components/ModalBackdrop.svelte`            | Align modal backdrop layering with the approved z-index scale                                                                         |
| `frontend/src/lib/components/ContextMenu.svelte`              | Align menu layering and hover contract with the shared interaction rules                                                              |
| `frontend/src/lib/components/ui/ActionBadge.svelte`           | Recheck disabled, hover, and fixed-width hover-label behavior against Section 8                                                       |
| `frontend/src/lib/components/ui/TabStrip.svelte`              | Recheck focus-visible, hover, and disabled treatment against the shared interaction contract                                          |
| `frontend/src/lib/components/AddSoftwareModal.svelte`         | Align add-software modal with the shared modal, form, and validation contract used by the rest of the Software route stack            |
| `frontend/src/lib/components/AssignToHostModal.svelte`        | Align Software route assign-host modal with shared modal, form, loading, and validation rules                                         |
| `frontend/src/lib/components/EditHostAssignmentModal.svelte`  | Align assignment edit modal with shared form, toggle, error, and destructive-action treatment                                         |
| `frontend/src/lib/components/SoftwareMergeWizard.svelte`      | Align built-in merge wizard with the shared workflow-shell contract                                                                   |
| `frontend/src/lib/components/AddSoftwareModal.test.ts`        | Create or extend add-software modal regressions if focused coverage is missing                                                        |
| `frontend/src/lib/components/AssignToHostModal.test.ts`       | Create or extend assign-host modal regressions if focused coverage is missing                                                         |
| `frontend/src/lib/components/EditHostAssignmentModal.test.ts` | Create or extend assignment-edit modal regressions if focused coverage is missing                                                     |
| `frontend/src/lib/components/SoftwareMergeWizard.test.ts`     | Preserve merge-wizard workflow-shell alignment                                                                                        |
| `frontend/src/lib/components/TerminalOutput.svelte`           | Converge the terminal experience onto the canonical modal shell from Section 6                                                        |
| `frontend/src/routes/history/+page.svelte`                    | Replace route-local inline-first terminal UX with the shared terminal shell integration                                               |
| `frontend/src/routes/software/[id]/+page.svelte`              | Consume the shared terminal shell and align Software Detail trigger behavior                                                          |
| `frontend/tests/e2e/`                                         | Own deterministic parity harness details if new helper files or capture-profile config are needed for enforced thresholds and masking |
| `frontend/src/lib/theme/adapter-manifest.test.ts`             | Preserve full canonical token coverage in the adapter-manifest CI gate                                                                |
| `frontend/src/lib/components/*.test.ts`                       | Add or extend shared interaction, modal, wizard, and terminal regressions                                                             |
| `frontend/src/routes/history/*.test.ts`                       | Preserve History terminal-launch behavior and shared-shell usage                                                                      |
| `frontend/src/routes/software/[id]/*.test.ts`                 | Preserve Software Detail terminal-launch behavior and modal-stack convergence                                                         |
| `frontend/tests/e2e/ui-parity.test.ts`                        | Add or refresh deterministic parity for interaction and terminal-shell chrome                                                         |
| `frontend/tests/e2e/ui-parity-responsive.test.ts`             | Add or refresh responsive terminal-shell parity where Section 6 requires it                                                           |
| `frontend/tests/e2e/parity-config.ts`                         | Modify or create checked-in parity helper/config for deterministic capture and masking thresholds if needed                           |
| `docs/development/frontend-components.md`                     | Refresh docs for the shared interaction contract, built-in modal/wizard stack, and terminal shell                                     |
| `docs/development/web-ui-inventory.md`                        | Update inventory notes for modal, wizard, and terminal UX ownership                                                                   |

---

### Task 0: Record The Remaining Interaction And Modal Gap Checklist

**Files:**

- Verify against: `docs/development/web-ui-inventory.md`
- Verify against: `docs/development/ui-design-language.md`
- Verify against: `docs/superpowers/specs/2026-04-16-ui-design-language-design.md`

- [ ] **Step 1: Enumerate every remaining cross-cutting discrepancy before implementation**

Build a checklist for all remaining gaps in this plan scope, including:

- transition rules from Section 2.5
- focus-visible rules from Section 2.6
- z-index scale from Section 2.7
- no-layout-reflow hover rules and disabled-state conventions from Section 8
- Software route assign, edit, and merge modal or wizard drift
- terminal-shell subsection drift from Section 6, including chrome, maximize, status bar, and responsive behavior

Expected: every remaining discrepancy in these cross-cutting areas maps to one of the tasks below before implementation starts.

---

### Task 1: Normalize The Shared Interaction Contract

**Files:**

- Modify: `frontend/src/app.css`
- Modify if needed: `frontend/src/theme/adapter-manifest.json`
- Modify if needed: `frontend/src/routes/+layout.svelte`
- Modify if needed: `frontend/src/lib/components/Modal.svelte`
- Modify if needed: `frontend/src/lib/components/ModalBackdrop.svelte`
- Modify if needed: `frontend/src/lib/components/ContextMenu.svelte`
- Modify if needed: `frontend/src/lib/components/ui/ActionBadge.svelte`
- Modify if needed: `frontend/src/lib/components/ui/TabStrip.svelte`
- Modify or create focused shared tests as needed

- [ ] **Step 1: Write the failing interaction-contract regressions**

Add focused tests or assertions so they cover:

- `:focus-visible` ring behavior on keyboard-focusable controls without relying on default browser outlines
- error-state fields keeping their error border while also receiving the accent focus ring
- allowed transition properties only (`background`, `border-color`, `color`) for ordinary interactive controls
- full adapter-manifest completeness for all canonical Sections 2.1 and 2.2 tokens via the checked-in manifest test, not a single-token spot-check
- fixed-width hover treatment for badge-style controls so hover labels do not cause layout reflow
- shared disabled treatment using dimming rather than visibility removal
- z-index ordering for shell, toast, dropdown, modal backdrop, and modal content layers

Run:

```bash
(cd frontend && npm run test -- src/lib/components/ui/ActionBadge.test.ts src/lib/components/ContextMenu.test.ts src/lib/components/ui/TabStrip.test.ts src/lib/components/Modal.test.ts src/lib/components/ModalBackdrop.test.ts src/lib/theme/adapter-manifest.test.ts)
(cd frontend && npm run check)
```

Expected: FAIL where current shared behavior still allows forbidden motion, missing focus-visible styling, or off-spec layering.

- [ ] **Step 2: Implement the shared interaction contract**

Bring the shared interaction layer into contract so:

- ordinary interactive controls use only the Section 2.5 transition allowances
- keyboard focus uses the spec `:focus-visible` ring with `--accent-rgb`
- disabled controls stay visible but dim, with `pointer-events: none` where required
- hover label swaps for badge-style controls happen inside fixed-width containers with no layout reflow
- shell, dropdown, toast, backdrop, and modal content stack to the approved z-index scale

Run:

```bash
(cd frontend && npm run test -- src/lib/components/ui/ActionBadge.test.ts src/lib/components/ContextMenu.test.ts src/lib/components/ui/TabStrip.test.ts src/lib/components/Modal.test.ts src/lib/components/ModalBackdrop.test.ts src/lib/theme/adapter-manifest.test.ts)
(cd frontend && npm run check)
(cd frontend && npm run lint)
```

Expected: PASS with the shared interaction contract matching Sections 2.5, 2.6, 2.7, and 8.

- [ ] **Step 3: Commit**

```bash
git add frontend/src/app.css frontend/src/theme/adapter-manifest.json frontend/src/routes/+layout.svelte frontend/src/lib/components/Modal.svelte frontend/src/lib/components/Modal.test.ts frontend/src/lib/components/ModalBackdrop.svelte frontend/src/lib/components/ModalBackdrop.test.ts frontend/src/lib/components/ContextMenu.svelte frontend/src/lib/components/ContextMenu.test.ts frontend/src/lib/components/ui/ActionBadge.svelte frontend/src/lib/components/ui/ActionBadge.test.ts frontend/src/lib/components/ui/TabStrip.svelte frontend/src/lib/components/ui/TabStrip.test.ts frontend/src/lib/theme/adapter-manifest.test.ts
git commit -m "refactor: normalize shared ui interaction contract"
```

---

### Task 2: Align The Software Route Modal And Wizard Stack

**Files:**

- Modify: `frontend/src/lib/components/AssignToHostModal.svelte`
- Modify: `frontend/src/lib/components/AddSoftwareModal.svelte`
- Modify: `frontend/src/lib/components/EditHostAssignmentModal.svelte`
- Modify: `frontend/src/lib/components/SoftwareMergeWizard.svelte`
- Create or modify if needed: `frontend/src/lib/components/AddSoftwareModal.test.ts`
- Create or modify if needed: `frontend/src/lib/components/AssignToHostModal.test.ts`
- Create or modify if needed: `frontend/src/lib/components/EditHostAssignmentModal.test.ts`
- Modify: `frontend/src/lib/components/SoftwareMergeWizard.test.ts`
- Modify if needed: `frontend/src/routes/software/+page.svelte`
- Modify if needed: `frontend/src/routes/software/[id]/+page.svelte`
- Create or modify focused tests for the affected components and route integrations

- [ ] **Step 1: Write the failing modal and wizard regressions**

Add or extend tests so they cover:

- add-software modal field rhythm, loading states, and inline validation
- assign-host modal field rhythm, loading states, empty states, and inline validation
- edit-assignment modal error handling, toggle treatment, and destructive confirmation flow
- merge wizard step-shell, loading preview state, and action ordering
- shared confirm-dialog usage and button hierarchy in all three flows

Run:

```bash
(cd frontend && npm run test -- src/lib/components/AddSoftwareModal.test.ts src/lib/components/AssignToHostModal.test.ts src/lib/components/EditHostAssignmentModal.test.ts src/lib/components/SoftwareMergeWizard.test.ts src/routes/software/software-trigger-status.test.ts 'src/routes/software/[id]/software-detail.test.ts')
```

Expected: FAIL or missing coverage where these Software route-owned overlays still use bespoke shells or validation treatment.

- [ ] **Step 2: Implement modal and wizard convergence**

Bring the built-in Software route overlay stack into the shared design language so:

- `AddSoftwareModal` uses the shared field, validation, and loading-state contract
- `AssignToHostModal` uses the shared field, validation, loading, and empty-state contract
- `EditHostAssignmentModal` uses the same error, toggle, callout, and destructive-action hierarchy as the shared design language
- `SoftwareMergeWizard` uses the shared workflow-shell language rather than a route-local stepper style
- route integrations in Software list and Software Detail use the same launch semantics and visual hierarchy

Run:

```bash
(cd frontend && npm run test -- src/lib/components/AddSoftwareModal.test.ts src/lib/components/AssignToHostModal.test.ts src/lib/components/EditHostAssignmentModal.test.ts src/lib/components/SoftwareMergeWizard.test.ts src/routes/software/software-trigger-status.test.ts 'src/routes/software/[id]/software-detail.test.ts')
(cd frontend && npm run check)
(cd frontend && npm run lint)
```

Expected: PASS with the Software route’s assign, edit, and merge overlays visually native to the same modal and workflow system as the rest of the
app.

- [ ] **Step 3: Commit**

```bash
git add frontend/src/lib/components/AddSoftwareModal.svelte frontend/src/lib/components/AssignToHostModal.svelte frontend/src/lib/components/EditHostAssignmentModal.svelte frontend/src/lib/components/SoftwareMergeWizard.svelte frontend/src/lib/components/AddSoftwareModal.test.ts frontend/src/lib/components/AssignToHostModal.test.ts frontend/src/lib/components/EditHostAssignmentModal.test.ts frontend/src/lib/components/SoftwareMergeWizard.test.ts frontend/src/routes/software/+page.svelte 'frontend/src/routes/software/[id]/+page.svelte'
git commit -m "refactor: align software modal flows with design language"
```

---

### Task 3: Converge The Canonical Terminal Shell

**Files:**

- Modify: `frontend/src/lib/components/TerminalOutput.svelte`
- Modify: `frontend/src/routes/history/+page.svelte`
- Modify: `frontend/src/routes/software/[id]/+page.svelte`
- Modify associated terminal-focused tests
- Modify: `frontend/tests/e2e/ui-parity.test.ts`
- Modify if needed: `frontend/tests/e2e/ui-parity-responsive.test.ts`

- [ ] **Step 1: Write the failing terminal-shell regressions**

Add or extend tests so they cover:

- centered modal shell over the darker terminal backdrop
- titlebar with macOS-style traffic lights
- close-path behavior for red-dot close, `Escape`, and backdrop click
- yellow-dot disabled behavior
- hover-any-dot behavior revealing all three glyph icons
- maximize and restore behavior, including reset-on-close
- terminal status bar layout and metadata rhythm
- shared terminal-shell use by both History and Software Detail
- mobile full-screen terminal behavior and tablet sizing rules

Run:

```bash
(cd frontend && npm run test -- src/routes/history/history.test.ts src/routes/history/history-trigger-status.test.ts 'src/routes/software/[id]/software-detail-update-trigger.test.ts' 'src/routes/software/[id]/software-detail.test.ts' src/lib/components/TerminalOutput.test.ts)
```

Expected: FAIL where the current `SectionCard`-style terminal or route-local expansion behavior still diverges from Section 6.

- [ ] **Step 2: Implement the canonical Section 6 terminal shell**

Converge `TerminalOutput.svelte` and its route integrations so:

- the terminal opens as the canonical modal shell with the approved backdrop, size rules, and titlebar
- red, yellow, and green traffic lights behave exactly as specified
- maximize or restore state uses the approved size, radius, and reset-on-close behavior
- the status bar shows the approved badge plus metadata layout
- History and Software Detail use the same shared terminal-shell component and launch pattern
- inline-only terminal UX is removed as the primary interaction path; any residual non-interactive inline transcript must use the same approved
  inner-body styling as the modal shell and must not preserve a route-local terminal shell

Run:

```bash
(cd frontend && npm run test -- src/routes/history/history.test.ts src/routes/history/history-trigger-status.test.ts 'src/routes/software/[id]/software-detail-update-trigger.test.ts' 'src/routes/software/[id]/software-detail.test.ts' src/lib/components/TerminalOutput.test.ts)
(cd frontend && npm run check)
(cd frontend && npm run lint)
```

Expected: PASS with Section 6 implemented, including shared-shell usage in both route families.

- [ ] **Step 3: Refresh terminal parity coverage**

Skip this step until Task 5 in this same plan has landed. After Task 5 is complete, return here and capture the final terminal baselines under the
governed parity harness. Before committing, verify the explicit Section 6 exit criteria are all satisfied: shared terminal-shell component in both
History and Software Detail, removal of legacy route-local shell styling, and parity captures at `<= 0.5%` diff with the approved capture regions.

Run:

```bash
(cd frontend && npm run test:e2e -- --grep "terminal|history|software detail|ui parity")
(cd frontend && npm run test:e2e -- --grep "terminal|history|software detail|ui parity" --update-snapshots)
(cd frontend && npm run test:e2e -- --grep "terminal|history|software detail|ui parity")
```

Expected: PASS with terminal-shell chrome snapshots in both themes, responsive terminal-shell baselines where Section 6 requires them, and parity
capture regions limited to the titlebar/frame/status-bar chrome unless a waiver explicitly narrows or expands the capture region for the terminal
body.

- [ ] **Step 4: Commit**

```bash
git add frontend/src/lib/components/TerminalOutput.svelte frontend/src/lib/components/TerminalOutput.test.ts frontend/src/routes/history/+page.svelte frontend/src/routes/history/history.test.ts frontend/src/routes/history/history-trigger-status.test.ts 'frontend/src/routes/software/[id]/+page.svelte' 'frontend/src/routes/software/[id]/software-detail-update-trigger.test.ts' 'frontend/src/routes/software/[id]/software-detail.test.ts' frontend/tests/e2e/ui-parity.test.ts frontend/tests/e2e/ui-parity-responsive.test.ts
if [ -d frontend/tests/e2e/ui-parity.test.ts-snapshots ]; then git add frontend/tests/e2e/ui-parity.test.ts-snapshots; fi
if [ -d frontend/tests/e2e/ui-parity-responsive.test.ts-snapshots ]; then git add frontend/tests/e2e/ui-parity-responsive.test.ts-snapshots; fi
git commit -m "refactor: converge terminal shell with design language"
```

If Step 3 was deferred until after Task 5, include the resulting terminal parity snapshot directories in Task 5 Step 3 instead of creating an extra
commit.

---

### Task 4: Update Docs For The Converged Interaction Layer

**Files:**

- Modify: `docs/development/frontend-components.md`
- Modify: `docs/development/web-ui-inventory.md`
- Modify if needed: `docs/development/ui-design-language.md`

- [ ] **Step 1: Update docs to reflect the shipped interaction contract**

Document:

- the shared focus-visible and disabled-state rules now enforced in code
- the built-in Software route modal and wizard stack ownership
- the final canonical terminal-shell behavior and route integration points

- [ ] **Step 2: Verify docs**

Run:

```bash
markdownlint --config .markdownlint.json docs/development/frontend-components.md docs/development/web-ui-inventory.md docs/development/ui-design-language.md docs/development/README.md docs/README.md
```

Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add docs/development/frontend-components.md docs/development/web-ui-inventory.md docs/development/ui-design-language.md
git commit -m "docs: document interaction and terminal convergence"
```

---

### Task 5: Enforce The Parity-Gate Mechanics Required By The Spec

**Files:**

- Modify: `frontend/tests/e2e/ui-parity.test.ts`
- Modify if needed: `frontend/tests/e2e/ui-parity-responsive.test.ts`
- Modify or create if needed: `frontend/tests/e2e/parity-config.ts`
- Modify if needed: `docs/development/ui-design-language.md`

- [ ] **Step 1: Write the failing parity-governance checks**

Add or document checks so the parity suite enforces:

- `<= 0.5%` visual-diff failure threshold after approved masking
- approved masking only through checked-in selector lists or `data-visual-dynamic` markers
- `<= 15%` maximum masked area per snapshot unless narrowed by waiver
- deterministic capture profile requirements: pinned browser channel, fixed DPR, fixed viewport presets, locked locale/timezone, fixed font package,
  and reduced motion

Run:

```bash
(cd frontend && npm run test:e2e -- --grep "ui parity")
```

Expected: the parity suite or its helper/config layer clearly fails when these constraints are violated instead of relying on informal reviewer
judgment.

- [ ] **Step 2: Implement the parity-governance mechanics**

Bring the test harness into contract so:

- threshold and masking limits are encoded in the parity workflow rather than left implicit
- deterministic capture settings are owned in checked-in test code or helper config
- if helper config is needed, keep it in a checked-in file such as `frontend/tests/e2e/parity-config.ts`
- dynamic regions use the approved selector-marker mechanism
- any wider dynamic region still requires a waiver-linked capture-region narrowing path

Run:

```bash
(cd frontend && npm run test:e2e -- --grep "ui parity")
(cd frontend && npm run test:e2e -- --grep "ui parity" --update-snapshots)
(cd frontend && npm run test:e2e -- --grep "ui parity")
```

Expected: PASS with the required mismatch, masking, and deterministic-capture mechanics enforced by the parity suite itself.

- [ ] **Step 3: Commit**

```bash
git add frontend/tests/e2e/ui-parity.test.ts frontend/tests/e2e/ui-parity-responsive.test.ts
if [ -f frontend/tests/e2e/parity-config.ts ]; then git add frontend/tests/e2e/parity-config.ts; fi
if [ -d frontend/tests/e2e/ui-parity.test.ts-snapshots ]; then git add frontend/tests/e2e/ui-parity.test.ts-snapshots; fi
if [ -d frontend/tests/e2e/ui-parity-responsive.test.ts-snapshots ]; then git add frontend/tests/e2e/ui-parity-responsive.test.ts-snapshots; fi
git add docs/development/ui-design-language.md
git commit -m "test: enforce ui parity gate mechanics"
```

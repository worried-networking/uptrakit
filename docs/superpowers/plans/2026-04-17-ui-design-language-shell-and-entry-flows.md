<!-- markdownlint-disable MD013 -->

# UI Design Language Shell And Entry Flows Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close every remaining spec-versus-reality gap in the global app shell, public entry flows, and shell-owned feedback surfaces so navigation, auth entry, and global overlays match the approved UI design language with no undocumented divergence left in this scope.

**Architecture:** Treat the shared shell and public entry flows as one subsystem. Normalize the authenticated layout chrome first, then move the public routes (`/login`, `/register`, `/device`, `+error`) onto the same design-language primitives or a deliberately shared public-shell variant. Finish by tightening global feedback elements such as banners, loading or empty states owned by the shell, toasts, and terminal chrome so route-level redesign work inherits a correct shell. This plan owns the remaining gaps for spec Sections 3, 4.6, 4.8, 7, shell-owned parts of Section 8, and the public-entry slice of Section 4.10 form validation.

**Tech Stack:** SvelteKit routes, Svelte 5 shared UI primitives, shared auth/theme/network stores, Vitest, Playwright, Markdown docs.

**Execution Context:** Run commands from the repository root. Use the inventory at `docs/development/web-ui-inventory.md`, the implementation guide at `docs/development/ui-design-language.md`, and the approved spec at `docs/superpowers/specs/2026-04-16-ui-design-language-design.md` while implementing this plan. Treat removal of the legacy persistent-sidebar-only mobile shell as explicit Task 1 scope, not as an informal precondition. Do not start Task 1 until `2026-04-17-ui-design-language-interaction-contract-and-modal-convergence.md` Task 1 has landed if both plans need to touch `frontend/src/routes/+layout.svelte`. Do not refresh responsive or shell parity snapshots until `2026-04-17-ui-design-language-interaction-contract-and-modal-convergence.md` Task 5 has landed, because that plan owns the parity-gate mechanics. Serialize any task that edits `frontend/src/routes/home.test.ts`, `frontend/src/routes/surfaces/surfaces-page.test.ts`, `frontend/tests/e2e/ui-parity.test.ts`, `frontend/tests/e2e/ui-parity-responsive.test.ts`, `docs/development/frontend-components.md`, or their snapshots with the other active UI plans.

---

## File Map

| File | Change |
| --- | --- |
| `frontend/src/routes/+layout.svelte` | Align desktop/tablet/mobile shell chrome, nav treatment, banners, and responsive spacing with the approved shell contract |
| `frontend/src/routes/+error.svelte` | Replace bare error page with design-language-compliant public error presentation |
| `frontend/src/routes/login/+page.svelte` | Migrate login, OIDC, linking, and first-user setup states onto shared public-entry styling |
| `frontend/src/routes/register/+page.svelte` | Align registration page with the public-entry shell and shared field/action rhythm |
| `frontend/src/routes/device/+page.svelte` | Align device-auth approval page with the public-entry shell and semantic callouts |
| `frontend/src/lib/components/ToastNotifications.svelte` | Re-check spacing, placement, and tone treatment against the design-language guide and spec |
| `frontend/src/lib/components/ToastNotifications.test.ts` | Lock the full toast contract, not just shell placement |
| `frontend/src/routes/home.test.ts` | Add shell-level assertions that the home route still renders inside the shared shell after shell changes |
| `frontend/src/routes/public-entry.test.ts` | Create or extend focused route-level regressions for login, register, device, and public error shell treatment |
| `frontend/src/routes/surfaces/surfaces-page.test.ts` | Preserve built-in page-shell versus `surface.page` shell parity states touched by shell work |
| `frontend/tests/e2e/auth.test.ts` | Add or refresh Playwright coverage for login, register, device, and public-entry redirects |
| `frontend/tests/e2e/public-entry.test.ts` | Add focused Playwright coverage for public entry shell framing and public error presentation if the auth suite is not the right fit |
| `frontend/tests/e2e/ui-parity.test.ts` | Refresh built-in versus `surface.page` nav and shell/body parity coverage in both themes |
| `frontend/tests/e2e/ui-parity-responsive.test.ts` | Add or refresh shell and entry-flow responsive parity coverage |
| `docs/development/web-ui-inventory.md` | Update inventory notes if implementation changes page-shell/public-shell ownership |
| `docs/development/frontend-components.md` | Document any new shared public-shell primitive if one is introduced |

---

### Task 0: Record The Remaining Shell And Entry Gap Checklist

**Files:**

- Verify against: `docs/development/web-ui-inventory.md`
- Verify against: `docs/development/ui-design-language.md`
- Verify against: `docs/superpowers/specs/2026-04-16-ui-design-language-design.md`

- [ ] **Step 1: Enumerate every in-scope discrepancy before implementation**

Build a checklist for all shell and public-entry gaps still open in this scope, including:

- sidebar, top-bar, and content-area measurements
- built-in and `surface.page` nav ordering, typography, and active states
- shell-owned loading, empty, banner, and toast presentation
- public-entry title rhythm, callouts, field spacing, and error framing
- public-entry validation, inline error presentation, and keyboard-focus treatment
- responsive shell behavior for tablet, mobile bottom-nav, and overflow sheet
- any shell-owned interaction-convention drift from Section 8

Expected: a worker can point to an owning task for every remaining shell or public-entry discrepancy before implementation starts; nothing in this scope is left as an implicit “follow-up”.

---

### Task 1: Align The Authenticated App Shell

**Files:**

- Modify: `frontend/src/routes/+layout.svelte`
- Modify if needed: `frontend/src/lib/components/ToastNotifications.svelte`
- Modify: `frontend/src/lib/components/ToastNotifications.test.ts`
- Modify: `frontend/tests/e2e/ui-parity-responsive.test.ts`
- Modify: `frontend/tests/e2e/ui-parity.test.ts`
- Modify: `frontend/src/routes/home.test.ts`
- Modify: `frontend/src/routes/surfaces/surfaces-page.test.ts`

- [ ] **Step 1: Write the failing shell regressions**

Add coverage for:

- desktop sidebar spacing and active-link treatment
- nav sorting tie-breakers when built-in and `surface.page` entries share `priority` and `label`, including origin-order fallback and the final stable-ID fallback (`href` for built-ins, `surface_id` for surfaces)
- tablet overlay and mobile overflow behavior
- banner and toast placement relative to the shared shell
- desktop and tablet toast dismissal semantics, timeout durations, hover pause or resume, and progress-bar countdown
- mobile toast position and swipe-dismiss direction only as part of the full Section 7 shell rollout in this task, not as a standalone pre-shell assertion
- shell persistence around a `PageShell` route such as `/`
- built-in top-level nav item versus `surface.page` nav item treatment
- built-in page shell/body versus `surface.page` shell/body in loaded, `permission_denied`, targeted `no_compatible_provider`, `contract_mismatch`, and `hydration_action_failure` states

Run:

```bash
(cd frontend && npm run test -- src/routes/home.test.ts src/routes/surfaces/surfaces-page.test.ts src/lib/components/ToastNotifications.test.ts)
(cd frontend && npm run test:e2e -- --grep "responsive|shell|navigation|surface\\.page|ui parity")
```

Expected: FAIL where shell metrics, nav typography, or feedback placement still drift from the spec.

- [ ] **Step 2: Implement the shell alignment**

Update `frontend/src/routes/+layout.svelte` so:

- desktop, tablet, and mobile nav all use the same approved typography, active treatment, and spacing contract
- header, banners, and main content gutters match the shared-shell measurements from the spec and `ui-design-language.md`
- mobile bottom-nav and overflow sheet look like the same design language as desktop/tablet nav
- the legacy persistent-sidebar-only mobile shell is removed rather than left as a hidden fallback
- shell-owned banners and toasts remain visually subordinate to route content but still native to the same token/component system

If `ToastNotifications.svelte` still drifts after shell changes, bring it back into contract in the same task. Terminal shell ownership belongs to `2026-04-17-ui-design-language-interaction-contract-and-modal-convergence.md`; do not edit `TerminalOutput.svelte` here.

Run:

```bash
(cd frontend && npm run test -- src/routes/home.test.ts src/routes/surfaces/surfaces-page.test.ts src/lib/components/ToastNotifications.test.ts)
(cd frontend && npm run check)
(cd frontend && npm run lint)
```

Expected: PASS with the global shell and shell-owned feedback surfaces aligned to the shared design language.

- [ ] **Step 3: Refresh responsive shell parity**

Only run this step after `2026-04-17-ui-design-language-interaction-contract-and-modal-convergence.md` Task 5 has landed in the same branch or is already merged below this work.

Run:

```bash
(cd frontend && npm run test:e2e -- --grep "responsive|shell|navigation|surface\\.page|ui parity")
(cd frontend && npm run test:e2e -- --grep "responsive|shell|navigation|surface\\.page|ui parity" --update-snapshots)
(cd frontend && npm run test:e2e -- --grep "responsive|shell|navigation|surface\\.page|ui parity")
```

Expected: PASS with updated desktop/tablet/mobile shell baselines plus built-in versus `surface.page` nav and page-shell parity snapshots in both themes.

- [ ] **Step 4: Commit**

```bash
git add frontend/src/routes/+layout.svelte frontend/src/lib/components/ToastNotifications.svelte frontend/src/lib/components/ToastNotifications.test.ts frontend/src/routes/home.test.ts frontend/src/routes/surfaces/surfaces-page.test.ts frontend/tests/e2e/ui-parity.test.ts frontend/tests/e2e/ui-parity-responsive.test.ts
if [ -d frontend/tests/e2e/ui-parity.test.ts-snapshots ]; then git add frontend/tests/e2e/ui-parity.test.ts-snapshots; fi
if [ -d frontend/tests/e2e/ui-parity-responsive.test.ts-snapshots ]; then git add frontend/tests/e2e/ui-parity-responsive.test.ts-snapshots; fi
git commit -m "refactor: align app shell with design language"
```

---

### Task 2: Align Public Entry Flows

**Files:**

- Modify: `frontend/src/routes/login/+page.svelte`
- Modify: `frontend/src/routes/register/+page.svelte`
- Modify: `frontend/src/routes/device/+page.svelte`
- Modify: `frontend/src/routes/+error.svelte`
- Create or modify if needed: `frontend/src/lib/components/ui/PublicEntryShell.svelte`
- Create or modify: `frontend/src/routes/public-entry.test.ts`

- [ ] **Step 1: Write the failing public-entry regressions**

Add focused tests or extend existing route tests so they assert:

- shared title/subtitle rhythm and semantic callout usage
- consistent field spacing and button treatment across login/register/device
- inline validation, required-field messaging, invalid-code treatment, and `:focus-visible` behavior across login/register/device
- error page use of shared public-entry framing instead of a bare text dump

Run:

```bash
(cd frontend && npm run test -- src/routes/public-entry.test.ts)
```

Expected: FAIL or no coverage for the public-entry shell contract, confirming the gap.

- [ ] **Step 2: Move public routes onto a shared design-language shell**

Implement a shared public-entry shell if the existing `PageShell` is not the right fit for unauthenticated routes. Use one approach consistently across:

- `/login`
- `/register`
- `/device`
- `+error.svelte`

The result must keep the distinct product semantics of auth/device/error flows, but:

- use the same tokens, field rhythm, and action sizing as the main design language
- replace ad hoc `aside` presets with shared `Callout`-style semantics where appropriate
- use the approved inline validation, error-border, and focus-visible treatment rather than route-local auth-form presets
- eliminate route-specific card styling drift between login/register/device

Run:

```bash
(cd frontend && npm run test -- src/routes/public-entry.test.ts)
(cd frontend && npm run check)
(cd frontend && npm run lint)
```

Expected: PASS with public routes now sharing one consistent shell treatment.

- [ ] **Step 3: Refresh public-flow visual coverage**

Add or refresh Playwright coverage for `/login`, `/register`, `/device`, and the public error presentation. Use `frontend/tests/e2e/public-entry.test.ts` if `auth.test.ts` is too broad to express the page-shell assertions cleanly.

Run:

```bash
(cd frontend && npm run test:e2e -- auth.test.ts)
if [ -f frontend/tests/e2e/public-entry.test.ts ]; then (cd frontend && npm run test:e2e -- public-entry.test.ts); fi
```

Expected: PASS with public-entry coverage in place, including login/register/device flows and unauthenticated redirect behavior.

- [ ] **Step 4: Commit**

```bash
git add frontend/src/routes/login/+page.svelte frontend/src/routes/register/+page.svelte frontend/src/routes/device/+page.svelte frontend/src/routes/+error.svelte frontend/src/routes/public-entry.test.ts frontend/tests/e2e/auth.test.ts
if [ -f frontend/src/lib/components/ui/PublicEntryShell.svelte ]; then git add frontend/src/lib/components/ui/PublicEntryShell.svelte; fi
if [ -f frontend/tests/e2e/public-entry.test.ts ]; then git add frontend/tests/e2e/public-entry.test.ts; fi
if [ -d frontend/tests/e2e/auth.test.ts-snapshots ]; then git add frontend/tests/e2e/auth.test.ts-snapshots; fi
if [ -d frontend/tests/e2e/public-entry.test.ts-snapshots ]; then git add frontend/tests/e2e/public-entry.test.ts-snapshots; fi
git commit -m "refactor: align public entry flows with design language"
```

---

### Task 3: Update Documentation For The Final Shell Contract

**Files:**

- Modify: `docs/development/web-ui-inventory.md`
- Modify if needed: `docs/development/frontend-components.md`
- Modify if needed: `docs/development/ui-design-language.md`

- [ ] **Step 1: Update the docs to match the shipped shell**

Document:

- final ownership of authenticated vs public-entry shells
- shell navigation behavior by viewport
- any new shared public-shell primitive
- any intentionally preserved exception between public entry flows and authenticated routes

- [ ] **Step 2: Verify docs**

Run:

```bash
markdownlint --config .markdownlint.json docs/development/web-ui-inventory.md docs/development/frontend-components.md docs/development/ui-design-language.md docs/development/README.md docs/README.md
```

Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add docs/development/web-ui-inventory.md docs/development/frontend-components.md docs/development/ui-design-language.md
git commit -m "docs: document shell and entry flow design language"
```

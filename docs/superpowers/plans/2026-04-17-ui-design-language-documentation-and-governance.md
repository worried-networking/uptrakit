# UI Design Language Documentation And Governance Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Publish the approved UI design language as durable project
documentation, wire it into existing frontend and Surfaces development guides,
and make parity/waiver governance explicit and enforceable.

**Architecture:** Keep
`docs/superpowers/specs/2026-04-16-ui-design-language-design.md` as the
normative product spec, then add stable development-facing documentation that
translates the spec into day-to-day engineering guidance. Couple that
documentation to one checked-in waiver file and one updated quality-gates
document so the visual language is not just descriptive but operational.
This documentation track should land after the shared frontend foundation plan
has established the token adapter and shared primitive vocabulary, or it should
describe those artifacts explicitly as pending foundation deliverables rather
than already-shipped components.

**Tech Stack:** Markdown, JSON, markdownlint, existing `docs/development/` and `docs/README.md` documentation structure.

**Execution Context:** Run commands from the repository root.

---

## Task 0: Confirm Foundation Inputs Or Use Future-Tense Guidance

**Files:**

- Verify: `frontend/src/theme/adapter-manifest.json`
- Verify: `frontend/src/lib/components/ui/`

- [ ] **Step 1: Check whether the shared frontend foundation has landed**

Before documenting the shared component vocabulary as already available, check
whether the foundation plan has already created the token adapter and shared UI
directory.

Run:

```bash
test -f frontend/src/theme/adapter-manifest.json && test -d frontend/src/lib/components/ui
```

Expected: PASS if the documentation should describe the shared primitives as
present-day artifacts. If this check fails, keep the docs plan in future tense
for those artifacts until the foundation work lands.

---

## File Map

| File | Change |
| --- | --- |
| `docs/development/ui-design-language.md` | New stable developer guide for tokens, shell rules, primitives, parity model, and page-authoring expectations |
| `docs/development/frontend-components.md` | Link modal/component guidance to the new design-language guide and shared primitives |
| `docs/development/surfaces.md` | Align Surfaces authoring guidance with the parity rules and slot/runtime-state terminology |
| `docs/development/quality-gates.md` | Add visual parity gates, adapter-manifest expectations, and waiver-file process |
| `docs/development/README.md` | Add the new guide to the development docs catalogue |
| `docs/README.md` | Link the new guide from the top-level documentation catalogue |
| `docs/superpowers/ui-parity-waivers.json` | Seed checked-in waiver file required by the design-language spec |

---

### Task 1: Publish The Stable Developer-Facing Design Language Guide

**Files:**

- Create: `docs/development/ui-design-language.md`
- Modify: `docs/development/README.md`
- Modify: `docs/README.md`

- [ ] **Step 1: Create the durable guide skeleton**

Write `docs/development/ui-design-language.md` with this exact top-level outline:

```md
# UI Design Language

## Purpose
## Relationship To The Approved Spec
## Status Model (`Implemented` / `Transitional` / `Target`)
## Theme And Token Adapter Contract
## Shared Shell Rules
## Shared Components And Primitives
## Surface Parity Rules
## Page Authoring Checklist
## Verification And Waivers
```

The guide must clearly say the spec remains normative and this document is the developer-facing implementation guide.

Run:

```bash
markdownlint --config .markdownlint.json docs/development/ui-design-language.md
```

Expected: the new file passes markdownlint with the outline in place.

- [ ] **Step 2: Fill the guide with stable implementation guidance**

Populate the new guide with concrete content drawn from the approved spec, including:

- the semantic token families and adapter-manifest requirement
- the current slot registry and parity expectations for `surface.page`,
  `settings.tabs`, `settings.below.global`, `software.tabs`,
  `host_detail.tabs`, and `software_item.host_context_menu`
- the canonical shared primitives (`PageShell`, `SectionCard`, `TabStrip`,
  `EmptyState`, `Callout`, `StatusBadge`, `DataTable`, `ProviderSelector`,
  modal/workflow shells)
- one explicit page-authoring checklist for new routes and one explicit checklist for new surface-backed UI

Use a checklist block like:

```md
## Page Authoring Checklist

1. Start from the shared page-shell primitive, not route-local spacing.
2. Use semantic tokens through the adapter, never one-off color classes when an equivalent token exists.
3. Reuse shared tab, card, empty-state, callout, and modal primitives before creating route-local variants.
4. Keep built-in and surface-backed variants visually identical unless the spec documents an allowed exception.
5. Add or update parity fixtures before landing a new shared visual pattern.
```

Run:

```bash
markdownlint --config .markdownlint.json docs/development/ui-design-language.md
```

Expected: the filled guide still passes markdownlint.

- [ ] **Step 3: Register the new guide in the documentation catalogues**

Update `docs/development/README.md` and `docs/README.md` so both catalogues
link to `docs/development/ui-design-language.md` using wording that makes it
the stable engineering entry point for frontend visual-language work.

Use link text equivalent to:

```md
- `docs/development/README.md`: [UI design language](ui-design-language.md) — stable frontend design-system and Surfaces parity guide
- `docs/README.md`: [UI design language](development/ui-design-language.md) — stable frontend design-system and Surfaces parity guide
```

Run:

```bash
markdownlint --config .markdownlint.json docs/development/README.md docs/README.md
```

Expected: both modified catalogue files pass markdownlint.

- [ ] **Step 4: Commit**

```bash
git add docs/development/ui-design-language.md docs/development/README.md docs/README.md
git commit -m "docs: publish stable ui design language guide"
```

---

### Task 2: Align Existing Frontend And Surfaces Development Guides

**Files:**

- Modify: `docs/development/frontend-components.md`
- Modify: `docs/development/surfaces.md`

- [ ] **Step 1: Update the frontend component guide to point at shared primitives**

Edit `docs/development/frontend-components.md` so it no longer reads as a mostly modal-only guide. Add a short section near the top that says:

```md
## Design Language Entry Point

Before adding or restyling frontend UI, read [UI design language](ui-design-language.md).
`frontend/src/lib/components/` and `frontend/src/lib/components/surfaces/` must both consume the same shared primitives and token adapter.
```

Only land this step after the foundation plan has introduced
`frontend/src/lib/components/ui/`, or phrase the section in future tense until
that directory exists.

Also add one new section enumerating the planned shared primitives directory:

```md
## Shared UI Primitives

- `frontend/src/lib/components/ui/` owns reusable page-shell, card, tab, callout, empty-state, badge, field-row, provider-selector, and table primitives.
- Existing shared shells `frontend/src/lib/components/Modal.svelte` and `frontend/src/lib/components/ContextMenu.svelte` are part of the same canonical primitive set.
- Route files should compose these primitives instead of recreating layout and color choices locally.
```

Run:

```bash
markdownlint --config .markdownlint.json docs/development/frontend-components.md
```

Expected: the guide passes markdownlint after the new sections are added.

- [ ] **Step 2: Update the Surfaces guide to use the design-language vocabulary**

Edit `docs/development/surfaces.md` so the authoring guidance uses the current terms from the approved spec:

- “shared Surfaces runtime” instead of extension-framework language
- explicit parity requirement between built-in and surface-backed UI
- runtime-state names `loading`, `permission_denied`, `no_compatible_provider`, `contract_mismatch`, `hydration_action_failure`, and `no_surface_content`
- explicit note that slot IDs are fixed by `crates/shared/surfaces/src/slot.rs` and do not imply a separate visual system

Add an admonition block equivalent to:

```md
> Surface-backed UI must render through the same visual primitives and token adapter as built-in UI.
> If a new primitive is needed, promote it into the shared frontend component set first.
```

Run:

```bash
markdownlint --config .markdownlint.json docs/development/surfaces.md
```

Expected: the Surfaces guide passes markdownlint and uses the same vocabulary as the design-language spec.

- [ ] **Step 3: Commit**

```bash
git add docs/development/frontend-components.md docs/development/surfaces.md
git commit -m "docs: align frontend and surfaces guides with design language"
```

---

### Task 3: Add Governance And Waiver Artifacts

**Files:**

- Modify: `docs/development/quality-gates.md`
- Create: `docs/superpowers/ui-parity-waivers.json`

- [ ] **Step 1: Seed the required waiver file**

Create `docs/superpowers/ui-parity-waivers.json` as an empty JSON array:

```json
[]
```

Then add one short comment-free companion explanation in
`docs/development/quality-gates.md` saying this file is the only accepted
source of visual-parity waivers.

Run:

```bash
jq empty docs/superpowers/ui-parity-waivers.json
```

Expected: `jq` exits `0` and the file is valid JSON.

- [ ] **Step 2: Document the parity gate and waiver process**

Append a new section titled `UI Visual Parity` to
`docs/development/quality-gates.md` and document:

- the adapter-manifest requirement from the design language
- desktop parity coverage required now
- mobile parity coverage deferred until the responsive shell leaves `Target`
- the waiver file path and required schema fields
- the rule that expired waivers require explicit renewal or removal

Use a compact checklist block like:

```md
## UI Visual Parity

Before merging frontend visual changes:

1. Update the adapter manifest if semantic token mappings changed.
2. Add or update deterministic parity fixtures for changed built-in and surface-backed patterns.
3. Keep `docs/superpowers/ui-parity-waivers.json` empty unless a temporary exception is explicitly needed.
4. Remove or renew expired waivers in the same change window.
```

Run:

```bash
markdownlint --config .markdownlint.json docs/development/quality-gates.md
```

Expected: the markdown file passes and
`jq empty` succeeded in Step 1.

- [ ] **Step 3: Final documentation verification**

Run:

```bash
markdownlint --config .markdownlint.json \
  docs/development/ui-design-language.md \
  docs/development/frontend-components.md \
  docs/development/surfaces.md \
  docs/development/quality-gates.md \
  docs/development/README.md \
  docs/README.md
```

Expected: all touched Markdown files pass in one batch.

- [ ] **Step 4: Commit**

```bash
git add docs/development/quality-gates.md docs/superpowers/ui-parity-waivers.json
git commit -m "docs: add ui parity governance artifacts"
```

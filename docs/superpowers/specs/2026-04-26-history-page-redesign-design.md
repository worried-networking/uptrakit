# History Page Redesign

**Date:** 2026-04-26
**Status:** Approved

## Goal

Redesign `/history` so it reads as a cleaner, more deliberate hybrid of timeline and operations
view while staying inside the established Uptrakit UI design language.

The redesign must:

- remove the `Input Required` badge from the feed
- move `Attach terminal` / `View logs` next to each row title
- surface actor information directly in collapsed list rows
- surface the actor's display name in collapsed list rows when available
- keep chronology as the primary organizing structure
- preserve the shared terminal modal as the detailed output surface

## Constraints

- Follow the approved UI design language in
  `docs/development/ui/README.md`, `tokens.md`, `primitives.md`, and `layout.md`.
- Use existing shared primitives and semantic tokens rather than inventing route-local chrome.
- Keep `/history` as a built-in page that visually matches other authenticated routes.
- Do not infer or guess terminal input state in the feed from delayed output or interactive timing.
- Treat terminal opening as a modal-launch action, not a toggle embedded in the list row.
- Do not require a backend API change for aggregate status counts.
- Treat design-language conformance as an implementation gate: if the route cannot be built cleanly
  within `docs/development/ui/`, stop and resolve the design-language gap explicitly rather than
  drifting in route-local markup or styling.

## Problem

The current page is functional but visually flat:

- the top of the page is split into a generic filters card and a separate feed card, with little
  summary signal about what needs attention
- row actions are pushed to the far trailing edge, which makes `Attach terminal` / `View logs`
  feel detached from the history item they act on
- actor information is hidden in the terminal callouts instead of being visible in the main list
- the `Input Required` badge suggests precision the frontend does not actually have, because the
  signal is inferred from terminal interactivity and output timing rather than from a reliable
  backend state

The result is a page that exposes all the raw data but does not guide the operator’s attention
well enough.

## Solution

Restructure `/history` into a three-layer page:

1. A compact summary strip for the visible first-page result set.
2. A tighter controls card for filters and the trigger action.
3. A redesigned grouped feed whose rows emphasize title-adjacent actions and first-line metadata.

The page remains timeline-first. The redesign does not add a separate live console view or move
the user away from date-grouped history.

## Page Structure

`PageShell` remains the top-level primitive. The body becomes:

```text
PageShell
├── summary strip
│   ├── Running count
│   ├── Waiting count
│   ├── Failed count
│   └── Completed count
├── controls card
│   ├── status filter controls
│   └── Trigger Update action
└── history feed card
    ├── date group
    │   ├── row
    │   └── row
    ├── date group
    └── existing pagination footer
```

### Summary Strip

The summary strip sits above the controls and uses the same visual language as existing built-in
summary cards:

- compact card-like blocks
- semantic status tones only
- no decorative gradients, shadows, or palette exceptions

The strip is informative, not analytical. It must summarize the visible first-page result set in a
single glance without implying tenant-wide totals the current API does not provide.

### Summary Data Source

The summary strip is derived only from the currently loaded `items` array returned by the existing
`listUpdateHistory()` route call.

To avoid misleading totals:

- render the summary strip only when `statusFilter === 'all'`
- render the summary strip only when `currentPage === 1`
- omit the strip on later pages and on narrowed status views
- do not render stale counts while a fresh page-1 unfiltered request is loading

When the page is loading the page-1 unfiltered result set, the summary strip must either be hidden
or replaced by a neutral loading placeholder. It must not continue showing counts from a previous
result set.

No separate API call or backend aggregate endpoint is part of this redesign.

### Summary Taxonomy

The summary strip uses four deterministic buckets:

- `Running` = `in_progress`
- `Waiting` = `queued` + `pending`
- `Failed`
- `Completed`

`Failed` and `Running` must carry the strongest visual emphasis. `Completed` must remain
present but quieter.

### Controls Card

The existing filter area becomes a more deliberate operator toolbar:

- keep the current visible status filters: `All`, `Pending`, `In Progress`, `Completed`, `Failed`
- keep `Trigger Update`
- reduce the visual weight of the “Filters” framing
- make the active filter state clearer using existing button and token patterns

This remains a standard card surface aligned with other built-in pages. No custom control bar
component is introduced.

### Feed Card

The grouped chronological feed remains the main content area and retains date-grouped sections such
as `Today` and `Yesterday`.

The feed card is still the source of truth for recent and historical execution activity. The
summary strip exists to support scanning, not to replace the feed.

## Row Design

Each history item becomes a denser two-band row within the existing grouped feed model.

### Header Band

Left side:

- status glyph
- primary title in the form `<software> on <host>`

Right side:

- row action beside the title area, not in a distant trailing stack

Action placement changes from “far-right utility column” to “title-adjacent action”. This makes
the action feel attached to the run it opens.

### Metadata Band

The second line surfaces the row’s key metadata inline:

- version transition
- relative start time
- status badge
- actor label with actor display name

Actor info is promoted into the row itself rather than remaining hidden in terminal callouts.

Actor copy must be deterministic:

- `Triggered by user <name>`
- `Triggered by scheduler <name>`
- `Triggered by service <name>`

When a display name is available, the collapsed row must show both actor type and actor name in a
human-readable label, for example:

- `Triggered by user Alice Smith`
- `Triggered by service External Scheduler`

For any other non-empty `actor_type`, normalize underscores and hyphens to spaces, lowercase the
result, and render `Triggered by <normalized actor_type> <name>` when a display name is available.

If the actor display name is missing, fall back to type-only copy:

- `Triggered by user`
- `Triggered by scheduler`
- `Triggered by service`
- `Triggered by <normalized actor_type>`

If `actor_type` is empty or missing, render `Trigger source unknown`.

The collapsed row must not include raw `actor_id`. Full raw actor details remain available in the
terminal modal’s additional details area.

## Row Actions

The row action label becomes stable and non-toggle:

- interactive in-progress rows use `Attach terminal`
- every other row uses `View logs`

Do not switch these labels to `Close terminal` or `Hide logs`.

Reasoning:

- the terminal opens as a modal shell, not as inline disclosure inside the row
- the close action already belongs to the modal itself
- stable labels reduce cognitive churn in the feed and better match the actual interaction model

The row action must use dialog-launch semantics rather than disclosure semantics:

- visible label stays stable
- loading state is allowed
- `aria-haspopup="dialog"` is appropriate if needed
- `aria-expanded` must not be used for this action

If the modal is already open:

- clicking the action for the currently open row is a no-op
- clicking the action for a different row retargets the existing modal to that row rather than
  opening a second modal

## Input State Handling

The `Input Required` badge is removed from the feed.

This is an intentional correctness change, not only a visual cleanup. The frontend must not
claim to know that input is required when that state is inferred from heuristics such as:

- terminal interactivity
- delayed text updates
- stdin-attention guesses

The feed must surface only reliable, explicit states:

- queued
- pending
- in progress
- completed
- failed

If the terminal modal itself has stronger local evidence that user attention is needed, that
detail may still be shown there. The list view must not rely on guessed “input required” magic.

## Responsive Behavior

The redesign must define narrow-width behavior explicitly rather than relying on incidental wrapping.

### Desktop (`>= 1024px`)

- summary strip renders as four columns
- header band keeps title on the left and row action on the right
- metadata band stays on one wrapped row where space permits

### Tablet (`640px–1023px`)

- summary strip renders as two columns
- row header may wrap, but the action stays visually attached to the title block
- metadata band may wrap to a second line without horizontal scrolling

### Mobile (`< 640px`)

- summary strip renders as a single column
- row header stacks vertically: title block first, action directly below or beside it within the
  same visual cluster
- metadata wraps naturally into multiple short lines
- no row content may require horizontal scrolling

The implementation must preserve a timeline-first scan path on mobile rather than turning each row
into a mini dashboard card.

## Terminal Modal Relationship

The shared terminal shell defined by the design language remains unchanged as the detailed
inspection and interaction surface.

This redesign does not:

- replace the terminal modal with inline expansion
- move output into each feed row
- introduce a second log viewer component

The history feed remains a scan-and-launch surface. The terminal modal remains the focused
inspection surface.

## Design Language Alignment

This redesign must stay inside the existing design language contract.

### Use Existing Primitives

Use existing primitives where applicable:

- `PageShell`
- `SectionCard`
- `StatusBadge`
- `Button`
- existing shared terminal output shell

If the summary strip needs route-level card markup rather than an extracted primitive, it must
still use existing tokens, radius utilities, spacing utilities, and transition rules.

### Pagination Footer

Preserve the current route pagination behavior.

This redesign does not introduce a new pagination pattern and does not treat standalone history
pagination as a new design-language precedent. If future cleanup is needed to align History
pagination with the primitive contract, that should be handled as separate follow-up work.

### Use Existing Tokens and Utilities

All colors, radii, typography, spacing, focus treatments, and transitions must come from the
approved design-language token system.

Specifically:

- semantic status colors only
- named typography utilities only
- named radius utilities only
- approved focus ring only
- approved interactive transition triplet only

No route-specific palette, no arbitrary decoration, and no visual language that makes `/history`
read differently from Hosts, Software, or Services.

### Implementation Guardrails

Implementation must be checked directly against `docs/development/ui/README.md`,
`tokens.md`, `primitives.md`, and `layout.md` while the route is being built.

Concretely:

- prefer shared primitives before introducing route-local structure
- if route-local structure is necessary, compose it from existing semantic tokens and named
  utilities only
- do not introduce arbitrary visual values where the design language already defines a role
- do not treat a route-specific workaround as acceptable if it would read as a new visual pattern
- if the work reveals a real missing primitive or documented gap, capture that as explicit follow-up
  work or update the design documentation rather than silently diverging in `/history`

Design-language alignment is part of the acceptance criteria for implementation, not a secondary
polish pass.

## Data and Behavior Impact

This redesign does require a backend/API addition for actor display name in update-history
responses. It does not require a backend aggregate endpoint for summary-strip counts.

The work is a frontend composition and presentation change that reuses existing route data:

- history item status
- software and host labels
- version transition
- timestamps
- actor type / actor id
- actor display name
- interactivity flag

The main interpretation change is subtractive:

- stop representing “input required” as a confident feed-level state
- derive summary-strip counts from the visible first-page unfiltered result set only

### Actor Display Name Dependency

The current history payload exposes `actor_type` and `actor_id`, but not an actor display name.
To satisfy this redesign, the update-history response contract must add an optional
`actor_name: Option<String>` field on the backend and `actor_name?: string | null` in frontend
types.

Behavior:

- if `actor_name` is present and non-empty, render it in the collapsed row label
- if `actor_name` is absent or empty, use the type-only fallback rules above
- raw `actor_id` stays out of the collapsed row and remains available only in the terminal modal

## Testing

Update the existing history route test suite to cover the redesign.

### Keep Existing Behavioral Coverage

Preserve tests for:

- chronological grouping
- status glyph rendering
- terminal modal opening
- live terminal session wiring
- Ctrl+C action in the terminal modal

### Add or Update Assertions

Add or revise assertions for:

- absence of the `Input Required` badge in the feed
- `Attach terminal` / `View logs` appearing next to the row title area
- stable visible labels for those actions after opening the modal
- clicking the currently open row action does not close the modal
- clicking a different row action retargets the existing modal
- actor info appearing in the collapsed row metadata
- actor display name appearing in the collapsed row when provided by the payload
- type-only actor fallback copy when actor display name is absent
- summary strip rendering only on `status=all` page 1
- summary strip bucket mapping: `Running`, `Waiting`, `Failed`, `Completed`
- summary strip hidden or replaced with neutral loading state during a fresh page-1 unfiltered load
- absence of `aria-expanded` on the row action when it launches the dialog
- responsive row wrapping behavior at narrow widths where practical in route tests or visual checks
- visual and structural conformance with `docs/development/ui/README.md`, `tokens.md`,
  `primitives.md`, and `layout.md`

### Out of Scope for This Redesign

- no backend aggregate endpoint for summary-strip counts
- no terminal-shell redesign
- no new analytics or historical aggregation model
- no alternate “live only” history mode
- no changes to non-history routes

## File Scope

Expected primary implementation targets:

- `frontend/src/routes/history/+page.svelte`
- `frontend/src/routes/history/history.test.ts`
- `crates/shared/web-api-types/src/update_history.rs`
- `frontend/src/lib/types.ts`

Shared primitives and design-language docs must remain untouched unless the implementation
reveals a genuine reusable gap. The current approved direction is route-level composition plus the
minimal shared API/type contract change required for `actor_name`.

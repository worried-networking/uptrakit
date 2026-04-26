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
- keep chronology as the primary organizing structure
- preserve the shared terminal modal as the detailed output surface

## Constraints

- Follow the approved UI design language in
  `docs/development/ui/README.md`, `tokens.md`, `primitives.md`, and `layout.md`.
- Use existing shared primitives and semantic tokens rather than inventing route-local chrome.
- Keep `/history` as a built-in page that visually matches other authenticated routes.
- Do not infer or guess terminal input state in the feed from delayed output or interactive timing.
- Treat terminal opening as a modal-launch action, not a toggle embedded in the list row.

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

1. A compact summary strip for current operational counts.
2. A tighter controls card for filters and the trigger action.
3. A redesigned grouped feed whose rows emphasize title-adjacent actions and first-line metadata.

The page remains timeline-first. The redesign does not add a separate live console view or move
the user away from date-grouped history.

## Page Structure

`PageShell` remains the top-level primitive. The body becomes:

```text
PageShell
├── summary strip
│   ├── In Progress count
│   ├── Queued count
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
    └── TableFooterBar
```

### Summary Strip

The summary strip sits above the controls and uses the same visual language as existing built-in
summary cards:

- compact card-like blocks
- semantic status tones only
- no decorative gradients, shadows, or palette exceptions

The strip is informative, not analytical. It must answer “what needs attention right now?” in a
single glance without turning the page into a dashboard.

Recommended counts:

- `In Progress`
- `Queued`
- `Failed`
- `Completed`

`Failed` and `In Progress` must carry the strongest visual emphasis. `Completed` must remain
present but quieter.

### Controls Card

The existing filter area becomes a more deliberate operator toolbar:

- keep status filters
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
- actor label

Actor info is promoted into the row itself rather than remaining hidden in terminal callouts.

Preferred row copy is human-readable first, for example:

- `Triggered by user`
- `Triggered by scheduler`
- `Triggered by service`

If the actor id adds diagnostic value, it can still remain available in the terminal modal’s
additional details area.

## Row Actions

The row action label becomes stable and non-toggle:

- interactive in-progress rows use `Attach terminal`
- non-interactive or completed rows use `View logs`

Do not switch these labels to `Close terminal` or `Hide logs`.

Reasoning:

- the terminal opens as a modal shell, not as inline disclosure inside the row
- the close action already belongs to the modal itself
- stable labels reduce cognitive churn in the feed and better match the actual interaction model

The action can still carry `aria-expanded` or loading state if required by the implementation, but
the visible copy must remain stable.

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
- `TableFooterBar`
- existing shared terminal output shell

If the summary strip needs route-level card markup rather than an extracted primitive, it must
still use existing tokens, radius utilities, spacing utilities, and transition rules.

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

## Data and Behavior Impact

No backend contract change is required for the approved redesign.

The work is a frontend composition and presentation change that reuses existing route data:

- history item status
- software and host labels
- version transition
- timestamps
- actor type / actor id
- interactivity flag

The main interpretation change is subtractive:

- stop representing “input required” as a confident feed-level state

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
- actor info appearing in the collapsed row metadata
- summary strip rendering with the expected statuses

### Out of Scope for This Redesign

- no backend protocol changes
- no terminal-shell redesign
- no new analytics or historical aggregation model
- no alternate “live only” history mode
- no changes to non-history routes

## File Scope

Expected primary implementation targets:

- `frontend/src/routes/history/+page.svelte`
- `frontend/src/routes/history/history.test.ts`

Shared primitives and design-language docs must remain untouched unless the implementation
reveals a genuine reusable gap. The current approved direction assumes route-level composition
only.

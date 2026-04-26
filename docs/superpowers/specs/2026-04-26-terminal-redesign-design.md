# Terminal Redesign

**Date:** 2026-04-26
**Status:** Approved

## Goal

Redesign the shared frontend terminal shell so non-terminal context stops dominating the modal.

The redesign must:

- apply everywhere the shared terminal shell is used
- remove stacked large infoboxes from the main terminal body
- keep `Output truncated` prominent without consuming much height
- reduce interactive/input-needed messaging to a compact badge
- hide additional details by default behind an explicit disclosure
- preserve the terminal as the primary visual focus

## Constraints

- Follow the approved UI design language in
  `docs/development/ui/README.md`, `tokens.md`, `primitives.md`, and `layout.md`.
- Keep the shared shell model rather than introducing route-local terminal chrome.
- Use semantic tokens and existing primitives where they fit; add shared terminal-specific styling
  only where existing primitives are too heavy for the required density.
- Do not remove the global `Callout` component from the codebase. This redesign removes it from the
  terminal shell only; `Callout` remains in active use across auth, settings, surfaces, toasts,
  route-level errors, and data-table error states.

## Problem

The current terminal shell renders a generic `callouts[]` stack above the xterm viewport. When a
run has multiple bits of context, the shell can accumulate several full-width boxes before the user
reaches the actual output:

- waiting/pending copy
- output truncation warning
- actor information
- recovery or protection details
- interactive/input-needed messaging

This creates the wrong hierarchy. The modal is supposed to be a terminal first, but the current
structure makes the informational chrome compete with the output area.

## Solution

Replace the generic stacked-callout model with a priority-based shell:

1. Primary shell chrome: title, status, metadata, terminal viewport, and actions.
2. One critical banner slot: reserved for truncation-level warnings only.
3. Compact operational badges: low-height badges for interactive/live state.
4. Hidden details: actor and explanatory text behind a collapsed disclosure.

This produces a deterministic layout:

- at most one prominent banner
- no large “input required” infobox
- no multi-box preamble above the terminal
- details available on demand without stealing viewport height

## Shared Component Contract

The shared terminal component must stop taking a route-provided generic `callouts[]` array and
instead accept explicit priority-based data.

Required shape:

- `criticalBanner`: optional single object for the top warning slot; at most one may be rendered
- `inlineBadges`: array of compact badges for operational state such as interactive/live
- `details`: optional hidden-by-default detail groups rendered behind a disclosure
- `emptyState`: optional lightweight text for waiting/no-output cases when there is no live
  interactive session and no non-whitespace captured output to render in the terminal viewport
- existing title, status, metadata, actions, output, and `onInput` inputs remain

This API makes priority obvious in code and prevents callers from recreating large stacked message
blocks accidentally.

## Layout Rules

### Top Area

The top of the shell keeps the titlebar and existing shell framing.

Below the titlebar:

- render the critical banner only when present
- do not render any other full-width callout block there

If no critical banner is present, the terminal body must visually start immediately after the
titlebar. There is no separate top status row in this redesign.

### Terminal Body

The xterm viewport remains the dominant body element when rendered and must reclaim the vertical
space currently used by stacked callouts.

The shell body must remain usable in:

- live interactive sessions
- captured-output sessions
- empty-state waiting/no-output sessions

### Footer / Status Area

The footer keeps:

- status badge
- compact metadata
- terminal actions such as `Ctrl+C`
- compact interactive badge when relevant
- `Details` disclosure trigger when detail content exists

This is the correct place for small operational cues that should remain visible but not loud.

## State Mapping

### Output Truncated

`output_truncated` maps to the single critical banner slot.

Requirements:

- visually prominent
- narrow in height
- always more prominent than any other terminal-side status
- never rendered using the shared `Callout` component inside the terminal shell

This is the only currently approved state for the critical banner slot.

### Interactive / Input Needed

Interactive state must no longer render as a large warning box.

Requirements:

- show a compact badge such as `Interactive terminal` or equivalent
- keep it in the footer/status area
- if stdin attention is active, adjust badge tone/label rather than escalating to a banner

This keeps the operator informed without implying that the terminal shell itself is a warning
surface.

### Additional Details

The following move behind a collapsed `Details` affordance by default:

- actor information
- pre-update protection summary
- recovery hint
- future long-form explanatory text of similar severity

The details area must expand inline within the shell without displacing the terminal more than
necessary. Closed by default is the required behavior everywhere.

### Waiting / Pending / No Output

When there is no live interactive session and no non-whitespace captured terminal output, the shell
may show lightweight explanatory copy,
but it must not reuse the critical banner slot unless the state is actually truncation-level.

Examples:

- queued behind another update
- pending start on the agent
- no recorded output

These should be visually quiet and secondary.

## Composition Rules

The shell must resolve simultaneous states deterministically.

Required precedence:

1. `criticalBanner` renders first when present and occupies the only prominent banner slot.
2. `inlineBadges` may coexist with the banner and remain compact in the footer/status area.
3. `details` may coexist with both banner and badges, but remain collapsed by default.
4. `emptyState` is mutually exclusive with the rendered terminal viewport and is used only when
   there is no live interactive session and no non-whitespace captured terminal output to show.

Examples:

- truncation + interactive: show truncation in the critical banner and interactive in a footer
  badge
- truncation + actor + recovery hint: show truncation in the critical banner and keep actor and
  recovery hint inside collapsed details
- interactive + waiting metadata: keep interactive as a compact badge and waiting text as
  lightweight secondary copy, not as a banner
- no output + details: show the lightweight empty state and keep details collapsed behind the
  disclosure

## Route Integration

The redesign applies everywhere the shared terminal shell is used, including history and software
update flows.

Route code must map backend state into the shared contract rather than constructing route-local
terminal presentation rules.

Specifically:

- history should stop building `Callout` lists for the terminal
- software-triggered live terminal flows should inherit the same compact hierarchy automatically
- future terminal entry points should consume the same priority-based API

## Callout Component Scope

The terminal refactor must eliminate `Callout` usage inside the terminal shell itself.

It should not delete `frontend/src/lib/components/ui/Callout.svelte`, because that component is
still actively used across the rest of the frontend. Removing it would be unrelated scope and would
create unnecessary churn outside the terminal redesign.

## Testing

Update the terminal and route tests to match the new hierarchy.

Required coverage:

- shared terminal shell still renders modal chrome, titlebar, statusbar, and actions
- `output_truncated` renders the single critical banner slot
- terminal-shell tests explicitly verify that no shared `Callout` component renders inside the
  terminal shell
- interactive/live state renders as a compact badge, not a `Callout`
- additional details are hidden by default and become visible only after expansion
- waiting/no-output states remain lightweight
- combined-state coverage verifies the precedence rules for banner, badges, details, and empty
  state behavior
- route-level tests continue to verify that terminal entry points use the shared shell rather than
  route-local chrome

Tests that currently assert visible `Actor` / `Additional details` callout headings should be
rewritten around the collapsed `Details` behavior.

## Non-Goals

This redesign does not:

- remove the terminal modal entirely
- move logs inline into history rows
- create a second terminal component
- remove `Callout` from the broader frontend
- add new backend terminal states

## Recommended Implementation Direction

Implement the redesign by first reshaping the shared terminal component API, then updating route
state mapping to the new contract, and finally adjusting tests to match the new priority rules.

This keeps the redesign centered in the shared shell and prevents route-specific drift.

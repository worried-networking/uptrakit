<!-- markdownlint-disable MD013 -->

# UI Design Language

Developer-facing implementation guide for the approved Uptrakit UI design language.

## Purpose

This guide makes the approved spec executable inside the Uptrakit frontend. It documents:

- the semantic token and theme contract
- shared shell and shared component rules
- built-in versus surface-backed parity rules
- the current status of each spec area: `Implemented`, `Transitional`, or `Target`
- the verification and waiver requirements that gate merges

Built-in product UI and surface-backed UI must read as one product. Visual origin leakage is a bug.

## Relationship To The Approved Spec

The approved spec at
[`docs/superpowers/specs/2026-04-16-ui-design-language-design.md`](../../superpowers/specs/2026-04-16-ui-design-language-design.md) is normative. If
this guide conflicts with that spec, the spec wins.

Use this guide when:

- adding or restyling a route
- building or updating a shared primitive
- wiring a new surface-backed view
- deciding whether a change needs a parity fixture or a waiver
- reviewing whether a built-in and surface-backed pattern are genuinely equivalent

## Status Model

| Status         | Meaning                                                                      |
| -------------- | ---------------------------------------------------------------------------- |
| `Implemented`  | Part of the current contract; required now.                                  |
| `Transitional` | Present in the runtime but still converging on the final target.             |
| `Target`       | Approved future state; do not assume the current runtime already matches it. |

Sections without an explicit status label are `Implemented`.

## Technology Stack

| Concern             | Technology                                                                  |
| ------------------- | --------------------------------------------------------------------------- |
| Component framework | Svelte 5 (runes API: `$props`, `$bindable`, `$derived`, `$state`, Snippets) |
| Styling             | Tailwind CSS v4 with semantic CSS custom properties                         |
| Form styling        | `@tailwindcss/forms` plugin                                                 |
| Theme adapter       | CSS custom properties via `frontend/src/theme/`                             |
| Visual regression   | Playwright (macOS + Chromium, reduced-motion, DPR 1)                        |

## Themes

**Status:** `Implemented`

- Dark and light themes are both first-class; neither is a fallback.
- Dark is the default when system preference is unavailable; otherwise follows `prefers-color-scheme`.
- A UI theme switcher provides manual override.
- Both themes must maintain WCAG AA contrast on their intended backgrounds.
- Never implement a dark-only shared primitive.
- Built-in and surface-backed UI must use the same theme adapter and semantic tokens.

## Documentation Index

Read in this order when onboarding:

| Order | Page                           | Content                                                                                                                                |
| ----- | ------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------- |
| 1     | [tokens.md](tokens.md)         | Design tokens, typography, border-radius, transitions, focus states, z-index, runtime adapter                                          |
| 2     | [primitives.md](primitives.md) | Shared non-form UI components — props, variants, usage rules                                                                           |
| 3     | [forms.md](forms.md)           | Form primitives, FormLayout context, Save/Discard placement, `createFormDraft`, surface form draft mode, `submit_label`                |
| 4     | [layout.md](layout.md)         | App shell measurements, sidebar, public entry shell, responsive layout                                                                 |
| 5     | [pages.md](pages.md)           | Feature page conventions (Software, Hosts, History, Settings, slot-backed panels) and the unified Filter Bar Convention                |
| 6     | [surfaces.md](surfaces.md)     | Surface parity contract, slot registry, runtime states, verification, waivers (read last — assumes knowledge of tokens and primitives) |

## Interaction Conventions

**Status:** `Implemented`

- No layout reflow on hover.
- Clickable badge-style controls use the flat hover treatment defined in `primitives.md`.
- Only the animated properties listed in `tokens.md` are allowed.
- Dim means disabled, not hidden; disable interaction with `pointer-events: none` where required.
- Destructive actions always use danger treatment plus confirmation — use `ConfirmDialog` (see `primitives.md`).
- Focus rings appear on `:focus-visible` only; never on mouse click.

## Quick Reference: What To Use

| Need                                           | Use                                                | Lives in                       |
| ---------------------------------------------- | -------------------------------------------------- | ------------------------------ |
| Section header + contained body                | `SectionCard`                                      | [primitives.md](primitives.md) |
| Full page with eyebrow + actions               | `PageShell`                                        | [primitives.md](primitives.md) |
| Tab switching                                  | `TabStrip`                                         | [primitives.md](primitives.md) |
| Semantic callout (info/warning/danger/success) | `Callout`                                          | [primitives.md](primitives.md) |
| Inline info tooltip                            | `Tooltip`                                          | [primitives.md](primitives.md) |
| No-data placeholder                            | `EmptyState`                                       | [primitives.md](primitives.md) |
| Status indicator label                         | `StatusBadge`                                      | [primitives.md](primitives.md) |
| Navigable or action-triggering badge           | `ActionBadge`                                      | [primitives.md](primitives.md) |
| Categorical pill label                         | `PillBadge`                                        | [primitives.md](primitives.md) |
| Labeled form field + validation                | `FormFieldRow`                                     | [forms.md](forms.md)           |
| Read-only labeled value inside a form          | `FormFieldReadOnly`                                | [forms.md](forms.md)           |
| Single-line text input                         | `Input`                                            | [forms.md](forms.md)           |
| Multi-line text input                          | `Textarea`                                         | [forms.md](forms.md)           |
| Boolean (checkbox or toggle role)              | `Checkbox`                                         | [forms.md](forms.md)           |
| Mutually exclusive card-tile selector          | `RadioCardGroup`                                   | [forms.md](forms.md)           |
| Form Save/Discard state tracking               | `createFormDraft`                                  | [forms.md](forms.md)           |
| Data listing with pagination                   | `DataTable` + `TableFooterBar`                     | [primitives.md](primitives.md) |
| Navigable summary stat card                    | `StatCard`                                         | [primitives.md](primitives.md) |
| Context action in a dropdown                   | `ContextMenuItem` inside `ContextMenuShell`        | [primitives.md](primitives.md) |
| Destructive confirmation                       | `ConfirmDialog` (import directly, not from barrel) | [primitives.md](primitives.md) |
| Auth-consent card (OAuth, device flow)         | `ConsentPrompt`                                    | [primitives.md](primitives.md) |
| Arbitrary / form dialog                        | `ModalShell`                                       | [primitives.md](primitives.md) |
| Primary / ghost / secondary / danger action    | `Button`                                           | [primitives.md](primitives.md) |
| Targeted provider selection                    | `ProviderSelector` (surfaces only)                 | [primitives.md](primitives.md) |
| Table filter shell (filters + actions row)     | `FilterBar`                                        | [primitives.md](primitives.md) |
| Inline text search (collapsible)               | `ExpandableSearch`                                 | [primitives.md](primitives.md) |
| URL-reactive filter state                      | `createUrlParam`                                   | [primitives.md](primitives.md) |

See `frontend/src/lib/components/ui/index.ts` for all barrel exports.

## Where To Find Specific Rules

Recently-asked patterns and their canonical homes:

| Pattern                                       | Lives in                                                                              |
| --------------------------------------------- | ------------------------------------------------------------------------------------- |
| Form action button placement (Save/Discard)   | [forms.md — Form Action Buttons](forms.md#form-action-buttons)                        |
| `SectionCard` header vs body button rules     | [primitives.md — SectionCard](primitives.md#sectioncard) + forms.md for the body half |
| Surface `Section` layout + header actions     | [surfaces.md — Section Layout Rules](surfaces.md#section-layout-rules)                |
| Form draft mode (built-in and surface)        | [forms.md — createFormDraft](forms.md#createformdraft) and Surface Form Draft Mode    |
| `submit_label` override on surface forms      | [forms.md — submit_label](forms.md#submit_label)                                      |
| `FormLayout` modal vs page label-column width | [forms.md — FormLayout Context](forms.md#formlayout-context)                          |
| Table filter shell convention                 | [pages.md — Filter Bar Convention](pages.md#filter-bar-convention)                    |

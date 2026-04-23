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
[`docs/superpowers/specs/2026-04-16-ui-design-language-design.md`](../../superpowers/specs/2026-04-16-ui-design-language-design.md)
is normative. If this guide conflicts with that spec, the spec wins.

Use this guide when:

- adding or restyling a route
- building or updating a shared primitive
- wiring a new surface-backed view
- deciding whether a change needs a parity fixture or a waiver
- reviewing whether a built-in and surface-backed pattern are genuinely equivalent

## Status Model

| Status | Meaning |
| --- | --- |
| `Implemented` | Part of the current contract; required now. |
| `Transitional` | Present in the runtime but still converging on the final target. |
| `Target` | Approved future state; do not assume the current runtime already matches it. |

Sections without an explicit status label are `Implemented`.

## Technology Stack

| Concern | Technology |
| --- | --- |
| Component framework | Svelte 5 (runes API: `$props`, `$bindable`, `$derived`, `$state`, Snippets) |
| Styling | Tailwind CSS v4 with semantic CSS custom properties |
| Form styling | `@tailwindcss/forms` plugin |
| Theme adapter | CSS custom properties via `frontend/src/theme/` |
| Visual regression | Playwright (macOS + Chromium, reduced-motion, DPR 1) |

## Themes

**Status:** `Implemented`

- Dark and light themes are both first-class; neither is a fallback.
- Dark is the default when system preference is unavailable; otherwise follows `prefers-color-scheme`.
- A UI theme switcher provides manual override.
- Both themes must maintain WCAG AA contrast on their intended backgrounds.
- Never implement a dark-only shared primitive.
- Built-in and surface-backed UI must use the same theme adapter and semantic tokens.

## Documentation Index

| Page | Content |
| --- | --- |
| [tokens.md](tokens.md) | Design tokens, typography, border-radius, transitions, focus states, z-index, runtime adapter |
| [layout.md](layout.md) | App shell measurements, sidebar, top bar, public entry shell, responsive layout |
| [primitives.md](primitives.md) | All shared UI components — props, variants, usage rules |
| [surfaces.md](surfaces.md) | Surface parity contract, slot registry, runtime states, verification, waivers |

## Interaction Conventions

**Status:** `Implemented`

- No layout reflow on hover.
- Clickable badge-style controls use the flat hover treatment defined in `primitives.md`.
- Only the animated properties listed in `tokens.md` are allowed.
- Dim means disabled, not hidden; disable interaction with `pointer-events: none` where required.
- Destructive actions always use danger treatment plus confirmation.
- Focus rings appear on `:focus-visible` only; never on mouse click.

## Quick Reference: What To Use

| Need | Use |
| --- | --- |
| Section header + contained body | `SectionCard` |
| Full page with eyebrow + actions | `PageShell` |
| Tab switching | `TabStrip` |
| Semantic callout (info/warning/danger/success) | `Callout` |
| No-data placeholder | `EmptyState` |
| Status indicator label | `StatusBadge` |
| Navigable or action-triggering badge | `ActionBadge` |
| Categorical pill label | `PillBadge` |
| Labeled form field + validation | `FormFieldRow` + `Input` / `Textarea` / `Checkbox` |
| Data listing with pagination | `DataTable` + `TableFooterBar` |
| Context action in a dropdown | `ContextMenuItem` inside `ContextMenuShell` |
| Confirmation / form dialog | `ModalShell` |
| Primary / ghost / danger action | `Button` |

See `frontend/src/lib/components/ui/index.ts` for all exports.

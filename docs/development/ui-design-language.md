# UI Design Language

## Purpose

This guide is the developer-facing implementation companion for the approved UI design language.
It translates the normative spec into day-to-day engineering rules for route authors, shared
component authors, and Surfaces implementers.

The goal is consistency: built-in product UI and surface-backed UI use the same semantic tokens,
shared shell primitives, and parity rules so they read as one product.

## Relationship To The Approved Spec

The approved spec remains normative. If this guide conflicts with the approved spec, the spec wins.
This document does not redefine the product visual language; it explains how to implement it in the
current frontend and how to judge parity work.

Use this guide when you are:

- adding or restyling a route
- building or updating a shared primitive
- wiring a new surface-backed view
- deciding whether a change needs a parity fixture or a waiver

## Status Model (`Implemented` / `Transitional` / `Target`)

- `Implemented` means the pattern is part of the current contract and must be used now.
- `Transitional` means the current runtime already exposes the pattern, but a paired rollout may
  still be converging some details.
- `Target` means the pattern is approved future state. Do not rely on it until the spec promotes it.

When a section in this guide describes a current shell, slot, or primitive without a different
status label, treat it as `Implemented`.

## Theme And Token Adapter Contract

The semantic token families in the approved spec are the design contract. The frontend theme adapter
maps those semantic tokens to the runtime theme system through the checked-in adapter manifest at
`frontend/src/theme/adapter-manifest.json`.

The manifest is mandatory. It is the enforcement artifact that keeps built-in UI and surface-backed
UI on the same visual contract.

Semantic token families:

| Spec semantic token family | Runtime family |
| --- | --- |
| `--bg-base`, `--bg-surface`, `--bg-raised` | `surface-*` background utilities |
| `--border-subtle`, `--border-default` | `surface-*` border utilities |
| `--text-primary`, `--text-secondary`, `--text-muted` | `text-surface-*` utilities |
| `--accent`, `--accent-*`, `--accent-rgb` | `primary-*` theme utilities and preset variants |
| `--color-success-*` | `success-*` theme utilities and preset variants |
| `--color-warning-*` | `warning-*` theme utilities and preset variants |
| `--color-error-*` | `error-*` theme utilities and preset variants |
| `--color-info-*` | `info-*` semantic preset utilities exposed through the shared adapter |

Adapter requirements:

- every semantic token from the spec is present in the manifest
- each token maps to exactly one runtime token, utility, or preset per theme
- built-in and surface-backed UI both consume the same adapter layer
- no component uses one-off raw color classes when an equivalent semantic token exists
- the adapter manifest is checked in, versioned, and reviewed with the UI changes that depend on it

## Shared Shell Rules

The shared shell is the default route authoring surface. Route-local chrome is the exception, not the
baseline.

- Start every new route from `PageShell`; do not rebuild page spacing, heading rhythm, or shell
  borders locally.
- Use `SectionCard` for grouped content blocks and `TabStrip` for tabbed navigation.
- Use `EmptyState` for no-data, no-result, and no-provider states instead of ad hoc placeholder text.
- Use `Callout` for info, warning, and danger feedback; do not invent route-specific alert boxes.
- Use `StatusBadge` for compact state labels and `DataTable` for list/table views.
- Use `ProviderSelector` for targeted surfaces that need provider choice in the host chrome.
- Use the shared modal shell and workflow shell for action flows; do not substitute drawers or
  full-screen takeover flows unless the spec explicitly allows it.
- Keep built-in and surface-backed layouts visually identical unless the approved spec names a
  narrow exception.

## Shared Components And Primitives

These are the canonical primitives for the current UI foundation:

- `PageShell` - top-level page framing, heading rhythm, and content column structure
- `SectionCard` - reusable card shell for settings blocks, detail blocks, and inline sections
- `TabStrip` - route-owned tab navigation and body framing
- `EmptyState` - centered empty/no-results/no-provider layout
- `Callout` - semantic info, warning, and danger messaging
- `StatusBadge` - compact state and status labels
- `DataTable` - canonical list and table treatment
- `ProviderSelector` - compact provider-choice control for targeted surfaces
- modal shell - canonical dialog shell for actions, confirmations, and launcher-triggered flows
- workflow shell - canonical multi-step modal shell with shared step indicators

Any new shared visual pattern belongs in this primitive set first. Route-specific reuse should
consume the shared primitive rather than cloning its structure or colors.

## Surface Parity Rules

The current slot registry treats Surfaces as first-class UI content, not an alternate visual system.
Each slot below has a fixed host container and a matching visual treatment.

| Slot ID | Registry shape | Current parity expectation | No-content behavior |
| --- | --- | --- | --- |
| `surface.page` | Top-level nav page; single entry per provider registration | Behaves like a built-in top-level page inside the standard shell, including the same page heading pattern, content container, and nav item treatment | Render the surface page route shell; if no compatible provider is connected, show the shared empty state with provider guidance |
| `settings.tabs` | Settings tab strip; multi-entry | Uses the route-owned tab strip and tab-body container with the same active, hover, and overflow behavior as built-in tabs | Keep the built-in settings structure and omit only the surface entries when none exist |
| `settings.below.global` | Global settings body; multi-entry | Renders as an inline card and section stack below the built-in global settings content | Omit the surface section entirely when no content exists |
| `software.tabs` | Software tab strip; multi-entry | Uses the route-owned tab strip and tab-body container with the same active, hover, and overflow behavior as built-in tabs | Keep the built-in software structure and omit only the surface entries when none exist |
| `host_detail.tabs` | Host detail body; multi-entry | Renders as an inline card stack inside the host detail route with the same heading rhythm, spacing, and empty/error treatment as built-in host detail content | Omit the surface section entirely when no content exists |
| `software_item.host_context_menu` | Software-item host context menu; single entry per provider registration, aggregated across providers | Uses the standard launcher-row styling and opens the shared modal shell for actions | Omit the launcher entirely when no content exists |

Slot-order rules:

- multi-entry slot aggregation sorts by `priority`, then `label`, then stable surface ID
- `surface.page` entries share the main navigation sort rules with built-in pages
- `settings.tabs` and `software.tabs` are structural tab slots; they keep the built-in tabs even when
  surface content is absent
- `settings.below.global`, `host_detail.tabs`, and `software_item.host_context_menu` are omitted
  when there is no registered surface content
- the slot IDs are fixed by `crates/shared/surfaces/src/slot.rs` and do not imply a separate visual
  system

Runtime state IDs:

- `loading`
- `permission_denied`
- `no_compatible_provider`
- `contract_mismatch`
- `hydration_action_failure`
- `no_surface_content`

Use those state names in fixtures, stories, and parity tests. Do not invent surface-only fallback
labels or contract identifiers in the UI.

## Page Authoring Checklist

### New Route Checklist

1. Start with `PageShell`, not route-local margins or card shells.
2. Use semantic tokens through the adapter manifest, never raw color utilities when a semantic token
   exists.
3. Choose `SectionCard`, `TabStrip`, `DataTable`, `EmptyState`, `Callout`, and `StatusBadge` before
   creating a local variant.
4. Keep headings, spacing, and empty states consistent with the shared shell rules.
5. Add a parity fixture for the route if the change introduces a new shared visual pattern.

### New Surface-Backed UI Checklist

1. Render through the same shared shell and primitives as the built-in equivalent.
2. Match the approved slot registry shape and state IDs for the target slot.
3. Keep the built-in and surface-backed variants visually identical unless the approved spec names a
   permitted exception.
4. Use `EmptyState`, `Callout`, `ProviderSelector`, and the shared modal or workflow shell for the
   corresponding runtime states.
5. Add or update parity fixtures before merge, and file a waiver only if parity cannot be met yet.

## Verification And Waivers

Run markdownlint on this guide and the catalogues that point to it:

```bash
markdownlint --config .markdownlint.json \
  docs/development/ui-design-language.md \
  docs/development/README.md \
  docs/README.md
```

Parity verification is broader than markdown linting:

- every new shared shell or surface-backed visual path needs a deterministic parity fixture
- the fixture must cover the relevant slot, runtime state, theme, and viewport
- waivers are the exception path, not a shortcut for missing shared primitives
- a waiver must be checked in, scoped to a single issue, time-limited, and linked to review evidence

When a waiver is needed, use `docs/superpowers/ui-parity-waivers.json` as the
governance artifact, and keep the approved spec as the final authority on what
the product should look like.

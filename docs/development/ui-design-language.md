<!-- markdownlint-disable MD013 -->

# UI Design Language

## Purpose

This guide is the developer-facing implementation companion for the approved UI design language.
It is not a replacement for the approved spec. Its job is to make the spec executable inside the
current Uptrakit frontend by spelling out:

- the exact semantic token and theme contract
- the shared-shell and shared-component rules
- the built-in versus surface-backed parity rules
- the current status of each spec area: `Implemented`, `Transitional`, or `Target`
- the verification and waiver requirements that gate merges

Built-in product UI and surface-backed UI must read as one product. Visual origin leakage is a bug.

## Relationship To The Approved Spec

The approved spec at
[`docs/superpowers/specs/2026-04-16-ui-design-language-design.md`](/Users/andreyyantsen/Development/uptrakit/docs/superpowers/specs/2026-04-16-ui-design-language-design.md)
is normative. If this guide conflicts with that spec, the spec wins.

Use this guide when you are:

- adding or restyling a route
- building or updating a shared primitive
- wiring a new surface-backed view
- deciding whether a change needs a parity fixture or a waiver
- reviewing whether a built-in and surface-backed pattern are genuinely equivalent

## Status Model

- `Implemented`: part of the current contract and required now
- `Transitional`: already present in the runtime, but still converging on the final target shell
  or interaction model
- `Target`: approved future state; do not assume current runtime already matches it

If a section in this guide does not carry an explicit status label, treat it as `Implemented`.
Status labels describe contract intent and runtime behavior, not parity-closure completeness by
themselves.

---

## 1. Themes

**Status:** `Implemented`

- The UI supports dark and light themes.
- Dark is the default when system preference is unavailable.
- Otherwise the initial theme follows `prefers-color-scheme`.
- A UI theme switcher provides manual override.
- Light theme is a first-class design, not a fallback. Both themes must remain visually comparable
  and text must meet WCAG AA contrast on intended backgrounds.

### Theme Rules

- Never implement a dark-only shared primitive.
- Never use color choices that exist only in one theme unless the spec explicitly says so.
- Built-in and surface-backed UI must use the same theme adapter and the same semantic tokens.

---

## 2. Design Tokens

### 2.1 Dark Theme Colors

**Status:** `Implemented`

| Role | Token | Value |
| --- | --- | --- |
| Page background | `--bg-base` | `#09090b` |
| Sidebar / card surface | `--bg-surface` | `#111113` |
| Elevated surface | `--bg-raised` | `#18181b` |
| Subtle border | `--border-subtle` | `#1c1c1f` |
| Standard border | `--border-default` | `#27272a` |
| Muted text | `--text-muted` | `#52525b` |
| Secondary text | `--text-secondary` | `#a1a1aa` |
| Primary text | `--text-primary` | `#e4e4e7` |
| Inverted | `--text-inverted` | `#fafafa` |
| Accent | `--accent` | `#06b6d4` |
| Accent RGB | `--accent-rgb` | `6 182 212` |
| Accent bright | `--accent-bright` | `#22d3ee` |
| Accent dark | `--accent-dark` | `#0891b2` |
| Accent deep | `--accent-deep` | `#0e7490` |
| Success | `--color-success` | `#4ade80` |
| Success background tint | `--color-success-bg` | `rgba(74,222,128,.10)` |
| Success border | `--color-success-border` | `rgba(74,222,128,.25)` |
| Warning | `--color-warning` | `#fbbf24` |
| Warning background tint | `--color-warning-bg` | `rgba(251,191,36,.12)` |
| Warning border | `--color-warning-border` | `rgba(251,191,36,.3)` |
| Error | `--color-error` | `#fdba74` |
| Error background tint | `--color-error-bg` | `rgba(234,88,12,.15)` |
| Error border | `--color-error-border` | `rgba(234,88,12,.35)` |
| Info | `--color-info` | `#67e8f9` |
| Info background tint | `--color-info-bg` | `rgba(6,182,212,.10)` |
| Info border | `--color-info-border` | `rgba(6,182,212,.22)` |

### 2.2 Light Theme Colors

**Status:** `Implemented`

| Role | Token | Value |
| --- | --- | --- |
| Page background | `--bg-base` | `#f8fafc` |
| Sidebar / card surface | `--bg-surface` | `#ffffff` |
| Elevated surface | `--bg-raised` | `#f1f5f9` |
| Subtle border | `--border-subtle` | `#e2e8f0` |
| Standard border | `--border-default` | `#cbd5e1` |
| Muted text | `--text-muted` | `#94a3b8` |
| Secondary text | `--text-secondary` | `#64748b` |
| Primary text | `--text-primary` | `#0f172a` |
| Inverted | `--text-inverted` | `#ffffff` |
| Accent | `--accent` | `#2563eb` |
| Accent RGB | `--accent-rgb` | `37 99 235` |
| Accent bright | `--accent-bright` | `#3b82f6` |
| Accent dark | `--accent-dark` | `#1d4ed8` |
| Accent deep | `--accent-deep` | `#1e40af` |
| Success | `--color-success` | `#16a34a` |
| Success background tint | `--color-success-bg` | `rgba(22,163,74,.08)` |
| Success border | `--color-success-border` | `rgba(22,163,74,.3)` |
| Warning | `--color-warning` | `#d97706` |
| Warning background tint | `--color-warning-bg` | `rgba(217,119,6,.08)` |
| Warning border | `--color-warning-border` | `rgba(217,119,6,.28)` |
| Error | `--color-error` | `#dc2626` |
| Error background tint | `--color-error-bg` | `rgba(220,38,38,.07)` |
| Error border | `--color-error-border` | `rgba(220,38,38,.3)` |
| Info | `--color-info` | `#0891b2` |
| Info background tint | `--color-info-bg` | `rgba(8,145,178,.08)` |
| Info border | `--color-info-border` | `rgba(8,145,178,.22)` |

### 2.3 Border Radius

**Status:** `Implemented`

| Element | Radius |
| --- | --- |
| Page panels, modals, sidebar | `4px` |
| Terminal modal window | `6px` |
| Cards, table wrappers, buttons | `3px` |
| Badges, pills, small chips | `2px` |
| Traffic light dots | `50%` |
| Toggle track | `10px` |

### 2.4 Typography

**Status:** `Implemented`

- Font stack: `-apple-system, BlinkMacSystemFont, 'Segoe UI', 'Inter', sans-serif`
- Monospace stack: `'SF Mono', 'Roboto Mono', monospace`
- No custom web fonts

Heading scale:

| Element | Size | Weight | Color |
| --- | --- | --- | --- |
| `h1` | `20px` | `700` | `--text-primary` |
| `h2` | `16px` | `700` | `--text-primary` |
| `h3` | `13px` | `700` | `--text-primary` |

### 2.5 Transitions

**Status:** `Implemented`

Use one flat interaction transition:

```css
transition: background .12s, border-color .12s, color .12s;
```

Rules:

- no hover transforms
- no hover shadows
- keep controls visually flat
- ordinary controls (including shared shell links) stay on the same transition triplet; transform
  transitions are not allowed for those controls

Allowed animated properties:

- interactive controls: `background`, `border-color`, `color`
- loading affordances: `opacity`, `transform`, `background-position`
- toast progress bar: `transform: scaleX()`
- terminal maximize: `width`, `height`

### 2.6 Focus States

**Status:** `Implemented`

Keyboard focus must use `:focus-visible`, not the browser default outline:

```css
outline: none;
box-shadow: 0 0 0 3px rgba(var(--accent-rgb), .25);
```

Rules:

- show focus ring only on `:focus-visible`
- do not show click-triggered mouse-focus rings
- error-state fields keep the error border and also gain the accent focus ring

### 2.7 Z-Index Scale

**Status:** `Implemented`

| Layer | Value | Use |
| --- | --- | --- |
| Base content | `0` | normal page content |
| Sticky top bar | `10` | shell top bar |
| Sidebar | `20` | tablet overlay sidebar |
| Dropdown / tooltip | `100` | inline popovers |
| Toast stack | `500` | toasts |
| Modal backdrop | `900` | dialog or terminal backdrop |
| Modal content | `910` | dialog or terminal window |

Do not invent ad hoc z-index values when one of these applies.

### 2.8 Runtime Token Adapter

**Status:** `Implemented`

The semantic tokens above are the design contract. The runtime adapter is the enforcement layer.

Required artifacts:

- manifest: `frontend/src/theme/adapter-manifest.json`
- completeness test: `frontend/src/lib/theme/adapter-manifest.test.ts`

Family-level mapping:

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

Conformance rules:

- every semantic token from spec Sections 2.1 and 2.2 must exist in the manifest
- CI must fail if any required token is missing
- built-in and surface-backed UI must consume the same adapter
- no one-off raw color classes where an equivalent semantic token exists

---

## 3. Layout Shell

**Status:** `Target` for shell measurements and responsive shell behavior  
**Status:** `Transitional` for shared-surface parity closure, slot governance, and parity CI rollout

### Shared Shell Measurements

- Sidebar width: `180px`
- Top bar height: `40px`
- Content padding: `12px 14px`

### Public Entry Shell

`/login`, `/register`, `/device`, and `frontend/src/routes/+error.svelte`
share `PublicEntryShell.svelte`.

Rules:

- public-entry routes do not render authenticated shell chrome (sidebar, mobile
  bottom nav, current-user controls)
- the auth guard redirects protected routes to `/login`, but 4xx/5xx routes
  stay on the public-entry error shell instead of redirecting
- pre-auth forms use the same token contract as built-in forms through shared
  field/callout primitives and public-entry class exports
- route-specific semantics remain allowed inside the shell: device-code block,
  account-linking flow, first-user setup, and public error recovery action
- Content scrolls independently

Sidebar rules:

- background `--bg-surface`
- right border `--border-subtle`
- nav section headers uppercase at `7.5px`
- nav items `28px` tall, `3px` radius, `10px` font
- active state uses accent tint, accent text, and colored nav icon

Top-bar rules:

- title `12px` bold
- optional chip for item count
- search input and primary action live on the right

### Shared Surfaces

**Status:** `Implemented`

Surfaces are not a separate visual system.

Hard rules:

- built-in and surface-backed UI use the same tokens, primitives, spacing, type scale, and states
- users must not be able to infer visual origin
- origin-specific chrome is forbidden
- new primitives needed for Surfaces must become shared design-system primitives
- raw contract IDs or renderer internals must never leak into user-facing UI

### Slot Registry And Parity Contract

| Slot ID | Host container | Visual rule |
| --- | --- | --- |
| `surface.page` | top-level nav page | same shell and nav treatment as built-in top-level pages |
| `settings.tabs` | settings tab strip | same `TabStrip` and body container as built-in settings tabs |
| `settings.below.global` | global settings body | same inline card stack as built-in global settings content |
| `software.tabs` | software tab strip | same `TabStrip` and body container as built-in software tabs |
| `host_detail.tabs` | host detail body | same inline card stack as built-in host detail content |
| `software_item.host_context_menu` | software-item host context menu | same launcher-row shell and standard modal shell as built-in actions |

Registration and aggregation rules:

- `surface.page` and `software_item.host_context_menu` are single-entry per provider registration
- `settings.tabs`, `software.tabs`, `host_detail.tabs`, and `settings.below.global` are multi-entry
- aggregation order is `priority`, then `label`, then `surface_id`
- mixed built-in and `surface.page` nav order is `priority`, then `label`, then origin
  (`built-in` before `surface.page`), then stable ID

Targeted-provider rules:

- provider selector is host-owned chrome, not surface-owned content
- surfaces must not render a nested duplicate selector
- targeted `surface.page` selector lives below the heading, above content
- no-provider body uses the shared empty-state pattern

### Required Parity Gates

The following pairs and matrices are mandatory:

- built-in settings tab vs `settings.tabs`
- built-in software tab vs `software.tabs`
- built-in inline settings card vs `settings.below.global`
- route-owned host-detail slot container vs `host_detail.tabs`
- standard form field row vs targeted-surface provider selector
- built-in context-menu item vs `software_item.host_context_menu` launcher
- built-in action modal vs `software_item.host_context_menu` opened modal
- built-in top-level nav item vs `surface.page` nav item
- built-in page shell/body vs `surface.page` page shell/body

Required slot-state fixtures:

- `surface.page`: loaded, `permission_denied`, targeted `no_compatible_provider`,
  `contract_mismatch`, `hydration_action_failure`
- `settings.tabs` and `software.tabs`: loaded, `permission_denied`, targeted
  `no_compatible_provider`, `contract_mismatch`, `no_surface_content`
- `settings.below.global` and `host_detail.tabs`: loaded, `permission_denied`, targeted
  `no_compatible_provider`, `contract_mismatch`, `hydration_action_failure`, omitted
- `software_item.host_context_menu`: launcher row, opened modal, fallback, omitted

CI fail conditions:

- any visual diff above `0.5%` after approved masking
- any leaked contract ID or raw renderer fallback
- any missing required pair or state fixture without a waiver

Dynamic masking rules:

- only relative timestamps, versions, digests, animated spinners, and live log text
- use checked-in selectors or `data-visual-dynamic`
- non-allowlisted selectors must fail the parity harness
- mask area budget is computed from union area so overlapping masks are not double-counted
- masked area max `15%` unless narrowed by waiver

Current rollout status:

- the pair/state matrix above remains the required target contract
- paired dark+light coverage is still incomplete for some required pairs, so parity closure is
  not complete yet
- removed built-in-only audit/profile captures are intentionally excluded and do not count as
  required built-in-vs-surface parity coverage

---

## 4. Components

### 4.1 Badges

**Status:** `Implemented`

- `14px` tall
- `2px` radius
- `7.5px` bold uppercase
- 1px border
- always include background tint plus border

Use semantic variants for success, info/update, warning, error, dim, and violet interactive-attention.

Clickable badges:

- use hover text swap inside a fixed-width container
- no layout reflow on hover
- hover state also increases badge background and border opacity
- violet and dim badges are static-only, not hover-swap

### 4.2 Pills

**Status:** `Implemented`

- `12px` tall
- `2px` radius
- `7px` bold uppercase
- no border

Used for categorical labels such as agent type, OS, and plugin type.

### 4.3 Buttons

**Status:** `Implemented`

- standard height `23px`
- `3px` radius
- `9px` bold text
- primary, ghost, and danger variants
- disabled uses `opacity: 0.4` plus `pointer-events: none`

`↑ Update all` is a badge-like interactive control, not a standard page-level button.

### 4.4 Toggles

**Status:** `Implemented`

- track `28×15px`
- thumb `11×11px`
- on state uses accent tint and accent border
- disabled uses `opacity: 0.4` plus `pointer-events: none`

### 4.5 Stat Cards

**Status:** `Implemented`

- `3px` radius
- `--bg-surface` background
- `--border-subtle` border
- label `7.5px` uppercase
- value `14px` bold

State-value colors:

- healthy: success
- attention/updates: info
- error: error
- offline/unknown: muted

### 4.6 Loading States

**Status:** `Implemented`

Three patterns:

- skeleton placeholders for known shapes
- spinner for user-triggered or item-scoped in-flight actions
- indeterminate top loading bar for page-level navigation or polling

Use centered `Loading...` copy only when layout is intentionally unconstrained.

### 4.7 Empty States

**Status:** `Implemented`

Structure:

- `32×32` neutral icon
- `13px` bold title
- `11px` secondary description
- optional ghost action button

Variants:

- global empty
- filtered empty

### 4.8 Toasts

**Status:** `Implemented` for desktop and tablet  
**Status:** `Target` for mobile repositioning and swipe-down behavior

Implemented behavior:

- top-right stack
- `300px` width
- `16px` top/right offset
- `6px` gap
- click body to dismiss
- close button
- tablet swipe-right dismiss
- auto-dismiss: 4s success/info, 8s warning/error
- hover pauses timer and progress bar
- `2px` bottom progress bar

Target mobile behavior:

- bottom-center positioning
- swipe-down dismiss
- promotes together with Section 7

### 4.9 Confirmation Dialogs

**Status:** `Implemented`

- centered modal
- backdrop `rgba(0,0,0,.55)`
- `380px` width
- `4px` radius
- right-aligned cancel + confirm
- confirm button uses the danger variant
- close on backdrop click or `Escape`

### 4.10 Form Validation

**Status:** `Implemented`

Default field rules:

- inputs/selects `32px` high
- textarea `72px` minimum
- background `--bg-surface`
- border `--border-default`
- label width `110px`

Validation rules:

- inline errors below fields
- error state uses error border and error background tint
- focus uses accent ring
- error-state fields keep error border and also gain accent focus ring
- success state is optional and only where meaningful

### 4.11 Tab Strip

**Status:** `Implemented`

- tab height `28px`
- horizontal padding `10px`
- `3px` radius
- text `10px`, `600`
- active uses accent tint plus accent text
- hover uses raised background plus primary text
- horizontal scroll on narrow widths

`host_detail.tabs` is currently not a tab strip; it remains an inline card stack.

### 4.12 Data Tables

**Status:** `Implemented`

- header row `28px`
- body row min `32px`
- cell horizontal padding `10px`
- header background `--bg-raised`
- header text `9px` uppercase muted
- body text `10px`
- row hover `--bg-raised`
- mobile fallback is card-stack layout

### 4.13 Context Menus

**Status:** `Implemented`

- `--bg-surface` background
- `--border-default` border
- `4px` radius
- menu row `32px`
- horizontal padding `12px`
- item text `10px`
- hover fill `--bg-raised`
- destructive items use error text token

`software_item.host_context_menu` contributes launcher entries, not grouped nested menus.

### 4.14 Workflow / Wizard Shell

**Status:** `Implemented`

- uses the standard modal shell
- explicit step indicator row
- step chips `18px` tall
- completed = success
- active = accent
- upcoming = raised background plus secondary text

### 4.15 Shared Surface Primitives

**Status:** `Implemented`

Current primitive mappings:

| Primitive | Design treatment |
| --- | --- |
| `Section` | vertical stack with `16px` gap |
| `TextBlock` | standard body copy |
| `KeyValue` | same label/value rhythm as settings/detail views |
| `Table` | canonical data-table treatment |
| `Form` | same form validation and field layout as built-in forms |
| `ActionBar` | right-aligned action row |
| `Tabs` | canonical tab strip |
| `Callout` | semantic info/warning/danger |
| `EmptyState` | canonical empty state |
| `ModalTrigger` | standard modal shell |
| `WorkflowTrigger` | standard workflow shell |

No surface-only visual widgets are allowed.

### 4.15.1 Interaction Label Contract

**Status:** `Implemented`

- shared-surface actions must provide a non-empty human-authored `interaction.label`
- workflow steps must provide a non-empty human-authored `workflow_step.label`
- shared runtime components must not synthesize generic fallback copy such as `Run action`,
  `Run workflow`, `Step`, `Open details`, or `Details`
- malformed unlabeled interactions must degrade to the shared `Action unavailable` callout instead
  of rendering actionable UI

### 4.16 Shared Surface Runtime States

**Status:** `Implemented`

Canonical state IDs:

- `loading`
- `permission_denied`
- `no_compatible_provider`
- `contract_mismatch`
- `hydration_action_failure`
- `no_surface_content`

State rules:

- `loading`: skeletons where shape is known
- `permission_denied`: empty-state or callout explanation
- `no_compatible_provider`: shared empty-state body, not toast
- `contract_mismatch`: warning callout
- `hydration_action_failure`: inline error callout, keep layout intact
- `no_surface_content`: structural slots stay structural, non-structural slots omit themselves

---

## 5. Page Patterns

### 5.1 Software Page

**Status:** `Implemented`

Rules:

- software items are top-level groups
- hosts are sub-rows
- built-in and surface-backed `software.tabs` share one tab strip and one body
- active tab persists in `?tab=<tab-id>`
- column grid is `16px 1fr 120px 88px`
- header-row version column is always empty
- version column is a two-line installed/latest stack on host rows
- host-row background is transparent until hover
- truncation row uses `▸ N more`
- built-in Software route overlays use shared owners:
  `AddSoftwareModal.svelte`, `AssignToHostModal.svelte`,
  `EditHostAssignmentModal.svelte`, and `SoftwareMergeWizard.svelte`

### 5.2 Hosts Page

**Status:** `Implemented`

Rules:

- standard table layout
- software-status badge uses navigable badge pattern
- `N updates` navigates to Software
- `X error` navigates to History
- `Up to date` and `Unknown` are static

### 5.3 History Page

**Status:** `Implemented`

Rules:

- chronological feed grouped by date
- icon square + body + right meta
- row-level "view log" actions open the shared terminal modal from Section 6
- waiting/no-output, truncation, recovery, and actor details render as terminal callouts inside
  the modal
- interactive sessions expose live controls (for example `Ctrl+C`) inside terminal status actions

### 5.4 Settings Page

**Status:** `Implemented`

Rules:

- built-in settings sections and `settings.tabs` share one tab strip
- active tab persists in `?tab=<tab-id>`
- form-heavy views use `110px` label width
- destructive actions live in a danger zone
- `settings.below.global` renders below built-in global settings content

### 5.5 Slot-Backed Detail Panels

**Status:** `Implemented`

Rules:

- `host_detail.tabs` is an inline card stack
- `settings.below.global` is an inline panel stack
- targeted surfaces keep selector inside host-owned panel chrome, above rendered nodes
- no-provider uses shared empty-state treatment
- parity capture regions should use stable host markers such as `data-parity-region`

---

## 6. Terminal Output Shell

**Status:** `Implemented`

Canonical implementation:

- one shared terminal-shell component for History and Software Detail
- legacy inline-only shell styling is no longer the primary UX
- close resets shell maximize state
- live/captured mode can change after mount; stdin wiring follows `onInput` dynamically
- parity snapshots for final shell stay at `<= 0.5%`
- desktop parity captures titlebar and status-bar chrome regions
- mobile parity captures full-screen shell plus titlebar and status-bar regions

Terminal rules:

- centered modal over `rgba(0,0,0,.78)`
- default size `580px × 380px`
- titlebar `36px`, status bar `28px`
- default radius `6px`
- maximize to `92vw × 88vh`, radius `4px`
- mobile becomes full-screen with no maximize affordance

Traffic lights:

- red closes
- yellow is visible but disabled
- green toggles maximize
- hover any dot reveals all three icons

Close paths:

- red button
- `Escape`
- backdrop click

Title format:

- `<software-name> on <hostname>`

Status bar format:

- badge on the left
- metadata on the right
- `<hostname> · started <relative-time> · <duration>`

---

## 7. Responsive Layout

**Status:** `Target`

Breakpoints:

| Breakpoint | Range | Layout |
| --- | --- | --- |
| Desktop | `>= 1024px` | full sidebar |
| Tablet | `640–1023px` | overlay sidebar |
| Mobile | `< 640px` | bottom navigation |

Target rules:

- tablet sidebar becomes overlay drawer
- mobile bottom nav shows the 4 highest-priority top-level nav items
- overflow moves into a shared bottom sheet
- built-in and `surface.page` entries use the same sort rules everywhere
- mobile software rows expand inline, not via separate views
- mobile toasts move to bottom-center and swipe-down dismiss promotes with the section

Built-in nav priorities:

| Item | Priority |
| --- | --- |
| Home | `100` |
| Services | `200` |
| System Services | `300` |
| Hosts | `400` |
| Tags | `450` |
| Software | `500` |
| History | `800` |
| Audit Logs | `900` |
| Settings | `1000` |

---

## 8. Interaction Conventions

**Status:** `Implemented`

- no layout reflow on hover
- clickable badge-style controls use the same flat hover treatment
- only the explicitly allowed motion categories from Section 2.5 are permitted
- dim means disabled, not hidden; disable interaction with `pointer-events: none` where required
- destructive actions always use danger treatment plus confirmation
- focus rings appear on `:focus-visible` only

---

## 9. Verification And Waivers

### Verification

Markdown verification for this guide and its catalogues:

```bash
markdownlint --config .markdownlint.json \
  docs/development/ui-design-language.md \
  docs/development/README.md \
  docs/README.md
```

Design-language verification also requires:

- deterministic parity fixtures for every new shared visual path
- required built-in vs surface-backed parity pairs
- required slot-state matrices
- dark and light theme coverage for required pairs
- parity closure stays open while any required pair is missing paired dark/light captures
- removed built-in-only captures (such as prior audit/profile parity captures) do not count
- adapter-manifest completeness via checked-in manifest test
- parity harness enforcement from `frontend/tests/e2e/parity-config.ts`:
  Chromium project guard, fixed locale (`en-US`), fixed timezone (`UTC`), reduced-motion capture,
  DPR `1`, and viewport preset checks
- parity mask selector allowlist enforcement (default `data-visual-dynamic`) plus mask-area
  union-budget enforcement

### Waiver Schema

**Status:** `Implemented`

Governance file:

- `docs/superpowers/ui-parity-waivers.json`

Every waiver entry must include:

- `scope`
- `owner`
- `expiry_date`
- `capture_region`
- `justification`
- `review_ref`

Example shape:

```json
[
  {
    "scope": "host_context_modal/mobile",
    "owner": "frontend-owner",
    "expiry_date": "2026-06-30",
    "capture_region": "[data-parity-region='host-context-modal']",
    "justification": "Temporary mobile overflow mismatch during shell convergence.",
    "review_ref": "PR-1234"
  }
]
```

Waiver rules:

- waivers are exception paths, not shortcuts
- keep them scoped to one issue
- time-limit them
- link them to review evidence
- the approved spec remains the final authority

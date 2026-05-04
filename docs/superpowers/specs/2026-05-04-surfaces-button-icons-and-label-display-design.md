# Surfaces Button Icons & Label Display — Design

Date: 2026-05-04
Status: Draft (awaiting user review)
Owners: Frontend (Svelte) + shared/wire (Rust) + plugin authors
Supersedes (in part): `2026-05-02-icon-rollout-design.md` — extends the icon
allowlist with action icons and unifies the registry around lucide canonical
kebab-case identifiers.

---

## Goal

Two coupled improvements to the Surfaces framework:

1. **Action icons end-to-end.** Plugin authors can declare a lucide icon
   alongside a label on any Surface action. The icon flows from
   `SurfaceActionDescriptor` (legacy plugin-authoring builder) and
   `surfaces::InteractionDescriptor` (wire) through to the rendered Button.
2. **Adaptive label visibility.** A Button can hide its label automatically
   based on container width and font scaling, or be permanently icon-only.
   Surface action bars opt into auto-collapse; DataTable row actions default
   to icon-only when an icon is set, so several actions fit on one row.

The first concrete consumers are the **Proxmox VE Hosts** and **SSH Hosts**
surfaces, which currently render text-only buttons that wrap awkwardly inside
DataTable rows.

---

## Non-Goals

These are explicitly out of scope and deferred:

- **Workflow step icons.** `WorkflowStepDescriptor.label` already renders
  buttons per step, but no step needs an icon today. Add later if asked.
- **Sweeping non-surface buttons across the app.** The new surface-layer
  component `SurfaceActionButton` gains `labelDisplay` (see § 4);
  `Button.svelte` is unchanged; `EnrollmentTokenSettings`, `+layout.svelte`,
  etc. are not retrofitted.
- **A new tooltip component.** Hidden labels reuse the existing native
  `title` attribute pattern already used in `SurfaceInteractionButton.svelte`.
- **JS-driven dynamic label measurement.** Container queries and font scaling
  cover the cases described; no `ResizeObserver` orchestration.
- **Replacing the `Box` fallback for nav icons.** Behaviour changes from
  silent fallback to logged fallback, but the placeholder stays `Box`.

---

## Background — current state

### Wire (Rust)

- `crates/shared/surfaces/src/interaction.rs` defines `InteractionDescriptor`
  with `label: String` and no icon field.
- `crates/shared/surfaces/src/surface.rs` already has
  `SurfaceDescriptor.nav_icon: Option<String>` (PascalCase today, e.g.
  `"Package"`).
- `crates/plugins/infrastructure/core/src/surface_form_authoring.rs` defines
  `SurfaceActionDescriptor` (the builder plugins call) with `label: String`
  and no icon field.
- `crates/shared/wire/src/wire_validate_impls.rs` validates `nav_icon`:
  non-empty, max 64 chars (`MAX_NAV_ICON_LEN`). No charset check.

### Frontend

- `frontend/src/lib/components/Button.svelte` already supports `leadingIcon`
  and `trailingIcon` Snippets, plus `ariaLabel`. No label-visibility prop.
- `frontend/src/lib/components/surfaces/SurfaceInteractionButton.svelte`
  passes only `interaction.label`; nothing else flows to Button.
- `frontend/src/lib/components/surfaces/SurfaceActionBar.svelte` wraps
  buttons in `flex flex-wrap justify-end gap-2`.
- `frontend/src/lib/components/surfaces/SurfaceTable.svelte` row actions wrap
  in `flex flex-wrap gap-2` (entity-link path line ~261; `rowActions` snippet
  path line ~311).
- `frontend/src/lib/nav-icons.ts` is a small registry mapping PascalCase keys
  (`Box`, `Package`, ...) to lucide-svelte components, fallback `Box`, no
  logging on miss.
- `lucide-svelte` is the only icon library in `package.json`.

### Surfaces affected

- `proxmox.hosts` — Proxmox VE Hosts (`crates/plugins/infrastructure/proxmox/src/plugin.rs::proxmox_hosts_surface`).
  Action-bar actions: `discover`, `test-connection`. Row actions:
  `approve-match`, `match`, `unmatch`. Authored as
  `surfaces::InteractionDescriptor` literals (not via `SurfaceActionDescriptor`).
- `ssh-agent.hosts` — SSH Hosts
  (`crates/core/agent-ssh/src/surface_runtime.rs::build_actions`).
  Action-bar action: `bootstrap`. Row actions: `sync-host`, `remove-host`.
  Authored via `SurfaceActionDescriptor`.
- `bootstrap-proxmox-guest` action contributed by
  `crates/plugins/infrastructure/proxmox/src/agent/plugin.rs::bootstrap_proxmox_guest_action`,
  joined into the `ssh-agent.hosts` action bar via the infra registry.
  Authored via `SurfaceActionDescriptor`.

---

## Design overview

```text
┌──────────────────────────────────────────────────────────────────────┐
│ Plugin authoring (Rust)                                              │
│                                                                      │
│   SurfaceActionDescriptor::new(id, label).with_icon("trash-2")  ─┐   │
│                                                                  │   │
│   InteractionDescriptor { label, icon: Some("trash-2"), ... } ◀──┘   │
│                                                                      │
└──────────────────────────────┬───────────────────────────────────────┘
                               │   wire (validated kebab-case ≤ 64 chars)
                               ▼
┌──────────────────────────────────────────────────────────────────────┐
│ Frontend (Svelte 5)                                                  │
│                                                                      │
│   contract.ts          : InteractionDescriptor.icon?: string         │
│                                                                      │
│   icons.ts (NEW)       : kebab-case → lucide component               │
│                          resolveIcon(name) → { component, ok }       │
│                          (replaces nav-icons.ts)                     │
│                                                                      │
│   surfaces/label-display.ts (NEW)                                    │
│                          : LabelDisplay = 'always' | 'auto' |        │
│                            'icon-only'                               │
│                                                                      │
│   Button.svelte        : UNCHANGED                                   │
│                                                                      │
│   SurfaceActionButton (NEW)                                          │
│                        : owns icon resolve + effective-display +     │
│                          label-span (button-label-auto / sr-only) +  │
│                          <span title=…> tooltip wrap. Renders Button │
│                                                                      │
│   SurfaceInteractionButton  : delegates to SurfaceActionButton       │
│                               (active + context-gated branches)      │
│                                                                      │
│   SurfaceActionBar     : @container/buttons + labelDisplay='auto'    │
│                          when icon present                           │
│                                                                      │
│   SurfaceTable         : @container/buttons + labelDisplay=          │
│   (row actions)          'icon-only' when icon present;              │
│                          flex-nowrap, single line                    │
│                                                                      │
│   SurfaceWorkflow      : delegates trigger to SurfaceActionButton    │
│                                                                      │
└──────────────────────────────────────────────────────────────────────┘
```

---

## Detailed design

### 1. Wire field: `icon: Option<String>`

Add to two structs:

`crates/shared/surfaces/src/interaction.rs`:

```rust
pub struct InteractionDescriptor {
    pub interaction_id: InteractionId,
    pub kind: InteractionKind,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    // ... existing fields unchanged
}
```

`crates/plugins/infrastructure/core/src/surface_form_authoring.rs`:

```rust
pub struct SurfaceActionDescriptor {
    pub action_id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    // ... existing fields unchanged
}

impl SurfaceActionDescriptor {
    pub fn with_icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
    }
}
```

The action-descriptor → interaction-descriptor conversion happens inside
`crates/core/agent-ssh/src/surface_runtime.rs` (search for the existing
mapping that builds `InteractionDescriptor` values from
`SurfaceActionDescriptor` entries before assembling the
`SurfaceRegistration`). Carry `icon` through that mapping. The contract is:
`SurfaceActionDescriptor.icon` ⇒ `InteractionDescriptor.icon`.

Both descriptors are already `#[non_exhaustive]`. No wire enum change. No
`Other(String)` pattern (icon is `Option<String>` on a struct, not an enum
variant).

### 2. Validation

The validation helper lives in the `surfaces` crate (upstream of `wire`)
so both layers can share a single implementation.

`crates/shared/surfaces/src/lib.rs` (or a new `validation.rs` submodule):

```rust
pub const MAX_ICON_NAME_LEN: usize = 64;

#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IconNameError {
    #[error("icon name must not be empty")]
    Empty,
    #[error("icon name exceeds {MAX_ICON_NAME_LEN} characters")]
    TooLong,
    #[error("icon name must match lucide kebab-case (lowercase letters, digits, hyphens)")]
    InvalidFormat,
}

pub fn validate_icon_name(name: &str) -> Result<(), IconNameError> {
    if name.is_empty() {
        return Err(IconNameError::Empty);
    }
    if name.len() > MAX_ICON_NAME_LEN {
        return Err(IconNameError::TooLong);
    }
    // ^[a-z][a-z0-9-]*[a-z0-9]$ — at least 2 chars, kebab-case, no leading/trailing dash.
    // Hand-rolled to avoid pulling in `regex` for a four-character class.
    // Note: single-char names like "x" are rejected; lucide does not ship any.
    let bytes = name.as_bytes();
    let [first, middle @ .., last] = bytes else {
        return Err(IconNameError::InvalidFormat);
    };
    if !first.is_ascii_lowercase() {
        return Err(IconNameError::InvalidFormat);
    }
    if !(last.is_ascii_lowercase() || last.is_ascii_digit()) {
        return Err(IconNameError::InvalidFormat);
    }
    for &b in middle {
        let ok = b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-';
        if !ok {
            return Err(IconNameError::InvalidFormat);
        }
    }
    Ok(())
}
```

Notes for the implementation:

- `IconNameError` is `#[non_exhaustive]` per the project's enum-extensibility rule
  (`docs/development/coding-standards.md#Public Enum Extensibility`).
- The slice pattern `[first, middle @ .., last]` covers the empty- and
  single-element cases via the destructuring `else`, satisfying clippy's
  `unwrap_used = deny` and `indexing_slicing = deny` lints without an
  `#[expect(...)]` escape.

`crates/shared/wire/src/limits.rs` and
`crates/shared/wire/src/wire_validate_impls.rs`:

- Delete the `MAX_NAV_ICON_LEN` constant from `limits.rs` (currently ≈ line
  199). The `surfaces::MAX_ICON_NAME_LEN` constant introduced above is the
  single source of truth; wire callers reference it directly. This avoids
  parallel constants in two crates.
- In `wire_validate_impls.rs`, replace the `nav_icon` non-empty + length
  checks with a call to `surfaces::validate_icon_name`, mapping
  `IconNameError` to the existing wire `ValidationError { field, .. }`
  shape. Field names stay `"surfaces[].descriptor.nav_icon"` and gain
  `"surfaces[].interactions[].icon"`.
- Iterate every `interactions[].icon` on `SurfaceRegistration` and run
  the same validator when present.

Plugin authors get a wire validation error during enrollment if they typo
an icon name. Runtime `console.error` in the dashboard (§ 3) is the
second line of defence, not the first.

### 3. `frontend/src/lib/icons.ts` (replaces `nav-icons.ts`)

Single unified registry, kebab-case keys (lucide canonical):

```ts
import type { ComponentType, SvelteComponent } from "svelte";
import {
  Box,
  Boxes,
  Check,
  Cpu,
  Database,
  FileText,
  Globe,
  HardDrive,
  History,
  Layers,
  Link,
  Package,
  PlugZap,
  Puzzle,
  Radar,
  RefreshCw,
  ScrollText,
  Server,
  ServerCog,
  Settings,
  Shield,
  Tag,
  Tags,
  Trash2,
  Unlink,
  Wrench,
} from "lucide-svelte";

export type IconComponent = ComponentType<SvelteComponent>;

export const ICONS: Record<string, IconComponent> = {
  box: Box,
  boxes: Boxes,
  check: Check,
  cpu: Cpu,
  database: Database,
  "file-text": FileText,
  globe: Globe,
  "hard-drive": HardDrive,
  history: History,
  layers: Layers,
  link: Link,
  package: Package,
  "plug-zap": PlugZap,
  puzzle: Puzzle,
  radar: Radar,
  "refresh-cw": RefreshCw,
  "scroll-text": ScrollText,
  server: Server,
  "server-cog": ServerCog,
  settings: Settings,
  shield: Shield,
  tag: Tag,
  tags: Tags,
  "trash-2": Trash2,
  unlink: Unlink,
  wrench: Wrench,
};

export interface ResolvedIcon {
  component: IconComponent;
  ok: boolean;
}

const FALLBACK: IconComponent = Box;

export function resolveIcon(name: string | null | undefined): ResolvedIcon {
  if (!name) {
    return { component: FALLBACK, ok: false };
  }
  const component = ICONS[name];
  if (!component) {
    console.error(`[surfaces] Unknown icon name: "${name}"`);
    return { component: FALLBACK, ok: false };
  }
  return { component, ok: true };
}
```

`lucide-svelte` v1.0.1 ships its icons as Svelte 4-style class components
(`class Icon extends SvelteComponentTyped<IconProps, ...>`), so the runtime
type is `ComponentType<SvelteComponent>`. This matches the pattern already
used elsewhere in the codebase (`EmptyState.svelte`, the file being
replaced) and works under `tsconfig.json`'s `strict: true`. The Svelte 5
function-component type `Component<…>` is not what lucide currently exports
and would fail strict assignability if used here.

`nav-icons.ts` and `nav-icons.test.ts` are deleted. The single consumer of
`nav_icon` (`frontend/src/lib/surfaces/registry.svelte.ts:55`) switches to
`resolveIcon` and to the kebab-case fallback `'box'`. Existing built-in nav
items in `+layout.svelte` (which import lucide components directly, not
through the registry) are unchanged.

The previous PascalCase keys from `2026-05-02-icon-rollout-design.md` (e.g.
`"Package"`, `"Server"`) are migrated to kebab-case (`"package"`,
`"server"`). At time of writing, no producer in the Rust workspace calls
`SurfaceDescriptor::nav_icon(...)`, so the producer-side migration cost is
zero. `registry.svelte.ts` and the spec example in `registry.test.ts` are
the only call sites updated.

### 4. `labelDisplay` lives in the Surface layer, not in `Button`

`Button.svelte` has ~40 consumers across the dashboard, only four of which
are surface-related. Adding a `labelDisplay` prop to the base component
would (a) widen the public API for every consumer, (b) create a copy-paste
footgun where developers move a `labelDisplay='auto'` button outside an
`@container/buttons` ancestor and silently lose the collapse behaviour,
and (c) couple Button to the surfaces' `aria-label` invariant without a
type-level enforcement. So the prop and its behaviour live in the
surface-layer wrappers (`SurfaceInteractionButton.svelte` and
`SurfaceWorkflow.svelte`) and `Button.svelte` is **left unchanged**.

The shared type lives in the surfaces frontend module
(`frontend/src/lib/surfaces/label-display.ts`):

```ts
export const LabelDisplay = {
  Always: "always",
  Auto: "auto",
  IconOnly: "icon-only",
} as const;
export type LabelDisplay = (typeof LabelDisplay)[keyof typeof LabelDisplay];
```

A shared component **`frontend/src/lib/components/surfaces/SurfaceActionButton.svelte`**
(NEW) owns the icon + label-display rendering for every surface-rendered
button. Both `SurfaceInteractionButton` (the non-workflow active path and
the context-gated disabled path) and `SurfaceWorkflow` (the trigger button)
delegate to it. This keeps the icon-resolve / effective-display /
label-span / tooltip-wrap logic in a single place and prevents drift.

`SurfaceActionButton` accepts these props (informative — final shape lives
in the implementation):

```ts
let {
  label,
  icon, // string | null | undefined
  labelDisplay = "always",
  variant, // ButtonVariant from Button.svelte
  size = "md", // ButtonSize from Button.svelte
  loading = false,
  disabled = false,
  onclick,
  dataUi,
}: {
  label: string;
  icon?: string | null;
  labelDisplay?: LabelDisplay;
  variant: ButtonVariant;
  size?: ButtonSize;
  loading?: boolean;
  disabled?: boolean;
  onclick?: () => void;
  dataUi?: string;
} = $props();
```

Internal logic:

- Resolve the icon component once via `resolveIcon(icon)`.
- Compute the **effective** display: if `icon` is unset or the resolver
  returned `ok: false`, force `'always'`. Otherwise use the prop value.
  Never request a hidden label when there is no icon.
- For `'always'`: pass the label as Button's `children` snippet.
- For `'auto'`: render the label as
  `<span class="button-label-auto">{label}</span>` inside Button's
  `children` snippet. The browser tooltip is supplied by the wrapping
  span (next bullet). Pass `ariaLabel={label}` to Button.
- For `'icon-only'`: render the label as
  `<span class="sr-only">{label}</span>` inside Button's `children` snippet.
  Pass `ariaLabel={label}` to Button.
- When `effectiveDisplay !== 'always'` **and** `disabled === false`, wrap
  the Button in `<span title={label} class="contents">…</span>` so the
  browser tooltip appears on hover. The `class="contents"` keeps the
  wrapper invisible to layout. This mirrors the existing context-gated
  wrapper at `SurfaceInteractionButton.svelte:102`.
- When `disabled === true`, **do not** add a `title` wrapper.
  `SurfaceInteractionButton`'s context-gated branch keeps its outer
  `<span title="Select a configuration first">` to surface the gate
  reason; sighted users hovering a disabled icon-only button see the gate
  message rather than a redundant action label. Screen readers still get
  `aria-label={label}` on the Button.
- When the resolver returned `ok === true`, pass the icon component into
  Button's `leadingIcon` snippet:

  ```svelte
  {#snippet leadingIcon()}
      <Component size={size === 'sm' ? 14 : 16} aria-hidden="true" />
  {/snippet}
  ```

Button keeps its current API surface — no new prop, no new degrade logic,
no dev-mode warning. `SurfaceActionButton` is the single owner of the
contract on the surface side.

Why both `aria-label` and `title`? `aria-label` is what screen readers
announce; `title` is the visual hover tooltip. Browsers do not announce
`title` consistently and sighted users do not see `aria-label`. Both are
necessary when the label text is hidden.

### 5. Container query for `'auto'`

Tailwind v4 (the project runs `@tailwindcss/vite` v4.2.4) ships container
queries natively — the `@tailwindcss/container-queries` plugin is **not**
needed and must not be added.

The implementation uses Tailwind v4 utilities exclusively — no hand-rolled
`.button-cluster` class is added to `app.css`. The Tailwind variant
`@container/buttons` on the wrapper element generates `container-type:
inline-size` + `container-name: buttons` inline; the `@max-[28em]/buttons:sr-only`
variant on the label `<span>` generates the container-query rule. The
following CSS block is the **contract** for what those utilities must
produce — it documents the intended output and the rationale, but is not
itself written into any stylesheet:

```css
/* Generated by Tailwind from `@container/buttons` on the wrapper element. */
.wrapper-with-@container\/buttons {
  container-type: inline-size;
  container-name: buttons;
}

/* Generated by Tailwind from `@max-[28em]/buttons:sr-only` on the label span.
   The CSS Containment spec rejects `var()` inside `@container` query
   conditions, so the `28em` is a literal — Tailwind's bracket-arbitrary-value
   syntax preserves the literal end-to-end. */
@container buttons (max-width: 28em) {
  .button-label-auto {
    position: absolute;
    width: 1px;
    height: 1px;
    padding: 0;
    margin: -1px;
    overflow: hidden;
    clip: rect(0, 0, 0, 0);
    white-space: nowrap;
    border-width: 0;
  }
}
```

The `28em` literal lives in exactly one place — the
`@max-[28em]/buttons:sr-only` utility class on the label span. Duplicating
it would defeat the single-source goal.

`@container/buttons` parent class is added to:

- `SurfaceActionBar.svelte` outer `<div>` (currently
  `flex flex-wrap justify-end gap-2`).
- `SurfaceTable.svelte` row-actions `<div>` in both branches:
  - entity-link path (line ~261)
  - rowActions snippet path (line ~311)
- `SurfaceWorkflow.svelte` trigger wrapper.

Em-based threshold (not px) means user font scaling collapses the labels
proportionally — large-text users see icon-only mode at the same content
width.

### 6. `SurfaceTable.svelte` — row layout

Two row-action call sites need the same shape, but they currently differ:

- **Entity-link path** (line ~261) already wraps row actions in
  `<div class="flex flex-wrap gap-2">`. Change it to
  `<div class="@container/buttons flex flex-nowrap items-center gap-1">`.
- **`rowActions` snippet path** (line ~311) emits buttons directly into
  the snippet without a wrapping `<div>` — the `DataTable` slot consumes
  whatever the snippet emits. Add a wrapping
  `<div class="@container/buttons flex flex-nowrap items-center gap-1">`
  around the buttons inside the snippet body.

Cell `<td class="table-cell-pad">` adds `whitespace-nowrap` so the column
width grows to fit. The existing `overflow-x-auto` wrapper handles
horizontal scrolling on narrow viewports.

This satisfies "buttons inside DataTable should render on the same line —
they'll be short" because they default to icon-only when an icon is set
(see § 7 below).

### 7. `SurfaceInteractionButton.svelte`

New optional prop:

```ts
let {
  // ... existing
  labelDisplay = "always",
}: {
  // ... existing
  labelDisplay?: LabelDisplay;
} = $props();
```

Both render branches (active and context-gated) replace their
`<Button …>{actionLabel}</Button>` with a `<SurfaceActionButton …>`
delegating call:

```svelte
<SurfaceActionButton
    label={actionLabel}
    icon={interaction.icon}
    {labelDisplay}
    variant={interaction.confirmation?.severity === 'danger' ? 'danger' : 'primary'}
    {size}
    {loading}
    disabled={isContextGated}
    onclick={isContextGated ? undefined : requestAction}
/>
```

The context-gated branch keeps its existing
`<span title="Select a configuration first">` outer wrapper, with
`SurfaceActionButton` rendered inside it; `SurfaceActionButton`'s own
tooltip-wrap is suppressed when `disabled` is true (the outer span owns
the message). The icon-only/auto/always rendering and the
`title={actionLabel}` tooltip wrapping live entirely in
`SurfaceActionButton` — `SurfaceInteractionButton` only forwards data.

### 8. `SurfaceActionBar.svelte`

For each child (`SurfaceInteractionButton` for non-workflow interactions,
`SurfaceWorkflow` for workflow interactions — both components declare the
`labelDisplay?: LabelDisplay` prop per § 7 and § 10), pass:

```ts
labelDisplay={interaction.icon ? 'auto' : 'always'}
```

Add `@container/buttons` class to the outer `<div>`. Existing
`flex flex-wrap` is kept (multiple buttons may wrap to a second row on
narrow surfaces; that's fine for action bars).

### 9. `SurfaceTable.svelte` row actions — labelDisplay

For each child (`SurfaceInteractionButton` for non-workflow interactions,
`SurfaceWorkflow` for workflow interactions), in both row-action branches
(entity-link path and `rowActions` snippet path), pass:

```ts
labelDisplay={interaction.icon ? 'icon-only' : 'always'}
```

This is the "DataTable buttons hide text by default when an icon is set"
requirement.

### 10. `SurfaceWorkflow.svelte`

`SurfaceInteractionButton.svelte` delegates the `kind: 'workflow'` case to
`<SurfaceWorkflow />` (see lines 89–99 of the existing component). The
workflow trigger button is rendered **inside `SurfaceWorkflow`** (around
line 339), not by `SurfaceInteractionButton`. Icon and `labelDisplay`
threading must therefore happen in `SurfaceWorkflow` directly:

- Accept a `labelDisplay?: LabelDisplay` prop on `SurfaceWorkflow` (default
  `'always'`); the parent (`SurfaceActionBar` or `SurfaceTable` row actions)
  passes the same value it would have passed to a non-workflow
  `SurfaceInteractionButton`.
- Replace the existing `<Button …>{label}</Button>` trigger render with a
  `<SurfaceActionButton …>` call mirroring § 7. The icon, label-span,
  effective-display, and tooltip-wrap logic lives in
  `SurfaceActionButton` — there is no separate copy here.

Step-internal buttons (currently text-only for "Next", "Back") are out of
scope per § Non-Goals.

### 11. Icon assignments

Wired up via `with_icon(...)` builder calls or struct-literal field on the
existing surface definitions. No new actions, no renamed actions.

| Surface           | Action                    | Icon         | Site                                                                         |
| ----------------- | ------------------------- | ------------ | ---------------------------------------------------------------------------- |
| Proxmox VE Hosts  | `discover`                | `radar`      | `crates/plugins/infrastructure/proxmox/src/plugin.rs::proxmox_hosts_surface` |
| Proxmox VE Hosts  | `test-connection`         | `plug-zap`   | same                                                                         |
| Proxmox VE Hosts  | `approve-match`           | `check`      | same                                                                         |
| Proxmox VE Hosts  | `match`                   | `link`       | same                                                                         |
| Proxmox VE Hosts  | `unmatch`                 | `unlink`     | same                                                                         |
| SSH Hosts         | `bootstrap`               | `server-cog` | `crates/core/agent-ssh/src/surface_runtime.rs::build_actions`                |
| SSH Hosts         | `sync-host`               | `refresh-cw` | same                                                                         |
| SSH Hosts         | `remove-host`             | `trash-2`    | same                                                                         |
| SSH Hosts (infra) | `bootstrap-proxmox-guest` | `boxes`      | `crates/plugins/infrastructure/proxmox/src/agent/plugin.rs`                  |

The Proxmox VE Hosts surface authors interactions as
`surfaces::InteractionDescriptor` struct literals — set the `icon` field
directly. SSH Hosts and the infra Proxmox guest action use
`SurfaceActionDescriptor::with_icon(...)`.

The `list` (DataLoad) interactions stay icon-less; they're not rendered as
buttons.

### 12. Documentation

Add an "Action icons" section to `docs/development/plugin-guidelines.md`:

- **Identifier scheme.** Lucide canonical kebab-case (`refresh-cw`,
  `trash-2`). Wire validation enforces `^[a-z][a-z0-9-]*[a-z0-9]$`,
  ≤ 64 chars.
- **Source.** Icons come from `lucide-svelte`. A curated allowlist lives at
  `frontend/src/lib/icons.ts`.
- **Adding a new icon.** Single-PR change to `frontend/src/lib/icons.ts`:
  import the lucide component, add the kebab-case key. No wire-crate
  release needed; the field is `Option<String>`.
- **Authoring example.**

  ```rust
  SurfaceActionDescriptor::new("sync-host", "Sync")
      .with_icon("refresh-cw")
      .with_permission(Permission::UpdateHosts.to_string())
  ```

- **Where icons render.** Action-bar buttons use `'auto'` label display
  (collapse on narrow containers). DataTable row-action buttons use
  `'icon-only'` (default-hide when an icon is set). Workflow triggers
  inherit from their parent context.

Update `docs/superpowers/specs/2026-05-02-icon-rollout-design.md` with a
banner pointing to this spec for the kebab-case migration. (Don't rewrite
its body — historical context.)

---

## Testing

### Frontend (vitest + @testing-library/svelte)

| File                                           | Cases                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| ---------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `Button.test.ts`                               | No changes — `Button.svelte` is unchanged.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| `icons.test.ts` (replaces `nav-icons.test.ts`) | `resolveIcon('box')` → `{ component: Box, ok: true }`. `resolveIcon('Trash2')` → `{ component: Box, ok: false }` + `console.error` called. `resolveIcon(undefined)` → `{ component: Box, ok: false }`, no error log (omitted name is fine). Required keys present.                                                                                                                                                                                                                                                                                                                                                                                                                 |
| `SurfaceActionButton.test.ts` (NEW)            | The owning matrix for the icon + label-display contract: `labelDisplay='always'` → label visible, no tooltip wrapper, no `sr-only` / `button-label-auto` class. `'icon-only'` + icon → label inside `<span class="sr-only">`, Button gets `aria-label`, wrapping `<span title="…" class="contents">` present. `'auto'` + icon → label inside `<span class="button-label-auto">`, same tooltip wrapper. `'icon-only'` + missing icon → effective `'always'` (no sr-only, no tooltip wrapper). `'icon-only'` + `disabled=true` → label inside `<span class="sr-only">`, Button gets `aria-label`, **no** tooltip wrapper from `SurfaceActionButton` (caller owns gate-message wrap). |
| `SurfaceInteractionButton.test.ts` (extend)    | Forwards `labelDisplay`, `icon`, `loading`, `variant` to `SurfaceActionButton` for the active branch. Forwards `disabled=true` for the context-gated branch and keeps the outer `<span title="Select a configuration first">`. Behavioural tests for tooltip / sr-only / icon move to `SurfaceActionButton.test.ts`.                                                                                                                                                                                                                                                                                                                                                               |
| `SurfaceTable.test.ts` (extend)                | Row-actions wrapper has `flex flex-nowrap` and `@container/buttons` classes. With icon → child `SurfaceInteractionButton` receives `labelDisplay='icon-only'`. Without icon → receives `'always'`. Cell `<td>` has `whitespace-nowrap`. Both branches (entity-link path + rowActions snippet path) covered.                                                                                                                                                                                                                                                                                                                                                                        |
| `SurfaceActionBar.test.ts` (extend)            | Outer `<div>` has `@container/buttons`. Child receives `labelDisplay='auto'` when interaction has icon, `'always'` otherwise.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| `SurfaceWorkflow.test.ts` (extend)             | Forwards `labelDisplay`, `interaction.icon`, `loading`, `variant` to `SurfaceActionButton`. Behavioural tests for the rendered output live in `SurfaceActionButton.test.ts`.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| `dev/button-preview/+page.svelte`              | Manual visual: render `SurfaceInteractionButton` (and the underlying `Button`) examples for each effective display state, with-icon and without-icon variants. The page already exists; extend it with these cases. Confirms `Button` itself remains unmodified.                                                                                                                                                                                                                                                                                                                                                                                                                   |

### Rust (cargo test)

| File                                                                          | Cases                                                                                                                                                                                                                                                                                                                                                                      |
| ----------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `crates/shared/surfaces/src/lib.rs` (or new `validation.rs`)                  | `validate_icon_name` unit tests: empty rejected, 65-char rejected, `"trash-2"` accepted, `"Trash2"` rejected, `"trash_2"` rejected, `"-trash"` rejected, `"trash-"` rejected, `"x"` rejected (single-char fails the ≥2-char constraint).                                                                                                                                   |
| `crates/shared/wire/src/wire_validate_impls.rs` (extend existing test module) | Existing `nav_icon` tests: rename constant references to `MAX_ICON_NAME_LEN` and confirm they still pass via the shared validator. New: `nav_icon` PascalCase rejected (regression for kebab-case migration). New: `interaction.icon` non-empty / oversized / kebab-valid / PascalCase-rejected / underscore-rejected; field names match `surfaces[].interactions[].icon`. |
| `crates/plugins/infrastructure/core/src/surface_form_authoring.rs`            | Unit test: `SurfaceActionDescriptor::new("a", "A").with_icon("trash-2").icon == Some("trash-2")`. Builder is fluent.                                                                                                                                                                                                                                                       |
| `crates/shared/surfaces/src/interaction.rs`                                   | `InteractionDescriptor::validate_for_provider` already runs label/timeout/workflow checks. Extend its `InteractionValidationError` with an `IconInvalid { interaction_id, reason }` variant and call `validate_icon_name` from § 2 when `icon.is_some()`. Add unit tests mirroring the wire test cases (empty / oversized / kebab-valid / PascalCase-rejected).            |

### Conditional test runs

Per `CLAUDE.md`, no DB migrations or async-locking changes are introduced,
so the conditional Docker integration test gate does not trigger. The
standard frontend gate runs:

```bash
cd frontend && npm run lint && npm run format:check && npm run check && npm run test && npm run build
```

The standard Rust gate runs:

```bash
cargo fmt --all
cargo check --no-default-features --features db-sqlite
cargo check --all-features
cargo clippy --all-targets --no-default-features --features db-sqlite
cargo clippy --all-targets --all-features
cargo test --all-features
cargo deny check
markdownlint --config .markdownlint.json '**/*.md'
```

---

## Risks and trade-offs

- **Container query browser support.** Modern only (Safari 16+, Chrome 105+,
  Firefox 110+). All browsers supported by uptrakit's existing UI already
  meet this bar; no new fallback needed.
- **Em-based threshold tunability.** The `28em` literal is the single knob,
  defined once in the container-query rule. CSS custom properties cannot
  appear inside `@container` query conditions (CSS Containment Level 3
  rejects `var()` there), so the threshold cannot be lifted to a variable.
  If a future bar needs a different threshold, the implementation adds a
  per-bar Tailwind variant (e.g. `@max-[24em]/buttons:sr-only` on a
  narrower-collapse cluster). Not an immediate need.
- **`'auto'` is best-effort for ≤ 3 button bars.** A 28em container fits
  one or two text buttons comfortably; with four or more buttons in a
  bar, the bar will overflow or wrap before the 28em threshold ever
  triggers. `'auto'` is therefore best-effort: it handles narrow surfaces
  with small action bars well, but is not a substitute for a layout
  redesign on bars that grow beyond three actions. The DataTable
  row-action path uses `'icon-only'` unconditionally, which is the path
  the original "buttons wrap awkwardly" complaint was about — that case
  is fully addressed.
- **Unicode in icon names.** The regex restricts to ASCII kebab-case. This
  is a deliberate constraint: lucide names are ASCII; allowing Unicode
  invites confusable-character bugs.
- **Backward compatibility.** `icon: Option<String>` defaults to `None`
  on the wire; older controllers/services serialize without the field
  (`#[serde(default, skip_serializing_if = "Option::is_none")]`) and newer
  consumers handle absence as "no icon". No breaking change.
- **Kebab-case migration of `nav_icon`.** The single producer-side
  migration is `registry.svelte.ts` consumer-side (PascalCase fallback
  `'Box'` → `'box'`) and the spec-style test data. No surfaces in the
  current Rust tree call `SurfaceDescriptor::nav_icon(...)` with a
  PascalCase string, confirmed via `grep`. The wire validation regex now
  rejects PascalCase, which would have been invalid against the lucide
  canonical scheme anyway.

---

## Open questions

None at spec time. All decisions resolved in the grilling phase.

---

## File-touch summary (informative — implementation plan owns the breakdown)

Rust:

- `crates/shared/surfaces/src/lib.rs` (or new `validation.rs`) —
  `MAX_ICON_NAME_LEN`, `IconNameError`, `validate_icon_name`. Re-export
  from crate root.
- `crates/shared/surfaces/src/interaction.rs` — add `icon` field to
  `InteractionDescriptor`; extend `InteractionValidationError` with
  `IconInvalid`; call `validate_icon_name` from `validate_for_provider`.
- `crates/plugins/infrastructure/core/src/surface_form_authoring.rs` —
  add `icon` field + `with_icon` builder.
- `crates/plugins/infrastructure/registry/...` — `SurfaceActionDescriptor`
  is exposed via `all_descriptors()` here; no `InteractionDescriptor`
  conversion happens in this crate, so adding the `icon` field to the
  authoring struct is sufficient.
- `crates/core/agent-ssh/src/surface_runtime.rs` — same forwarding for the
  SSH path; apply icons to `bootstrap`, `sync-host`, `remove-host`.
- `crates/plugins/infrastructure/proxmox/src/plugin.rs` — set `icon` on
  the five Proxmox VE Hosts interactions.
- `crates/plugins/infrastructure/proxmox/src/agent/plugin.rs` — apply
  `with_icon("boxes")` on `bootstrap_proxmox_guest_action`.
- `crates/shared/wire/src/limits.rs` — delete `MAX_NAV_ICON_LEN`; call
  sites move to `surfaces::MAX_ICON_NAME_LEN`.
- `crates/shared/wire/src/wire_validate_impls.rs` — call
  `surfaces::validate_icon_name` for `nav_icon` and every
  `interactions[].icon`; map `IconNameError` to wire `ValidationError`.

Frontend:

- `frontend/src/lib/icons.ts` — NEW (replaces `nav-icons.ts`).
- `frontend/src/lib/nav-icons.ts` — DELETED.
- `frontend/src/lib/nav-icons.test.ts` — DELETED (replaced by
  `icons.test.ts`).
- `frontend/src/lib/surfaces/label-display.ts` — NEW. `LabelDisplay` const
  object + type union. Used by surface wrappers only.
- `frontend/src/lib/surfaces/contract.ts` — add `icon?: string` to
  `InteractionDescriptor`.
- `frontend/src/lib/surfaces/registry.svelte.ts` — switch to `resolveIcon`
  - kebab-case fallback.
- `frontend/src/lib/components/Button.svelte` — **unchanged**. The
  `labelDisplay` mechanism lives entirely in the surface wrappers.
- `frontend/src/lib/components/surfaces/SurfaceActionButton.svelte` —
  NEW. Single owner of icon resolve, effective-display, label-span
  rendering, and tooltip wrap. Both `SurfaceInteractionButton` and
  `SurfaceWorkflow` delegate to it.
- `frontend/src/lib/components/surfaces/SurfaceInteractionButton.svelte` —
  accept `labelDisplay`, replace inline `<Button …>` with
  `<SurfaceActionButton …>` in both render branches (active +
  context-gated).
- `frontend/src/lib/components/surfaces/SurfaceActionBar.svelte` —
  `@container/buttons` + `labelDisplay='auto'` when icon present.
- `frontend/src/lib/components/surfaces/SurfaceTable.svelte` —
  `flex-nowrap`, `@container/buttons`, `labelDisplay='icon-only'` when
  icon present (both branches).
- `frontend/src/lib/components/surfaces/SurfaceWorkflow.svelte` — accept
  `labelDisplay`, replace inline `<Button …>` trigger with
  `<SurfaceActionButton …>`.
- `frontend/src/app.css` — no changes. The container context and the
  query rule are produced by Tailwind utilities applied directly in the
  Svelte components (see § 5).
- `frontend/src/routes/dev/button-preview/+page.svelte` — examples.
- Test files listed in § Testing.

Docs:

- `docs/development/plugin-guidelines.md` — new "Action icons" section.
- `docs/superpowers/specs/2026-05-02-icon-rollout-design.md` — banner
  pointing here for kebab-case migration.

---

## Acceptance criteria

- [ ] Plugin authors can declare `with_icon("trash-2")` on
      `SurfaceActionDescriptor` and the icon renders inside the button.
- [ ] `SurfaceDescriptor::nav_icon` and `InteractionDescriptor::icon` both
      fail wire validation on PascalCase, snake_case, oversized, or empty
      values.
- [ ] Proxmox VE Hosts surface buttons render with the assigned icons in
      the dashboard. Action-bar buttons collapse to icon-only when the action
      bar is narrower than 28em. Row actions (`approve-match`, `match`,
      `unmatch`) render icon-only on a single line.
- [ ] SSH Hosts surface buttons render with the assigned icons. Same
      collapse/icon-only behaviour.
- [ ] An unknown icon name in any interaction degrades to text-only and
      logs a single `console.error` per name per session (no spam loop).
- [ ] `cargo test --all-features` and `cd frontend && npm run test` pass.
- [ ] `docs/development/plugin-guidelines.md` documents the icon allowlist
      and the workflow for adding a new icon.

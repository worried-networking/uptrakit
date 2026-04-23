<!-- markdownlint-disable MD013 -->

# Shared Primitives

All primitives are exported from `frontend/src/lib/components/ui/index.ts`.

Import pattern:

```typescript
import { Button, Callout, StatusBadge } from '$lib/components/ui';
```

Non-UI-barrel components (`Button`, `Input`, `Textarea`, `Checkbox`, `Modal`, `ContextMenu`) are in
`$lib/components` and re-exported through the barrel where noted.

**Status:** `Implemented` (all components below)

---

## Layout Primitives

### PageShell

Full-page container with eyebrow, title, description, and an actions slot.

```typescript
// frontend/src/lib/components/ui/PageShell.svelte
{
  eyebrow?: string;       // small uppercase label above the title
  title: string;          // h1 at 20px bold
  description?: string;   // secondary paragraph below title
  actions?: Snippet;      // right-aligned action area (buttons, etc.)
  children: Snippet;      // page body content
}
```

Usage:

```svelte
<PageShell title="Hosts" description="Registered agents in this tenant.">
  {#snippet actions()}
    <Button variant="primary">Add host</Button>
  {/snippet}
  <!-- body -->
</PageShell>
```

- Renders a `<section data-ui="page-shell">` with a responsive header that stacks on narrow screens.
- `actions` is always `shrink-0` and wraps naturally if multiple controls are present.

---

### SectionCard

Bordered surface card for grouped settings, detail panels, or any contained section body.

```typescript
// frontend/src/lib/components/ui/SectionCard.svelte
{
  title?: string;          // h2 section heading
  description?: string;    // secondary paragraph below heading
  actions?: Snippet;       // right-aligned header actions
  children: Snippet;       // card body
}
```

Usage:

```svelte
<SectionCard title="Authentication" description="Configure login options.">
  {#snippet actions()}
    <Button variant="ghost" size="sm">Edit</Button>
  {/snippet}
  <!-- form rows -->
</SectionCard>
```

- Renders `rounded-[3px] border border-[var(--border-subtle)] bg-[var(--bg-surface)] shadow-sm`.
- Header section has a bottom border separating it from the body.
- Omit `title`, `description`, and `actions` to render the card without a header.

---

## Navigation Primitives

### TabStrip

Accessible tab list with keyboard navigation, ARIA roles, and panel association.

```typescript
// frontend/src/lib/components/ui/TabStrip.svelte
export type TabStripItem = {
  id: string;
  label: string;
  panelId?: string;   // aria-controls value; derived from idBase if omitted
  tabId?: string;     // id on the button; derived from idBase if omitted
  disabled?: boolean;
};

{
  items: TabStripItem[];
  activeId?: string;          // currently selected tab id
  ariaLabel?: string;         // defaults to "Tabs"
  idBase?: string;            // prefix for auto-generated tab/panel ids
  onSelect?: (id: string) => void;
}
```

Usage:

```svelte
<script lang="ts">
  let activeTab = $state('general');
</script>

<TabStrip
  items={[
    { id: 'general', label: 'General' },
    { id: 'security', label: 'Security' },
  ]}
  activeId={activeTab}
  idBase="settings"
  onSelect={(id) => { activeTab = id; }}
/>
```

![TabStrip showing built-in and surface-contributed tabs, one active](../../../frontend/tests/e2e/ui-parity.test.ts-snapshots/ui-parity-settings-tabs-chromium.png)

Keyboard behavior: `ArrowRight`/`ArrowDown` → next enabled tab, `ArrowLeft`/`ArrowUp` → previous,
`Home` → first, `End` → last. Focus moves with selection.

Visual rules:

- Active tab: `bg-[rgba(var(--accent-rgb),0.12)] text-[var(--accent-bright)]`
- Inactive tab hover: `hover:bg-[var(--bg-raised)] hover:text-[var(--text-primary)]`
- Disabled: `opacity-40 pointer-events-none`

`host_detail.tabs` is currently an inline card stack, not a `TabStrip`.

---

## Feedback Primitives

### Callout

Semantic message block for inline notices. Maps to ARIA `role="alert"` for warning/danger and
`role="status"` for info/success.

```typescript
// frontend/src/lib/components/ui/Callout.svelte
export type CalloutTone = 'info' | 'success' | 'warning' | 'danger';

{
  tone?: CalloutTone;   // default: 'info'
  title?: string;       // bold leading line
  message?: string;     // body text at 90% opacity
  children?: Snippet;   // arbitrary body content (use instead of or alongside message)
}
```

Usage:

```svelte
<Callout tone="danger" message="Failed to load hosts." />

<Callout tone="warning" title="Requires restart">
  Changes take effect after the agent restarts.
</Callout>
```

Tone → token mapping:

| Tone | Text | Background | Border |
| --- | --- | --- | --- |
| `info` | `--color-info` | `--color-info-bg` | `--color-info-border` |
| `success` | `--color-success` | `--color-success-bg` | `--color-success-border` |
| `warning` | `--color-warning` | `--color-warning-bg` | `--color-warning-border` |
| `danger` | `--color-error` | `--color-error-bg` | `--color-error-border` |

Do not use `<aside class="preset-filled-error-500">` or similar Skeleton utilities. Always use
`<Callout>`.

---

### EmptyState

Placeholder for list and table views with no rows, or filtered views with no matches.

```typescript
// frontend/src/lib/components/ui/EmptyState.svelte
{
  title: string;          // bold heading
  description?: string;   // secondary explanation
  actions?: Snippet;      // optional centered action area (e.g. a ghost Button)
}
```

Usage:

```svelte
<EmptyState
  title="No hosts registered"
  description="Enroll your first agent to get started."
>
  {#snippet actions()}
    <Button variant="ghost" href="/enroll">Enroll a host</Button>
  {/snippet}
</EmptyState>
```

Renders with a dashed border and centered content. Max width of the inner block is `28rem`.

---

## Badge Primitives

### StatusBadge

Static label badge. Used for status indicators, tags, and any inline label that does not trigger
an action.

```typescript
// frontend/src/lib/components/ui/StatusBadge.svelte
export type StatusBadgeTone = 'neutral' | 'info' | 'success' | 'warning' | 'danger';

{
  tone?: StatusBadgeTone;   // default: 'neutral'
  label: string;
}
```

Usage:

```svelte
<StatusBadge tone="success" label="Approved" />
<StatusBadge tone="warning" label="Requires restart" />
<StatusBadge tone="danger" label="Error" />
<StatusBadge tone="neutral" label="Pending" />
```

Tone → token mapping:

| Tone | Text | Background | Border |
| --- | --- | --- | --- |
| `neutral` | `--text-secondary` | `--bg-raised` | `--border-default` |
| `info` | `--color-info` | `--color-info-bg` | `--color-info-border` |
| `success` | `--color-success` | `--color-success-bg` | `--color-success-border` |
| `warning` | `--color-warning` | `--color-warning-bg` | `--color-warning-border` |
| `danger` | `--color-error` | `--color-error-bg` | `--color-error-border` |

Dimensions: `min-h-[14px]`, `2px` radius, `7.5px` bold uppercase text with 1px border.

Do not use `<span class="badge preset-tonal-*">`. Always use `<StatusBadge>`.

---

### ActionBadge

Clickable badge with idle/hover label swap inside a fixed-width container. Used for navigable
counts (e.g. "3 updates") and batch-update triggers.

```typescript
// frontend/src/lib/components/ui/ActionBadge.svelte
export type ActionBadgeVariant = 'navigation' | 'bulk-update';
export type ActionBadgeTone = 'info' | 'accent' | 'danger';

{
  variant?: ActionBadgeVariant;   // default: 'navigation'
  tone: ActionBadgeTone;
  idleLabel: string;              // text shown at rest
  hoverLabel: string;             // text shown on hover (same width container)
  disabled?: boolean;
  onclick?: (event: MouseEvent) => void;
}
```

Usage:

```svelte
<ActionBadge
  tone="info"
  idleLabel="3 updates"
  hoverLabel="Update all"
  onclick={handleUpdateAll}
/>
```

![ActionBadge idle state showing count](../../../frontend/tests/e2e/ui-parity.test.ts-snapshots/ui-parity-clickable-badge-chromium.png) ![ActionBadge hover state showing action label](../../../frontend/tests/e2e/ui-parity.test.ts-snapshots/ui-parity-clickable-badge-hover-chromium.png)

Rules:

- The hover label overlays the idle label using CSS grid; no layout reflow occurs.
- `navigation` variant: `min-h-[14px]`. `bulk-update` variant: `min-h-[16px]`.
- `violet` and `dim` static-only badges are `StatusBadge` with `neutral` tone; `ActionBadge` does
  not support them.
- Disabled state: `opacity-40 pointer-events-none`.

---

### PillBadge

Pill-shaped categorical label. Used for agent type, OS, plugin type, and similar taxonomy labels
where the value is not a status.

```typescript
// frontend/src/lib/components/ui/PillBadge.svelte
{
  label: string;
}
```

Usage:

```svelte
<PillBadge label="Linux" />
<PillBadge label="NPM" />
```

![PillBadge showing a categorical label](../../../frontend/tests/e2e/ui-parity.test.ts-snapshots/ui-parity-pill-badge-chromium.png)

Renders `rounded-full border border-[var(--border-default)] bg-[var(--bg-raised)]`, no tone
variants. Use `StatusBadge` for status-carrying values.

---

## Form Primitives

### FormFieldRow

Labeled form field wrapper with hint and error display. Provides ARIA `aria-describedby` context
to child `Input` and `Textarea` components automatically.

```typescript
// frontend/src/lib/components/ui/FormFieldRow.svelte
{
  label: string;
  hint?: string;          // secondary helper text below the label
  error?: string;         // error message; triggers red text below the field
  inputId?: string;       // connects label[for] and generates error id
  required?: boolean;     // shows red asterisk next to label
  children: Snippet;      // the input control(s)
}
```

Usage:

```svelte
<FormFieldRow label="Email address" inputId="user-email" required>
  <Input id="user-email" type="email" bind:value={email} />
</FormFieldRow>

<FormFieldRow
  label="Webhook URL"
  inputId="webhook-url"
  hint="Must be HTTPS."
  error={urlError}
>
  <Input id="webhook-url" type="url" bind:value={webhookUrl} error={urlError} />
</FormFieldRow>
```

Layout: `grid gap-3 md:grid-cols-[minmax(0,16rem)_minmax(0,1fr)]` — label column is up to `16rem`
wide, input column fills the rest. Stacks to a single column on narrow screens.

---

### Input

Single-line text input. Supports error state, disabled state, and bindable value.

```typescript
// frontend/src/lib/components/Input.svelte
export type InputType = 'text' | 'email' | 'password' | 'url' | 'number' | 'search';

{
  id: string;
  type: InputType;
  value: string | number;         // bindable
  name?: string;
  placeholder?: string;
  autocomplete?: string;
  disabled?: boolean;
  required?: boolean;
  error?: string;                 // sets aria-invalid and error styling
  min?: number | string;
  max?: number | string;
  oninput?: (e: Event) => void;
  onblur?: (e: FocusEvent) => void;
  onkeydown?: (e: KeyboardEvent) => void;
  'aria-describedby'?: string;
  'aria-label'?: string;
  class?: string;
}
```

When placed inside `<FormFieldRow inputId="...">`, `aria-describedby` is wired automatically via
Svelte context — no manual prop needed.

Error state: `border-[var(--color-error-border)] bg-[var(--color-error-bg)]` via `aria-invalid`.
Height: `h-8` (`32px`). Radius: `3px`.

---

### Textarea

Multi-line text input. Identical token contract to `Input`.

```typescript
// frontend/src/lib/components/Textarea.svelte
export type TextareaVariant = 'default' | 'mono';

{
  id: string;
  value: string;            // bindable
  name?: string;
  placeholder?: string;
  rows?: number;
  disabled?: boolean;
  required?: boolean;
  error?: string;
  variant?: TextareaVariant;   // 'mono' applies font-mono text-[13px]
  oninput?: (e: Event) => void;
  onblur?: (e: FocusEvent) => void;
  'aria-describedby'?: string;
  class?: string;
}
```

Minimum height `4rem` (`min-h-[4rem]`), resizable vertically.

---

### Checkbox

Single checkbox input with bindable `checked` and `indeterminate` states.

```typescript
// frontend/src/lib/components/Checkbox.svelte
{
  id: string;
  checked?: boolean;          // bindable, default false
  indeterminate?: boolean;    // bindable, default false
  name?: string;
  disabled?: boolean;
  onchange?: (e: Event) => void;
  class?: string;
  'aria-label'?: string;
}
```

Color note: `@tailwindcss/forms` sets `appearance: none` on checkboxes making `accent-color` inert.
The checked fill uses `currentColor` from the `text-[var(--accent)]` class. Do not use
`accent-[var(--accent)]`.

---

## Data Display Primitives

### DataTable

Accessible table with loading, error, and empty states built in. Accepts column definitions for the
default header/row rendering, or fully custom `header`/`row` snippets.

```typescript
// frontend/src/lib/components/ui/DataTable.svelte
export type DataTableColumn = {
  key: string;
  label: string;
  align?: 'left' | 'center' | 'right';
};

{
  columns: DataTableColumn[];
  rows: Record<string, unknown>[];
  caption?: string;                // screen-reader caption
  loading?: boolean;               // shows "Loading..." text
  error?: string | null;           // shows Callout with danger tone
  emptyTitle?: string;             // default: 'No rows available'
  emptyDescription?: string;
  header?: Snippet;                // replaces default <thead> row
  row?: Snippet<[Record<string, unknown>]>;  // replaces default <tr>
  footer?: Snippet;                // rendered below the table (e.g. TableFooterBar)
  rowKey?: (row, index) => string | number;
  errorActions?: Snippet;          // action area inside the error Callout
  rowActions?: Snippet<[Record<string, unknown>]>;
  rowActionsLabel?: string;        // default: 'Actions'
}
```

Usage (simple):

```svelte
<DataTable
  columns={[
    { key: 'name', label: 'Name' },
    { key: 'status', label: 'Status', align: 'right' },
  ]}
  {rows}
  {loading}
  {error}
  emptyTitle="No hosts"
/>
```

Usage (custom row):

```svelte
<DataTable {columns} {rows} {loading} {error}>
  {#snippet row(r)}
    <tr class="border-b border-[var(--border-subtle)] hover:bg-[var(--bg-raised)]">
      <td class="px-[10px] py-3 text-[12px] text-[var(--text-primary)]">{r.name}</td>
      <td class="px-[10px] py-3 text-right">
        <StatusBadge tone={r.status === 'ok' ? 'success' : 'danger'} label={r.status} />
      </td>
    </tr>
  {/snippet}
  {#snippet footer()}
    <TableFooterBar {total} {currentPage} {totalPages} {onPageChange} />
  {/snippet}
</DataTable>
```

Visual rules:

- Header: `bg-[var(--bg-raised)]`, `text-[11px] font-semibold uppercase tracking-[0.12em]`.
- Body rows: `text-[12px]`, hover `bg-[var(--bg-raised)]`, bottom border except last row.
- Container: `rounded-[4px] border border-[var(--border-subtle)]`.

---

### TableFooterBar

Pagination footer for `DataTable`.

```typescript
// frontend/src/lib/components/ui/TableFooterBar.svelte
{
  total: number;
  currentPage: number;
  totalPages: number;
  onPageChange: (page: number) => void;
}
```

![Table footer bar with total count and pagination controls](../../../frontend/tests/e2e/ui-parity.test.ts-snapshots/ui-parity-table-footer-chromium.png)

Renders a `border-t border-[var(--border-subtle)]` bar with "N total" on the left and a
`Pagination` control on the right. Always used inside the `footer` snippet of `DataTable`.

---

### ContextMenuItem

Single menu item inside a `ContextMenuShell`. Carries `role="menuitem"` and handles destructive
styling automatically.

```typescript
// frontend/src/lib/components/ui/ContextMenuItem.svelte
{
  label: string;
  destructive?: boolean;   // renders text in --color-error
  disabled?: boolean;
  onclick?: (event: MouseEvent) => void;
}
```

Usage:

```svelte
<ContextMenuShell>
  <ContextMenuItem label="View details" onclick={handleView} />
  <ContextMenuItem label="Delete" destructive onclick={handleDelete} />
</ContextMenuShell>
```

![Context menu shell with standard and destructive items](../../../frontend/tests/e2e/ui-parity.test.ts-snapshots/ui-parity-context-menu-shell-chromium.png)

Row height: `min-h-8` (`32px`). Horizontal padding: `px-3`. Font: `12px medium`. Hover fill:
`bg-[var(--bg-raised)]`.

Context menu shell dimensions:

- Background: `--bg-surface`
- Border: `--border-default`
- Radius: `4px`
- Destructive items use `--color-error` text token.

`software_item.host_context_menu` contributes launcher entries using this component — it does not
render nested grouped sub-menus.

---

## Action Primitives

### Button

Renders either an `<a role="button">` (when `href` is set) or a `<button>` element. Both share
identical visual treatment and state management.

```typescript
// frontend/src/lib/components/Button.svelte
export type ButtonVariant = 'primary' | 'ghost' | 'danger' | 'secondary';
export type ButtonSize = 'sm' | 'md';

// Link form (href required):
{
  variant: ButtonVariant;
  href: string;
  target?: string;
  rel?: string;
  size?: ButtonSize;         // default: 'md'
  disabled?: boolean;
  loading?: boolean;
  leadingIcon?: Snippet;
  trailingIcon?: Snippet;
  ariaLabel?: string;
  class?: string;
  children?: Snippet;
}

// Button form (no href):
{
  variant: ButtonVariant;
  size?: ButtonSize;
  type?: 'button' | 'submit' | 'reset';   // default: 'button'
  disabled?: boolean;
  loading?: boolean;
  leadingIcon?: Snippet;
  trailingIcon?: Snippet;
  ariaLabel?: string;
  onclick?: MouseEventHandler<HTMLButtonElement>;
  class?: string;
  children?: Snippet;
}
```

Usage:

```svelte
<Button variant="primary" onclick={handleSave}>Save changes</Button>
<Button variant="ghost" size="sm" href="/settings">Back</Button>
<Button variant="danger" onclick={handleDelete}>Delete</Button>
<Button variant="primary" loading={saving}>Saving…</Button>
```

Variants:

| Variant | Background | Border | Text |
| --- | --- | --- | --- |
| `primary` | accent gradient (`--accent-deep` → `--accent`) | none | `--text-inverted` |
| `ghost` | transparent | `--border-default` | `--text-primary` |
| `danger` | `--color-error-bg` | `--color-error-border` | `--color-error` |
| `secondary` | `--bg-raised` | `--border-default` | `--text-primary` |

Sizes:

| Size | Height | Padding | Font |
| --- | --- | --- | --- |
| `md` | `23px` | `px-3` | `9px` |
| `sm` | `19px` | `px-2` | `8.5px` |

Loading state: replaces leading icon with a `9px` spinning border ring; sets `aria-busy`. Disabled
state: `opacity-40 pointer-events-none` (also `aria-disabled` on link form).

Do not use `<a class="btn btn-sm preset-tonal">` Skeleton patterns. Always use `<Button>`.

---

## Dialog Primitives

### ModalShell

Re-exported as `ModalShell` from the barrel. Renders a centered dialog over a backdrop with an
optional title and footer slot.

```typescript
// frontend/src/lib/components/Modal.svelte
{
  onclose: () => void;
  title?: string;
  maxWidth?: string;    // Tailwind class, default: 'max-w-[380px]'
  children: Snippet;
  footer?: Snippet;
}
```

Usage:

```svelte
<ModalShell {onclose} title="Confirm deletion">
  <p class="text-sm text-[var(--text-secondary)]">This cannot be undone.</p>
  {#snippet footer()}
    <Button variant="ghost" onclick={onclose}>Cancel</Button>
    <Button variant="danger" onclick={handleConfirm}>Delete</Button>
  {/snippet}
</ModalShell>
```

Rules:

- Backdrop: `rgba(0,0,0,.55)`, z-index `900`.
- Modal window: z-index `910`, `4px` radius, `--bg-surface` background, `--border-subtle` border.
- `max-h-[calc(100vh-4rem)]` with `overflow-y-auto` on the body region.
- Close on `Escape` or backdrop click via `ModalBackdrop`.
- Footer is always right-aligned with `gap-2`.
- `aria-modal="true"`, `role="dialog"`, `aria-labelledby` wired when `title` is provided.

---

## Not Yet Built

The following patterns are specified but have no component implementation. Do not reference them
as if they exist.

| Pattern | Notes |
| --- | --- |
| Toggle / Switch | Replaced by `Checkbox` throughout. Boolean settings use `<Checkbox>`, not a track+thumb switch. The border-radius entry for "Toggle track" in `tokens.md` is a spec remnant. |
| Stat Card | No dedicated component. The spec describes a `3px` card with `7.5px` uppercase label and `14px` bold value. Currently ad-hoc per route. |
| Skeleton placeholders | No component. `DataTable` renders `"Loading..."` text during load. Skeleton shapes are per-spec for the loading state pattern but unbuilt. |
| Top navigation loading bar | No component. The spec lists an indeterminate top bar for page-level navigation; it does not exist. `Button` has a spinner for action-scoped loading only. |

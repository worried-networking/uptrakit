# Frontend Components

This document covers the reusable Svelte components in `frontend/src/lib/components/` that are
relevant to development.

## Design Language Entry Point

Before adding or restyling frontend UI, read [UI design language](ui-design-language.md).
`frontend/src/lib/components/` and `frontend/src/lib/components/surfaces/` must both consume the
same shared primitives and token adapter.

## Shared Interaction Contract

These interaction rules are enforced in shared CSS and shared primitives:

- ordinary interactive controls only animate `background`, `border-color`, and `color`
- keyboard focus uses `:focus-visible` with the `--accent-rgb` ring treatment
- error-state fields keep the error border while also receiving the focus-visible ring
- disabled controls stay visible, dimmed, and non-interactive (`pointer-events: none` where
  applicable)
- clickable badge hover swaps must not cause layout reflow; the hover label must fit in the same
  reserved badge width

## Shared UI Primitives

- `frontend/src/lib/components/ui/` owns reusable page-shell, card, tab, callout, empty-state,
  badge, field-row, provider-selector, and table primitives.
- `frontend/src/lib/components/Modal.svelte` and `frontend/src/lib/components/ContextMenu.svelte`
  are part of the same canonical primitive set.
- Route files should compose these primitives instead of recreating layout and color choices
  locally.

This document still includes modal-specific guidance because `Modal.svelte` remains one of the most
frequently reused primitives.

## Modal system

### Architecture

Two components form the modal stack:

| Component              | Role                                                                                                            |
| ---------------------- | --------------------------------------------------------------------------------------------------------------- |
| `ModalBackdrop.svelte` | Low-level primitive. Provides the dark overlay, focus trap, Escape key handler, and click-outside-to-close.     |
| `Modal.svelte`         | High-level wrapper. Combines `ModalBackdrop` with the standard card container. **Use this for all new modals.** |

`ModalBackdrop` is kept as a public primitive so that specialised layouts (e.g. full-screen
panels, tour overlays) can compose it directly when `Modal` is too constraining. For ordinary
dialog modals, always use `Modal`.

### `Modal.svelte`

```text
ModalBackdrop (overlay, focus trap, Escape, click-outside)
  └── div.card.bg-surface-50.dark:bg-surface-900.w-full.{maxWidth}…
        role="dialog"  aria-modal="true"  [aria-labelledby="modal-title"]
        ├── h3.h3#modal-title          ← rendered when title prop is provided
        ├── {@render children()}       ← required: main content
        └── div.flex.justify-end.gap-2 ← rendered when footer snippet is provided
              └── {@render footer()}
```

#### Props

| Prop       | Type         | Default      | Description                                                                                               |
| ---------- | ------------ | ------------ | --------------------------------------------------------------------------------------------------------- |
| `onclose`  | `() => void` | —            | Called when the modal should close (Escape, backdrop click, or explicit cancel).                          |
| `title`    | `string`     | `undefined`  | Optional heading rendered as `<h3 class="h3">`. Omit for complex custom headers.                          |
| `maxWidth` | `string`     | `'max-w-md'` | One or more Tailwind classes appended to the card. Use to control size, max-height, and layout direction. |
| `children` | `Snippet`    | —            | Required. The main modal body.                                                                            |
| `footer`   | `Snippet`    | `undefined`  | Optional. Rendered inside `flex justify-end gap-2`. Use for standard Cancel / Confirm button pairs.       |

#### Basic usage

```svelte
<Modal title="Edit Host Name" onclose={cancelEdit}>
  <label class="label">
    <span>Friendly Name</span>
    <input class="input" type="text" bind:value={editHost.friendlyName} />
  </label>
  {#snippet footer()}
    <button class="btn preset-tonal-surface" onclick={cancelEdit}>Cancel</button>
    <button class="btn preset-filled-primary-500" disabled={submitting} onclick={save}>
      {submitting ? 'Saving…' : 'Save'}
    </button>
  {/snippet}
</Modal>
```

#### Larger modal with scroll

Pass width + height constraints through `maxWidth`. Use `overflow-y-auto` on the card when the
content may overflow:

```svelte
<Modal
  title="Edit OIDC Provider"
  onclose={close}
  maxWidth="max-w-2xl max-h-[90vh] overflow-y-auto"
>
  <!-- long form content -->
</Modal>
```

#### Scrollable list + sticky footer

When the modal body contains a list that must scroll while the footer stays fixed, use
`flex flex-col` and apply `flex-1 min-h-0` on the scrollable child:

```svelte
<Modal title="Assign to Hosts" {onclose} maxWidth="max-w-2xl max-h-[85vh] flex flex-col">
  <ul class="overflow-y-auto flex-1 min-h-0 space-y-1">
    <!-- host rows -->
  </ul>
  {#snippet footer()}
    <button class="btn preset-tonal-surface" onclick={onclose}>Cancel</button>
    <button class="btn preset-filled-primary-500" onclick={submit}>Save</button>
  {/snippet}
</Modal>
```

#### Custom header (no title prop)

When the header contains more than a plain string (e.g. a subtitle, a badge, or an action
link), omit the `title` prop and render the header as the first child:

```svelte
<Modal onclose={close} maxWidth="max-w-2xl">
  <div class="flex items-start justify-between gap-4">
    <div>
      <h3 class="h3">{item.name}</h3>
      <p class="text-sm text-surface-500">v{item.version} · {item.date}</p>
    </div>
    <a href={item.url} class="btn btn-sm preset-tonal-surface">View ↗</a>
  </div>
  <!-- body -->
  {#snippet footer()}
    <button class="btn preset-tonal-surface" onclick={close}>Close</button>
  {/snippet}
</Modal>
```

#### Complex footer (non-standard layout)

When the footer needs `items-center` or a left-aligned element (e.g. an "Offline" warning),
place the entire footer `<div>` in `children` instead of using the `footer` snippet:

```svelte
<Modal title="Add MQTT Client" {onclose} maxWidth="max-w-2xl max-h-[90vh] overflow-y-auto">
  <!-- form fields -->
  <div class="flex justify-end gap-2 items-center">
    {#if !isOnline}<span class="text-warning-500 text-sm mr-auto">Offline</span>{/if}
    <button class="btn preset-tonal-surface" onclick={onclose}>Cancel</button>
    <button class="btn preset-filled-primary-500" onclick={save} disabled={!isOnline}>Save</button>
  </div>
</Modal>
```

### `ConfirmDialog.svelte`

A pre-built destructive-action confirmation dialog built on top of `Modal`. It handles the
standard title / message / warnings / Cancel + Confirm pattern.

```svelte
<ConfirmDialog
  title="Delete Plugin Config"
  messagePrefix="Are you sure you want to delete"
  entityName={config.name}
  confirmLabel="Delete"
  confirmClass="preset-filled-error-500"
  onconfirm={executeDelete}
  oncancel={() => (deleteConfirm = null)}
/>
```

Props:

| Prop              | Type         | Default                     | Description                                                             |
| ----------------- | ------------ | --------------------------- | ----------------------------------------------------------------------- |
| `title`           | `string`     | —                           | Dialog heading.                                                         |
| `messagePrefix`   | `string`     | —                           | Text before the entity name (e.g. `"Are you sure you want to delete"`). |
| `entityName`      | `string`     | —                           | Highlighted entity name shown in bold.                                  |
| `confirmLabel`    | `string`     | —                           | Label for the confirm button.                                           |
| `confirmClass`    | `string`     | `'preset-filled-error-500'` | Tailwind class(es) for the confirm button.                              |
| `confirmDisabled` | `boolean`    | `false`                     | Disables the confirm button (e.g. while submitting).                    |
| `warnings`        | `string[]`   | `[]`                        | Optional list of warning messages shown above the buttons.              |
| `onconfirm`       | `() => void` | —                           | Called when the user clicks the confirm button.                         |
| `oncancel`        | `() => void` | —                           | Called when the user cancels (also used as `onclose`).                  |

### `ModalBackdrop.svelte`

Low-level primitive. Only use this directly when `Modal` cannot accommodate the layout.

| Prop       | Type         | Description                                   |
| ---------- | ------------ | --------------------------------------------- |
| `onclose`  | `() => void` | Called on Escape key press or backdrop click. |
| `children` | `Snippet`    | Content rendered inside the backdrop.         |

Behaviour:

- Traps Tab and Shift+Tab focus within the rendered content.
- Focuses the first focusable element on mount; restores the previously focused element on
  unmount.
- Calls `onclose` when Escape is pressed or when the backdrop overlay itself is clicked (not
  its children).

### `TerminalOutput.svelte`

Canonical shared terminal shell for both built-in integrations:

- `frontend/src/routes/history/+page.svelte` history-feed row log viewer
- `frontend/src/routes/software/[id]/+page.svelte` live update session modal

Behavior contract:

- modal shell with shared backdrop (`data-ui="terminal-backdrop"`) and terminal window
  (`data-ui="terminal-shell"`)
- traffic-light titlebar: red closes, yellow stays visible but disabled, green toggles
  maximize/restore on non-mobile breakpoints
- status bar carries state badge, metadata, and optional destructive live actions (for example
  `Ctrl+C`)
- mobile uses the full-screen shell treatment; desktop/tablet use the bounded shell with maximize
  support
- switching from live to captured mode updates stdin behavior at runtime (`onInput` controls
  terminal input wiring after mount)

## Checklist for new modals

1. Use `<Modal>` for standard dialog modals - do not compose `ModalBackdrop` + an inline card
   `<div>` directly for those cases.
2. If `Modal` is too constraining for a specialized layout, compose `ModalBackdrop` directly and
   keep the layout exception narrow.
3. Pass `title` for a plain string heading; render a custom `<h3>` in children for complex headers.
4. Use the `footer` snippet for standard `flex justify-end gap-2` button pairs.
5. Do **not** add a `svelte:window onkeydown` handler for Escape — `ModalBackdrop` already handles it.
6. Do **not** duplicate `role="dialog"` or `aria-modal` attributes — `Modal` sets them on the card.

## Software Overlay Ownership

Software-route-owned modal and wizard flows live in shared route components and should be extended
there instead of recreated in route-local markup:

- `AddSoftwareModal.svelte` (`/software`) owns create-software field rhythm, inline validation,
  and submit/loading states.
- `AssignToHostModal.svelte` (`/software` and `/software/[id]`) owns host selection, role
  assignment, loading, empty, and validation states.
- `EditHostAssignmentModal.svelte` (`/software/[id]`) owns assignment editing, override handling,
  error flows, and destructive hook-removal confirmation.
- `SoftwareMergeWizard.svelte` (`/software` and `/software/[id]`) owns the merge workflow shell,
  survivor selection, preview step, and execute ordering.

`/software` also ships a route-local `IgnoreRulesTab.svelte` for ignore-rule CRUD. Keep that tab
route-local unless another route needs the exact same behavior, and keep it composed from shared
primitives (`DataTable`, `TableFooterBar`, `ModalShell`, `ConfirmDialog`, `BatchActionBar`).

## Route design-language primitives

Built-in route pages now standardize on shared design-language wrappers from
`frontend/src/lib/components/ui/index.ts`.

| Primitive         | Use when                                                                                          | Notes                                                                                                                          |
| ----------------- | ------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------ |
| `PageShell`       | Any full route page with a title/description and top-level action cluster                         | Adds canonical page spacing and `data-ui="page-shell"` marker.                                                                 |
| `SectionCard`     | Distinct grouped block inside a page (filters, table container, summary block, read-only details) | Use one card per user-comprehensible section; avoid route-local card wrappers.                                                 |
| `DataTable`       | Any tabular list/index view                                                                       | Handles loading/error/empty states and centralizes table shell styling.                                                        |
| `TableFooterBar`  | Totals + pagination row aligned to a `DataTable` shell                                            | Pair with the `DataTable` `footer` snippet; do not place raw pagination blocks outside the wrapper.                            |
| `EmptyState`      | No-result view in a section or table                                                              | Usually reached via `DataTable` `rows.length === 0` fallback.                                                                  |
| `Callout`         | Inline error/warning/info/success feedback in-page                                                | Use this instead of ad hoc `aside` color presets.                                                                              |
| `StatusBadge`     | Compact status/state labels in cells, headers, and metadata rows                                  | Use tone mapping helpers per route (`success`, `warning`, `danger`, etc.).                                                     |
| `ActionBadge`     | Interactive badges such as `N updates` -> `-> Software` and `Update all`                          | Use variants for navigation vs bulk-update semantics; keep shared sizing, hover swap, and disabled treatment in one primitive. |
| `PillBadge`       | Compact neutral taxonomy pills                                                                    | Use for plugin labels and tag-like chips that should stay in the same visual family as badges.                                 |
| `ContextMenuItem` | Standard menu rows inside `ContextMenuShell`                                                      | Owns row height, text size, hover fill, and destructive color treatment.                                                       |

### `DataTable` expansion points

`DataTable.svelte` supports route-specific behavior without reintroducing
route-local table wrappers:

| Hook           | Type                                        | Purpose                                                                             |
| -------------- | ------------------------------------------- | ----------------------------------------------------------------------------------- |
| `header`       | snippet                                     | Replace the default header row (e.g., add batch-select checkbox columns).           |
| `row`          | snippet `(row)`                             | Render custom rich rows (links, badges, inline action buttons, expandable details). |
| `errorActions` | snippet                                     | Add retry or remediation actions inside the shared error callout.                   |
| `rowKey`       | function `(row, index) => string \| number` | Stable keyed row identity for dynamic lists and expanded rows.                      |

Recommended default: set `columns={[]}` and provide `header`/`row` snippets for
complex route tables. Keep `emptyTitle`/`emptyDescription` route-specific.

## Route migration pattern

The invariant is primary: any route that uses the shared shell should compose
`PageShell` and `SectionCard` instead of recreating layout, spacing, or color
choices locally. This applies to built-in routes and surface-backed routes
alike, and it is enforced by route tests that look for the shared shell markers
(`data-ui="page-shell"`, `data-ui="section-card"`).

Route-local code should keep data flow and interaction logic, but move visual
framing into the shared primitives. `DataTable`, `Callout`, `StatusBadge`, and
`EmptyState` should fill the inner content where they fit naturally.

Examples of routes that currently follow this pattern include `/hosts`,
`/services`, `/system-services`, `/host-tags`, `/audit-logs`, `/history`,
`/profile`, and `/`. Treat that list as illustrative, not exhaustive.

Surface-backed routes continue to use the shared surfaces runtime
(`frontend/src/lib/components/surfaces/`), but they must still compose inside
the same shell language:

- `PageShell` and `SectionCard` provide the outer route frame.
- Surface panels, tables, and actions render inside those containers.
- Route tests should assert shell-level parity plus slot/runtime behavior, not
  a separate visual treatment.

## `PublicEntryShell`

`frontend/src/lib/components/ui/PublicEntryShell.svelte` owns the shared shell
for the unauthenticated route family:

- `/login`
- `/register`
- `/device`
- `frontend/src/routes/+error.svelte`

Behavior:

- shared eyebrow/title/subtitle rhythm inside one constrained-width card shell
- shared content and footer slots so routes can keep auth/device-specific
  semantics without inventing route-local chrome
- shared pre-auth action/input/link class exports for route-owned forms
- intentionally does not render authenticated app-shell navigation, user
  controls, or bottom-nav chrome

Preserved exceptions:

- `/device` keeps the emphasized device-code block and anonymous redirect action
- `+error.svelte` uses the same shell but skips authenticated app chrome even
  for signed-in users

## Shared surface runtime components

Shared surface rendering is implemented under `frontend/src/lib/components/surfaces/` and is used
for both built-in and provider-backed UI flows.

Core pieces:

- `SurfaceReadPanel.svelte` — read-model hydration and provider targeting (`targeted` vs
  `universal`).
- `SurfaceRenderer.svelte` — recursive node renderer for the shared contract
  (`section`, `tabs`, `table`, `form`, `workflow`, and related node kinds).
- `SurfaceSlot.svelte` — slot-level composition using `getSurfacesBySlot()` from
  `frontend/src/lib/surfaces/registry.svelte.ts`.

For shared-surface pages, sidebar navigation uses `surface_id` and routes to
`/surfaces/{surface_id}`. Refresh therefore keeps the user on the same surface page.

`frontend/src/lib/components/surfaces/` is the active renderer path for provider-backed UI.

### Interaction Label Contract

Current shipped behavior enforces the design-language label contract:

- surface interactions must provide a non-empty human-authored `interaction.label`
- workflow steps must provide a non-empty human-authored `workflow_step.label`
- shared runtime components (`SurfaceInteractionButton`, `SurfaceForm`,
  `SurfaceRenderer`, `SurfaceWorkflow`) do not synthesize generic fallback copy
- malformed unlabeled interactions degrade to the shared `Action unavailable` callout instead of
  rendering actionable UI

## Batch action components

Two shared components support multi-select batch operations across list pages and shared surface
tables.

### `BatchActionBar.svelte`

A fixed-position toolbar that appears at the bottom of the viewport when one or more items are
selected. It shows the selected count, action buttons, and a deselect-all button.

```svelte
<BatchActionBar
  selectedCount={selectedIds.size}
  actions={batchActions}
  onaction={requestBatchAction}
  oncancel={() => selectedIds.clear()}
/>
```

Props:

| Prop            | Type                                                     | Default | Description                                                                                                         |
| --------------- | -------------------------------------------------------- | ------- | ------------------------------------------------------------------------------------------------------------------- |
| `selectedCount` | `number`                                                 | —       | Number of currently selected items.                                                                                 |
| `actions`       | `{ id: string; label: string; destructive?: boolean }[]` | —       | Available batch actions. Destructive actions use `preset-filled-error-500`; others use `preset-filled-primary-500`. |
| `onaction`      | `(actionId: string) => void`                             | —       | Called when the user clicks an action button.                                                                       |
| `oncancel`      | `() => void`                                             | —       | Called when the user clicks "Deselect all".                                                                         |

Accessibility: `role="toolbar"` and `aria-label="Batch actions"`.

### `BatchResultDialog.svelte`

A modal dialog that displays partial-success results from a batch operation. It shows how many
items succeeded, how many failed, and lists per-item error messages for failures.

```svelte
{#if batchResult}
  <BatchResultDialog
    title="Batch Approve Results"
    response={batchResult}
    onclose={() => (batchResult = null)}
  />
{/if}
```

Props:

| Prop       | Type                  | Default | Description                                                    |
| ---------- | --------------------- | ------- | -------------------------------------------------------------- |
| `title`    | `string`              | —       | Dialog heading (e.g. `"Batch Approve Results"`).               |
| `response` | `BatchActionResponse` | —       | The batch response containing `succeeded` and `failed` arrays. |
| `onclose`  | `() => void`          | —       | Called when the dialog is dismissed.                           |

This dialog is only shown when the batch response contains failures. When all items succeed,
pages show a `showSuccess` toast instead.

### Page integration pattern

Each list page follows a consistent pattern for batch actions:

1. A `SvelteSet<string>` from `svelte/reactivity` tracks selected IDs. Use `SvelteSet` instead
   of native `Set` to satisfy the `svelte/prefer-svelte-reactivity` ESLint rule.
2. A select-all checkbox in `<thead>` supports checked, indeterminate, and unchecked states.
3. Per-row checkboxes are only visible when the user has the required manage permission.
4. `BatchActionBar` renders when `selectedIds.size > 0`.
5. Destructive actions show a `ConfirmDialog` before executing.
6. On success, a `showSuccess` toast is shown and the page reloads. On partial failure,
   `BatchResultDialog` displays the results.
7. Selection is cleared after a successful batch action.

See [Batch Actions API docs](../api/batch-actions.md) for backend details and
[end-user batch actions](../end-user/batch-actions.md) for the user-facing documentation.

## Testing modals

Both `Modal.svelte` and `ModalBackdrop.svelte` have unit tests in
`frontend/src/lib/components/Modal.test.ts` and `ModalBackdrop.test.ts` respectively.

When writing tests for a component that uses `Modal`, render the component under test and assert
on `screen.getByRole('dialog')`. For keyboard interaction, `fireEvent.keyDown` on the backdrop
`container.firstElementChild`.

See also: [Testing guide](testing.md).

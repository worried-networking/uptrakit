# Hosts Migration — Design

**Parent spec:** `docs/superpowers/specs/2026-04-16-ui-design-language-design.md` (§4.3 Buttons, §4.5 Cards, §4.6
Loading State, §4.7 Tables)

**Sub-spec #3h of the UI design-language rollout.** Depends on sub-spec #2 (Button primitive + UpdateAllButton primitive)
and #2c (secondary variant, ariaLabel, `--bg-hover` token) merged.

## Overview

Migrate host administration: `/hosts/+page.svelte` (590 lines — host list with per-row context menu, Retry error-state,
Batch Deactivate confirm, Edit Host Name modal) and `/hosts/[id]/+page.svelte` (796 — host detail with Retry, header
actions Edit Name / Deactivate / Trigger Discovery, Set Tags launcher, per-row software "View" link, Discovery Allowlist
CRUD, Edit Host Name / Add Discovery Plugin Type / Set Tags modals). Enumeration is against the current source; buttons
not listed here do not exist today and are out of scope.

Shared components embedded in these files are migrated by other sub- specs and are NOT touched here:

- `<BatchActionBar>` — sub-spec #3k (shared).
- `<ContextMenuShell>` and its raw `<button>` menu items (Edit Name, Trigger Discovery, Deactivate on the list page) —
  sub-spec #3k.
- `<ConfirmDialog>` internals (Batch Deactivate, Deactivate Host, Remove Allowlist Entry) — sub-spec #3k. This
  sub-spec's scope stops at callers' `confirmLabel` / `confirmClass` props; the text-swap
  `{submitting ? 'Processing…' : 'Deactivate'}` on `confirmLabel` is migrated to `loading={submitting}` + static label
  once #3k exposes a `loading` passthrough on the dialog's confirm button.

## Design decisions

**Q1 — Retry error-state buttons.**

- Options:
  - (chosen) `<Button variant="primary" onclick={() => loadHosts(...)}>` on the list page (line 484) and
    `<Button variant="primary" onclick={() => loadData()}>` on the detail page (line 398). Retry is the sole action
    visible inside an empty error container — primary matches §4.3 "main action in this context" (same rationale as #3e
    Q2 for the log-view Retry).
  - Use `secondary`. Rejected — secondary implies a reversible side action; error Retry is the only path forward, so
    primary reads correctly.
- Reasoning: error-recovery Retry is the primary action of its container, not a side affordance.

**Q2 — Header action cluster on host detail (Edit Name, Deactivate, Trigger Discovery).**

- Options:
  - (chosen) Edit Name → `<Button variant="secondary">` (reversible rename that opens a modal); Deactivate →
    `<Button variant="danger" onclick={() => (confirmDeactivate = true)}>` (launches the shared ConfirmDialog; no
    loading on this launcher — the async state lives inside the dialog's confirm); Trigger Discovery →
    `<Button variant="secondary" loading={discovering}>Trigger Discovery</Button>` with the `Triggering…` text-swap
    removed per the #3c Q4 contract.
  - Deactivate as `variant="secondary"`. Rejected — the current button uses `preset-filled-error-500` intentionally, and
    Deactivate is the destructive entry point on the page; `variant="danger"` preserves that signal.
- Reasoning: cluster reads left-to-right as reversible → destructive, with Trigger Discovery neither
  reversible-in-the-UI-sense nor destructive, so secondary + loading fits.

**Q3 — Per-row context-menu trigger vs context-menu items.**

- Options:
  - (chosen) The row-level context-menu trigger (`<button class="btn btn-sm preset-tonal">…⋮</button>` at line 468 of
    `+page.svelte`) migrates to an icon-only Button using an inline `{#snippet}` for the `⋮` glyph:

    ```svelte
    {#snippet moreIcon()}<span aria-hidden="true" class="leading-none">⋮</span>{/snippet}
    <Button
      variant="ghost"
      size="sm"
      leadingIcon={moreIcon}
      ariaLabel="Actions for {host.friendly_name}"
    />
    ```

    No icon import required — the current button already uses the `⋮` Unicode character (U+22EE VERTICAL ELLIPSIS)
    as its label; wrapping it in an `aria-hidden` span matches the canary pattern established by the Revoke button
    in `EnrollmentTokenSettings.svelte` (sub-spec #2 PR2). `ariaLabel` via #2c carries the accessible name.
    The context-menu items themselves stay raw `<button>` because `<ContextMenuShell>` owns their list-item styling —
    those migrate in #3k.

  - Use an `MoreIcon` SVG/component import. Rejected — no icon system exists in `frontend/src`; there is no
    importable `MoreIcon` component anywhere in the codebase.
  - Migrate menu items here. Rejected — `ContextMenuShell` internals are a shared concern and belong to #3k.
- Reasoning: trigger is a row affordance (#3h territory); list items are shell chrome (#3k). Inline snippet matches
  the established canary pattern from sub-spec #2 PR2 and avoids introducing an icon system dependency not yet spec'd.

**Q4 — Host-tag chips and audit filters.**

- Options:
  - (chosen) Tag chips stay as non-interactive `<span class="badge">` — no migration (they are display-only). Tag-editor
    _modal form_ inputs defer to #3h2 after #2b lands; the modal's footer Cancel / Save buttons DO migrate in this
    sub-spec (see Scope).
  - Migrate tag chips as buttons. Deferred — current behavior is non-interactive; feature work (click-to-filter) is a
    separate spec.
- Reasoning: tag chips are not interactive today; non-interactive elements are not Button candidates.

**Q5 — Software "View" row link on host detail.**

- Options:
  - (chosen) `<a href="/software/{item.id}" class="btn btn-sm preset-tonal">View</a>` migrates via Button polymorphic
    href branch (locked #2 contract): `<Button href="/software/{item.id}" variant="ghost" size="sm">View</Button>`.
  - Keep as raw `<a>`. Rejected — Goal 1 requires every button-shaped element to render through the primitive; the
    polymorphic branch exists exactly for this case.
- Reasoning: Button's `href` branch unifies button-shaped links.

**Q6 — Bulk-action bar.**

- Options:
  - (chosen) `<BatchActionBar>` panel and its internal action buttons defer to #3k end-to-end (matches #3f Q4). The
    _consumer_ of BatchActionBar on the list page does not render any launcher button of its own; the bar appears
    automatically from row selection. No #3h migration surface here.
  - Migrate inline. Rejected — shared primitive.
- Reasoning: consistency with #3f / #3g routing of shared components to #3k.

## Goals

1. Every interactive button in the two host files (that is not a shared-component internal) renders through `<Button>`
   or `<UpdateAllButton>`.
2. Destructive launchers adopt `variant="danger"`; reversible header/modal actions adopt `variant="secondary"`; primary
   recovery / submit adopts `variant="primary"`; row-level icon-only actions adopt `variant="ghost" size="sm"` with
   `ariaLabel` from #2c.
3. Every async handler surfaced on a migrated button uses `loading` binding with text-swap removed (no `Processing…`,
   `Triggering…`, `Saving…` in children expressions).
4. Every trigger-update call site on either host file (if any lands during this sub-spec's PR) uses `<UpdateAllButton>`
   with the locked `{state, count?, onclick, ariaLabel?, children?, class?}` contract from #2 — no `hostIds` prop exists
   on the primitive.

## Non-goals

- Approve / reject pending-enrollment buttons — DO NOT EXIST in the current source (host list shows registered hosts
  only). Adding that UI is feature work, separate spec.
- "Launch SSH" button — DOES NOT EXIST in the current source. SSH terminal attachment is out of scope until a future
  feature spec adds it; the parent design language's terminal theme (#2) remains unconsumed here.
- Filter chips on the host list — no filter-chip UI exists in `/hosts/+page.svelte`; stat cards are display-only
  counters, not interactive filter buttons.
- Form-input migration (Edit Host Name text field, Add Discovery Plugin Type select, Set Tags editor fields) — deferred
  to #3h2 after #2b primitives land. Modal footer buttons DO migrate (separate from form inputs).
- `<BatchActionBar>`, `<ContextMenuShell>`, `<ConfirmDialog>` internals — sub-spec #3k.
- Audit trail refactor — outside Button scope.
- Host enrollment backend — out of scope.

## Scope

Files migrated (button sites enumerated exhaustively against current source):

- `frontend/src/routes/hosts/+page.svelte`:
  - Per-row context-menu trigger `<button class="btn btn-sm preset-tonal">` (line 468) →
    `<Button variant="ghost" size="sm" leadingIcon={moreIcon} ariaLabel="…">` where `{#snippet moreIcon()}` wraps
    `<span aria-hidden="true" class="leading-none">⋮</span>` inline (no icon import — see Q3).
  - Error-state Retry (line 484) → `<Button variant="primary">`.
  - Edit Host Name modal footer Cancel (line 583) → `<Button variant="secondary">`; Save (line 584) →
    `<Button variant="primary" loading={submitting}>` with any `Saving…` text-swap children expression replaced by a
    static `Save`.
- `frontend/src/routes/hosts/[id]/+page.svelte`:
  - Error-state Retry (line 398) → `<Button variant="primary">`.
  - Header Edit Name (line 410) → `<Button variant="secondary">`.
  - Header Deactivate (lines 411–416) → `<Button variant="danger">` (launcher only; dialog internals are #3k).
  - Header Trigger Discovery (line 420) → `<Button variant="secondary" loading={discovering}>Trigger Discovery</Button>`
    with the `Triggering…` text-swap removed.
  - Set Tags launcher (line 487) → `<Button variant="secondary" size="sm">`.
  - Software row "View" link (line 581) → `<Button href="/software/{item.id}" variant="ghost" size="sm">View</Button>`.
  - Discovery Allowlist "Add Plugin Type" launcher (line 614) → `<Button variant="primary" size="sm">`.
  - Discovery Allowlist row Remove (line 646) → `<Button variant="danger" size="sm">` (launcher; confirm dialog
    internals are #3k).
  - Edit Host Name modal footer Cancel / Save (lines 730–731) → Cancel `<Button variant="secondary">`, Save
    `<Button variant="primary" loading={submitting}>` with text-swap removed.
  - Add Discovery Plugin Type modal footer Cancel / Save (lines 755–756) → same pattern.
  - Set Tags modal footer Cancel / Save (lines 778–779) → same pattern.

## Migration pattern

Standard translation rules (preset-filled-primary → primary, preset-filled-error → danger, preset-tonal-surface →
secondary, preset-tonal → ghost).

Special:

- **Trigger Discovery loading guards.** On the list page, existing `discoveringHostIds: Set<string>` already tracks
  per-row dispatch; if any per-row Trigger Discovery button migrates to a Button consumer in this PR, it binds
  `loading={discoveringHostIds.has(host.id)}`. (The current list page surfaces Trigger Discovery only through the
  context menu, which is out of scope per Q3 — no Button site today. This note is preserved for the detail page and in
  case #3k surfaces a direct Button consumer.) On the detail page, header Trigger Discovery binds
  `loading={discovering}`.
- **Modal submit buttons.** Every Cancel / Save pair in a file-local modal (Edit Host Name on both files, Add Discovery
  Plugin Type, Set Tags on detail) follows the #3c Q4 contract: Cancel renders `<Button variant="secondary">`; Save
  renders `<Button variant="primary" loading={submitting}>` with static children — `Save` text-swap with `Saving…` is
  removed; the primitive's spinner
  - preserved-children contract handles loading UI.
- **Polymorphic href.** Row "View" link uses Button's href branch (renders as `<a>`). `href` and `onclick` are the two
  mutually exclusive branches on the Button discriminated union — the href branch inherits the same variant class
  contract and does not require `role="button"` overrides (the #2 contract already applies the shape).
- **UpdateAllButton (if any trigger-update site lands in this PR).** Call shape is
  `<UpdateAllButton state={…} count={…} onclick={handleTriggerUpdate} ariaLabel="…" />`. The locked contract has NO
  `hostIds` / `software` / `host` props — call-site context is closed over inside `onclick`. An earlier draft used
  `<UpdateAllButton hostIds={[hostId]} />`; that syntax was never part of the #2 contract and is rejected.

## Data flow

Template-level only. Approve/reject/launch-SSH — none exist, so no data-flow changes on those axes. Existing
`discovering` / `discoveringHostIds` / `submitting` flags pass through unchanged; the only edits on those flags' call
sites are removing `{flag ? 'X…' : 'Y'}` expressions on button children in favor of `loading={flag}`.

## Error handling

Button discriminated union catches invalid prop combos at compile time. Existing toast / error pipelines on
approve-equivalent paths (`showError`, `showSuccess`) stay unchanged.

## Testing

### Unit tests

Extend `hosts/+page.test.ts`:

- Per-row context-menu trigger renders `variant="ghost" size="sm"` and an `aria-label` attribute containing the host's
  friendly name. The `leadingIcon` slot renders an `aria-hidden` `<span>` with the `⋮` glyph (no icon import — inline
  snippet per Q3).
- Retry error-state button renders `variant="primary"`.
- Edit Host Name modal Cancel renders `variant="secondary"`; Save renders `variant="primary" loading={submitting}` and
  its children stay static `Save` across the `submitting=false → true → false` cycle (regression guard that the
  `Saving…` expression is gone).
- No assertions on BatchActionBar / ContextMenuShell / ConfirmDialog internals — those are #3k's test plan.

Extend `hosts/[id]/+page.test.ts`:

- Error-state Retry renders `variant="primary"`.
- Header cluster: Edit Name `variant="secondary"`, Deactivate `variant="danger"`, Trigger Discovery
  `variant="secondary" loading={discovering}` with static children `Trigger Discovery` across the awaited dispatch
  window; `aria-busy="true"` is present while `discovering=true`.
- Set Tags launcher renders `variant="secondary" size="sm"`.
- Software row "View" renders as `<a>` (polymorphic href branch) with the ghost `size="sm"` class fragments.
- Allowlist Add Plugin Type renders `variant="primary" size="sm"`; row Remove renders `variant="danger" size="sm"`.
- Each file-local modal footer Save renders `variant="primary" loading={submitting}` with static children (regression
  guard for text-swap removal).

### Integration / e2e

- Playwright re-baseline:
  - `/hosts` default view.
  - `/hosts/[id]` default view + tag modal open + edit-name modal open + allowlist section expanded.
  - Each in dark + light themes.
- Delta enumeration per parent §9 (split by size class):
  - Row-level ghost actions + allowlist row Remove + pagination- equivalent header-row actions (`size="sm"`):
    `h-[19px]`, label `8.5px` uppercase.
  - Header cluster + modal submits + error-state Retry (`size="md"`, default): `h-[23px]`, label `9px` uppercase.
  - `variant="danger"` renders the error gradient on Deactivate, Remove allowlist entry.
  - `variant="secondary"` renders the `--bg-hover` token on hover per #2c.
- Snapshot masking per parent §3 approved dynamic categories:
  - Mask host-row volatile columns: `host.ip_address`, `host.hostname`, `host.last_seen_at`, `host.os_version`,
    `host.architecture`, and any stat-card count cells.
  - Mask `host.machine_id`, `host.created_at`, `host.updated_at` on the detail page.
  - Mask in-flight spinner rotation on every `<Button loading>` site.
  - Mask transient toast banners.
  - Total masked area stays under 15% per parent §3.
- Smoke test Trigger Discovery on detail: button flips to `aria-busy="true"` during the awaited dispatch, returns to
  idle on resolution; children stay static `Trigger Discovery` throughout.

## Rollout

Single PR titled "feat(frontend): migrate hosts area to Button primitive (sub-spec #3h)".

Commit granularity: one commit per file so bisect can isolate a regression to the specific migrated surface. Same
pattern as #3d / #3f.

1. `hosts/+page.svelte` — migrate context-menu trigger, Retry, Edit Host Name modal footer.
2. `hosts/[id]/+page.svelte` — migrate Retry, header cluster, Set Tags launcher, software View link, allowlist CRUD, all
   three file-local modal footers.
3. Extend unit tests per plan.
4. Re-baseline Playwright snapshots.
5. Full frontend gate.

### Risk + rollback

Revert of one PR restores preset classes across hosts. High- sensitivity surface — Deactivate is the destructive gate on
host detail; the launcher's `variant="danger"` must render correctly in both themes. Mitigated by dedicated unit tests
on each destructive launcher plus Playwright coverage in both themes.

### Dependencies + ordering

- **Blocks on:** sub-spec #2 merged (Button + UpdateAllButton primitives), sub-spec #2c merged (`variant="secondary"`
  for reversible header actions + modal Cancel, `ariaLabel` for the icon-only context-menu trigger). #3b merged for
  layout baseline. #3k is NOT a blocker (shared components' internals stay on presets until #3k lands — the launchers
  here migrate independently).
- **Blocks:** #3h2 form-input migration (depends on #2b Input + #2d Textarea).
- **Parallel-safe with:** sub-spec #3c–g, #3i–k, #4.

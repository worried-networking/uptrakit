# `--color-error` → `--color-danger` Token Rename — Design

**Parent spec:** `docs/superpowers/specs/2026-04-16-ui-design-language-design.md`

Rename the five `--color-error*` CSS custom properties to `--color-danger*` throughout the
frontend. Token values are unchanged. No visual difference.

---

## Motivation

`--color-error` conflates two distinct concepts:

- **Error** — a form-validation or system-failure state (e.g. invalid input outline)
- **Danger** — a destructive-action tone (e.g. delete button, warning callout)

All component primitives that consume this tone already use `tone="danger"` (`Button`,
`Callout`, `StatusBadge`, `StatCard`). The token name should match. `--color-danger` also
leaves `--color-error` available as a distinct token if a form-validation-specific color is
added in the future.

---

## Token rename table

| Old name | New name |
| --- | --- |
| `--color-error` | `--color-danger` |
| `--color-error-bg` | `--color-danger-bg` |
| `--color-error-border` | `--color-danger-border` |
| `--color-error-bg-hover` | `--color-danger-bg-hover` |
| `--color-error-border-hover` | `--color-danger-border-hover` |

Values for both themes are unchanged.

---

## Design decisions

**Q1 — Hard rename vs. CSS alias shim.**

- (chosen) Hard rename in one PR. No compatibility alias. `tokens.ts` is the sole source of
  truth; `TokenName` is a TypeScript union, so any missed `.ts` reference is a compile error.
  CSS-string usages in Svelte templates are verified by the step 6 grep. All consumers are
  internal — no external stylesheet references these tokens.
- Rejected: emit both `--color-error` and `--color-danger` from `tokens.ts` in a transition
  PR. No external consumers exist; the shim only adds noise.

**Q2 — Split into multiple PRs?**

- (chosen) One PR. The rename is mechanical; TypeScript enforces completeness; the diff is
  large but entirely find-and-replace. Splitting buys nothing and creates a window where the
  codebase references two names simultaneously.
- Rejected: separate PR per layer (token registry, tests, components, routes). Unnecessary
  overhead for a purely mechanical rename.

**Q3 — Rename internal `errorBase` const in `tokens.ts`?**

- (chosen) Yes — rename to `dangerBase` for consistency. It is a file-internal `const`, not
  exported; no downstream impact.
- Rejected: leave it as `errorBase`. A mismatched internal name is confusing for maintainers.

---

## Scope

### Token registry

**`frontend/src/theme/tokens.ts`**

- `TokenName` union: replace 5 `--color-error*` string literal members with `--color-danger*`
  equivalents.
- `tokens` object: rename the 5 corresponding keys.
- Internal `const errorBase` → `const dangerBase` (file-private; no export change).

### Pinned-value tests

**`frontend/src/theme/tokens.test.ts`**

- `EXPECTED` object: rename 5 keys (`--color-error*` → `--color-danger*`). Values unchanged.

**`frontend/src/lib/theme/design-token-values.test.ts`**

- `SPEC` object: rename 5 keys. Values unchanged.
- Two `cssForTheme` inline snapshots: update the 5 `--color-error*` lines in each snapshot
  to `--color-danger*`.

### Golden CSS test

**`frontend/vite-plugins/theme-tokens.test.ts`**

- `expected` string: update the 5 `--color-error*` lines in the `:root {}` block and the 5
  lines in the `.dark {}` block (10 lines total).

### Global CSS

**`frontend/src/app.css`**

- `aria-invalid` focus-visible rule: `var(--color-error-border)` → `var(--color-danger-border)`.

### CSS contract test

**`frontend/src/lib/theme/css-contract.test.ts`**

- `toMatch()` assertion for the `aria-invalid` focus-visible rule contains `--color-error-border`
  as a literal substring in the regex string. Update to `--color-danger-border`.

### UI documentation

**`docs/development/ui/tokens.md`**

Dark-theme table (rows 38–42): rename role labels and token names —
`Error` → `Danger`, `--color-error*` → `--color-danger*` — for all five rows.

Light-theme table (rows 74–78): same rename for all five rows.

Runtime Token Adapter table (row 226): `preset-filled-error-*` / `error-*` → `--color-error-*`
maps to `--color-danger-*`. Update the right-hand cell only (`--color-danger-*`).
The Skeleton utility name in the left-hand cell (`preset-filled-error-*`) is historical
and unchanged.

**`docs/development/ui/primitives.md`**

| Location | Change |
| --- | --- |
| Callout tone table (line 178) | `danger` row: all three token cells → `--color-danger`, `--color-danger-bg`, `--color-danger-border` |
| StatusBadge tone table (line 249) | `danger` row: same three cells → `--color-danger`, `--color-danger-bg`, `--color-danger-border` |
| Input/FormFieldRow error-state line (line 399) | `--color-error-border` → `--color-danger-border`; `--color-error-bg` → `--color-danger-bg` |
| ContextMenuItem prop comment (line 561) | `renders text in --color-error` → `renders text in --color-danger` |
| ContextMenuItem rule (line 614) | `--color-error text token` → `--color-danger text token` |
| Button variant table (line 680) | `danger` row: `--color-error-bg`, `--color-error-border`, `--color-error` → `--color-danger-bg`, `--color-danger-border`, `--color-danger` |
| StatCard tone table (added by `2026-04-23-stat-card-design.md`) | `danger` row: `--color-error` → `--color-danger` |

> The StatCard tone table does not exist yet — it is added by the stat-card spec. Update it as
> part of this rename along with the other primitives.md changes.

The `docs/development/ui/` docs are not under `frontend/src` and are not caught by the grep
command in the migration pattern. Update them explicitly as step 5b.

### Component and route files

All `.svelte` and `.ts` files containing `--color-error`. The grep command in the migration
pattern is authoritative — run it at implementation time to catch files added after this spec
was written. Known files at spec time:

| File | Notes |
| --- | --- |
| `src/lib/components/ui/ActionBadge.svelte` | |
| `src/lib/components/ui/StatusBadge.svelte` | |
| `src/lib/components/ui/Callout.svelte` | |
| `src/lib/components/ui/ContextMenuItem.svelte` | |
| `src/lib/components/ui/ContextMenuItem.test.ts` | |
| `src/lib/components/ui/FormFieldRow.svelte` | |
| `src/lib/components/ui/StatCard.svelte` | Created by `2026-04-23-stat-card-design.md`. `toneTokens` map: `danger: '--color-error'` → `danger: '--color-danger'` |
| `src/lib/components/Button.svelte` | |
| `src/lib/components/Button.test.ts` | |
| `src/lib/components/Input.svelte` | |
| `src/lib/components/Input.test.ts` | |
| `src/lib/components/Textarea.svelte` | |
| `src/lib/components/Textarea.test.ts` | |
| `src/lib/components/Link.svelte` | |
| `src/lib/components/Link.test.ts` | |
| `src/lib/components/ToastNotifications.svelte` | |
| `src/lib/components/EditHostAssignmentModal.svelte` | |
| `src/lib/components/AssignToHostModal.svelte` | |
| `src/lib/components/BatchResultDialog.svelte` | |
| `src/routes/+page.svelte` | |
| `src/routes/hosts/+page.svelte` | |
| `src/routes/history/+page.svelte` | |
| `src/routes/settings/OidcProvidersSettings.svelte` | |
| `src/routes/settings/NotificationRulesSettings.svelte` | |
| `src/routes/settings/NotificationLogView.svelte` | |
| `src/routes/settings/SchedulerTab.svelte` | |
| `src/routes/settings/PluginConfigsTab.svelte` | |
| `src/routes/settings/SystemServicesSettings.test.ts` | |
| `src/routes/settings/OidcProvidersSettings.test.ts` | |
| `src/routes/settings/NotificationRulesSettings.test.ts` | |
| `src/routes/settings/EnrollmentTokenSettings.test.ts` | |
| `src/routes/profile/profile.test.ts` | |
| `src/routes/hosts/hosts.test.ts` | |
| `src/routes/layout-button-migration.test.ts` | |
| `src/routes/software/ignore-rules-tab.test.ts` | |
| `src/routes/software/[id]/software-detail-update-trigger.test.ts` | |

The full set is reproduced by:

```sh
grep -rl '\-\-color-error' frontend/src
```

Run this before implementing to catch any files added after this spec was written.

---

## Sequencing

This rename must land **after** `2026-04-23-stat-card-design.md` merges. `StatCard.svelte`
contains `danger: '--color-error'` in its `toneTokens` map; that file is included in the
component/route file sweep above.

No other spec has a sequencing dependency on this rename. It is parallel-safe with all other
2026-04-23 specs once stat-card is merged.

Note: `primitives.md` will also gain a StatCard tone table (added by the stat-card spec). This
rename must update that table row too. Since the rename lands after stat-card, the StatCard
primitives.md entry will already exist at implementation time.

---

## Migration pattern

1. In `frontend/src/theme/tokens.ts`:
   - Rename `const errorBase` → `const dangerBase`.
   - Replace all 5 `--color-error*` keys in the `TokenName` union and in the `tokens` object.
2. Run `npm run check`. TypeScript will surface every `.ts` file that references old `TokenName`
   string literals and fail to compile. Fix each one.
   **Caveat:** `npm run check` does NOT catch `var(--color-error*)` in `.svelte` template class
   strings or `.css` files — those are CSS strings invisible to the TypeScript type checker.
   Steps 3–5b handle those sites explicitly.
3. Update the three test files with pinned token names and snapshots:
   - `tokens.test.ts`: rename 5 keys in `EXPECTED` object.
   - `design-token-values.test.ts`: rename 5 keys in `SPEC` object; update the two
     `toMatchInlineSnapshot(...)` calls (each contains 5 `--color-error*` lines to rename).
   - `vite-plugins/theme-tokens.test.ts`: update the `expected` array — 5 lines in the
     `:root {}` block and 5 lines in the `.dark {}` block (10 lines total).
4. Update `app.css` (`aria-invalid` rule) and `css-contract.test.ts` (`toMatch()` regex string).
5. Replace all `var(--color-error*)` occurrences in the component/route/test files.
   A project-wide find-and-replace of `--color-error` → `--color-danger` across `frontend/src`
   is sufficient — the suffix structure is identical across all five token variants.
5b. Update `docs/development/ui/tokens.md` and `docs/development/ui/primitives.md` per the
    UI documentation scope section above.
6. Verify no remaining occurrences:

   ```sh
   grep -r '\-\-color-error' frontend/src docs/development/ui
   ```

   Expected: zero matches. If any remain, fix before proceeding.
7. Run `cd frontend && npm run check && npm run test`.

**Note:** Steps 3–5 will have failing tests until all steps are complete. Run `npm run test`
only after step 6 confirms zero remaining occurrences.

---

## Testing

- `npm run check` — zero new type errors. TypeScript enforces the `TokenName` union for `.ts`
  files; run the step 6 grep to confirm CSS-string sites are also clean.
- `npm run test` — update inline snapshots in `design-token-values.test.ts` and the golden
  `expected` array in `theme-tokens.test.ts` before running; all tests must pass.
- Dark/light smoke: open a page using `tone="danger"` on `Button`, `Callout`, and
  `StatusBadge` in both themes; confirm the danger color renders correctly (orange-red in
  dark, red in light).

---

## Rollout

One PR titled:

```text
refactor(frontend): rename --color-error tokens to --color-danger
```

Blocks on: `2026-04-23-stat-card-design.md` implementation merged.

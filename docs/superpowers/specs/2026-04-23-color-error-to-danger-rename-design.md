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
  truth; `TokenName` is a TypeScript union, so any missed rename site is a compile error
  (`npm run check` catches it). All consumers are internal — no external stylesheet references
  these tokens.
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

- Pinned regex for the `aria-invalid` rule: `--color-error-border` → `--color-danger-border`.

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
41-file sweep above.

No other spec has a sequencing dependency on this rename. It is parallel-safe with all other
2026-04-23 specs once stat-card is merged.

---

## Migration pattern

1. In `frontend/src/theme/tokens.ts`:
   - Rename `const errorBase` → `const dangerBase`.
   - Replace all 5 `--color-error*` keys in the `TokenName` union and in the `tokens` object.
2. Run `npm run check` — TypeScript will surface every remaining reference to the old
   `TokenName` literals. Fix each one.
3. Update the three test files with pinned token names/snapshots (`tokens.test.ts`,
   `design-token-values.test.ts`, `theme-tokens.test.ts`).
4. Update `app.css` and `css-contract.test.ts`.
5. Replace all `var(--color-error*)` occurrences in the 41 component/route/test files.
   A project-wide find-and-replace on `--color-error` (5 patterns or a single
   `--color-error` → `--color-danger` pass) is sufficient — the suffix structure is
   identical.
6. Run `cd frontend && npm run check && npm run test`.

---

## Testing

- `npm run check` — zero new type errors. TypeScript enforces the `TokenName` union; any
  missed rename site fails to compile.
- `npm run test` — update inline snapshots in `design-token-values.test.ts` and the golden
  string in `theme-tokens.test.ts` before running; all tests must pass.
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

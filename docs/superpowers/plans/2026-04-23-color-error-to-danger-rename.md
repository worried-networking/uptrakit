# `--color-error` → `--color-danger` Token Rename Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task.
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rename the five `--color-error*` CSS custom properties to `--color-danger*`
across all frontend source files and UI documentation, without changing any token values.

**Architecture:** Three-phase execution — (1) rename the one internal const not caught by
string search, (2) a single bulk find-and-replace over all frontend source covering every
other reference, (3) update UI docs which live outside `frontend/src`. One commit at the
end once every reference is confirmed clean.

**Tech Stack:** TypeScript, Svelte, Vitest inline snapshots, CSS custom properties, Tailwind arbitrary-value syntax, macOS `sed` (BSD).

---

## Files modified

| File | Change |
| --- | --- |
| `frontend/src/theme/tokens.ts` | `errorBase` → `dangerBase`; 5 `TokenName` entries; 5 `tokens` object keys |
| `frontend/src/theme/tokens.test.ts` | 5 keys in `EXPECTED` object |
| `frontend/src/lib/theme/design-token-values.test.ts` | 5 keys in `SPEC`; 2 inline snapshot strings |
| `frontend/vite-plugins/theme-tokens.test.ts` | 10 lines in golden `expected` array |
| `frontend/src/app.css` | 1 line — aria-invalid rule |
| `frontend/src/lib/theme/css-contract.test.ts` | 1 regex string in `toMatch()` |
| `frontend/src/lib/components/ui/ActionBadge.svelte` | CSS var references |
| `frontend/src/lib/components/ui/Callout.svelte` | CSS var references |
| `frontend/src/lib/components/ui/ContextMenuItem.svelte` | CSS var references |
| `frontend/src/lib/components/ui/ContextMenuItem.test.ts` | expected string assertions |
| `frontend/src/lib/components/ui/FormFieldRow.svelte` | CSS var references |
| `frontend/src/lib/components/ui/StatCard.svelte` | `toneTokens` map `danger` entry |
| `frontend/src/lib/components/ui/StatCard.test.ts` | expected token assertion |
| `frontend/src/lib/components/ui/StatusBadge.svelte` | CSS var references |
| `frontend/src/lib/components/Button.svelte` | CSS var references |
| `frontend/src/lib/components/Button.test.ts` | expected class string assertions |
| `frontend/src/lib/components/Input.svelte` | CSS var references |
| `frontend/src/lib/components/Input.test.ts` | expected string assertions |
| `frontend/src/lib/components/Link.svelte` | CSS var references |
| `frontend/src/lib/components/Link.test.ts` | expected string assertions |
| `frontend/src/lib/components/Textarea.svelte` | CSS var references |
| `frontend/src/lib/components/Textarea.test.ts` | expected string assertions |
| `frontend/src/lib/components/ToastNotifications.svelte` | CSS var references |
| `frontend/src/lib/components/EditHostAssignmentModal.svelte` | CSS var references (8 occurrences) |
| `frontend/src/lib/components/AssignToHostModal.svelte` | CSS var references |
| `frontend/src/lib/components/BatchResultDialog.svelte` | CSS var references |
| `frontend/src/routes/history/+page.svelte` | CSS var references |
| `frontend/src/routes/hosts/+page.svelte` | CSS var references |
| `frontend/src/routes/hosts/hosts.test.ts` | expected string assertions |
| `frontend/src/routes/layout-button-migration.test.ts` | expected string assertions |
| `frontend/src/routes/profile/profile.test.ts` | expected string assertions |
| `frontend/src/routes/settings/EnrollmentTokenSettings.test.ts` | expected string assertions |
| `frontend/src/routes/settings/NotificationLogView.svelte` | CSS var references |
| `frontend/src/routes/settings/NotificationRulesSettings.svelte` | CSS var references |
| `frontend/src/routes/settings/NotificationRulesSettings.test.ts` | expected string assertions |
| `frontend/src/routes/settings/OidcProvidersSettings.svelte` | CSS var references |
| `frontend/src/routes/settings/OidcProvidersSettings.test.ts` | expected string assertions |
| `frontend/src/routes/settings/PluginConfigsTab.svelte` | CSS var references |
| `frontend/src/routes/settings/SchedulerTab.svelte` | CSS var references |
| `frontend/src/routes/settings/SystemServicesSettings.test.ts` | expected string assertions |
| `frontend/src/routes/software/[id]/software-detail-update-trigger.test.ts` | expected string assertions |
| `frontend/src/routes/software/ignore-rules-tab.test.ts` | expected string assertions |
| `docs/development/ui/tokens.md` | 10 token name cells + 10 role label cells + 1 adapter cell |
| `docs/development/ui/primitives.md` | 7 token references across 6 component sections |

---

## Task 1: Rename internal const `errorBase` → `dangerBase` in `tokens.ts`

`errorBase` is a file-private const identifier. It contains no occurrence of the substring
`--color-error`, so the bulk sed in Task 2 will not touch it. Rename it here first.

There are 9 occurrences: one `const` declaration and 8 usages inside `rgba()` calls (two per
token: one for `dark`, one for `light`).

**Files:**

- Modify: `frontend/src/theme/tokens.ts`

- [ ] **Step 1: Rename `errorBase` → `dangerBase` in `tokens.ts`**

  ```sh
  sed -i '' 's/errorBase/dangerBase/g' frontend/src/theme/tokens.ts
  ```

- [ ] **Step 2: Verify zero `errorBase` remain**

  ```sh
  grep -n "errorBase" frontend/src/theme/tokens.ts
  ```

  Expected: no output.

---

## Task 2: Bulk rename `--color-error` → `--color-danger` across all frontend files

One `sed` command replaces every occurrence of the substring `--color-error` with
`--color-danger` across all Svelte, TypeScript, and CSS files in `frontend/src` and
`frontend/vite-plugins`. This covers all five token variants simultaneously (the suffixes
`-bg`, `-border`, `-bg-hover`, `-border-hover` are all preserved).

**Files:** All 41 files in `frontend/src` and `frontend/vite-plugins` listed in the Files
Modified table above.

- [ ] **Step 1: Run the bulk rename**

  ```sh
  cd frontend
  find src vite-plugins -type f \( -name "*.svelte" -o -name "*.ts" -o -name "*.css" \) \
    | xargs sed -i '' 's/--color-error/--color-danger/g'
  ```

  This replaces `--color-error` → `--color-danger` as a literal substring, so:

  | Pattern replaced | Result |
  | --- | --- |
  | `--color-error` | `--color-danger` |
  | `--color-error-bg` | `--color-danger-bg` |
  | `--color-error-border` | `--color-danger-border` |
  | `--color-error-bg-hover` | `--color-danger-bg-hover` |
  | `--color-error-border-hover` | `--color-danger-border-hover` |

  Affected files include:
  - `tokens.ts` (`TokenName` union string literals + `tokens` object keys)
  - `tokens.test.ts`, `design-token-values.test.ts` (EXPECTED/SPEC object keys)
  - `design-token-values.test.ts` inline snapshot strings
  - `vite-plugins/theme-tokens.test.ts` golden `expected` array (10 lines)
  - `app.css` (`aria-invalid` rule)
  - `css-contract.test.ts` (`toMatch()` regex string)
  - All component `.svelte` files and their `.test.ts` counterparts
  - All route `.svelte` files and their `.test.ts` counterparts

- [ ] **Step 2: Verify zero `--color-error` occurrences remain in frontend**

  ```sh
  grep -r 'color-error' frontend/src frontend/vite-plugins
  ```

  Expected: no output. If any matches remain, fix them before continuing.

- [ ] **Step 3: Run TypeScript check**

  ```sh
  cd frontend && npm run check
  ```

  Expected: zero errors. TypeScript enforces the `TokenName` union — if any `.ts` file still
  references a removed `TokenName` literal, it will appear here. The grep in Step 2 already
  confirmed CSS-string usages are clean.

- [ ] **Step 4: Run tests**

  ```sh
  cd frontend && npm run test
  ```

  Expected: all tests pass. The inline snapshots in `design-token-values.test.ts` and the
  golden array in `theme-tokens.test.ts` were updated by the sed in Step 1, and the actual
  output from `cssForTheme()` now emits `--color-danger*` names — they match.

---

## Task 3: Update UI documentation

The `docs/development/ui/` files live outside `frontend/src` and were not touched by Task 2.
Two kinds of change:

1. Token names: `--color-error*` → `--color-danger*` (sed-handled)
2. Role labels in `tokens.md`: "Error" → "Danger" (separate sed pass)

**Files:**

- Modify: `docs/development/ui/tokens.md`
- Modify: `docs/development/ui/primitives.md`

- [ ] **Step 1: Replace token names in both docs**

  From the repo root:

  ```sh
  sed -i '' 's/--color-error/--color-danger/g' \
    docs/development/ui/tokens.md \
    docs/development/ui/primitives.md
  ```

  This updates all CSS custom property name occurrences in both files.

- [ ] **Step 2: Rename role labels in `tokens.md`**

  The dark and light theme tables use "Error", "Error background tint", etc. as the Role
  column. Replace all "Error" role labels with "Danger":

  ```sh
  sed -i '' 's/| Error/| Danger/g' docs/development/ui/tokens.md
  ```

  This matches all five role label patterns:

  | Before | After |
  | --- | --- |
  | `\| Error \|` | `\| Danger \|` |
  | `\| Error background tint \|` | `\| Danger background tint \|` |
  | `\| Error border \|` | `\| Danger border \|` |
  | `\| Error background tint (hover) \|` | `\| Danger background tint (hover) \|` |
  | `\| Error border (hover) \|` | `\| Danger border (hover) \|` |

  Each label appears twice — once in the dark theme table (rows 38–42) and once in the light
  theme table (rows 74–78). The sed replaces both in one pass.

- [ ] **Step 3: Verify `tokens.md` has no remaining `--color-error` or "Error" role labels**

  ```sh
  grep -n 'color-error\|^| Error' docs/development/ui/tokens.md
  ```

  Expected: no output.

  Spot-check the adapter table at line 226 manually to confirm it now reads:

  ```markdown
  | `preset-filled-error-*` / `error-*` | `--color-danger-*` |
  ```

  The left cell (`preset-filled-error-*`) is correct — the Skeleton utility family name is
  historical and is NOT renamed.

- [ ] **Step 4: Verify `primitives.md` has no remaining `--color-error`**

  ```sh
  grep -n 'color-error' docs/development/ui/primitives.md
  ```

  Expected: no output.

  Spot-check the following locations to confirm correctness:

  - Callout tone table (formerly line 178): `danger` row should read
    `| danger | --color-danger | --color-danger-bg | --color-danger-border |`
  - StatusBadge tone table (formerly line 249): same pattern.
  - Input/FormFieldRow line (formerly line 399):
    `Error state: border-[var(--color-danger-border)] bg-[var(--color-danger-bg)] via aria-invalid.`
  - ContextMenuItem prop comment (formerly line 561): `renders text in --color-danger`
  - ContextMenuItem rule (formerly line 614): `Destructive items use --color-danger text token.`
  - Button variant table (formerly line 680): `danger` row reads
    `| danger | --color-danger-bg | --color-danger-border | --color-danger |`
  - StatCard tone table (formerly line 839): `danger` row reads `| danger | --color-danger |`

---

## Task 4: Verify completeness and commit

- [ ] **Step 1: Confirm zero remaining `--color-error` occurrences everywhere**

  ```sh
  grep -r 'color-error' \
    frontend/src \
    frontend/vite-plugins \
    docs/development/ui
  ```

  Expected: no output. If any matches remain, fix before proceeding.

- [ ] **Step 2: Run TypeScript check from repo root**

  ```sh
  cd frontend && npm run check
  ```

  Expected: zero type errors.

- [ ] **Step 3: Run full test suite**

  ```sh
  cd frontend && npm run test
  ```

  Expected: all tests pass. Key tests that exercise the rename:

  - `src/theme/tokens.test.ts` — pins all 32 token name/value pairs; `--color-danger*` names must match.
  - `src/lib/theme/design-token-values.test.ts` — inline snapshot assertions for `cssForTheme('light')` and `cssForTheme('dark')`; both must contain `--color-danger*`.
  - `vite-plugins/theme-tokens.test.ts` — golden CSS string; both `:root {}` and `.dark {}` blocks must emit `--color-danger*`.
  - `src/lib/theme/css-contract.test.ts` — `aria-invalid` rule must reference `--color-danger-border`.
  - Component-level tests (`Button.test.ts`, `Input.test.ts`, etc.) — expected class/style strings must contain `--color-danger*`.

- [ ] **Step 4: Commit**

  ```sh
  cd .. && git add \
    frontend/src \
    frontend/vite-plugins \
    docs/development/ui/tokens.md \
    docs/development/ui/primitives.md
  git commit -m "refactor(frontend): rename --color-error tokens to --color-danger"
  ```

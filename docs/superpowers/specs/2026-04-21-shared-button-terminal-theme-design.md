# Shared Button Primitive + Terminal Theme Derivation — Design

**Parent spec:** `docs/superpowers/specs/2026-04-16-ui-design-language-design.md`
(§4.3 Buttons, §6 Terminal Output Shell)

**Sub-spec #2 of the UI design-language rollout.** Depends on sub-spec #1
(tokens + adapter migration) being merged first.

## Overview

Replace ad-hoc Skeleton preset-filled / preset-tonal button markup with a
prop-driven `Button.svelte` primitive plus a semantically-separate
`UpdateAllButton.svelte` primitive. Move the terminal-body ANSI palette out
of `TerminalOutput.svelte` into a typed `terminal-palette.ts` module that
imports shared colors from `tokens.ts` and defines ANSI-only constants
locally. Prove the primitives against five canary call sites (one per
variant plus a polymorphic-link exerciser) before sub-spec #3 begins the
full 244-site migration.

Terminal body stays **always-dark** regardless of app theme (parent spec §6
colour conventions are dark-only; matches VSCode / iTerm convention). Modal
chrome outside the xterm instance continues to follow `--bg-surface` /
`--border-default` / `--text-primary` and reflows automatically through the
sub-spec #1 CSS-var infrastructure.

## Goals

1. Ship a prop-driven `Button.svelte` covering primary / ghost / danger
   variants, sm / md sizes, polymorphic `href` branch, and loading state.
2. Ship a semantically-separate `UpdateAllButton.svelte` for the row-level
   badge control described in parent spec §4.3 "Update all" sub-table.
3. Extract the xterm theme into `frontend/src/theme/terminal-palette.ts`
   with shared spec §6 colors bound to `tokens.ts` entries — single source
   of truth for the colors both the modal chrome and the terminal body
   share.
4. Migrate five hand-picked canary call sites (one per variant + link
   branch) to validate the primitive API against real production shapes
   before #3a–k rollout.

## Non-goals

- Migration of the remaining 244 `<button>` call sites — those live in
  sub-specs #3a–k.
- Migration of `SurfaceInteractionButton.svelte` internals — covered by #4
  surface-layer parity.
- Light-theme terminal-body palette — always-dark per §6.
- Storybook / visual dev harness beyond the Playwright preview route.
- CI lint gate for "no raw hex outside `frontend/src/theme/`" — parent spec
  defers wiring to a later sub-spec.

## Components

### Button primitive

**Files:**

- `frontend/src/lib/components/Button.svelte`
- `frontend/src/lib/components/Button.test.ts`

**API (Svelte 5 runes + snippets):**

```ts
import type { Snippet } from 'svelte';
import type { MouseEventHandler } from 'svelte/elements';

export type ButtonVariant = 'primary' | 'ghost' | 'danger';
export type ButtonSize = 'sm' | 'md';

type CommonProps = {
  variant: ButtonVariant;
  size?: ButtonSize;
  disabled?: boolean;
  loading?: boolean;
  leadingIcon?: Snippet;
  trailingIcon?: Snippet;
  children: Snippet;
  class?: string;
};

export type ButtonProps =
  | (CommonProps & {
      href: string;
      type?: never;
      onclick?: never;
    })
  | (CommonProps & {
      href?: never;
      type?: 'button' | 'submit' | 'reset';
      onclick?: MouseEventHandler<HTMLButtonElement>;
    });
```

**Render branches:**

- No `href`: `<button type={type ?? 'button'} disabled={disabled || loading}
  aria-busy={loading || undefined} class={computedClass} onclick={onclick}>`
- `href` set: `<a href={href} role="button" aria-disabled={disabled ||
  loading || undefined} aria-busy={loading || undefined}
  class={computedClass}>` — native `<a>` has no `disabled`; `aria-disabled`
  plus `pointer-events-none` via class.

**Class contract (per parent spec §4.3):**

Base (applied to all variants):

```text
inline-flex items-center gap-1.5 rounded-[3px] font-bold uppercase tracking-wide
transition-[background,border-color,color] duration-[0.12s]
disabled:opacity-40 disabled:pointer-events-none
aria-disabled:opacity-40 aria-disabled:pointer-events-none
active:opacity-[0.88]
```

Size `md` (default):

```text
h-[23px] px-3 text-[9px]
```

Size `sm`:

```text
h-[19px] px-2 text-[8.5px]
```

Variant — primary:

```text
bg-[linear-gradient(90deg,var(--accent-deep),var(--accent))]
text-[var(--text-inverted)]
hover:bg-[linear-gradient(90deg,var(--accent-dark),var(--accent-bright))]
```

Variant — ghost:

```text
bg-transparent border border-[var(--border-default)]
text-[var(--text-primary)]
hover:bg-[var(--bg-raised)]
```

Variant — danger:

```text
bg-[rgba(var(--color-error-rgb),0.15)]
border border-[rgba(var(--color-error-rgb),0.35)]
text-[var(--color-error)]
hover:bg-[rgba(var(--color-error-rgb),0.22)]
hover:border-[rgba(var(--color-error-rgb),0.5)]
```

Loading: `leadingIcon` slot replaced by `<span class="animate-spin">…</span>`
inline spinner when `loading=true`.

Focus-visible: inherits from global `app.css` rule (sub-spec #1 §Interaction).
No variant-specific focus ring.

### UpdateAllButton primitive

**Files:**

- `frontend/src/lib/components/UpdateAllButton.svelte`
- `frontend/src/lib/components/UpdateAllButton.test.ts`

**API:**

```ts
import type { Snippet } from 'svelte';
import type { MouseEventHandler } from 'svelte/elements';

export type UpdateAllState = 'idle' | 'dim';

export type UpdateAllButtonProps = {
  state: UpdateAllState;
  count?: number;
  onclick: MouseEventHandler<HTMLButtonElement>;
  children?: Snippet;
  class?: string;
};
```

Renders `<button type="button">` only — row-level action, no link branch.
Default children fallback: literal `↑ Update all`. If `count !== undefined`,
append `·{count}` (with surrounding whitespace) after children.

**Class contract (parent spec §4.3 Update-all sub-table):**

Base:

```text
inline-flex items-center gap-1.5 h-[19px] px-2 rounded-[3px]
text-[8.5px] font-bold uppercase tracking-wide
transition-[background,border-color,color] duration-[0.12s]
```

State `idle`:

```text
bg-[rgba(var(--accent-rgb),0.06)]
border border-[rgba(var(--accent-rgb),0.20)]
text-[var(--accent)]
hover:bg-[rgba(var(--accent-rgb),0.18)]
hover:border-[rgba(var(--accent-rgb),0.45)]
hover:text-[var(--accent-bright)]
```

State `dim`:

```text
bg-transparent
border border-[var(--border-default)]
text-[var(--text-muted)]
pointer-events-none
```

`dim` intentionally does NOT use the `disabled` attr — element remains
focusable so screen readers describe the "nothing to update" context via
`aria-label`.

### Terminal palette module

**Files:**

- `frontend/src/theme/terminal-palette.ts`
- `frontend/src/theme/terminal-palette.test.ts`

**API:**

```ts
import type { ITheme } from '@xterm/xterm';
import { tokens } from './tokens';

const SUCCESS = tokens['--color-success'].dark;
const ERROR = tokens['--color-error'].dark;
const ACCENT_BRIGHT = tokens['--accent-bright'].dark;
const MUTED = tokens['--text-muted'].dark;
const PRIMARY = tokens['--text-primary'].dark;
const INVERTED = tokens['--text-inverted'].dark;
const INFO = tokens['--color-info'].dark;

// ANSI-only colors — not part of design language, kept local.
const TERM_BG = '#0c0c0e';
const TERM_FG = '#d4d4d8';
const SELECTION = '#3f3f46';
const ANSI_BLACK = '#18181b';
const ANSI_RED = '#f87171';
const ANSI_BLUE = '#60a5fa';
const ANSI_MAGENTA = '#c084fc';
const ANSI_BRIGHT_BLACK = '#3f3f46';
const ANSI_BRIGHT_RED = '#fb7185';
const ANSI_BRIGHT_GREEN = '#86efac';
const ANSI_BRIGHT_YELLOW = '#fde68a';
const ANSI_BRIGHT_BLUE = '#93c5fd';
const ANSI_BRIGHT_MAGENTA = '#d8b4fe';

// Parent spec §6 pins terminal yellow at `#fcd34d` (progress / in-flight
// layers). This is distinct from `--color-warning` dark (`#fbbf24`) by
// design — terminal amber sits higher on the ramp for readability.
const TERMINAL_AMBER = '#fcd34d';

export const TERMINAL_THEME: ITheme = {
  background: TERM_BG,
  foreground: TERM_FG,
  cursor: TERM_FG,
  selectionBackground: SELECTION,
  black: ANSI_BLACK,
  red: ANSI_RED,
  green: SUCCESS,
  yellow: TERMINAL_AMBER,
  blue: ANSI_BLUE,
  magenta: ANSI_MAGENTA,
  cyan: ACCENT_BRIGHT,
  white: PRIMARY,
  brightBlack: ANSI_BRIGHT_BLACK,
  brightRed: ANSI_BRIGHT_RED,
  brightGreen: ANSI_BRIGHT_GREEN,
  brightYellow: ANSI_BRIGHT_YELLOW,
  brightBlue: ANSI_BRIGHT_BLUE,
  brightMagenta: ANSI_BRIGHT_MAGENTA,
  brightCyan: INFO,
  brightWhite: INVERTED
};
```

**Parent spec §6 colour-convention binding:**

| Colour    | Use                         | Source in `terminal-palette.ts`                  |
| --------- | --------------------------- | ------------------------------------------------ |
| `#d4d4d8` | Default output              | `TERM_FG` (local constant)                       |
| `#52525b` | Timestamps, layer IDs (dim) | `brightBlack` (`ANSI_BRIGHT_BLACK`)              |
| `#22d3ee` | uptrakit annotations        | `cyan` ← `tokens['--accent-bright'].dark`        |
| `#fafafa` | Docker status lines         | `brightWhite` ← `tokens['--text-inverted'].dark` |
| `#4ade80` | Success lines               | `green` ← `tokens['--color-success'].dark`       |
| `#fcd34d` | Progress / in-flight layers | `yellow` (`TERMINAL_AMBER` local)                |
| `#fdba74` | Warnings / errors           | (not bound — see note below)                     |

Note on `#fdba74`: parent spec §6 lists it as "warnings / errors" but does
not pin it to an ANSI slot. Current terminal theme does not expose it as an
ANSI colour; output lines emit the hex directly via SGR sequences. Left
unchanged in this sub-spec.

### Terminal output consumer

**File (modified):** `frontend/src/lib/components/TerminalOutput.svelte`

One change: delete the inline `const TERMINAL_THEME = { ... }` literal
(currently lines 75–96), replace with
`import { TERMINAL_THEME } from '$lib/../theme/terminal-palette';` at the
top. No other changes to chrome, state machine, or xterm lifecycle. The
existing unit tests must continue to pass unchanged except for one
additional `it` block verifying the theme comes from the module.

### Tokens extension

**File (modified):** `frontend/src/theme/tokens.ts` (owned by sub-spec #1)

Add one token name: `--color-error-rgb` with values:

| Theme | Value           |
| ----- | --------------- |
| dark  | `234 88 12`     |
| light | `220 38 38`     |

Space-separated-RGB form, matches existing `--accent-rgb` shape. Required
by the `Button` danger variant's `rgba(var(--color-error-rgb), 0.15)`
compositions — CSS cannot multiply `var(--color-error)` hex against an
alpha channel in a single declaration.

Sub-spec #1's `tokens.test.ts` `EXPECTED` table, its type union, the
`cssForTheme` golden CSS, and sub-spec #1's `design-token-values.test.ts`
`SPEC` table plus both `toMatchInlineSnapshot` blocks all update as part
of this sub-spec's PR1 diff.

## Data flow

**Build time:**

1. `tokens.ts` exports `tokens` record (spec-pinned values, includes the
   new `--color-error-rgb`).
2. `terminal-palette.ts` imports `tokens` at build time — theme object
   resolves to concrete hex strings, no runtime CSS-var reads.
3. Button / UpdateAllButton templates embed `var(--accent-deep)`,
   `var(--color-error-rgb)`, etc. as literal CSS strings in their class
   attributes. Browser resolves them at paint time per active `.dark` class.

**Runtime:**

1. User toggles theme → `.dark` class toggles on `<html>` → sub-spec #1
   virtual-module CSS cascade updates every `var(--*)` reference → Button +
   UpdateAllButton recolor via global 120 ms transition → modal chrome
   recolors same way.
2. Terminal body stays at `#0c0c0e` because xterm consumed the theme
   object at mount time and never re-reads it. Intentional — always-dark
   body is the design target.

**Test time:**

- Vitest runs unit tests against `tokens.ts` (value pins + new
  `--color-error-rgb`), `Button.svelte` (variant × size matrix snapshot),
  `UpdateAllButton.svelte` (state branches), `terminal-palette.ts` (shared-
  token bindings + full theme inline snapshot).
- Playwright runs canary-route snapshots + a new `/dev/button-preview`
  route that renders every variant × state for 0.5 % visual-diff
  regression coverage.

## Error handling

- TS discriminated union on `ButtonProps` makes it a compile-time error to
  pass `type` with `href` or `onclick` with `href`. Tested via
  `// @ts-expect-error` fixtures.
- `loading=true` forces `aria-busy="true"` and short-circuits click
  handlers before they reach the consumer's `onclick`.
- `UpdateAllButton` dim state uses `pointer-events-none` at CSS level —
  verified in unit test by asserting the class is present, not by
  synthesising a click event (pointer-events is a CSS concern, not a JS
  one).
- `terminal-palette.ts` type signature (`ITheme` from `@xterm/xterm`)
  ensures every slot is filled at compile time — cannot ship a theme with
  a missing slot.
- Inline snapshot in `terminal-palette.test.ts` pins the full 20-slot
  object. Any unintentional drift fails CI.

## Testing

### Unit tests

Summarised in design section 6. Full breakdown:

**`Button.test.ts`** — renders branch by prop shape; variant × size class
snapshot matrix (6 cases); disabled + loading states; TS rejection of
invalid prop combinations via `// @ts-expect-error`; custom `class`
concatenation; `onclick` gating.

**`UpdateAllButton.test.ts`** — idle vs dim class contract; `count` suffix
rendering; default children fallback; click gating in dim state.

**`terminal-palette.test.ts`** — one assertion per parent spec §6 colour
binding that the shared slot equals the corresponding `tokens.ts` dark
value; inline snapshot of the full `TERMINAL_THEME` object.

**`TerminalOutput.test.ts`** (existing, extended) — one new `it` asserts
the imported `TERMINAL_THEME` identity is what xterm receives (mock xterm
constructor).

### Integration / e2e

- `button-primitive.spec.ts` (new) — renders `/dev/button-preview` in both
  themes, snapshots every variant × size × state permutation. Test route
  is gated behind a `dev` flag so it does not ship to production bundles.
- Canary routes re-baseline: `/profile`, confirm-dialog component-level
  snapshot, `PublicEntryShell` login route, `/settings` enrollment-tokens
  tab, `/software` index. Each delta documented in the PR2 description per
  parent spec §9 waiver schema.
- All other existing Playwright snapshots must remain within the 0.5 %
  threshold.

## Rollout

### PR1 — primitives + palette module

Pure additions plus one additive token.

1. Add `frontend/src/theme/tokens.ts` entry for `--color-error-rgb`;
   update sub-spec #1's `tokens.test.ts` `EXPECTED` table, the
   `cssForTheme` golden CSS in `theme-tokens.test.ts`, the
   `design-token-values.test.ts` `SPEC` table, and both
   `toMatchInlineSnapshot` blocks.
2. Add `frontend/src/lib/components/Button.svelte` +
   `Button.test.ts`.
3. Add `frontend/src/lib/components/UpdateAllButton.svelte` +
   `UpdateAllButton.test.ts`.
4. Add `frontend/src/theme/terminal-palette.ts` +
   `terminal-palette.test.ts`.
5. Run the full frontend gate (`lint`, `format:check`, `check`, `test`,
   `build`). Playwright e2e unchanged — no call site touched yet.
6. Commit, push, open PR titled "feat(frontend): add Button +
   UpdateAllButton primitives + terminal-palette module (sub-spec #2 PR 1)".

PR1 is trivially reversible — removing the four new files returns the
tree to post-#1 state.

### PR2 — canary migration + terminal consumer switch

1. `TerminalOutput.svelte`: delete inline `TERMINAL_THEME` literal,
   import from `terminal-palette.ts`. One-line change plus four lines of
   import. Extend `TerminalOutput.test.ts` with the mock-constructor
   assertion.
2. Migrate the five canary call sites:
   - `frontend/src/routes/profile/+page.svelte` — "Save changes" →
     `<Button variant="primary" type="submit" loading={saving}>`.
   - `frontend/src/lib/components/ConfirmDialog.svelte` — Cancel button →
     `<Button variant="ghost">`.
   - `frontend/src/lib/components/PublicEntryShell.svelte` — "Back to
     login" link → `<Button variant="ghost" href="/login">`.
   - `frontend/src/routes/settings/EnrollmentTokenSettings.svelte` — first
     "Revoke" action → `<Button variant="danger" size="sm" leadingIcon={...}>`.
   - `frontend/src/routes/software/+page.svelte` — header row "Update all"
     control → `<UpdateAllButton state={pendingCount > 0 ? 'idle' : 'dim'}
     count={pendingCount} onclick={...} />`.
3. Add `frontend/src/routes/dev/button-preview/+page.svelte` (dev-only).
4. Add `frontend/tests/e2e/button-primitive.spec.ts` — screenshots
   `/dev/button-preview` in dark + light themes.
5. Re-baseline the five canary route Playwright snapshots. PR description
   enumerates each deliberate delta (e.g., "profile Save button adopts
   primary gradient; 9 px weight 700, 23 px height per §4.3") per parent
   spec §9 waiver schema.
6. Run the full frontend gate (`lint`, `format:check`, `check`, `test`,
   `build`, `test:e2e`).
7. Commit, push, open PR titled "feat(frontend): migrate canary call sites
   and terminal palette consumer (sub-spec #2 PR 2)".

### Risk + rollback

PR1 is pure addition — revert removes four files, no consumer touched.
PR2 fallout reverts only the canary diffs + terminal import; primitives
stay in tree. Zero-downtime rollback posture.

### Divergence from parent-spec rollout

Parent spec §2.8 "Runtime Token Adapter" phase and §4.3 Button section
imply a sequence of "primitives, then migration." This sub-spec ships a
five-site canary **inside** PR2 of sub-spec #2 instead of deferring all
migration to #3a–k. Rationale: canaries prove the API against production
shapes before the 244-site sweep, so any API regret surfaces in isolation
rather than in a large cross-cutting PR.

## Dependencies + ordering

- **Blocks on:** sub-spec #1 PR2 merged (`tokens.ts` exists with semantic
  token entries).
- **Blocks:** sub-spec #3a–k (rely on Button / UpdateAllButton being
  available).
- **Parallel-safe with:** sub-spec #4 surface-layer parity (operates on
  `SurfaceInteractionButton` internals and surface shells, disjoint from
  Button primitive's scope). Both may ship concurrently once #1 merges.

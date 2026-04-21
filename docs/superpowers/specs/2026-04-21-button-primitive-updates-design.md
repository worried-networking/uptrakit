# Button Primitive Updates — Design

**Parent spec:** `docs/superpowers/specs/2026-04-16-ui-design-language-design.md`
(§4.3 Buttons)

**Sub-spec #2c of the UI design-language rollout.** Depends on sub-spec #2
merged. Blocks every #3 sub-spec that references `variant="secondary"`
(#3c–#3j) and every icon-only consumer sub-spec (#3b, #3j, #3k).

## Overview

Two additive changes to the `<Button>` primitive from sub-spec #2:

1. Add `'secondary'` to the `ButtonVariant` union and its class
   contract. Multiple #3 migration sub-specs reference `variant="secondary"`
   for reversible side actions (Deactivate, Test connection, Export, Back,
   Change password). Sub-spec #2 only shipped `'primary' | 'ghost' | 'danger'`
   — a real gap discovered during #3 drafting.
2. Add `ariaLabel?: string` prop on base `<Button>` for icon-only
   consumers (theme toggle, user menu trigger, toast dismiss, color
   swatch, pagination prev/next with icon-only labels). Sub-spec #2
   only exposed `ariaLabel` on the `UpdateAllButton` wrapper. Icon-only
   migration sites currently use `sr-only` text children as a tactical
   workaround — this sub-spec eliminates that workaround.

Pure primitive addition. No consumer migrations in this sub-spec;
those belong in #3k's sibling consumer sweep (and per-sub-spec use
in #3b / #3j).

## Design decisions

**Q1 — `'secondary'` variant class contract.**

- Options:
  - (chosen) Tonal shape per parent §4.3: muted background + default
    border + primary text. Distinct from `ghost` (which is transparent
    bg + border) and `primary` (which is gradient fill). Class contract:

    ```text
    bg-[var(--bg-raised)] border border-[var(--border-default)]
    text-[var(--text-primary)]
    hover:bg-[var(--bg-hover)]
    ```

  - Reuse `ghost` shape with a slight color swap. Rejected — visual
    collision; secondary and ghost should be distinguishable.
  - Introduce a fourth visual axis (e.g. outlined + filled). Rejected —
    YAGNI; no current consumer needs it.
- Reasoning: tonal background gives secondary a distinct visual weight
  from ghost (transparent) and primary (gradient); matches the "filled
  but muted" intent expressed by consumers like Deactivate, Back,
  Export.

**Q2 — `ariaLabel` prop: required or optional.**

- Options:
  - (chosen) `ariaLabel?: string` optional. Icon-only sites pass it;
    text-children sites omit it (accessible name already present via
    children). When present, primitive renders `aria-label={ariaLabel}`
    on the underlying element.
  - Require `ariaLabel` for every site. Rejected — noise at text-only
    sites where children already provide accessible name.
  - Require `ariaLabel` only when `children` is effectively empty.
    Rejected — not something the primitive can statically detect
    (snippets are opaque).
- Reasoning: optional prop lets consumers opt in based on their own
  semantic; primitive stays simple.

**Q3 — Icon-only detection heuristic.**

- Options:
  - (chosen) None. Consumers explicitly pass `ariaLabel` when the
    button is icon-only; the primitive does not inspect children.
  - Attempt runtime DOM inspection to detect empty text. Rejected —
    fragile; snippets don't expose rendered text.
- Reasoning: explicit > implicit; consumer ownership matches Svelte 5
  idiom.

**Q4 — UpdateAllButton `ariaLabel` prop already exists — dedupe?**

- Options:
  - (chosen) `UpdateAllButton` continues to expose its own `ariaLabel`
    prop; internally it passes the value through to the base `<Button
    ariaLabel={...}>` prop added in this sub-spec. No API break at the
    UpdateAllButton consumer layer.
  - Remove `UpdateAllButton.ariaLabel`; force consumers to pass via
    base Button. Rejected — UpdateAllButton is a consumer wrapper;
    consumers talk to UpdateAllButton's API, not to base Button's.
- Reasoning: wrapper primitives keep their own consumer contract;
  internal plumbing consolidates.

**Q5 — PR shape: bundle with #2 reviewer fixes vs standalone.**

- Options:
  - (chosen) Standalone PR. Cleaner bisect on regression (secondary
    class contract changes render output app-wide once consumers start
    using it; separable review).
  - Amend into sub-spec #2's merged work. Rejected — #2 merged; no
    post-hoc amendments.
  - Bundle into #3k (first heavy consumer). Rejected — couples primitive
    API change with its first consumer migration; two reviewer loops
    want separation.
- Reasoning: primitive-per-spec pattern, matching #2 and #2b.

## Goals

1. `ButtonVariant` union: `'primary' | 'secondary' | 'ghost' | 'danger'`.
2. `secondary` variant renders tonal background + default border + primary
   text per §4.3 (class contract above).
3. Base `<Button>` accepts `ariaLabel?: string` prop; renders
   `aria-label={ariaLabel}` on the underlying element when present.
4. `UpdateAllButton` wrapper continues to own its own `ariaLabel` prop;
   value passes through to base Button internally (no consumer-facing
   API change for UpdateAllButton callers).

## Non-goals

- Consumer migration — #3b, #3c, #3d, #3e, #3h, #3i, #3j, #3k all
  reference these props; their own PRs consume them.
- Additional variants beyond `secondary` — deferred until a fifth
  consumer emerges.
- `leadingIcon` / `trailingIcon` redesign — both already ship in #2.
- UpdateAllButton behavioral changes — scope limited to internal
  wiring of the new base-Button `ariaLabel` prop.

## Components

### Button primitive update

**File:** `frontend/src/lib/components/Button.svelte`

**API delta:**

```ts
// Before (from sub-spec #2):
export type ButtonVariant = 'primary' | 'ghost' | 'danger';

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

// After:
export type ButtonVariant = 'primary' | 'secondary' | 'ghost' | 'danger';

type CommonProps = {
  variant: ButtonVariant;
  size?: ButtonSize;
  disabled?: boolean;
  loading?: boolean;
  leadingIcon?: Snippet;
  trailingIcon?: Snippet;
  ariaLabel?: string;
  children: Snippet;
  class?: string;
};
```

**Render update:** underlying `<button>` / `<a>` element adds
`aria-label={ariaLabel ?? undefined}`. Undefined value produces no
attribute (native fallback: accessible name from children).

**Class contract — `secondary`:**

```text
bg-[var(--bg-raised)] border border-[var(--border-default)]
text-[var(--text-primary)]
hover:bg-[var(--bg-hover)]
```

All existing variants (`primary`, `ghost`, `danger`) unchanged.

### UpdateAllButton passthrough

**File:** `frontend/src/lib/components/UpdateAllButton.svelte`

**Change:** existing `ariaLabel?: string` prop now forwards to the base
`<Button ariaLabel={ariaLabel}>` call (instead of emitting `aria-label`
directly on the rendered DOM). No consumer-facing API change.

## Data flow

Build-time only. No runtime behavior changes. Secondary variant uses
tokens already shipped by sub-spec #1 (`--bg-raised`, `--bg-hover`,
`--border-default`, `--text-primary`); no new token additions.

## Error handling

- TS union update rejects invalid variant values at compile time.
  Existing `variant="secondary"` type-errors in in-flight consumer PRs
  become valid after this sub-spec merges.
- `ariaLabel` missing on icon-only site: not a compile error (prop is
  optional). Consumer unit tests (in #3b, #3j, #3k) assert the prop is
  passed on icon-only sites; regression caught there, not here.

## Testing

### Unit tests

Extend `frontend/src/lib/components/Button.test.ts`:

- Variant matrix: `secondary` renders `bg-[var(--bg-raised)]` +
  `border` class fragments; hover override renders `bg-[var(--bg-hover)]`.
- `ariaLabel` prop: when passed, DOM node carries `aria-label=<value>`;
  when omitted, DOM node has no `aria-label` attribute.
- Type-level: `@ts-expect-error` for variant values outside the new
  union (e.g. `variant="tertiary"`).
- Both `<button>` and `<a>` branches receive `aria-label` identically.

Extend `frontend/src/lib/components/UpdateAllButton.test.ts`:

- When consumer passes `ariaLabel`, base Button DOM node renders with
  the value (not just UpdateAllButton's outer wrapper).

### Integration / e2e

Extend `/dev/button-preview` (from sub-spec #2) to include:

- `secondary` variant at both sizes (md + sm).
- Icon-only button with `ariaLabel` set — Playwright asserts
  `aria-label` attribute present via locator.

Re-baseline `/dev/button-preview` snapshots; one variant row added.

## Rollout

Single PR titled
"feat(frontend): add Button 'secondary' variant + ariaLabel prop (sub-spec #2c)".

1. `Button.svelte` — extend `ButtonVariant` union; add `secondary`
   class contract; add `ariaLabel` prop; wire `aria-label` render.
2. `UpdateAllButton.svelte` — forward `ariaLabel` through to base
   Button.
3. Extend `Button.test.ts` per plan.
4. Extend `UpdateAllButton.test.ts` per plan.
5. Extend `/dev/button-preview` route.
6. Re-baseline Playwright snapshot for preview route.
7. Full frontend gate.

### Risk + rollback

Revert of one PR removes the `secondary` variant and the `ariaLabel`
prop. Consumer sub-specs (#3c–#3j) that ship after #2c will type-error
on `variant="secondary"` if this sub-spec is reverted — signals the
need to re-apply. Low-risk standalone.

### Dependencies + ordering

- **Blocks on:** sub-spec #2 merged.
- **Blocks:** sub-spec #3c (first `variant="secondary"` consumer),
  #3d, #3e, #3h, #3i, #3j, #3k; sub-spec #3b / #3j / #3k icon-only
  consumers wanting `ariaLabel` without `sr-only` fallback.
- **Parallel-safe with:** sub-spec #2b, sub-spec #3a, sub-spec #3a2
  (none reference `secondary` or base-Button `ariaLabel`).

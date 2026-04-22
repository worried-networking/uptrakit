# Button Primitive Updates — Design

**Parent spec:** `docs/superpowers/specs/2026-04-16-ui-design-language-design.md` (§4.3 Buttons)

**Sub-spec #2c of the UI design-language rollout.** Depends on sub-spec #2 merged. Blocks every #3 sub-spec that
references `variant="secondary"` (#3c–#3j) and every icon-only consumer sub-spec (#3b, #3j, #3k).

## Overview

Three additive changes to close gaps from sub-spec #2 / #1 discovered during #3 drafting:

1. Add `'secondary'` to the `ButtonVariant` union and its class contract. Multiple #3 migration sub-specs reference
   `variant="secondary"` for reversible side actions (Deactivate, Test connection, Export, Back, Change password).
   Sub-spec #2 only shipped `'primary' | 'ghost' | 'danger'` — a real gap.
2. Add `ariaLabel?: string` prop on base `<Button>` for icon-only consumers (theme toggle, user menu trigger, toast
   dismiss, color swatch, pagination prev/next with icon-only labels). Sub-spec #2 only exposed `ariaLabel` on the
   `UpdateAllButton` wrapper. Icon-only migration sites currently use `sr-only` text children as a tactical workaround —
   this sub-spec eliminates that workaround.
3. Add `--bg-hover` token to the sub-spec #1 token set. Used by the new `secondary` variant hover state and by
   active-nav / active-tab / active-filter override class applied across #3b, #3c, #3g, #3i. Sub-spec #1 shipped
   `--bg-base`, `--bg-surface`, `--bg-raised` only.

Pure primitive + token addition. No consumer migrations in this sub-spec; those belong in #3k's sibling consumer sweep
(and per-sub-spec use in #3b / #3c / #3g / #3i / #3j).

## Design decisions

**Q1 — `'secondary'` variant class contract.**

- Options:
  - (chosen) Tonal shape per parent §4.3: muted background + default border + primary text. Distinct from `ghost` (which
    is transparent bg + border) and `primary` (which is gradient fill). Class contract:

    ```text
    bg-[var(--bg-raised)] border border-[var(--border-default)]
    text-[var(--text-primary)]
    hover:bg-[var(--bg-hover)]
    active:opacity-[0.88]
    ```

    Border stays `--border-default` on hover (no border shift — unlike `danger`, which shifts border color).
    Focus-visible ring inherited from the global `:focus-visible` rule in `app.css` (parent §2.6 focus-ring) — no
    per-variant override. `disabled` state inherits the parent-spec-wide `opacity: 0.4` rule; no per-variant override.

  - Reuse `ghost` shape with a slight color swap. Rejected — visual collision; secondary and ghost should be
    distinguishable.
  - Introduce a fourth visual axis (e.g. outlined + filled). Rejected — YAGNI; no current consumer needs it.

- Reasoning: tonal background gives secondary a distinct visual weight from ghost (transparent) and primary (gradient);
  matches the "filled but muted" intent expressed by consumers like Deactivate, Back, Export.

**Q2 — `ariaLabel` prop: required or optional.**

- Options:
  - (chosen) `ariaLabel?: string` optional. Icon-only sites pass it; text-children sites omit it (accessible name
    already present via children). When present, primitive renders `aria-label={ariaLabel}` on the underlying element.
  - Require `ariaLabel` for every site. Rejected — noise at text-only sites where children already provide accessible
    name.
  - Require `ariaLabel` only when `children` is effectively empty. Rejected — not something the primitive can statically
    detect (snippets are opaque).
- Reasoning: optional prop lets consumers opt in based on their own semantic; primitive stays simple.

**Q3 — Icon-only detection heuristic.**

- Options:
  - (chosen) None. Consumers explicitly pass `ariaLabel` when the button is icon-only; the primitive does not inspect
    children.
  - Attempt runtime DOM inspection to detect empty text. Rejected — fragile; snippets don't expose rendered text.
- Reasoning: explicit > implicit; consumer ownership matches Svelte 5 idiom.

**Q4 — UpdateAllButton `ariaLabel` prop already exists — dedupe?**

- Options:
  - (chosen) `UpdateAllButton` continues to expose its own `ariaLabel` prop and emits `aria-label` natively on its
    standalone `<button>` element. Sub-spec #2 implemented `UpdateAllButton` as a standalone `<button>` (not a
    wrapper around `<Button>`), so there is no inner `<Button>` call to forward through. No internal change required
    in this sub-spec; no API break at the UpdateAllButton consumer layer.
  - Remove `UpdateAllButton.ariaLabel`; force consumers to pass via base Button. Rejected — UpdateAllButton is a
    standalone component with its own consumer contract.
- Reasoning: UpdateAllButton already writes `aria-label` directly to its DOM element. The forwarding model described
  in the original draft assumed a wrapper architecture that was not used in #2's implementation. No change needed.

**Q5 — PR shape: bundle with #2 reviewer fixes vs standalone.**

- Options:
  - (chosen) Standalone PR. Cleaner bisect on regression (secondary class contract changes render output app-wide once
    consumers start using it; separable review).
  - Amend into sub-spec #2's merged work. Rejected — #2 merged; no post-hoc amendments.
  - Bundle into #3k (first heavy consumer). Rejected — couples primitive API change with its first consumer migration;
    two reviewer loops want separation.
- Reasoning: primitive-per-spec pattern, matching #2 and #2b.

## Goals

1. `ButtonVariant` union: `'primary' | 'secondary' | 'ghost' | 'danger'`.
2. `secondary` variant renders tonal background + default border + primary text per §4.3 (class contract above).
3. Base `<Button>` accepts `ariaLabel?: string` prop; renders `aria-label={ariaLabel}` on the underlying element when
   present.
4. `UpdateAllButton` continues to expose its own `ariaLabel` prop and emits `aria-label` natively on its standalone
   `<button>` element. No internal change required for this sub-spec (no inner `<Button>` to forward through).
5. Token set gains `--bg-hover` in both dark and light themes. Added to `TokenName` union in
   `frontend/src/lib/theme/tokens.ts` (the tokens module created by sub-spec #1 PR2); the adapter-manifest generated
   fixture (also owned by sub-spec #1) picks the new token up automatically via the test-side `canonicalTokens` list,
   which this sub-spec extends. Value matches Skeleton's historical hover intensity (dark: one step above `--bg-raised`;
   light: one step darker than `--bg-surface`). Exact rgba values fixed during implementation against current production
   hover rendering.

## Non-goals

- Consumer migration — #3b, #3c, #3d, #3e, #3h, #3i, #3j, #3k all reference these props; their own PRs consume them.
- Additional variants beyond `secondary` — deferred until a fifth consumer emerges.
- `leadingIcon` / `trailingIcon` redesign — both already ship in #2.
- UpdateAllButton behavioral changes — scope limited to internal wiring of the new base-Button `ariaLabel` prop.

## Components

### Button primitive update

**File:** `frontend/src/lib/components/Button.svelte`

**API delta:**

```ts
// Before (from sub-spec #2):
export type ButtonVariant = "primary" | "ghost" | "danger";

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
export type ButtonVariant = "primary" | "secondary" | "ghost" | "danger";

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

**Render update:** both render branches (the `<button>` branch and the `<a>` branch when `href` is present) add
`aria-label={ariaLabel ?? undefined}`. Undefined value produces no attribute (native fallback: accessible name from
children — Svelte strips `undefined` attribute values).

**Class contract — `secondary`:**

```text
bg-[var(--bg-raised)] border border-[var(--border-default)]
text-[var(--text-primary)]
hover:bg-[var(--bg-hover)]
active:opacity-[0.88]
```

Border stays `--border-default` on hover; focus-visible ring inherited from the global rule (parent §2.6); `disabled`
inherits the variant-agnostic `opacity: 0.4` rule.

All existing variants (`primary`, `ghost`, `danger`) unchanged.

### UpdateAllButton passthrough

**File:** `frontend/src/lib/components/UpdateAllButton.svelte`

**Change:** none required. Sub-spec #2 implemented `UpdateAllButton` as a standalone `<button>` element (not a
wrapper around `<Button>`). The existing `ariaLabel?: string` prop already writes directly to `aria-label` on that
element. No internal wiring change is needed and no consumer-facing API change occurs.

## Data flow

Build-time only. No runtime behavior changes. Secondary variant uses tokens shipped by sub-spec #1 plus the new
`--bg-hover` token introduced here. Token inventory update: `TokenName` union in `tokens.ts` gains `'--bg-hover'`;
adapter-manifest fixture updated to cover both themes.

## Error handling

- TS union update rejects invalid variant values at compile time. Existing `variant="secondary"` type-errors in
  in-flight consumer PRs become valid after this sub-spec merges.
- `ariaLabel` missing on icon-only site: not a compile error (prop is optional). Consumer unit tests (in #3b, #3j, #3k)
  assert the prop is passed on icon-only sites; regression caught there, not here.

## Testing

### Unit tests

Extend `frontend/src/lib/components/Button.test.ts`:

- Variant matrix: `secondary` renders `bg-[var(--bg-raised)]` + `border` class fragments; hover override renders
  `hover:bg-[var(--bg-hover)]` (class-presence assertion — not computed-style; jsdom does not resolve `var(--*)`).
- Variant matrix: `secondary` + `disabled` carries `opacity-40` (the parent-spec-wide disabled rule applies; regression
  guard against a per-variant override sneaking in).
- Variant matrix: `secondary` renders `active:opacity-[0.88]`.
- `ariaLabel` prop: when passed, DOM node carries `aria-label=<value>`; when omitted, DOM node has no `aria-label`
  attribute (asserts that Svelte strips the `undefined` attribute value).
- Type-level: `@ts-expect-error` for variant values outside the new union (e.g. `variant="tertiary"`).
- Both branches receive `aria-label` identically: one assertion on the `<button>` render path (omit `href`) and one on
  the `<a>` render path (pass `href="/x"`), with the same `ariaLabel` value.

Extend `frontend/src/lib/components/UpdateAllButton.test.ts`:

- When consumer passes `ariaLabel`, the root `<button>` element has `aria-label` set to that value. No new
  behavior — this is a non-regression assertion confirming the existing implementation continues to emit the
  attribute directly. No inner `<Button>` node exists to assert against.

### Integration / e2e

Extend `/dev/button-preview` (from sub-spec #2) to include:

- `secondary` variant at both sizes (md + sm).
- Icon-only button with `ariaLabel` set — Playwright asserts `aria-label` attribute present via locator.

Re-baseline `/dev/button-preview` snapshots; one variant row added.

## Rollout

Single PR titled "feat(frontend): add Button 'secondary' variant + ariaLabel prop (sub-spec #2c)".

1. `frontend/src/lib/theme/tokens.ts` — add `'--bg-hover'` to `TokenName` union; add dark + light values to the `tokens`
   record.
2. `frontend/src/lib/theme/adapter-manifest.test.ts` — extend assertions to cover the new token in both themes.
3. `Button.svelte` — extend `ButtonVariant` union; add `secondary` class contract (consumes `--bg-hover`); add
   `ariaLabel` prop; wire `aria-label` render.
4. `UpdateAllButton.svelte` — no change required (ariaLabel already emits natively; see Q4).
5. Extend `Button.test.ts` per plan.
6. Extend `UpdateAllButton.test.ts` per plan.
7. Extend `/dev/button-preview` route.
8. Re-baseline Playwright snapshot for preview route.
9. Full frontend gate.

### Risk + rollback

Revert of one PR removes the `secondary` variant and the `ariaLabel` prop. Consumer sub-specs (#3c–#3j) that ship after
sub-spec #2c will type-error on `variant="secondary"` if this sub-spec is reverted — signals the need to re-apply.
Low-risk standalone.

### Dependencies + ordering

- **Blocks on:** sub-spec #2 merged.
- **Blocks:** sub-spec #3c (first `variant="secondary"` consumer), #3d, #3e, #3h, #3i, #3j, #3k; sub-spec #3b / #3j /
  #3k icon-only consumers wanting `ariaLabel` without `sr-only` fallback.
- **Parallel-safe with:** sub-spec #2b, sub-spec #3a, sub-spec #3a2 (none reference `secondary` or base-Button
  `ariaLabel`).

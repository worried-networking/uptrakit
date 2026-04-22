# Input + Checkbox + Link Primitives — Design

**Parent spec:** `docs/superpowers/specs/2026-04-16-ui-design-language-design.md` (§4.10 Form Validation, §2.6
Interaction)

**Sub-spec #2b of the UI design-language rollout.** Depends on sub-spec #1 (tokens + adapter migration). Drafted after
sub-spec #3a so the Button primitive pattern is established first.

## Overview

Introduce three new shared primitives in `frontend/src/lib/components/`: `Input.svelte`, `Checkbox.svelte`,
`Link.svelte`. Each is a thin wrapper around the corresponding native element that encodes parent-spec §4.10 field +
§2.6 focus-ring conventions as a single class contract. Ship them as pure additions — no consumer migration in this
sub-spec. Migration lives in downstream sub-specs (#3a2 and the #3b–k sweep).

## Design decisions

**Q1 — API shape: dedicated primitive vs utility class + FormFieldRow wrapper.**

- Options:
  - (chosen) Dedicated primitives (`<Input>`, `<Checkbox>`, `<Link>`) with `class?` passthrough and discriminated unions
    where the element shape varies (text vs password vs email).
  - Utility classes only (extend the existing `PUBLIC_ENTRY_*` pattern globally). Rejected — the whole sub-spec #2
    precedent is "primitives over utility classes"; repeating the utility pattern for inputs would fork the design
    language.
  - Co-locate all field logic inside `FormFieldRow.svelte`. Rejected — couples label-layout concerns to element-styling
    concerns; breaks reuse when an input is rendered outside a labeled row.
- Reasoning: the same argument that drove sub-spec #2 applies — primitive ownership of the design contract enables
  single-point design audits and consistent a11y wiring.

**Q2 — `Input` type coverage.**

- Options:
  - (chosen) Support `'text' | 'email' | 'password' | 'url' | 'number' | 'search'` via discriminated union; each
    enforces its own attributes (e.g. `autocomplete` mandatory for `password`, `inputmode="numeric"` default on
    `number`). Textarea excluded — separate primitive if needed later.
  - Single string type with open `type: string`. Rejected — loses compile-time enforcement of sensible defaults.
- Reasoning: public entry and settings forms use exactly these six types; textarea is niche (only one call site — plugin
  config editor) and deserves its own primitive if/when it comes up.

**Q3 — Validation API: error prop vs `aria-invalid` passthrough.**

- Options:
  - (chosen) Accept `error?: string | undefined`. When present: renders `aria-invalid="true"`, switches border to
    `--color-error-border`, switches bg to `--color-error-bg`. `aria-describedby` wiring is owned by the enclosing
    `FormFieldRow` — it renders the error copy node with a stable id and sets `aria-describedby` on the nested `<Input>`
    via the `aria-describedby` pass-through prop. `<Input>` itself does not render error copy or emit an internal
    describedby id; callers that need error copy outside a `FormFieldRow` must render their own `<p id="...">` and pass
    `aria-describedby` on `<Input>` (via the class/attrs passthrough or an explicit prop).
  - Accept only `aria-invalid` passthrough; error copy stays in `FormFieldRow`. Rejected — forces consumers to manage
    two props (`aria-invalid` on `<Input>`, `error` on `<FormFieldRow>`) that are always coupled.
- Reasoning: error state is a visual + a11y concern of the input itself; keeping it in one prop matches how consumers
  think about validation.

**Q4 — `Link` primitive scope: navigation only, or also button-styled links.**

- Options:
  - (chosen) Navigation only. `<Link href="..." variant="default | muted | danger">` for inline prose links.
    Button-styled links (e.g. login card footer "Register") already handled by `<Button href="..." variant="...">` from
    sub-spec #2.
  - Union primitive covering prose + button shapes. Rejected — doubles the variant matrix and duplicates Button's href
    branch.
- Reasoning: the purpose of `<Link>` is to replace `PUBLIC_ENTRY_LINK_CLASS` prose-link utility, not to re-derive
  Button's link branch.

**Q5 — `Checkbox`: native `<input type="checkbox">` vs custom div.**

- Options:
  - (chosen) Native `<input type="checkbox">` styled via `accent-color: var(--accent)` + focus ring rules.
  - Custom div-based toggle. Rejected — breaks form semantics (HTML form serialization, AT support, keyboard
    navigation).
- Reasoning: accessibility and form-serialization free out of the box with native.

**Q6 — Where the primitives live in `$lib/components/`.**

- Options:
  - (chosen) `frontend/src/lib/components/Input.svelte`, `Checkbox.svelte`, `Link.svelte` — peer of `Button.svelte` from
    #2.
  - Nested under `frontend/src/lib/components/ui/`. Rejected — sub-spec #2 placed Button at the `components/` root; keep
    primitive layout flat and consistent. The `ui/` subfolder is reserved for composite layouts (PageShell,
    PublicEntryShell, Callout).
- Reasoning: consistency with sub-spec #2 placement decision.

## Goals

1. Ship `<Input>`, `<Checkbox>`, `<Link>` primitives implementing §4.10 field conventions + §2.6 focus-ring rules.
2. Provide typed discriminated unions so invalid prop shapes fail at compile time.
3. Encode error state as a single `error?: string` prop that composes with `FormFieldRow`.
4. Provide a `/dev/form-primitive-preview` route for Playwright visual regression coverage.

## Non-goals

- Textarea primitive — niche, out of scope.
- Select / combobox primitives — handled by existing `ProviderSelector` etc.; if/when unified, a separate sub-spec.
- Migration of any consumer — #3a2, #3b–k, #3k.
- Radio group primitive — no current consumers use them.
- File upload primitive — current consumers have none.

## Components

### Input primitive

**Files:**

- `frontend/src/lib/components/Input.svelte`
- `frontend/src/lib/components/Input.test.ts`

**API (Svelte 5 runes):**

```ts
import type { Snippet } from "svelte";

export type InputType = "text" | "email" | "password" | "url" | "number" | "search";

export type InputProps = {
  id: string;
  type: InputType;
  value: string;
  name?: string;
  placeholder?: string;
  autocomplete?: string;
  disabled?: boolean;
  required?: boolean;
  error?: string;
  oninput?: (e: Event) => void;
  onblur?: (e: FocusEvent) => void;
  class?: string;
};
```

Note: `value` is bindable via `bind:value` — implementation declares it with `$bindable()` at `$props()` destructure
time (`let { value = $bindable(''), ... }: InputProps = $props();`).

**Class contract (parent §4.10):**

```text
h-8 w-full px-[10px] rounded-[3px]
bg-[var(--bg-surface)] border border-[var(--border-default)]
text-sm text-[var(--text-primary)]
placeholder:text-[var(--text-muted)]
focus-visible:outline-none
focus-visible:shadow-[0_0_0_3px_rgba(var(--accent-rgb),0.25)]
disabled:opacity-40 disabled:cursor-not-allowed
aria-[invalid=true]:border-[var(--color-error-border)]
aria-[invalid=true]:bg-[var(--color-error-bg)]
transition-[background,border-color] duration-[0.12s]
```

Note the radius change: `rounded-[3px]` (parent §4.3/§4.10 convention), not `rounded-lg` as used by the legacy
`PUBLIC_ENTRY_INPUT_CLASS`. This is a deliberate spec-conformance shift; flagged as a visual delta to enumerate in the
sub-spec #3a2 Playwright baseline.

### Checkbox primitive

**Files:**

- `frontend/src/lib/components/Checkbox.svelte`
- `frontend/src/lib/components/Checkbox.test.ts`

**API:**

```ts
export type CheckboxProps = {
  id: string;
  checked: boolean;
  name?: string;
  disabled?: boolean;
  onchange?: (e: Event) => void;
  class?: string;
};
```

Note: `checked` is bindable via `bind:checked` — implementation declares it with `$bindable()`
(`let { checked = $bindable(false), ... }: CheckboxProps = $props();`).

**Class contract:**

```text
h-4 w-4 rounded-[2px]
border border-[var(--border-default)]
accent-[var(--accent)]
focus-visible:outline-none
focus-visible:shadow-[0_0_0_3px_rgba(var(--accent-rgb),0.25)]
disabled:opacity-40 disabled:cursor-not-allowed
```

### Link primitive

**Files:**

- `frontend/src/lib/components/Link.svelte`
- `frontend/src/lib/components/Link.test.ts`

**API:**

```ts
import type { Snippet } from "svelte";

export type LinkVariant = "default" | "muted" | "danger";

export type LinkProps = {
  href: string;
  variant?: LinkVariant;
  external?: boolean;
  children: Snippet;
  class?: string;
};
```

When `external=true`, primitive adds `target="_blank"` and `rel="noopener noreferrer"` automatically.

**Class contract by variant:**

Base (all variants):

```text
font-medium underline underline-offset-4
focus-visible:outline-none
focus-visible:shadow-[0_0_0_3px_rgba(var(--accent-rgb),0.25)]
transition-colors duration-[0.12s]
```

`default`:

```text
text-[var(--accent)] hover:text-[var(--accent-bright)]
```

`muted`:

```text
text-[var(--text-muted)] hover:text-[var(--text-primary)]
```

`danger`:

```text
text-[var(--color-error)] hover:text-[var(--color-error)] hover:opacity-80
```

## Data flow

Build-time only. Each primitive imports no tokens at runtime; every `var(--*)` reference is a CSS literal in the class
attribute. Browser resolves per active `.dark` class. No changes to `tokens.ts` or the virtual-module CSS cascade.

## Error handling

- TS discriminated union on `InputType` prevents invalid `type` values at compile time.
- `error` prop non-empty → `aria-invalid="true"` automatically; paired with class-level `aria-[invalid=true]:`
  selectors.
- `external=true` without valid URL fails at runtime via browser navigation; primitive performs no URL validation
  (matches native behavior).

## Testing

### Unit tests

- `Input.test.ts` — type-branch snapshot matrix (6 types); error state toggle; disabled state; bind-value round-trip;
  `oninput` / `onblur` firing; TS `@ts-expect-error` for invalid `type` values.
- `Checkbox.test.ts` — checked state toggle; disabled state; `onchange` firing; `bind:checked` round-trip; a11y attrs.
- `Link.test.ts` — variant × external matrix; `rel`/`target` auto-set on external; default+muted+danger class
  assertions.

### Integration / e2e

- `/dev/form-primitive-preview` — dev-only route rendering each primitive with every variant × state permutation in both
  themes. Playwright snapshot gate.

## Rollout

Single PR, pure addition:

1. Add `frontend/src/lib/components/Input.svelte` + `Input.test.ts`.
2. Add `frontend/src/lib/components/Checkbox.svelte` + `Checkbox.test.ts`.
3. Add `frontend/src/lib/components/Link.svelte` + `Link.test.ts`.
4. Add `frontend/src/routes/dev/form-primitive-preview/+page.svelte`.
5. Add `frontend/tests/e2e/form-primitive.spec.ts`.
6. Full frontend gate.
7. Commit, PR titled "feat(frontend): add Input + Checkbox + Link primitives (sub-spec #2b)".

### Risk + rollback

Reverting one PR removes eight new files (three primitives + three test files + one preview route + one e2e spec). No
consumers touched.

### Dependencies + ordering

- **Blocks on:** sub-spec #1 PR2 merged.
- **Blocks:** sub-spec #3a2 (public-entry input/checkbox/link migration).
- **Parallel-safe with:** sub-spec #3a (Button-only migration), sub-spec #2 PR2, sub-spec #2c, sub-spec #2d, sub-spec
  #4.

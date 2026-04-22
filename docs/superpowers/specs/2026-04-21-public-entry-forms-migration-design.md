# Public Entry Forms Migration — Design

**Parent spec:** `docs/superpowers/specs/2026-04-16-ui-design-language-design.md` (§4.10 Form Validation)

**Sub-spec #3a2 of the UI design-language rollout.** Depends on sub-spec #2b (Input + Checkbox + Link primitives) and
sub-spec #3a (public-entry Button migration) both merged first.

## Overview

Second pass over public-entry routes. Replace `PUBLIC_ENTRY_INPUT_CLASS`, `PUBLIC_ENTRY_CHECKBOX_CLASS`,
`PUBLIC_ENTRY_LINK_CLASS` utilities on `login`, `register`, `device`, `+error`, and delete them from
`PublicEntryShell.svelte` module script. Consumers adopt `<Input>`, `<Checkbox>`, `<Link>` primitives from #2b.

## Design decisions

**Q1 — Migrate inline `<input>` elements inside `FormFieldRow` or preserve existing structure.**

- Options:
  - (chosen) Replace `<input class={PUBLIC_ENTRY_INPUT_CLASS} ...>` inside each `FormFieldRow` with
    `<Input id=... type=... bind:value=... error=... />`. `FormFieldRow` stays untouched — it already accepts arbitrary
    children via snippet.
  - Refactor `FormFieldRow` to own the `<Input>` directly. Rejected — couples label concerns to input concerns, and
    `FormFieldRow` needs to remain generic for non-Input children (e.g. custom widgets).
- Reasoning: keeps concerns layered; `FormFieldRow` is label-layout, `<Input>` is field-styling, consumer composes the
  two.

**Q2 — Error prop wiring: pass `error` to both `FormFieldRow` and `<Input>`, or only one.**

- Options:
  - (chosen) Pass to both. `FormFieldRow` renders the error copy + icon below the field (per parent §4.10); `<Input>`
    toggles `aria-invalid` + error-state border/bg.
  - Only `FormFieldRow`. Rejected — `<Input>` needs `aria-invalid` for the error-bg styling contract.
  - Only `<Input>`. Rejected — `FormFieldRow` owns the label + hint + error-copy triplet rendering.
- Reasoning: the two consume `error` for different purposes. Parent spec §4.10 treats them as one logical state
  expressed in two DOM nodes.

**Q3 — Input radius shift: `rounded-lg` → `rounded-[3px]`.**

- Options:
  - (chosen) Accept the shift. #2b primitive pins `rounded-[3px]` per parent §4.10/§4.3 conventions. Public entry inputs
    become visually sharper-cornered.
  - Override with `class="rounded-lg"` on each migrated `<Input>`. Rejected — overriding the design contract per call
    site defeats the point of the primitive.
  - Amend parent spec to add a page-level rounded variant. Deferred — same rationale as the #3a button-size decision:
    ship the spec- conformant shape; amend only if the visual outcome is clearly wrong.
- Reasoning: single source of truth on radius; if the compact radius fails usability, amend §4.10 not local overrides.

**Q4 — `aria-describedby` wiring at migration sites.**

- Options:
  - (chosen) `FormFieldRow` owns the error-copy node's stable `id` and passes `aria-describedby` to the nested `<Input>`
    when `error` is non-empty. Consumer call sites do not pass `aria-describedby` manually. This matches #2b Q3. If the
    `FormFieldRow` update that implements this wiring has not landed at #2b merge time, #3a2 blocks on that landing (or
    on a follow-up `FormFieldRow` PR) before the a11y contract is complete.
  - Each consumer passes `aria-describedby` manually at every call site. Rejected — duplicates the id-management concern
    across five sites; contradicts #2b Q3 ownership.
- Reasoning: keep the a11y contract in one place (`FormFieldRow`); call sites stay readable. The dependency footnote is
  captured in "Dependencies + ordering" below.

**Q5 — Checkbox controlled vs two-way-bound pattern at call site.**

- Options:
  - (chosen) Controlled: `<Checkbox checked={showToken} onchange={handleShowTokenChange}>`. The `showToken` state lives
    in the parent component (`register/+page.svelte`) with existing imperative logic around toggle handling; controlled
    pattern keeps the existing change-handler intact.
  - Two-way: `<Checkbox bind:checked={showToken} />`. Rejected for this call site — `showToken` updates trigger side
    effects (revealing the token input); binding hides the change point.
- Reasoning: both patterns are supported by #2b's `$bindable(false)` declaration; the consumer picks the one that
  matches its existing state shape. Documented here so the #2b `$bindable` note does not imply `bind:checked` is
  mandatory.

**Q6 — Prose link migration granularity.**

- Options:
  - (chosen) Migrate the two footer links (`Don't have an account? Register` and `Already have an account? Login`) and
    leave the `{#if}`- conditional "Back to login" link alone (already migrated in sub-spec #2 PR2 canary — uses
    `<Button variant="ghost" href="...">`).
  - Convert all link-looking things (including button-styled "Back to login") to `<Link>`. Rejected — that specific case
    is a button- shaped action, not a prose link.
- Reasoning: consistent with parent spec — prose-in-copy uses `<Link>`, action-shaped uses `<Button href>`.

## Goals

1. Every `<input>` in public-entry routes renders through `<Input>`.
2. The checkbox on `register/+page.svelte` renders through `<Checkbox>`.
3. The two prose footer links render through `<Link variant="default">`.
4. `PUBLIC_ENTRY_INPUT_CLASS`, `PUBLIC_ENTRY_CHECKBOX_CLASS`, `PUBLIC_ENTRY_LINK_CLASS` deleted from
   `PublicEntryShell.svelte`.
5. `PUBLIC_ENTRY_FORM_CLASS` is the only remaining utility export — kept because form-level `space-y-4` layout has no
   primitive equivalent.

## Non-goals

- Button migration — done in #3a.
- `FormFieldRow.svelte` refactor — consumer of primitive, not subject.
- New primitive design — done in #2b.
- Textarea migration — no public-entry textareas exist.

## Call-site migration

### `frontend/src/routes/login/+page.svelte`

- 4 `<input>` elements: `registration-token`, `link-password`, `login-email`, `login-password`. Each migrates to
  `<Input>` with appropriate `type`, `id`, `bind:value`, `autocomplete`, `error={fieldErrors.*}`,
  `oninput={clearLoginFieldError(...)}`.
  Note: the `registration-token` input has no existing `autocomplete` attribute in the source; specify
  `autocomplete="off"` on the migrated `<Input>`, or omit the prop (both are equivalent — `autocomplete="off"` is the
  explicit safe default for a one-time token field).
- 1 prose link in the footer snippet: `Don't have an account? Register` migrates to
  `<Link href="/register">Register</Link>`.
- Drop `PUBLIC_ENTRY_INPUT_CLASS` and `PUBLIC_ENTRY_LINK_CLASS` from the destructured `PublicEntryShell` import.

### `frontend/src/routes/register/+page.svelte`

- 4 `<input>` text elements: `register-email`, `register-first-name`, `register-last-name`, `register-password`. Each
  migrates to `<Input>`.
- 1 optional `<input>` for `register-token` (conditional on `showToken`). Migrates to `<Input type="text">`.
- 1 `<input type="checkbox">`: invite-token toggle. Migrates to `<Checkbox id="show-token" checked={showToken} onchange={...}>`.
  The outer `<label class="flex items-start gap-2 text-sm text-[var(--text-secondary)]">` wrapper element and the inner
  `<span>I have an invite token</span>` text node must be preserved as-is. Only the `<input type="checkbox">` is replaced
  by `<Checkbox>`. `<Checkbox>` requires a non-optional `id` prop; use `id="show-token"`.
- The footer "Login" link (`register/+page.svelte` line 197) is **already** `<Button variant="ghost" href="/login">Login</Button>` —
  already correct from the #3a migration. It is not a migration target under this sub-spec.
- Drop `PUBLIC_ENTRY_INPUT_CLASS` and `PUBLIC_ENTRY_CHECKBOX_CLASS` from the destructured import. `PUBLIC_ENTRY_LINK_CLASS`
  is not imported in `register/+page.svelte` and must not be listed as a drop target here.

### `frontend/src/routes/device/+page.svelte`

- No `<input>` or `<Checkbox>` sites — device route renders status callouts + a single approve action button.
- No migration needed under this sub-spec.

### `frontend/src/routes/+error.svelte`

- No `<input>`, `<Checkbox>`, or `<Link>` sites under this sub-spec — migration already complete after #3a.
- No migration needed under this sub-spec.

### `frontend/src/lib/components/ui/PublicEntryShell.svelte`

Module-script cleanup — delete three exports:

```ts
export const PUBLIC_ENTRY_INPUT_CLASS = "...";
export const PUBLIC_ENTRY_CHECKBOX_CLASS = "...";
export const PUBLIC_ENTRY_LINK_CLASS = "...";
```

Keep `PUBLIC_ENTRY_FORM_CLASS` as the only remaining export. Consider renaming it to `PUBLIC_ENTRY_SPACING_CLASS` if it
becomes clearer in context — deferred to post-migration cleanup.

## Data flow

No runtime changes. All migrations are template-level. No new state, no new stores, no new API calls.

## Error handling

- `<Input>` primitive's TS discriminated union rejects invalid `type` values at compile time.
- `<Input error>` prop auto-wires `aria-invalid` — consumers drop the manual
  `aria-invalid={fieldErrors.x ? 'true' : undefined}` binding.
- `<Checkbox>` onchange fires with native Event; no change to existing `showToken` state machine.

## Testing

### Unit tests

`public-entry.test.ts` extensions:

- Each migrated `<Input>` renders with correct `type`, `id`, `autocomplete`.
- Error toggle: set `fieldErrors.email = 'Required'`; assert the `<Input>` DOM node has `aria-invalid="true"` and the
  error-bg class, and that `FormFieldRow`'s error-copy node's id appears in the `<Input>`'s `aria-describedby`
  attribute.
- Checkbox: toggle `showToken` state; assert `<Checkbox checked>` reflects; assert `onchange` fires exactly once when
  the checkbox is clicked (regression guard against the controlled-pattern handler being dropped during migration);
  assert `disabled` state renders the primitive's opacity class fragment.
- Prose link: footer `<Link>` renders with `href="/register"` (or `/login`) + `variant="default"` default class
  fragment.
- Regression guard: `PUBLIC_ENTRY_INPUT_CLASS` / `CHECKBOX_CLASS` / `LINK_CLASS` literal class strings no longer appear
  in rendered DOM.
- **Breaking assertion to remove:** `public-entry.test.ts` line 77 currently asserts
  `expect(screen.getByLabelText('Email').className).toContain(PUBLIC_ENTRY_INPUT_CLASS)`. This assertion must be removed
  or rewritten during migration — after migration the `<Input>` primitive renders with its own BASE class, not with
  `PUBLIC_ENTRY_INPUT_CLASS`, so the assertion will fail. Replace it with an assertion that the element is rendered by
  the `<Input>` primitive (e.g. assert `aria-invalid` wiring or the primitive's own class fragment instead).
- Conditional inputs: login's `registration-token` and `link-password` inputs are rendered only under server-driven
  states (`registration_required`, `link_required`). Tests set the backing store to those states individually and assert
  the conditionally- rendered `<Input>` carries the expected `type` / `id` / `autocomplete` — same assertion shape as
  the always-rendered inputs, just gated on state.

### Integration / e2e

Playwright visual regression:

- Re-baseline `/login` (default + setup_required + registration-token- required + link-required permutations) and
  `/register`. Permutations are driven by the `PageData` contract the existing public-entry Playwright fixtures already
  use for #3a; #3a2 reuses those fixtures without extension.
- Delta enumeration (per parent §9 waiver schema):
  - Input radius `rounded-lg` → `rounded-[3px]`.
  - Input focus ring shadow offset unchanged (same `0 0 0 3px rgba(var(--accent-rgb), 0.25)` contract from #2b).
  - Checkbox radius new `rounded-[2px]` (no legacy radius — native default previously); checkbox now carries
    `accent-[var(--accent)]` and focus-visible shadow ring.
  - Link underline-offset new `underline-offset-4`; hover color swaps `--accent` → `--accent-bright` on `default`
    variant — no legacy equivalent to compare.
  - Error state: bg swap to `--color-error-bg`, border swap to `--color-error-border`. Text color unchanged — only
    container tone shifts. Regression guard: assert contrast ratio on the error-bg + text-primary pair remains ≥ WCAG AA
    in both themes (computed value, not class presence).
- `device`, `+error` re-baselines unchanged since no migration sites — assert snapshots match within 0.5 % threshold.

## Rollout

Single PR titled "feat(frontend): migrate public-entry inputs/checkbox/links to primitives (sub-spec #3a2)".

1. `frontend/src/lib/components/ui/PublicEntryShell.svelte` — delete three utility exports. Keep
   `PUBLIC_ENTRY_FORM_CLASS`.
2. `frontend/src/routes/login/+page.svelte` — migrate 4 inputs + 1 footer link; drop 2 utility imports.
3. `frontend/src/routes/register/+page.svelte` — migrate 4 required inputs + 1 optional input + 1 checkbox; drop 2
   utility imports (`PUBLIC_ENTRY_INPUT_CLASS`, `PUBLIC_ENTRY_CHECKBOX_CLASS`). The footer "Login" link is already
   `<Button variant="ghost">` — no change needed.
4. Extend `public-entry.test.ts` per unit-test plan.
5. Re-baseline `/login` + `/register` Playwright snapshots.
6. Full frontend gate.

### Risk + rollback

Reverting one PR restores the utility classes across public-entry. Critical-path login surface — mitigated by existing
unit tests + Playwright visual regression.

### Dependencies + ordering

- **Blocks on:** sub-spec #2b merged, sub-spec #3a merged. #2b's `FormFieldRow` update for `aria-describedby` injection
  must have landed as part of #2b (or shipped as a follow-up PR) — see Q4. `FormFieldRow` is listed as a non-goal for
  this sub-spec's own template work, but its a11y wiring must be in place at #3a2 merge time; if not, #3a2 blocks until
  it is.
- **Blocks:** nothing downstream.
- **Parallel-safe with:** sub-spec #3b–k (authenticated-app migrations).

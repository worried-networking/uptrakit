# Public Entry Forms Migration — Design

**Parent spec:** `docs/superpowers/specs/2026-04-16-ui-design-language-design.md`
(§4.10 Form Validation)

**Sub-spec #3a2 of the UI design-language rollout.** Depends on sub-spec #2b
(Input + Checkbox + Link primitives) and sub-spec #3a (public-entry Button
migration) both merged first.

## Overview

Second pass over public-entry routes. Replace
`PUBLIC_ENTRY_INPUT_CLASS`, `PUBLIC_ENTRY_CHECKBOX_CLASS`,
`PUBLIC_ENTRY_LINK_CLASS` utilities on `login`, `register`, `device`,
`+error`, and delete them from `PublicEntryShell.svelte` module script.
Consumers adopt `<Input>`, `<Checkbox>`, `<Link>` primitives from #2b.

## Design decisions

**Q1 — Migrate inline `<input>` elements inside `FormFieldRow` or
preserve existing structure.**

- Options:
  - (chosen) Replace `<input class={PUBLIC_ENTRY_INPUT_CLASS} ...>` inside
    each `FormFieldRow` with `<Input id=... type=... bind:value=...
    error=... />`. `FormFieldRow` stays untouched — it already accepts
    arbitrary children via snippet.
  - Refactor `FormFieldRow` to own the `<Input>` directly. Rejected —
    couples label concerns to input concerns, and `FormFieldRow` needs
    to remain generic for non-Input children (e.g. custom widgets).
- Reasoning: keeps concerns layered; `FormFieldRow` is label-layout,
  `<Input>` is field-styling, consumer composes the two.

**Q2 — Error prop wiring: pass `error` to both `FormFieldRow` and
`<Input>`, or only one.**

- Options:
  - (chosen) Pass to both. `FormFieldRow` renders the error copy + icon
    below the field (per parent §4.10); `<Input>` toggles
    `aria-invalid` + error-state border/bg.
  - Only `FormFieldRow`. Rejected — `<Input>` needs `aria-invalid` for
    the error-bg styling contract.
  - Only `<Input>`. Rejected — `FormFieldRow` owns the label + hint +
    error-copy triplet rendering.
- Reasoning: the two consume `error` for different purposes. Parent
  spec §4.10 treats them as one logical state expressed in two DOM
  nodes.

**Q3 — Input radius shift: `rounded-lg` → `rounded-[3px]`.**

- Options:
  - (chosen) Accept the shift. #2b primitive pins `rounded-[3px]` per
    parent §4.10/§4.3 conventions. Public entry inputs become visually
    sharper-cornered.
  - Override with `class="rounded-lg"` on each migrated `<Input>`. Rejected
    — overriding the design contract per call site defeats the point
    of the primitive.
  - Amend parent spec to add a page-level rounded variant. Deferred —
    same rationale as the #3a button-size decision: ship the spec-
    conformant shape; amend only if the visual outcome is clearly
    wrong.
- Reasoning: single source of truth on radius; if the compact radius
  fails usability, amend §4.10 not local overrides.

**Q4 — Prose link migration granularity.**

- Options:
  - (chosen) Migrate the two footer links (`Don't have an account? Register`
    and `Already have an account? Login`) and leave the `{#if}`-
    conditional "Back to login" link alone (already migrated in
    sub-spec #2 PR2 canary — uses `<Button variant="ghost" href="...">`).
  - Convert all link-looking things (including button-styled "Back to
    login") to `<Link>`. Rejected — that specific case is a button-
    shaped action, not a prose link.
- Reasoning: consistent with parent spec — prose-in-copy uses `<Link>`,
  action-shaped uses `<Button href>`.

## Goals

1. Every `<input>` in public-entry routes renders through `<Input>`.
2. The checkbox on `register/+page.svelte` renders through `<Checkbox>`.
3. The two prose footer links render through `<Link variant="default">`.
4. `PUBLIC_ENTRY_INPUT_CLASS`, `PUBLIC_ENTRY_CHECKBOX_CLASS`,
   `PUBLIC_ENTRY_LINK_CLASS` deleted from `PublicEntryShell.svelte`.
5. `PUBLIC_ENTRY_FORM_CLASS` is the only remaining utility export — kept
   because form-level `space-y-4` layout has no primitive equivalent.

## Non-goals

- Button migration — done in #3a.
- `FormFieldRow.svelte` refactor — consumer of primitive, not subject.
- New primitive design — done in #2b.
- Textarea migration — no public-entry textareas exist.

## Call-site migration

### `frontend/src/routes/login/+page.svelte`

- 4 `<input>` elements: `registration-token`, `link-password`,
  `login-email`, `login-password`. Each migrates to `<Input>` with
  appropriate `type`, `id`, `bind:value`, `autocomplete`,
  `error={fieldErrors.*}`, `oninput={clearLoginFieldError(...)}`.
- 1 prose link in the footer snippet: `Don't have an account? Register`
  migrates to `<Link href="/register">Register</Link>`.
- Drop `PUBLIC_ENTRY_INPUT_CLASS` and `PUBLIC_ENTRY_LINK_CLASS` from the
  destructured `PublicEntryShell` import.

### `frontend/src/routes/register/+page.svelte`

- 4 `<input>` text elements: `register-email`, `register-first-name`,
  `register-last-name`, `register-password`. Each migrates to `<Input>`.
- 1 optional `<input>` for `register-token` (conditional on
  `showToken`). Migrates to `<Input type="text">`.
- 1 `<input type="checkbox">`: invite-token toggle. Migrates to
  `<Checkbox checked={showToken} onchange={...}>`.
- 1 prose footer link `Already have an account? Login`. Migrates to
  `<Link href="/login">Login</Link>`.
- Drop `PUBLIC_ENTRY_INPUT_CLASS`, `PUBLIC_ENTRY_CHECKBOX_CLASS`, and
  `PUBLIC_ENTRY_LINK_CLASS` from the destructured import.

### `frontend/src/routes/device/+page.svelte`

- No `<input>` or `<Checkbox>` sites — device route renders status
  callouts + a single approve action button.
- No migration needed under this sub-spec.

### `frontend/src/routes/+error.svelte`

- No `<input>`, `<Checkbox>`, or `<Link>` sites under this sub-spec —
  migration already complete after #3a.
- No migration needed under this sub-spec.

### `frontend/src/lib/components/ui/PublicEntryShell.svelte`

Module-script cleanup — delete three exports:

```ts
export const PUBLIC_ENTRY_INPUT_CLASS = '...';
export const PUBLIC_ENTRY_CHECKBOX_CLASS = '...';
export const PUBLIC_ENTRY_LINK_CLASS = '...';
```

Keep `PUBLIC_ENTRY_FORM_CLASS` as the only remaining export. Consider
renaming it to `PUBLIC_ENTRY_SPACING_CLASS` if it becomes clearer in
context — deferred to post-migration cleanup.

## Data flow

No runtime changes. All migrations are template-level. No new state, no
new stores, no new API calls.

## Error handling

- `<Input>` primitive's TS discriminated union rejects invalid `type`
  values at compile time.
- `<Input error>` prop auto-wires `aria-invalid` — consumers drop the
  manual `aria-invalid={fieldErrors.x ? 'true' : undefined}` binding.
- `<Checkbox>` onchange fires with native Event; no change to existing
  `showToken` state machine.

## Testing

### Unit tests

`public-entry.test.ts` extensions:

- Each migrated `<Input>` renders with correct `type`, `id`, `autocomplete`.
- Error toggle: set `fieldErrors.email = 'Required'`; assert the
  `<Input>` DOM node has `aria-invalid="true"` and the error-bg class.
- Checkbox: toggle `showToken` state; assert `<Checkbox checked>`
  reflects.
- Prose link: footer `<Link>` renders with `href="/register"` (or
  `/login`) + `variant="default"` default class fragment.
- Regression guard: `PUBLIC_ENTRY_INPUT_CLASS` / `CHECKBOX_CLASS` /
  `LINK_CLASS` literal class strings no longer appear in rendered DOM.

### Integration / e2e

Playwright visual regression:

- Re-baseline `/login` (default + setup_required + registration-token-
  required + link-required permutations), `/register`. Delta expected:
  input radius `rounded-lg` → `rounded-[3px]`, focus ring shadow
  offset unchanged.
- `device`, `+error` re-baselines unchanged since no migration sites —
  assert snapshots match within 0.5 % threshold.

## Rollout

Single PR titled
"feat(frontend): migrate public-entry inputs/checkbox/links to primitives (sub-spec #3a2)".

1. `frontend/src/lib/components/ui/PublicEntryShell.svelte` — delete
   three utility exports. Keep `PUBLIC_ENTRY_FORM_CLASS`.
2. `frontend/src/routes/login/+page.svelte` — migrate 4 inputs + 1
   footer link; drop 2 utility imports.
3. `frontend/src/routes/register/+page.svelte` — migrate 4 required
   inputs + 1 optional input + 1 checkbox + 1 footer link; drop 3
   utility imports.
4. Extend `public-entry.test.ts` per unit-test plan.
5. Re-baseline `/login` + `/register` Playwright snapshots.
6. Full frontend gate.

### Risk + rollback

Reverting one PR restores the utility classes across public-entry.
Critical-path login surface — mitigated by existing unit tests +
Playwright visual regression.

### Dependencies + ordering

- **Blocks on:** sub-spec #2b merged, sub-spec #3a merged.
- **Blocks:** nothing downstream.
- **Parallel-safe with:** sub-spec #3b–k (authenticated-app migrations).

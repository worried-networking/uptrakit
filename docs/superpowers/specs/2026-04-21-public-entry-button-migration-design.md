# Public Entry Button Migration — Design

**Parent spec:** `docs/superpowers/specs/2026-04-16-ui-design-language-design.md` (§4.3 Buttons)

**Sub-spec #3a of the UI design-language rollout.** Depends on sub-spec #2 (Button primitive + terminal-palette) PR1
being merged first.

## Overview

Migrate four public-entry routes (`login`, `register`, `device`, `+error`) from the `PUBLIC_ENTRY_PRIMARY_BUTTON_CLASS`
and `PUBLIC_ENTRY_SECONDARY_BUTTON_CLASS` utility classes exported by `PublicEntryShell.svelte` to sub-spec #2's
`<Button>` primitive. Retire both button constants. Consumption patterns: `w-full justify-center` passed via the
existing `class?: string` prop, OIDC provider logos wired through `leadingIcon`, `loading` prop replaces manual
"Redirecting…" and "Authorizing…" text swaps, `href` branch replaces `onclick={goto(...)}` for navigation-only buttons.

Parent §4.3 pins a single standard button shape (`h-[23px]`, `3px` radius, `9px` bold uppercase). No page-level
carve-out exists. Public entry buttons shrink from the current `h-9 text-sm rounded-lg` shape to the §4.3 compact shape.
If the compact shape turns out to be wrong for auth cards, the fix is a parent-spec amendment adding a page-level size —
not a local exception in this sub-spec.

## Goals

1. Every page-level button on public-entry routes renders through `<Button>` with §4.3 compact shape.
2. OIDC provider buttons render the provider logo in the `leadingIcon` slot and switch to a spinner via
   `loading={oidcLoading}`.
3. Long-running actions (OIDC redirect, device approval) use the `loading` prop for `aria-busy` + spinner consistency
   with the rest of the app.
4. `PUBLIC_ENTRY_PRIMARY_BUTTON_CLASS` and `PUBLIC_ENTRY_SECONDARY_BUTTON_CLASS` deleted from `PublicEntryShell.svelte`.

## Non-goals

- Input, checkbox, and link migration — deferred to sub-spec #3a2 after the #2b Input/Checkbox/Link primitives sub-spec
  lands.
- `PublicEntryShell.svelte` chrome (section, article, header, footer structure) — already token-compliant from sub-spec
  #1.
- New Button primitive features or prop additions — consumer adopts the existing API.
- OIDC flow logic, auth state machines, redirect handling — template swap only.
- Parent §4.3 amendment for a page-level button size — explicit non-goal; handled separately if the compact shape proves
  wrong post-ship.

## Call-site migration

### `frontend/src/routes/login/+page.svelte`

Six migration sites across three flows (password, OIDC, registration token, account linking):

- Registration-token submit: primary, `type="submit"`, `disabled={!getIsOnline()}`.
- Link-required password submit: primary, `type="submit"`, `disabled={!getIsOnline()}`.
- Link-required OIDC provider button: ghost, `type="button"`, `loading={oidcLoading}`, `leadingIcon={providerLogo}`,
  `onclick={() => onLinkWithOidc(linkProviderId)}`.
- OIDC provider repeat (each provider in the list): ghost, `type="button"`, `loading={oidcLoading}`,
  `leadingIcon={providerLogo}`, `onclick={() => onOidcLogin(provider.id)}`.
- Password submit: primary, `type="submit"`, `disabled={!getIsOnline()}`.

Every consumer passes `class="w-full justify-center"`. Every migrated button drops the legacy
`class={PUBLIC_ENTRY_*_BUTTON_CLASS}` attribute. The manual `{oidcLoading ? 'Redirecting...' : 'Login with ...'}` text
swap is deleted; children stay literal `Login with {provider.name}`. The `flex items-center justify-center gap-2`
container modifier around the OIDC button is removed (Button's own base already sets
`inline-flex items-center gap-1.5`).

### `frontend/src/routes/register/+page.svelte`

One migration site:

- Registration form submit: primary, `type="submit"`, `disabled={!getIsOnline()}`, `class="w-full justify-center"`,
  children `Register`.

Drop `PUBLIC_ENTRY_PRIMARY_BUTTON_CLASS` from the destructured import.

### `frontend/src/routes/device/+page.svelte`

Two migration sites:

- "Log in" link shown when not authenticated: primary, `href` branch —
  `href={"/login?redirect=/device?code=" + encodeURIComponent(code)}`, `class="w-full justify-center"`, children
  `Log in`.
- "Approve" action: primary, `type="button"`, `loading={approving}`, `disabled={approving}` (redundant but explicit —
  primitive sets `disabled` when `loading`), `onclick={onApprove}`, `class="w-full justify-center"`, children `Approve`.
  The `{approving ? 'Authorizing...' : 'Approve'}` manual text swap is deleted.

Drop `PUBLIC_ENTRY_PRIMARY_BUTTON_CLASS` from the destructured import.

### `frontend/src/routes/+error.svelte`

One migration site, converting an `onclick` navigation into an `href` branch:

- "Go to Home" footer button: primary, `href="/"`, `class="w-full justify-center"`, children `Go to Home`.

Drop `goto` import from `$app/navigation`, drop `PUBLIC_ENTRY_PRIMARY_BUTTON_CLASS` from the destructured import.
Navigation semantics are correct for a link — sub-spec #2's href branch renders `<a href="/" role="button">` with the
§4.3 primary class contract.

### `frontend/src/lib/components/ui/PublicEntryShell.svelte`

Module-script cleanup — delete two exports:

```ts
export const PUBLIC_ENTRY_PRIMARY_BUTTON_CLASS = "...";
export const PUBLIC_ENTRY_SECONDARY_BUTTON_CLASS = "...";
```

Keep the other four (`PUBLIC_ENTRY_FORM_CLASS`, `PUBLIC_ENTRY_INPUT_CLASS`, `PUBLIC_ENTRY_CHECKBOX_CLASS`,
`PUBLIC_ENTRY_LINK_CLASS`). Sub-spec #3a2 retires those after #2b lands.

## Consumption patterns

### Width

Every public-entry consumer passes `class="w-full justify-center"` via Button's existing `class?: string` prop. Button
primitive base stays `inline-flex`; consumer stretches it to fill the form column. No primitive change.

`w-full` on an `inline-flex` element sets `width: 100%`. The primitive's base `items-center` centers children
vertically. Consumer adds `justify-center` so the label + optional leading icon center horizontally within the stretched
width.

### OIDC provider logo via `leadingIcon`

The current login template inlines an `<img>` tag inside the button. Target pattern:

```svelte
{#each authMethods.oidc_providers as provider (provider.id)}
  {#snippet providerLogo()}
    {#if isValidLogoUrl(provider.logo_url)}
      <img src={provider.logo_url} alt="" class="h-5 w-5" referrerpolicy="no-referrer" />
    {/if}
  {/snippet}

  <Button
    variant="ghost"
    type="button"
    class="w-full justify-center"
    disabled={oidcLoading}
    loading={oidcLoading}
    leadingIcon={providerLogo}
    onclick={() => onOidcLogin(provider.id)}
  >
    Login with {provider.name}
  </Button>
{/each}
```

The snippet is declared inside the `{#each}` block so each iteration captures its own `provider` via closure. Sub-spec #2
specifies that when `loading=true`, the spinner replaces whatever is in the `leadingIcon` slot — so during OIDC redirect
the provider logo hides and the spinner shows. Accepted trade-off: the spinner is the user-visible "something is
happening" signal; the logo is decorative during the idle state.

### Loading state

Two consumer sites: `login/+page.svelte` OIDC buttons bind `loading={oidcLoading}`; `device/+page.svelte` "Approve"
button binds `loading={approving}`. Manual `ternary ? 'Redirecting...' : 'Login'` children swaps are deleted — the
spinner plus unchanged label communicate the in-flight state, and the primitive sets `aria-busy="true"` automatically.

### Disabled vs offline

`disabled={!getIsOnline()}` stays on every submit button. The small "Offline" text paragraph below each disabled submit
stays as-is — separate accessible context, outside the Button primitive's scope.

### Submit forms

Primary submit buttons use `type="submit"`. Button's discriminated union permits `type` only when `href` is omitted, so
submit and href branches are statically distinct at the type level.

### Link branch

`device/+page.svelte` "Log in" and `+error.svelte` "Go to Home" use the `href` branch. Primitive renders
`<a href role="button" aria-disabled>` with the §4.3 primary gradient + compact shape. Sub-spec #2's `onkeydown` guard
for Space/Enter during disabled/loading states is inherited but not exercised on these two sites (always enabled, no
loading state).

### Ghost vs legacy secondary

Legacy `PUBLIC_ENTRY_SECONDARY_BUTTON_CLASS` used `bg-[var(--bg-surface)]` + `border --border-default`. Button
primitive's `ghost` variant uses `bg-transparent` + `border --border-default` + `hover:bg --bg-raised`. Small visual
delta — idle background transparent vs surface. Accepted as spec-conformant: parent §4.3 has no "secondary" variant;
`ghost` is the app-wide secondary equivalent.

## Data flow

No runtime behavior changes. All migrations are template-level. Button primitive's own props are all data already held
in component state: `oidcLoading`, `approving`, `!getIsOnline()`, `provider.id`, `provider.logo_url`. No new imports
beyond `import Button from '$lib/components/Button.svelte';` at the top of each migrated route.

## Error handling

- Button primitive's TS discriminated union prevents invalid prop combinations (`href` + `type`, `href` + `onclick`) at
  compile time.
- `loading=true` short-circuits consumer `onclick` handlers via sub-spec #2's render-branch contract — OIDC double-click
  protection inherited.
- Offline hint paragraphs below disabled submits stay unchanged — they read out of the same `getIsOnline()` signal the
  primitive's `disabled` prop binds to.
- Navigation errors on `<Button href>` branches fall through to SvelteKit's default navigation error handling; `<a>`
  element semantics unchanged from the legacy `<a>` tag with utility classes.

## Testing

### Unit tests

`frontend/src/routes/public-entry.test.ts` (existing, extended):

- Per-route render assertions: each migrated button carries `h-[23px]` class fragment, correct `type` attr for submit vs
  button branches, correct `href` attr for link branch.
- Loading states: with `oidcLoading = true`, assert `aria-busy="true"` on the OIDC button and spinner element present;
  with `oidcLoading = false`, assert the provider logo `<img>` renders inside the leading-icon slot.
- Offline: stub `getIsOnline()` returns `false`; assert `disabled` attr present on all submit buttons; assert offline
  paragraph rendered.
- Text swap removal: assert neither the `"Redirecting..."` literal (login OIDC) nor the `"Authorizing..."` literal
  (device Approve) is present in any DOM snapshot across the three migrated routes (regression guard — proves every
  manual ternary swap is retired).
- Combined state: with `getIsOnline() = false` and `oidcLoading = true`, assert the OIDC button carries both `disabled`
  attribute and `aria-busy="true"` — exercises the offline-during-loading interaction that Button's primitive must
  handle simultaneously.
- Href navigation: `+error` route "Go to Home" renders `<a href="/" role="button">` — assert both attrs via
  `getByRole('button')` query.

Existing assertions that matched legacy `PUBLIC_ENTRY_PRIMARY_BUTTON_CLASS` or `PUBLIC_ENTRY_SECONDARY_BUTTON_CLASS`
class fragments must be rewritten (not deleted) against the new Button contract — they capture the button's presence and
behavior, only the class target changes.

### Integration / e2e

Playwright visual regression (parent §9 waiver schema):

- Re-baseline snapshots for four routes × applicable permutations:
  - `/login` — default, `setup_required=true`, registration-token required (hash fragment), link-required.
  - `/register`.
  - `/device?code=ABCD-EFGH` — logged in + logged out states.
  - `/error` via a synthetic 404.
- PR description enumerates each deliberate delta per parent §9 waiver schema: "primary button shrinks from `h-9` large
  to `h-[23px]` §4.3 compact; uppercase 9px label replaces `text-sm` casing; radius `rounded-lg` → `rounded-[3px]`;
  primary gradient fill replaces solid accent; `ghost` variant with transparent idle bg replaces
  `bg-[var(--bg-surface)]` secondary for OIDC buttons".
- All other Playwright snapshots (authenticated app chrome) must stay within the 0.5 % threshold — #3a does not touch
  any authenticated route.

## Rollout

Single PR titled "feat(frontend): migrate public-entry buttons to Button primitive (sub-spec #3a)".

1. `frontend/src/lib/components/ui/PublicEntryShell.svelte` — delete the two button-class exports from the
   `<script module>` block. Keep the other four exports and the entire default script + template.
2. `frontend/src/routes/login/+page.svelte` — migrate six button sites, drop both button-class imports from the
   destructured `PublicEntryShell` import, keep the remaining three imports. Remove the
   `{oidcLoading ? 'Redirecting...' : 'Login with ...'}` ternary swaps in favor of `loading={oidcLoading}` plus literal
   children.
3. `frontend/src/routes/register/+page.svelte` — migrate the single submit button, drop
   `PUBLIC_ENTRY_PRIMARY_BUTTON_CLASS` from the import.
4. `frontend/src/routes/device/+page.svelte` — migrate the "Log in" href button and the "Approve" onclick button, drop
   `PUBLIC_ENTRY_PRIMARY_BUTTON_CLASS` from the import, drop the `{approving ? 'Authorizing...' : 'Approve'}` text swap
   in favor of `loading={approving}` plus literal `Approve` children.
5. `frontend/src/routes/+error.svelte` — migrate "Go to Home" from `onclick={() => goto('/')}` to `href="/"` on the
   Button primitive, drop `goto` import from `$app/navigation`, drop `PUBLIC_ENTRY_PRIMARY_BUTTON_CLASS` from the
   import.
6. Extend `public-entry.test.ts` per the unit-test plan above.
7. Re-baseline the four Playwright route snapshots.
8. Run the full frontend gate (`lint`, `format:check`, `check`, `test`, `build`, `test:e2e`).

### Risk + rollback

Reverting one PR restores all legacy button classes across public-entry surface. No schema changes, no API changes, no
runtime wire changes. Zero-downtime rollback. Critical-path concern (login form breakage) is mitigated by: sub-spec #2
canary already exercised `PublicEntryShell` via the "Back to login" link migration; new unit assertions in
`public-entry.test.ts` cover every migrated site; Playwright visual regression gates catch any rendering anomaly before
merge.

### Divergence from parent-spec rollout

Parent §2.8 did not number public-entry migration separately — it is part of the broader 244-site migration sweep. This
sub-spec carves public entry out as a standalone step so the critical-path auth flow is reviewed, tested, and visually
re-baselined in isolation from the rest of the app. Rationale: if something regresses on login, it should be easy to
identify the exact PR from git log without bisecting through a large cross-cutting migration.

## Dependencies + ordering

- **Blocks on:** sub-spec #2 PR1 merged (Button primitive exported from `$lib/components/Button.svelte`). Sub-spec #2
  PR2 canary already migrated the `PublicEntryShell` "Back to login" link variant — same surface, primitive proven.
- **Blocks:** nothing in the #3a–k lineage directly. #3a2 (public-entry Input/Checkbox/Link migration) blocks on
  sub-spec #2b, not this one; #3b+ operate on authenticated-app chrome and are disjoint.
- **Parallel-safe with:** sub-spec #2b (Input/Checkbox/Link primitive design — disjoint surface), sub-spec #4
  (surface-layer parity — authenticated-only).

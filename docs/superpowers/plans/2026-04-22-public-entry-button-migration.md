# Public Entry Button Migration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement
> this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Migrate all button-shaped elements on the four public-entry routes (login, register, device, +error) to the `<Button>` primitive and retire
`PUBLIC_ENTRY_PRIMARY_BUTTON_CLASS` + `PUBLIC_ENTRY_SECONDARY_BUTTON_CLASS` from `PublicEntryShell.svelte`.

**Architecture:** Template-level swap only — no new state, no new API calls. Each route's `<button>` and `<a>` elements are replaced with `<Button
variant="..." ...>` using the same event handlers and reactive variables already in scope. The `loading` prop eliminates manual `{flag ? 'X...' :
'Y'}` ternaries. The `href` branch replaces `onclick={() => goto(...)}` for navigation-only elements.

**Tech Stack:** Svelte 5, TypeScript, @testing-library/svelte, Playwright

---

## Dependency

**Blocks on:** sub-spec #2 merged — `Button` exported from `frontend/src/lib/components/Button.svelte`. The register page already imports `Button`
(added by sub-spec #2 PR2). The other three routes need the import added.

---

## Migration rules (quick reference)

| Legacy class | Button variant |
| --- | --- |
| `PUBLIC_ENTRY_PRIMARY_BUTTON_CLASS` / `preset-filled-primary-500` | `primary` |
| `PUBLIC_ENTRY_SECONDARY_BUTTON_CLASS` / `preset-tonal-surface` | `ghost` |

Every migrated button passes `class="w-full justify-center"` via Button's `class` prop.

`href` assertion note: `<Button href="...">` renders `<a role="button">`. In tests, assert `href` via CSS selector
`document.querySelector('a[href="..."]')`, **not** `getByRole('button').toHaveAttribute('href', ...)` — Playwright/Testing Library's role-based
locator does not surface the `href` attribute on `role=button` elements.

---

## Button site inventory (read source files to verify exact line numbers before editing)

**login/+page.svelte** — 5 sites (the spec rollout step says "six" but the spec's own migration prose names exactly five buttons; this appears to be
a spec authoring error — the five named sites are authoritative. Verify against the live file and migrate those five):

1. `registrationTokenRequired` branch: submit → `variant="primary"`
2. `linkRequired` + `linkProviderId` branch: "Verify with linked provider" OIDC button → `variant="ghost"` + `loading={oidcLoading}`
3. `linkRequired` password form: "Link account" submit → `variant="primary"`
4. OIDC providers `{#each}` loop: provider button → `variant="ghost"` + snippet + `loading={oidcLoading}`
5. Password form: "Login" submit → `variant="primary"`

**register/+page.svelte** — 1 site (the "Login" link was already migrated by sub-spec #2 PR2):

1. "Register" submit → `variant="primary"`

**device/+page.svelte** — 2 sites:

1. "Log in" `<a>` anchor → `<Button href="..." variant="primary">`
2. "Approve" button → `variant="primary"` + `loading={approving}`

**+error.svelte** — 1 site:

1. "Go to Home" `<button onclick={() => goto('/')}` → `<Button href="/" variant="primary">` (drop `goto` import)

**PublicEntryShell.svelte** — 2 exports to delete:

- `PUBLIC_ENTRY_PRIMARY_BUTTON_CLASS`
- `PUBLIC_ENTRY_SECONDARY_BUTTON_CLASS`

---

## Task 1: Delete two constants from PublicEntryShell.svelte

**Files:**

- Modify: `frontend/src/lib/components/ui/PublicEntryShell.svelte`

Read `frontend/src/lib/components/ui/PublicEntryShell.svelte` first to confirm the exact lines.

- [ ] **Step 1: Delete the two button-class exports from the `<script module>` block**

Remove `PUBLIC_ENTRY_PRIMARY_BUTTON_CLASS` and `PUBLIC_ENTRY_SECONDARY_BUTTON_CLASS` from the module script. Keep `PUBLIC_ENTRY_FORM_CLASS`,
`PUBLIC_ENTRY_INPUT_CLASS`, `PUBLIC_ENTRY_CHECKBOX_CLASS`, and `PUBLIC_ENTRY_LINK_CLASS` untouched.

Before (approximate — read file for exact text):

```svelte
export const PUBLIC_ENTRY_PRIMARY_BUTTON_CLASS =
  'inline-flex h-9 w-full items-center justify-center rounded-lg border border-transparent ...';
export const PUBLIC_ENTRY_SECONDARY_BUTTON_CLASS =
  'inline-flex h-9 w-full items-center justify-center rounded-lg border border-[var(--border-default)] ...';
```

After: both lines deleted.

- [ ] **Step 2: Verify the file compiles**

```bash
cd frontend && npm run check 2>&1 | grep -i 'PublicEntryShell'
```

Expected: no errors on this file itself. (Dependent files will error until Tasks 2–5 remove the imports.)

- [ ] **Step 3: Commit**

```bash
git add frontend/src/lib/components/ui/PublicEntryShell.svelte
git commit -m "refactor(ui): remove PUBLIC_ENTRY_PRIMARY/SECONDARY_BUTTON_CLASS from PublicEntryShell"
```

---

## Task 2: Migrate login/+page.svelte (5 button sites)

**Files:**

- Modify: `frontend/src/routes/login/+page.svelte`

Read the file before editing to confirm exact button markup and surrounding context.

- [ ] **Step 1: Add `Button` import and trim named imports**

In the `<script lang="ts">` block, add:

```ts
import Button from '$lib/components/Button.svelte';
```

Remove `PUBLIC_ENTRY_PRIMARY_BUTTON_CLASS` and `PUBLIC_ENTRY_SECONDARY_BUTTON_CLASS` from the `PublicEntryShell` destructured import. Keep
`PUBLIC_ENTRY_FORM_CLASS`, `PUBLIC_ENTRY_INPUT_CLASS`, `PUBLIC_ENTRY_LINK_CLASS`.

- [ ] **Step 2: Migrate site 1 — Complete registration submit**

Before:

```svelte
<button type="submit" class={PUBLIC_ENTRY_PRIMARY_BUTTON_CLASS} disabled={!getIsOnline()}>
  Complete registration
</button>
```

After:

```svelte
<Button variant="primary" type="submit" class="w-full justify-center" disabled={!getIsOnline()}>
  Complete registration
</Button>
```

- [ ] **Step 3: Migrate site 2 — "Verify with linked provider" OIDC button**

Before (in `{:else if linkRequired}` → `{#if linkProviderId}` branch):

```svelte
<button
  type="button"
  class={`... PUBLIC_ENTRY_SECONDARY_BUTTON_CLASS ...`}
  disabled={oidcLoading}
  onclick={() => onLinkWithOidc(linkProviderId)}
>
  {oidcLoading ? 'Redirecting...' : 'Verify with linked provider'}
</button>
```

After:

```svelte
<Button
  variant="ghost"
  type="button"
  class="w-full justify-center"
  disabled={oidcLoading}
  loading={oidcLoading}
  onclick={() => onLinkWithOidc(linkProviderId)}
>
  Verify with linked provider
</Button>
```

- [ ] **Step 4: Migrate site 3 — "Link account" submit**

Before (in `{:else if linkRequired}` password form):

```svelte
<button type="submit" class={PUBLIC_ENTRY_PRIMARY_BUTTON_CLASS} disabled={!getIsOnline()}>
  Link account
</button>
```

After:

```svelte
<Button variant="primary" type="submit" class="w-full justify-center" disabled={!getIsOnline()}>
  Link account
</Button>
```

- [ ] **Step 5: Migrate site 4 — OIDC provider loop**

The snippet must be declared **inside** the `{#each}` block so each iteration captures its own `provider` via closure.

Before:

```svelte
{#each authMethods.oidc_providers as provider (provider.id)}
  <button
    type="button"
    class={`... PUBLIC_ENTRY_SECONDARY_BUTTON_CLASS ...`}
    disabled={oidcLoading}
    onclick={() => onOidcLogin(provider.id)}
  >
    {#if isValidLogoUrl(provider.logo_url)}
      <img src={provider.logo_url} alt="" class="h-5 w-5" referrerpolicy="no-referrer" />
    {/if}
    {oidcLoading ? 'Redirecting...' : `Login with ${provider.name}`}
  </button>
{/each}
```

After:

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

When `loading=true`, Button's spinner replaces the `leadingIcon` slot — the manual ternary is gone.

- [ ] **Step 6: Migrate site 5 — "Login" submit**

Before (in the password form):

```svelte
<button type="submit" class={PUBLIC_ENTRY_PRIMARY_BUTTON_CLASS} disabled={!getIsOnline()}>
  Login
</button>
```

After:

```svelte
<Button variant="primary" type="submit" class="w-full justify-center" disabled={!getIsOnline()}>
  Login
</Button>
```

- [ ] **Step 7: Verify compilation**

```bash
cd frontend && npm run check 2>&1 | grep -i 'login'
```

Expected: no type errors on this file.

- [ ] **Step 8: Commit**

```bash
git add frontend/src/routes/login/+page.svelte
git commit -m "refactor(login): migrate all 5 button sites to Button primitive (#3a)"
```

---

## Task 3: Migrate register/+page.svelte (1 button site)

**Files:**

- Modify: `frontend/src/routes/register/+page.svelte`

Note: The file already imports `Button` (added by sub-spec #2 PR2). Do NOT add a duplicate import.

- [ ] **Step 1: Remove `PUBLIC_ENTRY_PRIMARY_BUTTON_CLASS` from the destructured import**

The import currently reads something like:

```ts
import PublicEntryShell, { PUBLIC_ENTRY_PRIMARY_BUTTON_CLASS, PUBLIC_ENTRY_LINK_CLASS } from ...
```

After:

```ts
import PublicEntryShell, { PUBLIC_ENTRY_LINK_CLASS } from ...
```

(Keep `PUBLIC_ENTRY_LINK_CLASS` — sub-spec #3a2 retires it after #2b lands.)

- [ ] **Step 2: Migrate the Register submit button**

Before:

```svelte
<button type="submit" class={PUBLIC_ENTRY_PRIMARY_BUTTON_CLASS} disabled={!getIsOnline()}>
  Register
</button>
```

After:

```svelte
<Button variant="primary" type="submit" class="w-full justify-center" disabled={!getIsOnline()}>
  Register
</Button>
```

The `<Button variant="ghost" href="/login">Login</Button>` in the footer already uses the primitive — do not touch it.

- [ ] **Step 3: Commit**

```bash
git add frontend/src/routes/register/+page.svelte
git commit -m "refactor(register): migrate Register submit button to Button primitive (#3a)"
```

---

## Task 4: Migrate device/+page.svelte (2 button sites)

**Files:**

- Modify: `frontend/src/routes/device/+page.svelte`

Read the file first to confirm exact button markup.

- [ ] **Step 1: Add `Button` import and trim named imports**

```ts
import Button from '$lib/components/Button.svelte';
```

Remove `PUBLIC_ENTRY_PRIMARY_BUTTON_CLASS` from the `PublicEntryShell` import.

- [ ] **Step 2: Migrate site 1 — "Log in" anchor (unauthenticated branch)**

Before:

```svelte
<a href="/login?redirect=/device?code={encodeURIComponent(code)}" class={PUBLIC_ENTRY_PRIMARY_BUTTON_CLASS}>
  Log in
</a>
```

After:

```svelte
<Button
  variant="primary"
  href="/login?redirect=/device?code={encodeURIComponent(code)}"
  class="w-full justify-center"
>
  Log in
</Button>
```

- [ ] **Step 3: Migrate site 2 — "Approve" button**

Before:

```svelte
<button
  type="button"
  class={PUBLIC_ENTRY_PRIMARY_BUTTON_CLASS}
  disabled={approving}
  onclick={onApprove}
>
  {approving ? 'Authorizing...' : 'Approve'}
</button>
```

After:

```svelte
<Button
  variant="primary"
  type="button"
  class="w-full justify-center"
  disabled={approving}
  loading={approving}
  onclick={onApprove}
>
  Approve
</Button>
```

`loading={approving}` replaces the ternary children (`Authorizing...` text removed). `disabled={approving}` is kept redundantly per spec — Button
sets `disabled` internally when loading, but the explicit binding makes the intent explicit in the source. The `Authorizing...` text is gone —
spinner + unchanged label communicates the in-flight state.

- [ ] **Step 4: Commit**

```bash
git add frontend/src/routes/device/+page.svelte
git commit -m "refactor(device): migrate Log in anchor and Approve button to Button primitive (#3a)"
```

---

## Task 5: Migrate +error.svelte (1 button site, drop `goto`)

**Files:**

- Modify: `frontend/src/routes/+error.svelte`

Read the file first to confirm exact button markup and imports.

- [ ] **Step 1: Add `Button` import, remove `goto` import and `PUBLIC_ENTRY_PRIMARY_BUTTON_CLASS`**

Remove:

```ts
import { goto } from '$app/navigation';
```

Add:

```ts
import Button from '$lib/components/Button.svelte';
```

Remove `PUBLIC_ENTRY_PRIMARY_BUTTON_CLASS` from the `PublicEntryShell` destructured import. After this change the `PublicEntryShell` import line has
no named imports left — simplify to:

```ts
import PublicEntryShell from '$lib/components/ui/PublicEntryShell.svelte';
```

- [ ] **Step 2: Migrate the "Go to Home" button**

Before (in `{#snippet footer()}`):

```svelte
<button class={PUBLIC_ENTRY_PRIMARY_BUTTON_CLASS} onclick={() => goto('/')}>Go to Home</button>
```

After:

```svelte
<Button variant="primary" href="/" class="w-full justify-center">Go to Home</Button>
```

`<Button href="/">` renders `<a href="/" role="button">` — navigation semantics are correct.

- [ ] **Step 3: Commit**

```bash
git add frontend/src/routes/+error.svelte
git commit -m "refactor(error): migrate Go to Home to Button href branch, drop goto import (#3a)"
```

---

## Task 6: Extend public-entry.test.ts

**Files:**

- Modify: `frontend/src/routes/public-entry.test.ts`

Read `frontend/src/routes/public-entry.test.ts` in full before editing. The file imports, mock setup, and fixture structure are essential context.

- [ ] **Step 1: Remove deleted constant imports**

Remove `PUBLIC_ENTRY_PRIMARY_BUTTON_CLASS` and `PUBLIC_ENTRY_SECONDARY_BUTTON_CLASS` from the imports. They no longer exist in
`PublicEntryShell.svelte`.

- [ ] **Step 2: Rewrite existing assertions that referenced the deleted constants**

Find all `toContain(PUBLIC_ENTRY_PRIMARY_BUTTON_CLASS)` and similar assertions. Replace them with Button primitive assertions that maintain
equivalent coverage strength. For each migrated **primary** button, assert both size and variant:

```ts
// Before
expect(btn.className).toContain(PUBLIC_ENTRY_PRIMARY_BUTTON_CLASS);

// After — assert size AND primary variant gradient so coverage is equivalent
expect(btn.className).toContain('h-[23px]');
expect(btn.className).toContain('bg-[linear-gradient(90deg,var(--accent-deep),var(--accent))]');
```

For each migrated **ghost** button, assert size and ghost variant:

```ts
expect(btn.className).toContain('h-[23px]');
expect(btn.className).toContain('bg-transparent'); // unique to ghost
```

Also assert:

- Correct `type` attribute (`submit` or `button` depending on the site)
- `aria-busy` not present at idle (`expect(btn).not.toHaveAttribute('aria-busy')`)

- [ ] **Step 3: Add new assertions for loading state and href navigation**

**Login page — OIDC button loading state:**

`oidcLoading` is internal `$state` in `login/+page.svelte` and is set when `onOidcLogin()` is called. To induce the loading state, mock the OIDC
login function to never resolve and click the OIDC provider button:

```ts
it('OIDC provider button shows aria-busy and no Redirecting text when loading', async () => {
  // Read the existing test file to find the mock for the OIDC login API call
  // (the login page likely calls an auth function that sets oidcLoading).
  // Pattern: mock the OIDC call to return a never-resolving Promise, render, click the button.
  // Example (adjust the mock target to match the existing test file's import and render setup):
  vi.mocked(auth.initiateOidcLogin).mockReturnValue(new Promise(() => {}));
  // Use the same authMethods fixture the existing test file uses to show OIDC providers:
  render(LoginPage, { /* same props/mock as existing OIDC tests */ });
  await waitFor(() => expect(screen.getByRole('button', { name: /Login with/i })).toBeInTheDocument());

  const oidcBtn = screen.getByRole('button', { name: /Login with/i });
  await fireEvent.click(oidcBtn);
  await waitFor(() => expect(oidcBtn).toHaveAttribute('aria-busy', 'true'));
  expect(oidcBtn).not.toHaveTextContent('Redirecting...');
});
```

> The exact mock target depends on the login page's implementation. Read the existing test file to find how `onOidcLogin` is wired and which API
> function to mock. If the existing tests provide no pattern for loading state, add this test after confirming the implementation in Task 2.

**Device page — Approve loading state:**

`approving` is internal `$state` set when `onApprove()` starts. Mock the approve API to never resolve and click the button:

```ts
it('Approve button shows aria-busy and no Authorizing text when loading', async () => {
  // Read the existing test file for the approve API mock target (e.g. api.approveDevice).
  vi.mocked(api.approveDevice).mockReturnValue(new Promise(() => {}));
  // Use the same render setup the existing device tests use (route params, mock setup):
  render(DevicePage, { /* same props as existing device approved-state tests */ });
  await waitFor(() => expect(screen.getByRole('button', { name: 'Approve' })).toBeInTheDocument());

  const approveBtn = screen.getByRole('button', { name: 'Approve' });
  await fireEvent.click(approveBtn);
  await waitFor(() => expect(approveBtn).toHaveAttribute('aria-busy', 'true'));
  expect(approveBtn).not.toHaveTextContent('Authorizing...');
});
```

**+error page — Go to Home href assertion (CSS selector, not role+toHaveAttribute):**

```ts
it('+error Go to Home renders as <a href="/"> with Button md-size class', () => {
  // PublicErrorPage reads page.status and page.error from $app/state.
  // Read the existing +error test for the mock setup (e.g. vi.mock('$app/state', ...)).
  render(PublicErrorPage);
  // Use CSS selector for href — NOT getByRole('button').toHaveAttribute('href', '/')
  // Button href branch renders <a role="button"> — role-based locator doesn't expose href
  const anchor = document.querySelector('a[href="/"]') as HTMLElement;
  expect(anchor).not.toBeNull();
  expect(anchor.className).toContain('h-[23px]');
  expect(anchor.className).toContain('bg-[linear-gradient(90deg,var(--accent-deep),var(--accent))]');
});
```

**Offline-state test (all submit buttons disabled when `getIsOnline()` returns false):**

```ts
it('submit buttons carry disabled attribute when offline', async () => {
  // Mock getIsOnline to return false — read existing test for how to wire this
  vi.mocked(getIsOnline).mockReturnValue(false);
  render(LoginPage, { /* standard setup */ });
  await waitFor(() => expect(screen.getByRole('button', { name: /login/i })).toBeInTheDocument());

  const loginBtn = screen.getByRole('button', { name: /^login$/i });
  expect(loginBtn).toBeDisabled();
});
```

**Text-swap removal guards (neither 'Redirecting...' nor 'Authorizing...' appears in any rendered state):**

```ts
it('no Redirecting... text appears anywhere in login DOM during loading', async () => {
  vi.mocked(auth.initiateOidcLogin).mockReturnValue(new Promise(() => {}));
  render(LoginPage, { /* OIDC provider setup */ });
  await waitFor(() => expect(screen.getByRole('button', { name: /Login with/i })).toBeInTheDocument());
  await fireEvent.click(screen.getByRole('button', { name: /Login with/i }));
  await waitFor(() =>
    expect(screen.getByRole('button', { name: /Login with/i })).toHaveAttribute('aria-busy', 'true')
  );
  expect(document.body.textContent).not.toContain('Redirecting...');
});

it('no Authorizing... text appears anywhere in device DOM during loading', async () => {
  vi.mocked(api.approveDevice).mockReturnValue(new Promise(() => {}));
  render(DevicePage, { /* standard setup */ });
  await waitFor(() => expect(screen.getByRole('button', { name: 'Approve' })).toBeInTheDocument());
  await fireEvent.click(screen.getByRole('button', { name: 'Approve' }));
  await waitFor(() =>
    expect(screen.getByRole('button', { name: 'Approve' })).toHaveAttribute('aria-busy', 'true')
  );
  expect(document.body.textContent).not.toContain('Authorizing...');
});
```

- [ ] **Step 4: Run full test suite**

```bash
cd frontend && npx vitest run src/routes/public-entry.test.ts
```

Expected: all tests pass.

- [ ] **Step 5: Commit**

```bash
git add frontend/src/routes/public-entry.test.ts
git commit -m "test(public-entry): replace legacy class assertions, add Button primitive contract tests (#3a)"
```

---

## Task 7: Re-baseline Playwright snapshots

**Files:**

- Modify or create: public-entry e2e spec

Check if `frontend/tests/e2e/` contains an existing public-entry spec file. Read it if it does; create one if not.

- [ ] **Step 1: Check for existing public-entry e2e spec**

```bash
ls frontend/tests/e2e/
```

If a spec covering `/login`, `/register`, `/device`, `/error` exists, read it and re-baseline by running:

```bash
cd frontend && npx playwright test <spec-file> --update-snapshots
```

If no such spec exists, create `frontend/tests/e2e/public-entry.spec.ts`. Read `frontend/tests/e2e/button-primitive.spec.ts` for the exact
`mockAuthApi` pattern to copy. The spec should snapshot:

- `/login` — default state
- `/login` — `setup_required` state
- `/login` — registration-token required state
- `/login` — `link-required` state (account linking flow — user has an existing account, must re-authenticate via OIDC or password to link)
- `/register`
- `/device?code=ABCD-EFGH` (unauthenticated — shows "Log in" button)
- `/error` (404 or synthetic error page — shows "Go to Home" button)

Each page × 2 themes (dark + light) using `toHaveScreenshot` with `threshold: 0.005`.

- [ ] **Step 2: Generate baselines**

```bash
cd frontend && npx playwright test tests/e2e/public-entry.spec.ts --update-snapshots
```

- [ ] **Step 3: Re-run to confirm stability**

```bash
cd frontend && npx playwright test tests/e2e/public-entry.spec.ts
```

Expected: all pass.

- [ ] **Step 4: Commit**

```bash
git add frontend/tests/e2e/public-entry.spec.ts "frontend/tests/e2e/public-entry.spec.ts-snapshots"
git commit -m "test(e2e): add/re-baseline public-entry snapshots after Button primitive migration (#3a)"
```

---

## Task 8: Full frontend gate

- [ ] **Step 1: Run full gate**

```bash
cd frontend && npm run lint && npm run format:check && npm run check && npm run test && npm run build
```

Expected: every command exits 0.

- [ ] **Step 2: Run full e2e suite**

```bash
cd frontend && npx playwright test 2>&1 | tail -20
```

Expected: 0 failures. All other snapshot suites unaffected (this migration only touches public-entry routes).

---

## Commit summary

| # | Commit | Files |
| --- | --- | --- |
| 1 | Delete 2 button-class exports | `PublicEntryShell.svelte` |
| 2 | Migrate 5 sites | `login/+page.svelte` |
| 3 | Migrate 1 site | `register/+page.svelte` |
| 4 | Migrate 2 sites | `device/+page.svelte` |
| 5 | Migrate 1 site + drop goto | `+error.svelte` |
| 6 | Update tests | `public-entry.test.ts` |
| 7 | E2e baselines | `public-entry.spec.ts` + PNGs |

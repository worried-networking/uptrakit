# Public Entry Forms Migration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task.
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Migrate all raw `<input>`, `<input type="checkbox">`, and `<a class={PUBLIC_ENTRY_*_CLASS}>`
link elements in the public-entry routes to the Input, Checkbox, and Link primitives, and retire the
three remaining `PUBLIC_ENTRY_*_CLASS` constants from `PublicEntryShell.svelte`.

**Architecture:** Three route files each get targeted primitive migrations; PublicEntryShell loses its
last three utility class exports; existing tests are updated to assert on primitive behaviour rather
than class-constant presence.

**Tech Stack:** Svelte 5, SvelteKit, TypeScript, Vitest, Playwright

---

## Dependency

**Blocks on:** sub-spec #2b merged (Input, Checkbox, Link primitives) and sub-spec #3a merged (Button
migration). Both confirmed merged: `Input.svelte`, `Checkbox.svelte`, `Link.svelte`, and `Button.svelte`
are all present. `FormFieldRow`'s `aria-describedby` injection must be in place at merge time (spec Q4);
if not, this plan blocks until that lands.

---

## Input migration rules (quick reference)

Each `<input class={PUBLIC_ENTRY_INPUT_CLASS} ...>` migrates to `<Input>` with the following prop
mapping:

| Raw attribute | Input prop |
| --- | --- |
| `id="..."` | `id="..."` |
| `type="..."` | `type="..."` |
| `bind:value={x}` | `bind:value={x}` |
| `autocomplete="..."` | `autocomplete="..."` |
| `aria-invalid={err ? 'true' : undefined}` | dropped — Input derives from `error` prop |
| `oninput={fn}` | `oninput={fn}` |
| `placeholder="..."` | `placeholder="..."` |

The `error` prop is passed to both `FormFieldRow` (for error-copy rendering) and `<Input>` (for
`aria-invalid` + error-bg styling). `aria-invalid` bindings at call sites are dropped — `<Input>` sets
them internally from `error`.

The `registration-token` input in `login/+page.svelte` has no existing `autocomplete` attribute — use
`autocomplete="off"` on the migrated `<Input>`.

---

## Input site inventory (read source files to verify exact line numbers before editing)

**login/+page.svelte** — 4 input sites:

1. `registration-token` (`registrationTokenRequired` branch): `type="text"`, no existing `autocomplete`
   → `autocomplete="off"`, `error={registrationTokenError || undefined}`,
   `oninput={clearRegistrationTokenError}`
2. `link-password` (`linkRequired` password form): `type="password"`,
   `autocomplete="current-password"`, `error={linkPasswordError || undefined}`,
   `oninput={clearLinkPasswordError}`
3. `login-email` (password form): `type="email"`, `autocomplete="email"`,
   `error={loginFieldErrors.email || undefined}`, `oninput={() => clearLoginFieldError('email')}`
4. `login-password` (password form): `type="password"`, `autocomplete="current-password"`,
   `error={loginFieldErrors.password || undefined}`, `oninput={() => clearLoginFieldError('password')}`

**register/+page.svelte** — 5 input sites:

1. `register-email`: `type="email"`, `autocomplete="email"`,
   `error={registerFieldErrors.email || undefined}`,
   `oninput={() => clearRegisterFieldError('email')}`
2. `register-first-name`: `type="text"`, `autocomplete="given-name"`,
   `error={registerFieldErrors.firstName || undefined}`,
   `oninput={() => clearRegisterFieldError('firstName')`
3. `register-last-name`: `type="text"`, `autocomplete="family-name"`,
   `error={registerFieldErrors.lastName || undefined}`,
   `oninput={() => clearRegisterFieldError('lastName')`
4. `register-password`: `type="password"`, `autocomplete="new-password"`,
   `error={registerFieldErrors.password || undefined}`,
   `oninput={() => clearRegisterFieldError('password')`
5. `register-token` (conditional, `{#if showToken}`): `type="text"`, `autocomplete="off"` (existing)

**register/+page.svelte** — 1 checkbox site:

- Inline `<input type="checkbox" class={PUBLIC_ENTRY_CHECKBOX_CLASS} bind:checked={showToken}`
  `onchange={...}>` inside a `<label class="flex items-start gap-2 ...">` wrapper.

**login/+page.svelte** — 1 prose link site:

- Footer snippet: `<a href="/register" class={PUBLIC_ENTRY_LINK_CLASS}>Register</a>`

**PublicEntryShell.svelte** — 3 exports to delete:

- `PUBLIC_ENTRY_INPUT_CLASS`
- `PUBLIC_ENTRY_CHECKBOX_CLASS`
- `PUBLIC_ENTRY_LINK_CLASS`

Keep `PUBLIC_ENTRY_FORM_CLASS` — it has no primitive equivalent.

---

## Task 1: Delete three constants from PublicEntryShell.svelte

**Files:**

- Modify: `frontend/src/lib/components/ui/PublicEntryShell.svelte`

Read `frontend/src/lib/components/ui/PublicEntryShell.svelte` first to confirm the exact lines.

- [ ] **Step 1: Delete the three utility-class exports from the `<script lang="ts" module>` block**

Remove `PUBLIC_ENTRY_INPUT_CLASS`, `PUBLIC_ENTRY_CHECKBOX_CLASS`, and `PUBLIC_ENTRY_LINK_CLASS` from
the module script. Keep `PUBLIC_ENTRY_FORM_CLASS` untouched.

Before (approximate — read file for exact text):

```svelte
export const PUBLIC_ENTRY_INPUT_CLASS =
  'input h-8 w-full rounded-lg border border-[var(--border-default)] ...';
export const PUBLIC_ENTRY_CHECKBOX_CLASS =
  'checkbox h-4 w-4 rounded border-[var(--border-default)] ...';
export const PUBLIC_ENTRY_LINK_CLASS =
  'font-medium text-[var(--accent)] underline underline-offset-4 ...';
```

After: all three lines deleted. `PUBLIC_ENTRY_FORM_CLASS` remains as the only export.

- [ ] **Step 2: Verify the file compiles**

```bash
cd frontend && npm run check 2>&1 | grep -i 'PublicEntryShell'
```

Expected: no errors on this file itself. (Dependent files will error until Tasks 2–3 remove the
imports.)

- [ ] **Step 3: Commit**

```bash
git add frontend/src/lib/components/ui/PublicEntryShell.svelte
git commit -m "refactor(ui): remove PUBLIC_ENTRY_INPUT/CHECKBOX/LINK_CLASS from PublicEntryShell"
```

---

## Task 2: Migrate login/+page.svelte (4 input sites + 1 prose link)

**Files:**

- Modify: `frontend/src/routes/login/+page.svelte`

Read the file before editing to confirm exact input markup and surrounding context.

- [ ] **Step 1: Add Input and Link imports, trim named imports from PublicEntryShell**

In the `<script lang="ts">` block, add:

```ts
import Input from '$lib/components/Input.svelte';
import Link from '$lib/components/Link.svelte';
```

Remove `PUBLIC_ENTRY_INPUT_CLASS` and `PUBLIC_ENTRY_LINK_CLASS` from the `PublicEntryShell`
destructured import. Keep `PUBLIC_ENTRY_FORM_CLASS`.

After the trim the import line reads:

```ts
import PublicEntryShell, { PUBLIC_ENTRY_FORM_CLASS } from '$lib/components/ui/PublicEntryShell.svelte';
```

- [ ] **Step 2: Migrate site 1 — registration-token input**

Before (in the `registrationTokenRequired` branch, inside `<FormFieldRow>`):

```svelte
<input
  id="registration-token"
  class={PUBLIC_ENTRY_INPUT_CLASS}
  type="text"
  bind:value={registrationTokenInput}
  placeholder="Paste the registration token here"
  aria-invalid={registrationTokenError ? 'true' : undefined}
  oninput={clearRegistrationTokenError}
/>
```

After:

```svelte
<Input
  id="registration-token"
  type="text"
  bind:value={registrationTokenInput}
  placeholder="Paste the registration token here"
  autocomplete="off"
  error={registrationTokenError || undefined}
  oninput={clearRegistrationTokenError}
/>
```

`autocomplete="off"` is added — the raw element had none. The `aria-invalid` binding is dropped;
`<Input>` derives it from `error`.

- [ ] **Step 3: Migrate site 2 — link-password input**

Before (in the `linkRequired` password form, inside `<FormFieldRow>`):

```svelte
<input
  id="link-password"
  class={PUBLIC_ENTRY_INPUT_CLASS}
  type="password"
  bind:value={linkPassword}
  autocomplete="current-password"
  placeholder="Enter your password to verify"
  aria-invalid={linkPasswordError ? 'true' : undefined}
  oninput={clearLinkPasswordError}
/>
```

After:

```svelte
<Input
  id="link-password"
  type="password"
  bind:value={linkPassword}
  autocomplete="current-password"
  placeholder="Enter your password to verify"
  error={linkPasswordError || undefined}
  oninput={clearLinkPasswordError}
/>
```

- [ ] **Step 4: Migrate site 3 — login-email input**

Before (in the password form):

```svelte
<input
  id="login-email"
  class={PUBLIC_ENTRY_INPUT_CLASS}
  type="email"
  bind:value={email}
  autocomplete="email"
  aria-invalid={loginFieldErrors.email ? 'true' : undefined}
  oninput={() => clearLoginFieldError('email')}
/>
```

After:

```svelte
<Input
  id="login-email"
  type="email"
  bind:value={email}
  autocomplete="email"
  error={loginFieldErrors.email || undefined}
  oninput={() => clearLoginFieldError('email')}
/>
```

- [ ] **Step 5: Migrate site 4 — login-password input**

Before (in the password form):

```svelte
<input
  id="login-password"
  class={PUBLIC_ENTRY_INPUT_CLASS}
  type="password"
  bind:value={password}
  autocomplete="current-password"
  aria-invalid={loginFieldErrors.password ? 'true' : undefined}
  oninput={() => clearLoginFieldError('password')}
/>
```

After:

```svelte
<Input
  id="login-password"
  type="password"
  bind:value={password}
  autocomplete="current-password"
  error={loginFieldErrors.password || undefined}
  oninput={() => clearLoginFieldError('password')}
/>
```

- [ ] **Step 6: Migrate the footer prose link**

Before (in `{#snippet footer()}`):

```svelte
<a href="/register" class={PUBLIC_ENTRY_LINK_CLASS}>Register</a>
```

After:

```svelte
<Link href="/register">Register</Link>
```

`variant="default"` is the default on `<Link>` and need not be stated explicitly.

- [ ] **Step 7: Verify compilation**

```bash
cd frontend && npm run check 2>&1 | grep -i 'login'
```

Expected: no type errors on this file.

- [ ] **Step 8: Commit**

```bash
git add frontend/src/routes/login/+page.svelte
git commit -m "refactor(login): migrate 4 inputs and footer link to Input/Link primitives (#3a2)"
```

---

## Task 3: Migrate register/+page.svelte (5 input sites + 1 checkbox)

**Files:**

- Modify: `frontend/src/routes/register/+page.svelte`

Read the file before editing to confirm exact markup.

- [ ] **Step 1: Add Input and Checkbox imports, trim named imports from PublicEntryShell**

In the `<script lang="ts">` block, add:

```ts
import Input from '$lib/components/Input.svelte';
import Checkbox from '$lib/components/Checkbox.svelte';
```

Remove `PUBLIC_ENTRY_INPUT_CLASS` and `PUBLIC_ENTRY_CHECKBOX_CLASS` from the `PublicEntryShell`
destructured import. Keep `PUBLIC_ENTRY_FORM_CLASS`. `PUBLIC_ENTRY_LINK_CLASS` is not imported in this
file — do not list it as a drop target.

After the trim the import line reads:

```ts
import PublicEntryShell, { PUBLIC_ENTRY_FORM_CLASS } from '$lib/components/ui/PublicEntryShell.svelte';
```

- [ ] **Step 2: Migrate site 1 — register-email input**

Before:

```svelte
<input
  id="register-email"
  class={PUBLIC_ENTRY_INPUT_CLASS}
  type="email"
  bind:value={email}
  autocomplete="email"
  aria-invalid={registerFieldErrors.email ? 'true' : undefined}
  oninput={() => clearRegisterFieldError('email')}
/>
```

After:

```svelte
<Input
  id="register-email"
  type="email"
  bind:value={email}
  autocomplete="email"
  error={registerFieldErrors.email || undefined}
  oninput={() => clearRegisterFieldError('email')}
/>
```

- [ ] **Step 3: Migrate site 2 — register-first-name input**

Before:

```svelte
<input
  id="register-first-name"
  class={PUBLIC_ENTRY_INPUT_CLASS}
  type="text"
  bind:value={firstName}
  autocomplete="given-name"
  aria-invalid={registerFieldErrors.firstName ? 'true' : undefined}
  oninput={() => clearRegisterFieldError('firstName')}
/>
```

After:

```svelte
<Input
  id="register-first-name"
  type="text"
  bind:value={firstName}
  autocomplete="given-name"
  error={registerFieldErrors.firstName || undefined}
  oninput={() => clearRegisterFieldError('firstName')}
/>
```

- [ ] **Step 4: Migrate site 3 — register-last-name input**

Before:

```svelte
<input
  id="register-last-name"
  class={PUBLIC_ENTRY_INPUT_CLASS}
  type="text"
  bind:value={lastName}
  autocomplete="family-name"
  aria-invalid={registerFieldErrors.lastName ? 'true' : undefined}
  oninput={() => clearRegisterFieldError('lastName')}
/>
```

After:

```svelte
<Input
  id="register-last-name"
  type="text"
  bind:value={lastName}
  autocomplete="family-name"
  error={registerFieldErrors.lastName || undefined}
  oninput={() => clearRegisterFieldError('lastName')}
/>
```

- [ ] **Step 5: Migrate site 4 — register-password input**

Before:

```svelte
<input
  id="register-password"
  class={PUBLIC_ENTRY_INPUT_CLASS}
  type="password"
  bind:value={password}
  autocomplete="new-password"
  aria-invalid={registerFieldErrors.password ? 'true' : undefined}
  oninput={() => clearRegisterFieldError('password')}
/>
```

After:

```svelte
<Input
  id="register-password"
  type="password"
  bind:value={password}
  autocomplete="new-password"
  error={registerFieldErrors.password || undefined}
  oninput={() => clearRegisterFieldError('password')}
/>
```

- [ ] **Step 6: Migrate site 5 — register-token input (conditional)**

Before (inside `{#if showToken}` block):

```svelte
<input
  id="register-token"
  class={PUBLIC_ENTRY_INPUT_CLASS}
  type="text"
  bind:value={registrationToken}
  autocomplete="off"
/>
```

After:

```svelte
<Input
  id="register-token"
  type="text"
  bind:value={registrationToken}
  autocomplete="off"
/>
```

- [ ] **Step 7: Confirm register footer link is already migrated (no-op)**

The spec notes the `register/+page.svelte` footer "Login" link is already `<Button variant="ghost"
href="/login">Login</Button>` from the #3a migration — not a target under this sub-spec. Verify
before proceeding:

```bash
grep -n 'href="/login"' frontend/src/routes/register/+page.svelte
```

Expected output includes `<Button` on the same line or the surrounding context. If it still reads
`<a class=...>`, migrate it to `<Link href="/login">Login</Link>` now (same pattern as Task 2 Step 6)
and commit before proceeding.

- [ ] **Step 8: Migrate the checkbox**

The outer `<label>` wrapper and inner `<span>` must be preserved exactly. Only the
`<input type="checkbox">` is replaced. `<Checkbox>` requires a non-optional `id` prop — use
`id="show-token"`.

The spec recommends the controlled pattern (`checked={showToken}` + explicit `onchange`) for this call
site because the `onchange` handler carries a side effect (clearing `registrationToken` when toggled
off). Both patterns are valid; the controlled pattern keeps the side-effect handler explicit.

Before:

```svelte
<label class="flex items-start gap-2 text-sm text-[var(--text-secondary)]">
  <input
    class={PUBLIC_ENTRY_CHECKBOX_CLASS}
    type="checkbox"
    bind:checked={showToken}
    onchange={() => {
      if (!showToken) registrationToken = '';
    }}
  />
  <span>I have an invite token</span>
</label>
```

After (controlled pattern):

```svelte
<label class="flex items-start gap-2 text-sm text-[var(--text-secondary)]">
  <Checkbox
    id="show-token"
    checked={showToken}
    onchange={(e) => {
      showToken = (e.target as HTMLInputElement).checked;
      if (!showToken) registrationToken = '';
    }}
  />
  <span>I have an invite token</span>
</label>
```

Alternatively, `bind:checked={showToken}` is valid (`$bindable(false)` in `Checkbox.svelte` supports
it) — retain the side-effect `onchange` as-is if using two-way binding.

- [ ] **Step 9: Verify compilation**

```bash
cd frontend && npm run check 2>&1 | grep -i 'register'
```

Expected: no type errors on this file.

- [ ] **Step 10: Commit**

```bash
git add frontend/src/routes/register/+page.svelte
git commit -m "refactor(register): migrate 5 inputs and checkbox to Input/Checkbox primitives (#3a2)"
```

---

## Task 4: Update public-entry.test.ts

**Files:**

- Modify: `frontend/src/routes/public-entry.test.ts`

Read the file in full before editing. The existing mock setup, fixture patterns, and assertion styles
are essential context. The `PUBLIC_ENTRY_INPUT_CLASS` import at line 28 and assertion at line 77 must
both be removed.

- [ ] **Step 1: Remove the `PUBLIC_ENTRY_INPUT_CLASS` import**

Line 28 currently reads:

```ts
import { PUBLIC_ENTRY_INPUT_CLASS } from '$lib/components/ui/PublicEntryShell.svelte';
```

Delete this import line. The constant no longer exists in `PublicEntryShell.svelte` after Task 1.

- [ ] **Step 2: Delete line 77 — the breaking `PUBLIC_ENTRY_INPUT_CLASS` assertion**

Line 77 currently reads:

```ts
expect(screen.getByLabelText('Email').className).toContain(PUBLIC_ENTRY_INPUT_CLASS);
```

Delete this line. After migration the Email input renders through `<Input>` with its own `BASE` class,
not `PUBLIC_ENTRY_INPUT_CLASS`. The surrounding assertions at lines 75–76 (`aria-invalid="true"` on
Email and Password) remain unchanged — they are already valid primitive-contract assertions.

- [ ] **Step 3: Add Input primitive contract assertions for always-rendered fields**

After the inline-error assertions in the existing login test, add assertions for the migrated
`login-email` and `login-password` inputs:

```ts
const emailInput = screen.getByLabelText('Email');
expect(emailInput.getAttribute('id')).toBe('login-email');
expect(emailInput.getAttribute('type')).toBe('email');
expect(emailInput.getAttribute('autocomplete')).toBe('email');

const passwordInput = screen.getByLabelText('Password');
expect(passwordInput.getAttribute('id')).toBe('login-password');
expect(passwordInput.getAttribute('type')).toBe('password');
expect(passwordInput.getAttribute('autocomplete')).toBe('current-password');
```

- [ ] **Step 4: Add `aria-describedby` wiring test for Input error state**

After an error is set on an Input and FormFieldRow renders the error copy, the `<input>` element's
`aria-describedby` attribute must point to the id of the error copy node rendered by `FormFieldRow`.
This verifies the `FormFieldRow` ↔ `Input` wiring from spec Q4.

```ts
it('login-email Input aria-describedby points to FormFieldRow error copy id after error set', async () => {
  render(LoginPage);
  await waitFor(() =>
    expect(screen.getByRole('heading', { name: 'Login' })).toBeInTheDocument()
  );

  // Trigger a login attempt that sets loginFieldErrors.email
  await fireEvent.input(screen.getByLabelText('Email'), { target: { value: 'bad' } });
  await fireEvent.input(screen.getByLabelText('Password'), { target: { value: 'pw' } });
  await fireEvent.click(screen.getByRole('button', { name: /log in/i }));

  await waitFor(() =>
    expect(screen.getByLabelText('Email')).toHaveAttribute('aria-invalid', 'true')
  );

  const emailInput = screen.getByLabelText('Email');
  const describedById = emailInput.getAttribute('aria-describedby');
  expect(describedById).toBeTruthy();
  const errorNode = document.getElementById(describedById!);
  expect(errorNode).not.toBeNull();
  expect(errorNode!.textContent?.trim().length).toBeGreaterThan(0);
});
```

Note: if the login form doesn't emit a client-side field error without a server round-trip, mock
`api.login` to reject with a structured field error object and check that `loginFieldErrors.email`
gets populated — mirror the existing mock pattern in the test file.

- [ ] **Step 5: Add test for conditionally-rendered registration-token input**

```ts
it('registration-token Input renders type=text and autocomplete=off under registrationTokenRequired', async () => {
  page.url = new URL(
    'http://localhost/login#registration_token_required=true&registration_code=RC123'
  ) as typeof page.url;

  render(LoginPage);

  await waitFor(() =>
    expect(screen.getByLabelText('Registration token')).toBeInTheDocument()
  );

  const tokenInput = screen.getByLabelText('Registration token');
  expect(tokenInput.getAttribute('id')).toBe('registration-token');
  expect(tokenInput.getAttribute('type')).toBe('text');
  expect(tokenInput.getAttribute('autocomplete')).toBe('off');
});
```

- [ ] **Step 6: Add test for conditionally-rendered link-password input**

```ts
it('link-password Input renders type=password and autocomplete=current-password under linkRequired', async () => {
  page.url = new URL(
    'http://localhost/login?link_required=true&email=user@example.com'
  ) as typeof page.url;

  render(LoginPage);

  await waitFor(() => expect(screen.getByLabelText('Password')).toBeInTheDocument());

  const linkPwInput = screen.getByLabelText('Password');
  expect(linkPwInput.getAttribute('id')).toBe('link-password');
  expect(linkPwInput.getAttribute('type')).toBe('password');
  expect(linkPwInput.getAttribute('autocomplete')).toBe('current-password');
});
```

- [ ] **Step 7: Add Checkbox `disabled` state test**

Add this to `frontend/src/routes/public-entry.test.ts`, importing `Checkbox` at the top of the
file alongside existing imports:

```ts
import Checkbox from '$lib/components/Checkbox.svelte';
```

Then add the test:

```ts
it('Checkbox renders opacity-40 class when disabled=true', () => {
  const { container } = render(Checkbox, { id: 'test-cb', disabled: true });
  const checkbox = container.querySelector('#test-cb') as HTMLInputElement;
  expect(checkbox).not.toBeNull();
  const wrapper = checkbox.closest('[class*="opacity"]') ?? checkbox.parentElement;
  expect(wrapper?.className ?? checkbox.className).toContain('opacity-40');
});
```

- [ ] **Step 8: Add Checkbox primitive contract test**

```ts
it('show-token Checkbox renders with id=show-token, toggles field, and fires onchange exactly once per click', async () => {
  page.url = new URL('http://localhost/register') as typeof page.url;

  render(RegisterPage);

  const checkbox = document.querySelector('#show-token') as HTMLInputElement;
  expect(checkbox).not.toBeNull();
  expect(checkbox.getAttribute('type')).toBe('checkbox');
  expect(checkbox.checked).toBe(false);

  const handler = vi.fn();
  checkbox.addEventListener('change', handler);

  await fireEvent.click(checkbox);

  expect(handler).toHaveBeenCalledTimes(1);
  expect(checkbox.checked).toBe(true);
  await waitFor(() => expect(screen.getByLabelText('Invite token')).toBeInTheDocument());

  await fireEvent.click(checkbox);
  expect(handler).toHaveBeenCalledTimes(2);
  await waitFor(() =>
    expect(screen.queryByLabelText('Invite token')).not.toBeInTheDocument()
  );
});
```

- [ ] **Step 9: Add Link primitive contract test for the login footer link**

```ts
it('login footer Register link renders as <Link> with href=/register', async () => {
  render(LoginPage);

  await waitFor(() =>
    expect(screen.getByRole('heading', { name: 'Login' })).toBeInTheDocument()
  );

  const registerLink = screen.getByRole('link', { name: 'Register' });
  expect(registerLink).toHaveAttribute('href', '/register');
  expect(registerLink.className).toContain('font-medium');
  expect(registerLink.className).toContain('underline');
});
```

- [ ] **Step 10: Add regression guard — deleted class literal strings not in DOM**

```ts
it('regression: deleted PUBLIC_ENTRY_INPUT/CHECKBOX/LINK_CLASS literal strings absent from DOM', async () => {
  render(LoginPage);
  await waitFor(() =>
    expect(screen.getByRole('heading', { name: 'Login' })).toBeInTheDocument()
  );
  // Input class fragment
  expect(document.body.innerHTML).not.toContain('rounded-lg border border-[var(--border-default)]');
  // Link class fragment
  expect(document.body.innerHTML).not.toContain('hover:text-[var(--accent-bright)] focus-visible:outline-none');

  // Checkbox class fragment — needs register page
  cleanup();
  page.url = new URL('http://localhost/register') as typeof page.url;
  render(RegisterPage);
  await waitFor(() =>
    expect(screen.getByRole('heading', { name: 'Register' })).toBeInTheDocument()
  );
  expect(document.body.innerHTML).not.toContain('checkbox h-4 w-4 rounded border-[var(--border-default)]');
});
```

- [ ] **Step 11: Run full unit test suite**

```bash
cd frontend && npx vitest run src/routes/public-entry.test.ts
```

Expected: all tests pass.

- [ ] **Step 12: Commit**

```bash
git add frontend/src/routes/public-entry.test.ts
git commit -m "test(public-entry): remove PUBLIC_ENTRY_INPUT_CLASS assertion, add Input/Checkbox/Link tests (#3a2)"
```

---

## Task 5: Re-baseline Playwright snapshots

**Files:**

- Modify: existing public-entry e2e spec and its snapshot directory

- [ ] **Step 1: Locate the existing public-entry e2e spec**

```bash
ls frontend/tests/e2e/
```

The #3a plan created or updated `public-entry.spec.ts`. Read it to understand covered permutations and
themes.

- [ ] **Step 2: Re-baseline /login permutations and /register snapshots**

Visual deltas introduced by this migration:

- Input radius: `rounded-lg` → `rounded-[3px]` (sharper corners on all inputs)
- Input error state: bg swaps to `--color-error-bg`, border to `--color-error-border`
- Checkbox: new `rounded-[2px]`, `accent-[var(--accent)]`, focus-visible ring (no legacy equivalent)
- Link: `underline-offset-4`, hover color `--accent` → `--accent-bright` on default variant

Run `--update-snapshots` to accept these deltas across both themes (dark + light):

```bash
cd frontend && npx playwright test tests/e2e/public-entry.spec.ts --update-snapshots
```

`device` and `+error` snapshots are unchanged (no migration sites in those routes) — assert they still
match within 0.5 % threshold by running without `--update-snapshots` after the re-baseline and checking
for 0 failures.

- [ ] **Step 3: Assert WCAG AA contrast ratio for error state in both themes**

After re-baseline, add a Playwright assertion block (or a separate test `it` block) in
`tests/e2e/public-entry.spec.ts` that evaluates the computed contrast ratio of the error-bg + body
text pair in both themes. Use the `getComputedStyle` approach:

```ts
test('Input error state meets WCAG AA contrast in light theme', async ({ page }) => {
  await page.goto('/login');
  // Force a field error (e.g., submit with empty fields)
  await page.getByRole('button', { name: /log in/i }).click();
  // Get the error-state input background and the text color
  const errorInput = page.locator('#login-email');
  const bg = await errorInput.evaluate((el) =>
    getComputedStyle(el).backgroundColor
  );
  const color = await errorInput.evaluate((el) =>
    getComputedStyle(el).color
  );
  // Parse rgb(r, g, b) strings to relative luminance and assert contrast >= 4.5
  function relativeLuminance(rgb: string): number {
    const [r, g, b] = rgb.match(/\d+/g)!.map(Number).map((c) => {
      const s = c / 255;
      return s <= 0.03928 ? s / 12.92 : Math.pow((s + 0.055) / 1.055, 2.4);
    });
    return 0.2126 * r + 0.7152 * g + 0.0722 * b;
  }
  function contrastRatio(l1: number, l2: number): number {
    const [lighter, darker] = l1 > l2 ? [l1, l2] : [l2, l1];
    return (lighter + 0.05) / (darker + 0.05);
  }
  const ratio = contrastRatio(relativeLuminance(bg), relativeLuminance(color));
  expect(ratio).toBeGreaterThanOrEqual(4.5);
});

test('Input error state meets WCAG AA contrast in dark theme', async ({ page }) => {
  await page.goto('/login');
  await page.emulateMedia({ colorScheme: 'dark' });
  await page.getByRole('button', { name: /log in/i }).click();
  const errorInput = page.locator('#login-email');
  const bg = await errorInput.evaluate((el) => getComputedStyle(el).backgroundColor);
  const color = await errorInput.evaluate((el) => getComputedStyle(el).color);
  function relativeLuminance(rgb: string): number {
    const [r, g, b] = rgb.match(/\d+/g)!.map(Number).map((c) => {
      const s = c / 255;
      return s <= 0.03928 ? s / 12.92 : Math.pow((s + 0.055) / 1.055, 2.4);
    });
    return 0.2126 * r + 0.7152 * g + 0.0722 * b;
  }
  function contrastRatio(l1: number, l2: number): number {
    const [lighter, darker] = l1 > l2 ? [l1, l2] : [l2, l1];
    return (lighter + 0.05) / (darker + 0.05);
  }
  const ratio = contrastRatio(relativeLuminance(bg), relativeLuminance(color));
  expect(ratio).toBeGreaterThanOrEqual(4.5);
});
```

Run to verify both pass:

```bash
cd frontend && npx playwright test tests/e2e/public-entry.spec.ts -g "WCAG AA"
```

Expected: 2 passing.

- [ ] **Step 4: Re-run to confirm stability**

```bash
cd frontend && npx playwright test tests/e2e/public-entry.spec.ts
```

Expected: all pass with 0 failures.

- [ ] **Step 5: Commit**

```bash
git add "frontend/tests/e2e/public-entry.spec.ts-snapshots"
git add frontend/tests/e2e/public-entry.spec.ts
git commit -m "test(e2e): re-baseline public-entry snapshots and add WCAG AA contrast assertions (#3a2)"
```

---

## Task 6: Full frontend gate

- [ ] **Step 1: Run full gate**

```bash
cd frontend && npm run lint && npm run format:check && npm run check && npm run test && npm run build
```

Expected: every command exits 0.

- [ ] **Step 2: Run full e2e suite**

```bash
cd frontend && npx playwright test 2>&1 | tail -20
```

Expected: 0 failures. All non-public-entry snapshot suites unaffected.

---

## Commit summary

| # | Commit | Files |
| --- | --- | --- |
| 1 | Delete 3 utility-class exports | `PublicEntryShell.svelte` |
| 2 | Migrate 4 inputs + 1 footer link | `login/+page.svelte` |
| 3 | Migrate 5 inputs + 1 checkbox | `register/+page.svelte` |
| 4 | Update unit tests | `public-entry.test.ts` |
| 5 | Re-baseline e2e snapshots | `public-entry.spec.ts-snapshots/` |

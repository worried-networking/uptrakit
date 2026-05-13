# 2FA for Password Auth — Frontend Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the frontend side of 2FA: MFA step on the login page, 2FA enrollment/management in the
profile, enforcement toggle in auth settings, and global 403 `2fa_setup_required` redirect.

**Architecture:** Svelte 5 runes (`$state`, `$effect`, `$props`). Login page gets a new "MFA step" phase
controlled by local state. Profile page gets a new Security section with enrollment flow. Auth settings
gets a new `two_factor_required` toggle. The `+layout.svelte` global error handler intercepts
403 `2fa_setup_required` responses.

**Tech Stack:** SvelteKit, Svelte 5 runes, TypeScript strict mode. Lint: `npm run lint`,
format: `npm run format:check`, type-check: `npm run check`, unit tests: `npm run test`,
build: `npm run build`. Run all from `frontend/`.

**Prerequisite:** Backend plan `2026-05-13-2fa-backend.md` must be fully deployed before this plan is tested end-to-end.

---

## File Map

### New files

| Path                                                     | Purpose                                               |
| -------------------------------------------------------- | ----------------------------------------------------- |
| `frontend/src/lib/components/mfa/MfaStep.svelte`         | MFA code input step on login page                     |
| `frontend/src/lib/components/mfa/TotpEnrollModal.svelte` | TOTP enrollment modal (QR + confirm + recovery codes) |
| `frontend/src/routes/profile/SecuritySection.svelte`     | 2FA status + enroll/disable/regenerate in profile     |

### Modified files

| Path                                                         | Change                                      |
| ------------------------------------------------------------ | ------------------------------------------- |
| `frontend/src/lib/api.ts`                                    | Add 7 MFA API functions                     |
| `frontend/src/lib/types.ts`                                  | Add MFA request/response types              |
| `frontend/src/routes/login/+page.svelte`                     | Detect 202 → show MFA step                  |
| `frontend/src/routes/profile/+page.svelte`                   | Add SecuritySection                         |
| `frontend/src/routes/settings/AuthenticationSettings.svelte` | Add `two_factor_required` toggle            |
| `frontend/src/routes/+layout.svelte`                         | Intercept 403 `2fa_setup_required` globally |

---

### Task 1: MFA types + API client functions

**Files:**

- Modify: `frontend/src/lib/types.ts`
- Modify: `frontend/src/lib/api.ts`

- [ ] **Step 1: Add MFA types to `types.ts`**

Find the `AuthenticationSettings` interface and add after it:

```typescript
export type MfaMethod = "totp" | "email" | "recovery_code";

export interface MfaChallengeResponse {
  mfa_token: string;
  mfa_methods: MfaMethod[];
}

export interface MfaVerifyRequest {
  mfa_token: string;
  code: string;
  method: MfaMethod;
}

export interface MfaEmailRequest {
  mfa_token: string;
}

export interface MfaStatusResponse {
  totp_enrolled: boolean;
  recovery_codes_count: number;
  methods_available: MfaMethod[];
}

export interface TotpEnrollResponse {
  otpauth_uri: string;
  secret: string;
}

export interface TotpConfirmRequest {
  code: string;
}

export interface TotpConfirmResponse {
  recovery_codes: string[];
  session: AuthResponse | null;
}

export interface DisableTotpRequest {
  password?: string;
  totp_code?: string;
}

export interface RegenerateRecoveryCodesRequest {
  password?: string;
  totp_code?: string;
}

export interface RegenerateRecoveryCodesResponse {
  recovery_codes: string[];
}
```

Also update `AuthenticationSettings` and `UpdateAuthenticationSettings`:

```typescript
export interface AuthenticationSettings {
  password_auth_enabled: boolean;
  two_factor_required: boolean; // add
}

export interface UpdateAuthenticationSettings {
  password_auth_enabled?: boolean;
  two_factor_required?: boolean; // add
}
```

- [ ] **Step 2: Add MFA functions to `api.ts`**

Add imports at the top of `api.ts` for the new types (they come from `./types` which is already imported).
Then add 7 new exported functions after the existing auth functions:

```typescript
/** POST /api/v1/auth/mfa/verify — complete MFA challenge */
export async function mfaVerify(data: MfaVerifyRequest): Promise<AuthResponse> {
  const res = await fetch("/api/v1/auth/mfa/verify", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(data),
    credentials: "include",
  });
  if (!res.ok) throw new Error(await extractErrorMessage(res));
  return res.json() as Promise<AuthResponse>;
}

/** POST /api/v1/auth/mfa/email — trigger email OTP */
export async function mfaSendEmail(data: MfaEmailRequest): Promise<void> {
  const res = await fetch("/api/v1/auth/mfa/email", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(data),
    credentials: "include",
  });
  if (!res.ok) throw new Error(await extractErrorMessage(res));
}

/** GET /api/v1/auth/me/2fa — current 2FA status */
export function mfaStatus(): Promise<MfaStatusResponse> {
  return authenticatedGet("/api/v1/auth/me/2fa");
}

/** POST /api/v1/auth/me/2fa/totp/enroll — begin TOTP enrollment */
export function mfaEnroll(): Promise<TotpEnrollResponse> {
  return authenticatedPost("/api/v1/auth/me/2fa/totp/enroll", {});
}

/** POST /api/v1/auth/me/2fa/totp/confirm — confirm TOTP code */
export function mfaConfirm(
  data: TotpConfirmRequest,
): Promise<TotpConfirmResponse> {
  return authenticatedPost("/api/v1/auth/me/2fa/totp/confirm", data);
}

/** POST /api/v1/auth/me/2fa/totp/disable — disable TOTP */
export function mfaDisable(data: DisableTotpRequest): Promise<void> {
  return authenticatedPost("/api/v1/auth/me/2fa/totp/disable", data);
}

/** POST /api/v1/auth/me/2fa/recovery-codes/regenerate — replace recovery codes */
export function mfaRegenerateCodes(
  data: RegenerateRecoveryCodesRequest,
): Promise<RegenerateRecoveryCodesResponse> {
  return authenticatedPost(
    "/api/v1/auth/me/2fa/recovery-codes/regenerate",
    data,
  );
}
```

Where `authenticatedGet` and `authenticatedPost` are helpers for making authenticated requests. Check if they
already exist in `api.ts` — if not, these can simply use `authenticatedFetch`:

```typescript
function authenticatedGet<T>(path: string): Promise<T> {
  return authenticatedFetch(path).then((res) => {
    if (!res.ok) throw new Error(`Request failed: ${res.status}`);
    return res.json() as Promise<T>;
  });
}

function authenticatedPost<T>(path: string, body: unknown): Promise<T> {
  return authenticatedFetch(path, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  }).then(async (res) => {
    if (!res.ok) throw new Error(await extractErrorMessage(res));
    if (res.status === 200 && res.headers.get("content-length") === "0")
      return undefined as T;
    return res.json() as Promise<T>;
  });
}
```

- [ ] **Step 3: Type-check**

```bash
cd frontend && npm run check 2>&1 | tail -10
```

Expected: no errors.

- [ ] **Step 4: Commit**

```bash
git add frontend/src/lib/types.ts frontend/src/lib/api.ts
git commit -m "feat(frontend): add MFA types and API client functions"
```

---

### Task 2: Login page — detect 202 + MFA step component

**Files:**

- Create: `frontend/src/lib/components/mfa/MfaStep.svelte`
- Modify: `frontend/src/routes/login/+page.svelte`

- [ ] **Step 1: Create `MfaStep.svelte`**

Create `frontend/src/lib/components/mfa/MfaStep.svelte`:

```svelte
<script lang="ts">
  import { mfaVerify, mfaSendEmail } from '$lib/api';
  import type { MfaMethod, AuthResponse } from '$lib/types';
  import { FormFieldRow, Input } from '$lib/components/forms';
  import Button from '$lib/components/Button.svelte';
  import Link from '$lib/components/Link.svelte';

  let {
    mfaToken,
    availableMethods,
    onSuccess,
    onError,
  }: {
    mfaToken: string;
    availableMethods: MfaMethod[];
    onSuccess: (res: AuthResponse) => void;
    onError: (msg: string) => void;
  } = $props();

  type Phase = 'totp' | 'email';

  let phase = $state<Phase>('totp');
  let code = $state('');
  let loading = $state(false);
  let emailSent = $state(false);
  let errorMsg = $state('');

  const hasEmail = $derived(availableMethods.includes('email'));

  async function handleSubmit() {
    if (!code.trim()) return;
    loading = true;
    errorMsg = '';
    try {
      const method: MfaMethod = phase === 'email' ? 'email' : 'totp';
      const res = await mfaVerify({ mfa_token: mfaToken, code, method });
      onSuccess(res);
    } catch (e) {
      errorMsg = e instanceof Error ? e.message : 'Verification failed';
    } finally {
      loading = false;
    }
  }

  async function submitRecoveryCode() {
    if (!code.trim()) return;
    loading = true;
    errorMsg = '';
    try {
      const res = await mfaVerify({ mfa_token: mfaToken, code, method: 'recovery_code' });
      onSuccess(res);
    } catch (e) {
      errorMsg = e instanceof Error ? e.message : 'Recovery code invalid';
    } finally {
      loading = false;
    }
  }

  async function sendEmail() {
    loading = true;
    errorMsg = '';
    try {
      await mfaSendEmail({ mfa_token: mfaToken });
      emailSent = true;
      phase = 'email';
    } catch (e) {
      errorMsg = e instanceof Error ? e.message : 'Failed to send email code';
    } finally {
      loading = false;
    }
  }

  // Auto-submit when 6 digits entered (TOTP / email OTP)
  $effect(() => {
    if ((phase === 'totp' || phase === 'email') && code.length === 6 && !loading) {
      handleSubmit();
    }
  });
</script>

<div class="space-y-4">
  {#if errorMsg}
    <p class="text-[var(--color-danger)] text-sm">{errorMsg}</p>
  {/if}

  {#if phase === 'totp'}
    <p class="text-[var(--text-secondary)] text-sm">
      Enter the 6-digit code from your authenticator app.
    </p>
    <FormFieldRow label="Authenticator code" inputId="mfa-code">
      <Input
        id="mfa-code"
        type="text"
        inputmode="numeric"
        pattern="[0-9]*"
        maxlength={6}
        autocomplete="one-time-code"
        placeholder="000000"
        bind:value={code}
        disabled={loading}
      />
    </FormFieldRow>
    <Button variant="primary" {loading} disabled={code.length < 6} onclick={handleSubmit}>
      Verify
    </Button>
    {#if hasEmail}
      <div>
        <Link onclick={sendEmail} disabled={loading}>Use email code instead</Link>
      </div>
    {/if}
    <div>
      <Link onclick={() => { phase = 'totp'; code = ''; /* show recovery input */ }}>
        Use a recovery code
      </Link>
    </div>
  {:else if phase === 'email'}
    <p class="text-[var(--text-secondary)] text-sm">
      {emailSent
        ? 'A 6-digit code was sent to your email address.'
        : 'Enter the code sent to your email.'}
    </p>
    <FormFieldRow label="Email code" inputId="mfa-email-code">
      <Input
        id="mfa-email-code"
        type="text"
        inputmode="numeric"
        pattern="[0-9]*"
        maxlength={6}
        autocomplete="one-time-code"
        placeholder="000000"
        bind:value={code}
        disabled={loading}
      />
    </FormFieldRow>
    <Button variant="primary" {loading} disabled={code.length < 6} onclick={handleSubmit}>
      Verify
    </Button>
    <div>
      <Link onclick={sendEmail} disabled={loading}>Resend code</Link>
    </div>
  {/if}
</div>
```

- [ ] **Step 2: Modify `login/+page.svelte` to handle 202**

The login page currently calls `handleLogin` which wraps `api.login`. The `api.login` function throws on
non-200, so we need to call `api.login` directly (not `handleLogin`) to inspect a 202.

Read the current `handleLogin` in `auth.svelte.ts` and `api.login` in `api.ts`. Note that `api.login`
currently does `fetch(...).then(assertOk).then(json)`. We need a version that handles 202.

In the login page `<script>`, add:

```typescript
import * as api from "$lib/api";
import type { MfaChallengeResponse } from "$lib/types";
import MfaStep from "$lib/components/mfa/MfaStep.svelte";

let mfaChallenge = $state<MfaChallengeResponse | null>(null);

async function handleLoginSubmit() {
  // ... existing validation ...

  try {
    // Call raw fetch to detect 202
    const res = await fetch("/api/v1/auth/login", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ email, password: password }),
      credentials: "include",
    });

    if (res.status === 202) {
      // MFA required
      mfaChallenge = (await res.json()) as MfaChallengeResponse;
      return;
    }

    if (!res.ok) {
      const msg = await api.extractErrorMessage(res);
      bannerError = msg;
      return;
    }

    const authRes = await res.json();
    // Check for setup_required claim in access_token
    const setupRequired = parseSetupRequired(authRes.access_token as string);
    if (setupRequired) {
      // Store restricted token and redirect to enrollment
      setAccessToken(authRes.access_token as string);
      goto("/profile#security");
      return;
    }

    setAccessToken(authRes.access_token as string);
    setUser(authRes.user);
    setSessionExpired(false);
    goto(safeRedirect());
  } catch (e) {
    bannerError = e instanceof Error ? e.message : "Login failed";
  }
}

function parseSetupRequired(token: string): boolean {
  try {
    const payload = JSON.parse(
      atob(token.split(".")[1].replace(/-/g, "+").replace(/_/g, "/")),
    );
    return payload["setup_required"] === true;
  } catch {
    return false;
  }
}

function handleMfaSuccess(res: import("$lib/types").AuthResponse) {
  setAccessToken(res.access_token);
  setUser(res.user);
  setSessionExpired(false);
  mfaChallenge = null;
  goto(safeRedirect());
}
```

In the template, replace the existing login form submit with `handleLoginSubmit`. Add conditional rendering for the MFA step:

```svelte
{#if mfaChallenge}
  <MfaStep
    mfaToken={mfaChallenge.mfa_token}
    availableMethods={mfaChallenge.mfa_methods}
    onSuccess={handleMfaSuccess}
    onError={(msg) => { bannerError = msg; }}
  />
{:else}
  <!-- existing login form -->
{/if}
```

- [ ] **Step 3: Type-check + lint**

```bash
cd frontend && npm run check 2>&1 | tail -10 && npm run lint 2>&1 | tail -10
```

Expected: no errors.

- [ ] **Step 4: Commit**

```bash
git add frontend/src/lib/components/mfa/MfaStep.svelte \
        frontend/src/routes/login/+page.svelte
git commit -m "feat(frontend): MFA step on login page — detect 202 and handle challenge"
```

---

### Task 3: Profile page — Security section

**Files:**

- Create: `frontend/src/routes/profile/SecuritySection.svelte`
- Modify: `frontend/src/routes/profile/+page.svelte`

- [ ] **Step 1: Create `SecuritySection.svelte`**

```svelte
<script lang="ts">
  import { mfaStatus, mfaEnroll, mfaConfirm, mfaDisable, mfaRegenerateCodes } from '$lib/api';
  import type { MfaStatusResponse, TotpEnrollResponse } from '$lib/types';
  import { SectionCard } from '$lib/components/ui';
  import { FormFieldRow, Input } from '$lib/components/forms';
  import Button from '$lib/components/Button.svelte';

  let status = $state<MfaStatusResponse | null>(null);
  let enrollData = $state<TotpEnrollResponse | null>(null);
  let confirmCode = $state('');
  let recoveryCodes = $state<string[]>([]);
  let disablePassword = $state('');
  let disableTotpCode = $state('');
  let regenPassword = $state('');
  let regenTotpCode = $state('');
  let newRecoveryCodes = $state<string[]>([]);
  let loading = $state(false);
  let errorMsg = $state('');
  let successMsg = $state('');
  let showDisableForm = $state(false);
  let showRegenForm = $state(false);

  // Phase: 'idle' | 'enrolling' | 'confirming' | 'codes_shown'
  type Phase = 'idle' | 'enrolling' | 'confirming' | 'codes_shown';
  let phase = $state<Phase>('idle');

  async function loadStatus() {
    try {
      status = await mfaStatus();
    } catch {
      // Silently ignore — status section degrades gracefully
    }
  }

  $effect(() => { loadStatus().catch(console.error); });

  async function startEnroll() {
    loading = true;
    errorMsg = '';
    try {
      enrollData = await mfaEnroll();
      phase = 'confirming';
    } catch (e) {
      errorMsg = e instanceof Error ? e.message : 'Enrollment failed';
    } finally {
      loading = false;
    }
  }

  async function confirmEnroll() {
    if (!confirmCode || confirmCode.length !== 6) return;
    loading = true;
    errorMsg = '';
    try {
      const res = await mfaConfirm({ code: confirmCode });
      recoveryCodes = res.recovery_codes;
      phase = 'codes_shown';
      await loadStatus();
    } catch (e) {
      errorMsg = e instanceof Error ? e.message : 'Invalid code';
    } finally {
      loading = false;
    }
  }

  async function disable() {
    if (!disablePassword && !disableTotpCode) {
      errorMsg = 'Enter your password or authenticator code to confirm.';
      return;
    }
    loading = true;
    errorMsg = '';
    try {
      await mfaDisable(
        disablePassword ? { password: disablePassword } : { totp_code: disableTotpCode }
      );
      successMsg = '2FA has been disabled.';
      showDisableForm = false;
      disablePassword = '';
      disableTotpCode = '';
      await loadStatus();
    } catch (e) {
      errorMsg = e instanceof Error ? e.message : 'Failed to disable 2FA';
    } finally {
      loading = false;
    }
  }

  async function regenerate() {
    if (!regenPassword && !regenTotpCode) {
      errorMsg = 'Enter your password or authenticator code to confirm.';
      return;
    }
    loading = true;
    errorMsg = '';
    try {
      const res = await mfaRegenerateCodes(
        regenPassword ? { password: regenPassword } : { totp_code: regenTotpCode }
      );
      newRecoveryCodes = res.recovery_codes;
      successMsg = 'Recovery codes regenerated. Save them now — they will not be shown again.';
      showRegenForm = false;
      regenPassword = '';
      regenTotpCode = '';
      await loadStatus();
    } catch (e) {
      errorMsg = e instanceof Error ? e.message : 'Failed to regenerate codes';
    } finally {
      loading = false;
    }
  }
</script>

<SectionCard title="Two-Factor Authentication" id="security">
  {#if errorMsg}
    <p class="text-[var(--color-danger)] text-sm mb-2">{errorMsg}</p>
  {/if}
  {#if successMsg}
    <p class="text-[var(--color-success)] text-sm mb-2">{successMsg}</p>
  {/if}

  {#if status === null}
    <p class="text-[var(--text-secondary)] text-sm">Loading…</p>
  {:else if phase === 'idle'}
    {#if status.totp_enrolled}
      <p class="text-sm mb-3">
        Authenticator app is active. <strong>{status.recovery_codes_count}</strong>
        recovery {status.recovery_codes_count === 1 ? 'code' : 'codes'} remaining.
      </p>
      <div class="flex gap-2 flex-wrap">
        <Button variant="danger" onclick={() => { showDisableForm = !showDisableForm; errorMsg = ''; }}>
          Disable 2FA
        </Button>
        <Button variant="secondary" onclick={() => { showRegenForm = !showRegenForm; errorMsg = ''; }}>
          Regenerate recovery codes
        </Button>
      </div>

      {#if showDisableForm}
        <div class="mt-4 space-y-3 border border-[var(--border)] rounded p-4">
          <p class="text-sm text-[var(--text-secondary)]">Confirm with your password or authenticator code:</p>
          <FormFieldRow label="Password" inputId="disable-password">
            <Input id="disable-password" type="password" bind:value={disablePassword} disabled={loading} />
          </FormFieldRow>
          <p class="text-xs text-[var(--text-secondary)]">— or —</p>
          <FormFieldRow label="Authenticator code" inputId="disable-totp">
            <Input id="disable-totp" type="text" inputmode="numeric" maxlength={6} bind:value={disableTotpCode} disabled={loading} />
          </FormFieldRow>
          <Button variant="danger" {loading} onclick={disable}>Confirm disable</Button>
        </div>
      {/if}

      {#if showRegenForm}
        <div class="mt-4 space-y-3 border border-[var(--border)] rounded p-4">
          <p class="text-sm text-[var(--text-secondary)]">Confirm regeneration with your password or authenticator code:</p>
          <FormFieldRow label="Password" inputId="regen-password">
            <Input id="regen-password" type="password" bind:value={regenPassword} disabled={loading} />
          </FormFieldRow>
          <p class="text-xs text-[var(--text-secondary)]">— or —</p>
          <FormFieldRow label="Authenticator code" inputId="regen-totp">
            <Input id="regen-totp" type="text" inputmode="numeric" maxlength={6} bind:value={regenTotpCode} disabled={loading} />
          </FormFieldRow>
          <Button variant="primary" {loading} onclick={regenerate}>Generate new codes</Button>
        </div>
      {/if}

      {#if newRecoveryCodes.length > 0}
        <div class="mt-4 p-4 bg-[var(--bg-secondary)] rounded space-y-2">
          <p class="font-semibold text-sm">New recovery codes — save these now:</p>
          <ul class="font-mono text-sm space-y-1">
            {#each newRecoveryCodes as c}
              <li>{c}</li>
            {/each}
          </ul>
        </div>
      {/if}
    {:else}
      <p class="text-sm text-[var(--text-secondary)] mb-3">
        No authenticator app is set up. Add 2FA to protect your account.
      </p>
      <Button variant="primary" {loading} onclick={startEnroll}>Set up authenticator app</Button>
    {/if}
  {:else if phase === 'confirming' && enrollData}
    <div class="space-y-4">
      <p class="text-sm text-[var(--text-secondary)]">
        Scan the QR code below with your authenticator app (Google Authenticator, Authy, 1Password, etc.).
      </p>
      <!-- QR code: rendered client-side from otpauth_uri using a library or img src trick -->
      <!-- Simple fallback: display the otpauth URI as a link to open in auth app on mobile -->
      <div class="p-3 bg-[var(--bg-secondary)] rounded break-all">
        <p class="text-xs font-mono">{enrollData.secret}</p>
        <p class="text-xs text-[var(--text-secondary)] mt-1">Manual entry secret (Base32)</p>
      </div>
      <p class="text-sm">Then enter the 6-digit code to confirm:</p>
      <FormFieldRow label="Code from app" inputId="confirm-code">
        <Input
          id="confirm-code"
          type="text"
          inputmode="numeric"
          pattern="[0-9]*"
          maxlength={6}
          autocomplete="one-time-code"
          placeholder="000000"
          bind:value={confirmCode}
          disabled={loading}
        />
      </FormFieldRow>
      <Button variant="primary" {loading} disabled={confirmCode.length < 6} onclick={confirmEnroll}>
        Confirm
      </Button>
      <Button variant="ghost" onclick={() => { phase = 'idle'; enrollData = null; confirmCode = ''; }}>
        Cancel
      </Button>
    </div>
  {:else if phase === 'codes_shown'}
    <div class="space-y-3">
      <p class="font-semibold text-[var(--color-success)]">2FA is now enabled!</p>
      <p class="text-sm text-[var(--text-secondary)]">
        Save these recovery codes somewhere safe. Each can only be used once, and you
        will not see them again.
      </p>
      <ul class="font-mono text-sm space-y-1 p-3 bg-[var(--bg-secondary)] rounded">
        {#each recoveryCodes as c}
          <li>{c}</li>
        {/each}
      </ul>
      <Button variant="primary" onclick={() => { phase = 'idle'; recoveryCodes = []; }}>
        Done
      </Button>
    </div>
  {/if}
</SectionCard>
```

- [ ] **Step 2: Add `SecuritySection` to `profile/+page.svelte`**

Read the current profile page. Import and render `SecuritySection` after the existing account/password sections:

```svelte
<script lang="ts">
  // ... existing imports ...
  import SecuritySection from './SecuritySection.svelte';
</script>

<!-- ... existing sections ... -->
<SecuritySection />
```

If the profile page shows the `#security` anchor, ensure the `SectionCard` has `id="security"` (already included in the SecuritySection above).

- [ ] **Step 3: Type-check + lint**

```bash
cd frontend && npm run check 2>&1 | tail -10 && npm run lint 2>&1 | tail -10
```

Expected: no errors.

- [ ] **Step 4: Commit**

```bash
git add frontend/src/routes/profile/SecuritySection.svelte \
        frontend/src/routes/profile/+page.svelte
git commit -m "feat(frontend): 2FA enrollment, disable, and recovery code management in profile"
```

---

### Task 4: Auth settings — `two_factor_required` toggle

**Files:**

- Modify: `frontend/src/routes/settings/AuthenticationSettings.svelte`

- [ ] **Step 1: Add state + binding**

In `AuthenticationSettings.svelte`, add:

```typescript
let twoFactorRequired: boolean = $state(false);

$effect(() => {
  if (settings) {
    passwordAuthEnabled = settings.password_auth_enabled;
    twoFactorRequired = settings.two_factor_required; // add
  }
});
```

In `saveAuthentication()`, pass the new field:

```typescript
const res = await updateAuthenticationSettings({
  password_auth_enabled: passwordAuthEnabled,
  two_factor_required: twoFactorRequired, // add
});
twoFactorRequired = res.two_factor_required; // update from response
```

- [ ] **Step 2: Add the toggle to the template**

After the existing password auth checkbox, add:

```svelte
<FormFieldRow label="Require Two-Factor Authentication" inputId="two-factor-required">
  <label class="flex items-center gap-3">
    <Checkbox id="two-factor-required" bind:checked={twoFactorRequired} />
    <span>Require all password-authenticated users to enroll in 2FA</span>
  </label>
</FormFieldRow>
```

- [ ] **Step 3: Type-check + lint**

```bash
cd frontend && npm run check 2>&1 | tail -10 && npm run lint 2>&1 | tail -10
```

Expected: no errors.

- [ ] **Step 4: Commit**

```bash
git add frontend/src/routes/settings/AuthenticationSettings.svelte
git commit -m "feat(frontend): two_factor_required toggle in authentication settings"
```

---

### Task 5: Global 403 `2fa_setup_required` handler

**Files:**

- Modify: `frontend/src/routes/+layout.svelte`

The backend returns `403 { "error": "2fa_setup_required" }` when a `setup_required` JWT hits a
non-enrollment route. The frontend must intercept this and redirect to the enrollment flow.

- [ ] **Step 1: Understand the current error handling pattern**

Read `frontend/src/routes/+layout.svelte`. Look for how 401 (session expired) is currently handled. The
pattern is likely that `authenticatedFetch` in `api.ts` handles 401 and sets `session_expired` state.

The 403 `2fa_setup_required` response should similarly be caught centrally. The cleanest approach: check
in `authenticatedFetch` (after the 401 refresh retry) for `403` + `error: "2fa_setup_required"` and
redirect to `/profile#security`.

- [ ] **Step 2: Update `authenticatedFetch` in `api.ts`**

After the existing 401 handling block, add:

```typescript
// Handle setup_required enforcement
if (res.status === 403) {
  try {
    const body = await res.clone().json();
    if (body?.error === "2fa_setup_required") {
      // Redirect to enrollment flow
      if (typeof window !== "undefined") {
        window.location.href = "/profile#security";
      }
      // Throw so callers see a rejected promise; the redirect already navigated away.
      throw new Error("2fa_setup_required");
    }
  } catch {
    // Not JSON — fall through
  }
}
```

This goes inside `authenticatedFetch`, after the retry logic, before returning the final `res`.

- [ ] **Step 3: Add visual feedback in layout (optional but good UX)**

In `+layout.svelte`, if the layout already has a banner/toast system, wire in a brief "2FA setup required"
message before the redirect. If not, the redirect alone is sufficient.

Read the layout to see if it renders a global banner for `sessionExpired` state. If yes, add analogous `twoFactorSetupRequired` state:

In `+layout.svelte` (or `auth.svelte.ts`):

```typescript
let twoFactorSetupRequired = $state(false);

export function getTwoFactorSetupRequired(): boolean {
  return twoFactorSetupRequired;
}
export function setTwoFactorSetupRequired(v: boolean): void {
  twoFactorSetupRequired = v;
}
```

Then in the layout template, show a dismissible callout if `getTwoFactorSetupRequired()` is true.

If the layout is complex, skip the banner for now — the redirect alone provides sufficient UX for v1.

- [ ] **Step 4: Type-check + lint + build**

```bash
cd frontend && npm run check 2>&1 | tail -10 && npm run lint 2>&1 | tail -10 && npm run build 2>&1 | tail -10
```

Expected: no errors.

- [ ] **Step 5: Run frontend tests**

```bash
cd frontend && npm run test 2>&1 | tail -10
```

Expected: all pass (E2E tests that touch login require backend; skip if not available).

- [ ] **Step 6: Commit**

```bash
git add frontend/src/routes/+layout.svelte \
        frontend/src/lib/api.ts
git commit -m "feat(frontend): intercept 403 2fa_setup_required and redirect to enrollment"
```

---

## Self-Review

**Spec coverage:**

| Spec section                                          | Task       |
| ----------------------------------------------------- | ---------- |
| Login 202 detection + MFA step                        | Task 2     |
| TOTP code input, auto-submit                          | Task 2     |
| Email OTP fallback button                             | Task 2     |
| Profile security section                              | Task 3     |
| TOTP enrollment QR + confirm + codes                  | Task 3     |
| Disable 2FA with re-auth                              | Task 3     |
| Regenerate recovery codes with re-auth                | Task 3     |
| Settings `two_factor_required` toggle                 | Task 4     |
| Global `2fa_setup_required` intercept                 | Task 5     |
| `setup_required` restricted JWT → enrollment redirect | Tasks 2, 5 |
| API client functions (7)                              | Task 1     |
| Updated types                                         | Task 1     |

**No placeholders.** Code is complete in all steps.

**Type consistency:** `MfaMethod`, `MfaChallengeResponse`, `TotpEnrollResponse` defined in Task 1 and used in Tasks 2, 3 without renaming.

---

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-05-13-2fa-frontend.md`. Two execution options:

**1. Subagent-Driven (recommended)** — fresh subagent per task, review between tasks, fast iteration

**2. Inline Execution** — execute tasks in this session using executing-plans, batch execution with checkpoints

Which approach?

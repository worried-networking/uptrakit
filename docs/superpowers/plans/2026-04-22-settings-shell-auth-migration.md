# Settings Shell + Auth + Danger-Zone Button Migration (#3c) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Migrate all interactive buttons in the settings area to the Button primitive, introduce isSaving loading states
in Auth/Registration components, and fix DangerZone's bespoke modal buttons.

**Architecture:** Each file migrated in its own task. Button import added where missing. isSaving state added to
AuthenticationSettings and RegistrationSettings. DangerZone inline modal buttons migrated without touching ConfirmDialog.

**Tech Stack:** Svelte 5, Button.svelte, Vitest, Playwright

---

## File Map

| File | Change |
| --- | --- |
| `frontend/src/routes/settings/+page.svelte` | Replace 5× raw `<button class="btn btn-sm preset-filled-primary-500">Retry All</button>` → `<Button variant="primary" size="sm">` |
| `frontend/src/routes/settings/GlobalSettingsTab.svelte` | Replace 7× raw `<button class="btn ...">` → `<Button>` with correct variants and existing loading flags |
| `frontend/src/routes/settings/AuthenticationSettings.svelte` | Add `isSaving` state, wrap save handler, replace raw `<button>` → `<Button loading={isSaving}>` |
| `frontend/src/routes/settings/RegistrationSettings.svelte` | Same isSaving pattern as Auth |
| `frontend/src/routes/settings/DangerZone.svelte` | Replace 3× raw `<button>` → `<Button>` (launcher=danger, cancel=secondary, confirm=danger) |
| `frontend/src/routes/settings/+page.test.ts` (new) | Unit tests for Retry All buttons |
| `frontend/src/routes/settings/GlobalSettingsTab.test.ts` (new) | Unit tests for GlobalSettingsTab buttons |
| `frontend/src/routes/settings/AuthenticationSettings.test.ts` (new) | Unit tests for auth save button + isSaving |
| `frontend/src/routes/settings/RegistrationSettings.test.ts` (new) | Unit tests for registration save button + isSaving |
| `frontend/src/routes/settings/DangerZone.test.ts` (new) | Unit tests for all three DangerZone buttons |

---

## Task 1: Migrate `+page.svelte` Retry All buttons

**Files:**

- Modify: `frontend/src/routes/settings/+page.svelte` (lines 258, 266, 279, 287, 295)
- Create: `frontend/src/routes/settings/+page.test.ts`

### Context

Five identical `<button class="btn btn-sm preset-filled-primary-500" onclick={() => loadAllSettings()}>Retry All</button>`
blocks exist at lines 258, 266, 279, 287, 295. Each is inside a `<Callout>` error block. All become
`<Button variant="primary" size="sm">`.

- [ ] **Step 1.1: Write failing test**

Create `frontend/src/routes/settings/+page.test.ts`:

```ts
import { describe, expect, it, vi } from 'vitest';
import { render, waitFor } from '@testing-library/svelte';

// Heavy mocks — +page.svelte pulls in many modules
vi.mock('$app/state', () => ({ page: { url: { searchParams: { get: () => null } } } }));
vi.mock('$app/navigation', () => ({ goto: vi.fn() }));
vi.mock('$lib/auth.svelte', () => ({ getUser: vi.fn(() => null) }));
vi.mock('$lib/api', () => ({
  getCombinedSettings: vi.fn().mockRejectedValue(new Error('network error')),
  getOidcProviders: vi.fn(() => new Promise(() => {}))
}));
vi.mock('$lib/notifications.svelte', () => ({
  showSuccess: vi.fn(),
  showError: vi.fn()
}));
vi.mock('$lib/surfaces/registry.svelte', () => ({
  getSurfaceReadLoading: vi.fn(() => false),
  getSurfaceReadModel: vi.fn(() => undefined),
  getSurfaceReadRequested: vi.fn(() => false),
  getSurfaceRegistryLoaded: vi.fn(() => true),
  getSurfacesBySlot: vi.fn(() => []),
  loadSurfaceReadModels: vi.fn()
}));
vi.mock('$lib/surfaces/read-model', () => ({
  filterSurfacesByPermission: vi.fn(() => []),
  isSurfaceTabPending: vi.fn(() => false)
}));

import * as auth from '$lib/auth.svelte';
import { Permission } from '$lib/types';
import SettingsPage from './+page.svelte';

function makeUser() {
  return {
    id: 'u1',
    email: 'a@b.com',
    first_name: 'A',
    last_name: 'B',
    permissions: [Permission.ManageAuthSettings]
  };
}

describe('+page.svelte Retry All buttons', () => {
  it('Retry All buttons render as Button variant="primary" size="sm" (h-[19px] class)', async () => {
    vi.mocked(auth.getUser).mockReturnValue(makeUser());

    // getCombinedSettings is mocked to reject — this triggers the *Error states
    // that make the Retry All buttons visible inside each <Callout> block.
    const { container } = render(SettingsPage);

    // Wait for the error Callout to appear (async rejection settles on next tick)
    await waitFor(() => {
      const rawBtns = container.querySelectorAll('button.btn.btn-sm.preset-filled-primary-500');
      // Pre-migration: 5 raw preset buttons exist — this assertion FAILS (rawBtns.length > 0)
      expect(rawBtns.length).toBe(0);
    });
  });
});
```

- [ ] **Step 1.2: Run test — expect it to FAIL because preset-filled-primary-500 buttons still exist**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npx vitest run src/routes/settings/+page.test.ts 2>&1 | tail -20
```

Expected: 1 test FAILS — `getCombinedSettings` rejects, error Callouts render, 5 raw
`preset-filled-primary-500` buttons are present, so `expect(rawBtns.length).toBe(0)` fails.

- [ ] **Step 1.3: Add Button import to `+page.svelte`**

Button is NOT in the `$lib/components/ui` barrel. Add a separate import after the existing
ui import block (around line 25):

```svelte
import Button from '$lib/components/Button.svelte';
```

- [ ] **Step 1.4: Replace 5× Retry All buttons in `+page.svelte`**

Replace each of the five occurrences. They all look identical:

```svelte
<button class="btn btn-sm preset-filled-primary-500" onclick={() => loadAllSettings()}>Retry All</button>
```

Replace every instance with:

```svelte
<Button variant="primary" size="sm" onclick={() => loadAllSettings()}>Retry All</Button>
```

All five are at lines 258, 266, 279, 287, 295 (in original; adjust after edits). Use a single search-replace pass.

- [ ] **Step 1.5: Run type check**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npx svelte-check --tsconfig ./tsconfig.json 2>&1 | grep -E "error|Error"
```

Expected: no errors.

- [ ] **Step 1.6: Run test — expect PASS**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npx vitest run src/routes/settings/+page.test.ts 2>&1 | tail -10
```

Expected: 1 passed.

- [ ] **Step 1.7: Run full suite to confirm no regressions**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npx vitest run 2>&1 | tail -5
```

Expected: 660+ tests pass, 0 failures.

- [ ] **Step 1.8: Commit**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npx prettier --write src/routes/settings/+page.svelte src/routes/settings/+page.test.ts
cd /Users/andreyyantsen/Development/uptrakit && git add frontend/src/routes/settings/+page.svelte frontend/src/routes/settings/+page.test.ts
git commit -m "feat(frontend): migrate settings page Retry All buttons to Button primitive (#3c)"
```

---

## Task 2: Migrate `GlobalSettingsTab.svelte` buttons

**Files:**

- Modify: `frontend/src/routes/settings/GlobalSettingsTab.svelte`
- Create: `frontend/src/routes/settings/GlobalSettingsTab.test.ts`

### Context

Six raw `<button>` elements in this file:

| Line (approx) | Current class | Handler | Loading flag | Target |
| --- | --- | --- | --- | --- |
| 364 | `btn preset-filled-primary-500` | `saveGitHubProviderSettings` | `githubProviderSaving` | `variant="primary"` |
| 405 | `btn preset-filled-primary-500` | `saveNatsUrl` | `natsSaving` | `variant="primary"` |
| 412 | `btn preset-tonal-error` | `clearNatsUrl` | `natsClearing` | `variant="danger"` |
| 467 | `btn preset-filled-primary-500` | `saveZeroconfSettings` | `zeroconfSaving` | `variant="primary"` |
| 518 | `btn preset-filled-primary-500` | `saveNetworkSettings` | none | `variant="primary"` |
| 540 | `btn preset-filled-primary-500` | `handleRenewServerCert` | `renewingCert` | `variant="primary"` |
| 556 | `btn preset-filled-error-500` | opens confirm dialog | `rotatingCa` | `variant="danger"` |

`saveNetworkSettings` has no loading flag — keep as `variant="primary"` without `loading` prop (no `networkSaving`
state exists). The spec only requires adding loading where existing state flags exist.

- [ ] **Step 2.1: Write failing tests**

Create `frontend/src/routes/settings/GlobalSettingsTab.test.ts`:

```ts
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/svelte';

vi.mock('$lib/auth.svelte', () => ({ getUser: vi.fn(() => null) }));
vi.mock('$lib/notifications.svelte', () => ({
  showSuccess: vi.fn(),
  showError: vi.fn(),
  clearError: vi.fn()
}));
vi.mock('$lib/surfaces/registry.svelte', () => ({
  getSurfaceReadModel: vi.fn(() => undefined),
  getSurfacesBySlot: vi.fn(() => []),
  loadSurfaceReadModels: vi.fn()
}));
vi.mock('$lib/surfaces/read-model', () => ({
  filterSurfacesByPermission: vi.fn(() => []),
  shouldUseSurfaceRoute: vi.fn(() => false)
}));
vi.mock('$lib/api', () => ({
  getGitHubProviderSettings: vi.fn(),
  getSystemAlerts: vi.fn(),
  renewServerCertificate: vi.fn(),
  getNetworkSettings: vi.fn(),
  updateNetworkSettings: vi.fn(),
  getNatsSettings: vi.fn(),
  updateNatsSettings: vi.fn(),
  updateGitHubProviderSettings: vi.fn(),
  getZeroconfSettings: vi.fn(),
  updateZeroconfSettings: vi.fn(),
  rotateCA: vi.fn()
}));

import * as api from '$lib/api';
import GlobalSettingsTab from './GlobalSettingsTab.svelte';

function stubAllApis() {
  vi.mocked(api.getNetworkSettings).mockResolvedValue({
    trusted_proxies: [],
    real_ip_header: 'X-Forwarded-For',
    sans: [],
    https_addr: '[::]:8443'
  });
  vi.mocked(api.getSystemAlerts).mockResolvedValue({ alerts: [] });
  vi.mocked(api.getNatsSettings).mockResolvedValue({ url: 'nats://host:4222' });
  vi.mocked(api.getZeroconfSettings).mockResolvedValue({
    enabled: false,
    ca_fingerprint: null,
    url: null,
    pki_addr: null
  });
  vi.mocked(api.getGitHubProviderSettings).mockResolvedValue({
    api_base_url: null,
    auth_token: null,
    has_auth_token: false
  });
}

describe('GlobalSettingsTab button variants', () => {
  beforeEach(() => stubAllApis());
  afterEach(() => vi.clearAllMocks());

  it('GitHub Provider Save button has no raw preset-filled-primary-500 class', async () => {
    const { container } = render(GlobalSettingsTab);
    await screen.findByText('Save GitHub Provider');
    const raw = container.querySelector('button.preset-filled-primary-500');
    expect(raw).toBeNull();
  });

  it('NATS Save button has no raw preset class', async () => {
    const { container } = render(GlobalSettingsTab);
    await waitFor(() => expect(screen.queryAllByText('Save').length).toBeGreaterThan(0));
    const raw = container.querySelector('button.preset-filled-primary-500');
    expect(raw).toBeNull();
  });

  it('NATS Clear button has no raw preset-tonal-error class', async () => {
    const { container } = render(GlobalSettingsTab);
    await screen.findByText('Clear');
    const raw = container.querySelector('button.preset-tonal-error');
    expect(raw).toBeNull();
  });

  it('CA Rotate button has no raw preset-filled-error-500 class', async () => {
    const { container } = render(GlobalSettingsTab);
    await screen.findByText('Rotate CA');
    const raw = container.querySelector('button.preset-filled-error-500');
    expect(raw).toBeNull();
  });

  it('Renew Server Certificate button has no raw preset-filled-primary-500 class', async () => {
    const { container } = render(GlobalSettingsTab);
    await screen.findByText('Renew Server Certificate');
    const raw = container.querySelector('button.preset-filled-primary-500');
    expect(raw).toBeNull();
  });
});
```

- [ ] **Step 2.2: Run test — expect FAIL**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npx vitest run src/routes/settings/GlobalSettingsTab.test.ts 2>&1 | tail -20
```

Expected: 5 tests fail — `raw` is not null because old classes still present.

- [ ] **Step 2.3: Add Button import to `GlobalSettingsTab.svelte`**

Current import at line 21:

```svelte
import { Callout, FormFieldRow, SectionCard } from '$lib/components/ui';
```

Change to:

```svelte
import { Callout, FormFieldRow, SectionCard } from '$lib/components/ui';
import Button from '$lib/components/Button.svelte';
```

- [ ] **Step 2.4: Replace GitHub Provider Save button (line ~364)**

Old:

```svelte
<button
  class="btn preset-filled-primary-500"
  onclick={saveGitHubProviderSettings}
  disabled={githubProviderSaving}
>
  {githubProviderSaving ? 'Saving…' : 'Save GitHub Provider'}
</button>
```

New:

```svelte
<Button variant="primary" loading={githubProviderSaving} onclick={saveGitHubProviderSettings}>
  Save GitHub Provider
</Button>
```

- [ ] **Step 2.5: Replace NATS Save button (line ~405)**

Old:

```svelte
<button
  class="btn preset-filled-primary-500"
  onclick={saveNatsUrl}
  disabled={natsSaving || !natsUrlInput.trim()}
>
  {natsSaving ? 'Saving…' : 'Save'}
</button>
```

New:

```svelte
<Button
  variant="primary"
  loading={natsSaving}
  disabled={!natsUrlInput.trim()}
  onclick={saveNatsUrl}
>
  Save
</Button>
```

Note: `disabled` and `loading` are separate props on Button. When `loading=true` the button is inert automatically
(Button sets `disabled={inert}` where `inert = disabled || loading`).

- [ ] **Step 2.6: Replace NATS Clear button (line ~412)**

Old:

```svelte
<button class="btn preset-tonal-error" onclick={clearNatsUrl} disabled={natsClearing}>
  {natsClearing ? 'Clearing…' : 'Clear'}
</button>
```

New:

```svelte
<Button variant="danger" loading={natsClearing} onclick={clearNatsUrl}>Clear</Button>
```

- [ ] **Step 2.7: Replace Zeroconf Save button (line ~467)**

Old:

```svelte
<button class="btn preset-filled-primary-500" onclick={saveZeroconfSettings} disabled={zeroconfSaving}>
  {zeroconfSaving ? 'Saving...' : 'Save'}
</button>
```

New:

```svelte
<Button variant="primary" loading={zeroconfSaving} onclick={saveZeroconfSettings}>Save</Button>
```

- [ ] **Step 2.8: Replace Network Settings Save button (line ~518)**

Old:

```svelte
<button class="btn preset-filled-primary-500" onclick={saveNetworkSettings}> Save </button>
```

New:

```svelte
<Button variant="primary" onclick={saveNetworkSettings}>Save</Button>
```

- [ ] **Step 2.9: Replace Renew Server Certificate button (line ~540)**

Old:

```svelte
<button class="btn preset-filled-primary-500" onclick={handleRenewServerCert} disabled={renewingCert}>
  {renewingCert ? 'Renewing...' : 'Renew Server Certificate'}
</button>
```

New:

```svelte
<Button variant="primary" loading={renewingCert} onclick={handleRenewServerCert}>
  Renew Server Certificate
</Button>
```

- [ ] **Step 2.10: Replace CA Rotate launcher button (line ~556)**

Old:

```svelte
<button class="btn preset-filled-error-500" onclick={() => (showRotateCaConfirm = true)} disabled={rotatingCa}>
  {rotatingCa ? 'Rotating...' : 'Rotate CA'}
</button>
```

New:

```svelte
<Button variant="danger" loading={rotatingCa} onclick={() => (showRotateCaConfirm = true)}>
  Rotate CA
</Button>
```

Note: `rotatingCa` used here as loading because it means the CA rotation is in progress (confirm was already accepted via ConfirmDialog).

- [ ] **Step 2.11: Run type check**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npx svelte-check --tsconfig ./tsconfig.json 2>&1 | grep -E "error|Error"
```

Expected: no errors.

- [ ] **Step 2.12: Run tests — expect PASS**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npx vitest run src/routes/settings/GlobalSettingsTab.test.ts 2>&1 | tail -10
```

Expected: 5 passed.

- [ ] **Step 2.13: Run full suite**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npx vitest run 2>&1 | tail -5
```

Expected: 660+ tests pass, 0 failures.

- [ ] **Step 2.14: Commit**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npx prettier --write src/routes/settings/GlobalSettingsTab.svelte src/routes/settings/GlobalSettingsTab.test.ts
cd /Users/andreyyantsen/Development/uptrakit && git add frontend/src/routes/settings/GlobalSettingsTab.svelte frontend/src/routes/settings/GlobalSettingsTab.test.ts
git commit -m "feat(frontend): migrate GlobalSettingsTab buttons to Button primitive (#3c)"
```

---

## Task 3: Migrate `AuthenticationSettings.svelte` — add isSaving + Button

**Files:**

- Modify: `frontend/src/routes/settings/AuthenticationSettings.svelte`
- Create: `frontend/src/routes/settings/AuthenticationSettings.test.ts`

### Context

Current file has no `isSaving` state. The save handler is bare `async function saveAuthentication()` with no loading
guard. The button is `<button class="btn preset-filled-primary-500" onclick={saveAuthentication} disabled={!getIsOnline()}>`.

- [ ] **Step 3.1: Write failing tests**

Create `frontend/src/routes/settings/AuthenticationSettings.test.ts`:

```ts
import { afterEach, describe, expect, it, vi } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/svelte';

vi.mock('$lib/api', () => ({ updateAuthenticationSettings: vi.fn() }));
vi.mock('$lib/stores/network.svelte', () => ({ getIsOnline: vi.fn(() => true) }));

import * as api from '$lib/api';
import AuthenticationSettings from './AuthenticationSettings.svelte';

const settingsProps = {
  settings: { password_auth_enabled: true },
  onSuccess: vi.fn(),
  onError: vi.fn()
};

afterEach(() => vi.clearAllMocks());

describe('AuthenticationSettings button', () => {
  it('Save button has no raw preset-filled-primary-500 class', () => {
    const { container } = render(AuthenticationSettings, settingsProps);
    expect(container.querySelector('button.preset-filled-primary-500')).toBeNull();
  });

  it('Save button carries aria-busy=true while save is in flight', async () => {
    let resolve!: () => void;
    vi.mocked(api.updateAuthenticationSettings).mockReturnValue(
      new Promise<{ password_auth_enabled: boolean }>((r) => {
        resolve = () => r({ password_auth_enabled: true });
      })
    );

    render(AuthenticationSettings, settingsProps);
    const btn = screen.getByRole('button', { name: 'Save' });
    await fireEvent.click(btn);

    await waitFor(() => expect(btn).toHaveAttribute('aria-busy', 'true'));

    resolve();
    await waitFor(() => expect(btn).not.toHaveAttribute('aria-busy'));
  });

  it('Save button text is static "Save" during loading — no text swap', async () => {
    let resolve!: () => void;
    vi.mocked(api.updateAuthenticationSettings).mockReturnValue(
      new Promise<{ password_auth_enabled: boolean }>((r) => {
        resolve = () => r({ password_auth_enabled: true });
      })
    );

    render(AuthenticationSettings, settingsProps);
    const btn = screen.getByRole('button', { name: 'Save' });
    await fireEvent.click(btn);

    await waitFor(() => expect(btn).toHaveAttribute('aria-busy', 'true'));
    expect(btn).toHaveTextContent('Save');

    resolve();
  });
});
```

- [ ] **Step 3.2: Run test — expect FAIL**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npx vitest run src/routes/settings/AuthenticationSettings.test.ts 2>&1 | tail -20
```

Expected: 3 failures — raw class found, aria-busy never set (no isSaving state).

- [ ] **Step 3.3: Modify `AuthenticationSettings.svelte`**

Replace the entire `<script lang="ts">` block:

Old:

```svelte
<script lang="ts">
	import { updateAuthenticationSettings } from '$lib/api';
	import type { AuthenticationSettings } from '$lib/types';
	import { getIsOnline } from '$lib/stores/network.svelte';
	import { FormFieldRow, SectionCard } from '$lib/components/ui';

	let {
		settings,
		onSuccess,
		onError
	}: {
		settings: AuthenticationSettings | undefined;
		onSuccess: (msg: string) => void;
		onError: (msg: string) => void;
	} = $props();

	let passwordAuthEnabled: boolean = $state(true);

	$effect(() => {
		if (settings) {
			passwordAuthEnabled = settings.password_auth_enabled;
		}
	});

	async function saveAuthentication() {
		try {
			const res = await updateAuthenticationSettings({
				password_auth_enabled: passwordAuthEnabled
			});
			passwordAuthEnabled = res.password_auth_enabled;
			onSuccess('Authentication settings saved.');
		} catch (e) {
			onError(e instanceof Error ? e.message : 'Failed to save authentication settings');
		}
	}
</script>
```

New:

```svelte
<script lang="ts">
	import { updateAuthenticationSettings } from '$lib/api';
	import type { AuthenticationSettings } from '$lib/types';
	import { getIsOnline } from '$lib/stores/network.svelte';
	import { FormFieldRow, SectionCard } from '$lib/components/ui';
	import Button from '$lib/components/Button.svelte';

	let {
		settings,
		onSuccess,
		onError
	}: {
		settings: AuthenticationSettings | undefined;
		onSuccess: (msg: string) => void;
		onError: (msg: string) => void;
	} = $props();

	let passwordAuthEnabled: boolean = $state(true);
	let isSaving: boolean = $state(false);

	$effect(() => {
		if (settings) {
			passwordAuthEnabled = settings.password_auth_enabled;
		}
	});

	async function saveAuthentication() {
		isSaving = true;
		try {
			const res = await updateAuthenticationSettings({
				password_auth_enabled: passwordAuthEnabled
			});
			passwordAuthEnabled = res.password_auth_enabled;
			onSuccess('Authentication settings saved.');
		} catch (e) {
			onError(e instanceof Error ? e.message : 'Failed to save authentication settings');
		} finally {
			isSaving = false;
		}
	}
</script>
```

- [ ] **Step 3.4: Replace the Save button in the template**

Old:

```svelte
<button class="btn preset-filled-primary-500" onclick={saveAuthentication} disabled={!getIsOnline()}>
  Save
</button>
```

New:

```svelte
<Button
  variant="primary"
  loading={isSaving}
  disabled={!getIsOnline()}
  onclick={saveAuthentication}
>Save</Button>
```

- [ ] **Step 3.5: Run type check**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npx svelte-check --tsconfig ./tsconfig.json 2>&1 | grep -E "error|Error"
```

Expected: no errors.

- [ ] **Step 3.6: Run tests — expect PASS**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npx vitest run src/routes/settings/AuthenticationSettings.test.ts 2>&1 | tail -10
```

Expected: 3 passed.

- [ ] **Step 3.7: Run full suite**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npx vitest run 2>&1 | tail -5
```

Expected: 660+ passed, 0 failures.

- [ ] **Step 3.8: Commit**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npx prettier --write src/routes/settings/AuthenticationSettings.svelte src/routes/settings/AuthenticationSettings.test.ts
cd /Users/andreyyantsen/Development/uptrakit && git add frontend/src/routes/settings/AuthenticationSettings.svelte frontend/src/routes/settings/AuthenticationSettings.test.ts
git commit -m "feat(frontend): migrate AuthenticationSettings to Button primitive with isSaving (#3c)"
```

---

## Task 4: Migrate `RegistrationSettings.svelte` — add isSaving + Button

**Files:**

- Modify: `frontend/src/routes/settings/RegistrationSettings.svelte`
- Create: `frontend/src/routes/settings/RegistrationSettings.test.ts`

### Context

Same pattern as Task 3. Current file has no `isSaving`. One save button:
`<button class="btn preset-filled-primary-500" onclick={saveRegistration} disabled={!getIsOnline()}>Save</button>`.
No "Generate new token" button exists in this file (token is set via input field, not an action button).

- [ ] **Step 4.1: Write failing tests**

Create `frontend/src/routes/settings/RegistrationSettings.test.ts`:

```ts
import { afterEach, describe, expect, it, vi } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/svelte';

vi.mock('$lib/api', () => ({ updateRegistrationSettings: vi.fn() }));
vi.mock('$lib/stores/network.svelte', () => ({ getIsOnline: vi.fn(() => true) }));

import * as api from '$lib/api';
import RegistrationSettings from './RegistrationSettings.svelte';

const settingsProps = {
  settings: { mode: 'open' as const, require_token_for_oidc: false },
  onSuccess: vi.fn(),
  onError: vi.fn()
};

afterEach(() => vi.clearAllMocks());

describe('RegistrationSettings button', () => {
  it('Save button has no raw preset-filled-primary-500 class', () => {
    const { container } = render(RegistrationSettings, settingsProps);
    expect(container.querySelector('button.preset-filled-primary-500')).toBeNull();
  });

  it('Save button carries aria-busy=true while save is in flight', async () => {
    let resolve!: () => void;
    vi.mocked(api.updateRegistrationSettings).mockReturnValue(
      new Promise<{ mode: 'open'; require_token_for_oidc: boolean }>((r) => {
        resolve = () => r({ mode: 'open', require_token_for_oidc: false });
      })
    );

    render(RegistrationSettings, settingsProps);
    const btn = screen.getByRole('button', { name: 'Save' });
    await fireEvent.click(btn);

    await waitFor(() => expect(btn).toHaveAttribute('aria-busy', 'true'));

    resolve();
    await waitFor(() => expect(btn).not.toHaveAttribute('aria-busy'));
  });

  it('Save button text is static "Save" during loading — no text swap', async () => {
    let resolve!: () => void;
    vi.mocked(api.updateRegistrationSettings).mockReturnValue(
      new Promise<{ mode: 'open'; require_token_for_oidc: boolean }>((r) => {
        resolve = () => r({ mode: 'open', require_token_for_oidc: false });
      })
    );

    render(RegistrationSettings, settingsProps);
    const btn = screen.getByRole('button', { name: 'Save' });
    await fireEvent.click(btn);

    await waitFor(() => expect(btn).toHaveAttribute('aria-busy', 'true'));
    expect(btn).toHaveTextContent('Save');

    resolve();
  });
});
```

- [ ] **Step 4.2: Run test — expect FAIL**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npx vitest run src/routes/settings/RegistrationSettings.test.ts 2>&1 | tail -20
```

Expected: 3 failures.

- [ ] **Step 4.3: Modify `RegistrationSettings.svelte` script block**

Old:

```svelte
<script lang="ts">
	import { updateRegistrationSettings } from '$lib/api';
	import type { RegistrationSettings } from '$lib/types';
	import { getIsOnline } from '$lib/stores/network.svelte';
	import { FormFieldRow, SectionCard } from '$lib/components/ui';

	let {
		settings,
		onSuccess,
		onError
	}: {
		settings: RegistrationSettings | undefined;
		onSuccess: (msg: string) => void;
		onError: (msg: string) => void;
	} = $props();

	let regMode: 'open' | 'invite' | 'closed' = $state('open');
	let regToken: string = $state('');
	let regRequireTokenForOidc: boolean = $state(false);

	$effect(() => {
		if (settings) {
			regMode = settings.mode;
			regRequireTokenForOidc = settings.require_token_for_oidc;
		}
	});

	async function saveRegistration() {
		try {
			const data: { mode: 'open' | 'invite' | 'closed'; token?: string; require_token_for_oidc?: boolean } = {
				mode: regMode
			};
			if (regMode === 'invite' && regToken) {
				data.token = regToken;
			}
			if (regMode === 'invite') {
				data.require_token_for_oidc = regRequireTokenForOidc;
			}
			const res = await updateRegistrationSettings(data);
			regMode = res.mode;
			regRequireTokenForOidc = res.require_token_for_oidc;
			regToken = '';
			onSuccess('Registration settings saved.');
		} catch (e) {
			onError(e instanceof Error ? e.message : 'Failed to save registration settings');
		}
	}
</script>
```

New (add import + isSaving + try/finally):

```svelte
<script lang="ts">
	import { updateRegistrationSettings } from '$lib/api';
	import type { RegistrationSettings } from '$lib/types';
	import { getIsOnline } from '$lib/stores/network.svelte';
	import { FormFieldRow, SectionCard } from '$lib/components/ui';
	import Button from '$lib/components/Button.svelte';

	let {
		settings,
		onSuccess,
		onError
	}: {
		settings: RegistrationSettings | undefined;
		onSuccess: (msg: string) => void;
		onError: (msg: string) => void;
	} = $props();

	let regMode: 'open' | 'invite' | 'closed' = $state('open');
	let regToken: string = $state('');
	let regRequireTokenForOidc: boolean = $state(false);
	let isSaving: boolean = $state(false);

	$effect(() => {
		if (settings) {
			regMode = settings.mode;
			regRequireTokenForOidc = settings.require_token_for_oidc;
		}
	});

	async function saveRegistration() {
		isSaving = true;
		try {
			const data: { mode: 'open' | 'invite' | 'closed'; token?: string; require_token_for_oidc?: boolean } = {
				mode: regMode
			};
			if (regMode === 'invite' && regToken) {
				data.token = regToken;
			}
			if (regMode === 'invite') {
				data.require_token_for_oidc = regRequireTokenForOidc;
			}
			const res = await updateRegistrationSettings(data);
			regMode = res.mode;
			regRequireTokenForOidc = res.require_token_for_oidc;
			regToken = '';
			onSuccess('Registration settings saved.');
		} catch (e) {
			onError(e instanceof Error ? e.message : 'Failed to save registration settings');
		} finally {
			isSaving = false;
		}
	}
</script>
```

- [ ] **Step 4.4: Replace Save button in the template**

Old:

```svelte
<button class="btn preset-filled-primary-500" onclick={saveRegistration} disabled={!getIsOnline()}>
  Save
</button>
```

New:

```svelte
<Button
  variant="primary"
  loading={isSaving}
  disabled={!getIsOnline()}
  onclick={saveRegistration}
>Save</Button>
```

- [ ] **Step 4.5: Run type check**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npx svelte-check --tsconfig ./tsconfig.json 2>&1 | grep -E "error|Error"
```

Expected: no errors.

- [ ] **Step 4.6: Run tests — expect PASS**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npx vitest run src/routes/settings/RegistrationSettings.test.ts 2>&1 | tail -10
```

Expected: 3 passed.

- [ ] **Step 4.7: Run full suite**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npx vitest run 2>&1 | tail -5
```

Expected: 660+ passed, 0 failures.

- [ ] **Step 4.8: Commit**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npx prettier --write src/routes/settings/RegistrationSettings.svelte src/routes/settings/RegistrationSettings.test.ts
cd /Users/andreyyantsen/Development/uptrakit && git add frontend/src/routes/settings/RegistrationSettings.svelte frontend/src/routes/settings/RegistrationSettings.test.ts
git commit -m "feat(frontend): migrate RegistrationSettings to Button primitive with isSaving (#3c)"
```

---

## Task 5: Migrate `DangerZone.svelte` buttons

**Files:**

- Modify: `frontend/src/routes/settings/DangerZone.svelte`
- Create: `frontend/src/routes/settings/DangerZone.test.ts`

### Context

Three buttons in this file:

1. **Launcher** (line 66):
   `<button class="btn preset-filled-error-500" onclick={openDialog} disabled={!getIsOnline()}>Reset Data</button>`
   → `variant="danger"`
2. **Cancel** (line 124 inside `{#snippet footer()}`):
   `<button class="btn preset-tonal-surface" onclick={closeDialog} disabled={submitting}>Cancel</button>`
   → `variant="secondary"`
3. **Reset All Data confirm** (line 125 inside `{#snippet footer()}`):
   `<button class="btn preset-filled-error-500" disabled={!isConfirmed || submitting} onclick={handleReset}>...</button>`
   → `variant="danger"` with `loading={submitting}`, static text "Reset All Data"

There is also a fourth button at line 122
(`<button class="btn preset-tonal-surface" onclick={closeDialog}>Close</button>`)
rendered in the `result` branch — also → `variant="secondary"`.

- [ ] **Step 5.1: Write failing tests**

Create `frontend/src/routes/settings/DangerZone.test.ts`:

```ts
import { afterEach, describe, expect, it, vi } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/svelte';

vi.mock('$lib/api', () => ({ resetData: vi.fn() }));
vi.mock('$lib/stores/network.svelte', () => ({ getIsOnline: vi.fn(() => true) }));

import * as api from '$lib/api';
import DangerZone from './DangerZone.svelte';

const props = { onSuccess: vi.fn(), onError: vi.fn() };

afterEach(() => vi.clearAllMocks());

describe('DangerZone button variants', () => {
  it('launcher Reset Data button has no raw preset-filled-error-500 class', () => {
    const { container } = render(DangerZone, props);
    expect(container.querySelector('button.preset-filled-error-500')).toBeNull();
  });

  it('inline Cancel button inside modal has no raw preset-tonal-surface class', async () => {
    const { container } = render(DangerZone, props);
    await fireEvent.click(screen.getByRole('button', { name: 'Reset Data' }));
    await screen.findByText('Reset All Data');
    expect(container.querySelector('button.preset-tonal-surface')).toBeNull();
  });

  it('inline Reset All Data button inside modal has no raw preset-filled-error-500 class', async () => {
    const { container } = render(DangerZone, props);
    await fireEvent.click(screen.getByRole('button', { name: 'Reset Data' }));
    await screen.findByText('Reset All Data');
    expect(container.querySelector('button.preset-filled-error-500')).toBeNull();
  });

  it('inline Reset All Data button carries aria-busy=true while submitting', async () => {
    let resolve!: () => void;
    vi.mocked(api.resetData).mockReturnValue(
      new Promise<{ deleted: Record<string, number> }>((r) => {
        resolve = () =>
          r({
            deleted: {
              hosts: 1,
              software_items: 0,
              plugin_configs: 0,
              host_tags: 0,
              update_history: 0,
              update_batches: 0
            }
          });
      })
    );

    render(DangerZone, props);
    await fireEvent.click(screen.getByRole('button', { name: 'Reset Data' }));
    await screen.findByText('Reset All Data');

    const confirmInput = screen.getByPlaceholderText('Type RESET to confirm');
    await fireEvent.input(confirmInput, { target: { value: 'RESET' } });

    const confirmBtn = screen.getByRole('button', { name: 'Reset All Data' });
    await fireEvent.click(confirmBtn);

    await waitFor(() => expect(confirmBtn).toHaveAttribute('aria-busy', 'true'));

    resolve();
    await waitFor(() => expect(confirmBtn).not.toHaveAttribute('aria-busy'));
  });

  it('inline Reset All Data button text is static "Reset All Data" during loading', async () => {
    let resolve!: () => void;
    vi.mocked(api.resetData).mockReturnValue(
      new Promise<{ deleted: Record<string, number> }>((r) => {
        resolve = () =>
          r({
            deleted: {
              hosts: 0,
              software_items: 0,
              plugin_configs: 0,
              host_tags: 0,
              update_history: 0,
              update_batches: 0
            }
          });
      })
    );

    render(DangerZone, props);
    await fireEvent.click(screen.getByRole('button', { name: 'Reset Data' }));
    await screen.findByText('Reset All Data');

    const confirmInput = screen.getByPlaceholderText('Type RESET to confirm');
    await fireEvent.input(confirmInput, { target: { value: 'RESET' } });

    const confirmBtn = screen.getByRole('button', { name: 'Reset All Data' });
    await fireEvent.click(confirmBtn);

    await waitFor(() => expect(confirmBtn).toHaveAttribute('aria-busy', 'true'));
    expect(confirmBtn).toHaveTextContent('Reset All Data');

    resolve();
  });
});
```

- [ ] **Step 5.2: Run test — expect FAIL**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npx vitest run src/routes/settings/DangerZone.test.ts 2>&1 | tail -20
```

Expected: 5 failures — raw preset classes found, aria-busy never set.

- [ ] **Step 5.3: Add Button import to `DangerZone.svelte`**

Old script imports:

```svelte
import Modal from '$lib/components/Modal.svelte';
import { SectionCard } from '$lib/components/ui';
```

New:

```svelte
import Modal from '$lib/components/Modal.svelte';
import { SectionCard } from '$lib/components/ui';
import Button from '$lib/components/Button.svelte';
```

- [ ] **Step 5.4: Replace launcher button (line ~66)**

Old:

```svelte
<button class="btn preset-filled-error-500" onclick={openDialog} disabled={!getIsOnline()}> Reset Data </button>
```

New:

```svelte
<Button variant="danger" disabled={!getIsOnline()} onclick={openDialog}>Reset Data</Button>
```

- [ ] **Step 5.5: Replace Cancel button in result branch (line ~122)**

Old (inside `{#snippet footer()}` when `result` is truthy):

```svelte
<button class="btn preset-tonal-surface" onclick={closeDialog}>Close</button>
```

New:

```svelte
<Button variant="secondary" onclick={closeDialog}>Close</Button>
```

- [ ] **Step 5.6: Replace Cancel button in confirm branch (line ~124)**

Old:

```svelte
<button class="btn preset-tonal-surface" onclick={closeDialog} disabled={submitting}> Cancel </button>
```

New:

```svelte
<Button variant="secondary" disabled={submitting} onclick={closeDialog}>Cancel</Button>
```

- [ ] **Step 5.7: Replace Reset All Data confirm button (lines ~125–130)**

Old:

```svelte
<button class="btn preset-filled-error-500" disabled={!isConfirmed || submitting} onclick={handleReset}>
  {#if submitting}
    Resetting...
  {:else}
    Reset All Data
  {/if}
</button>
```

New:

```svelte
<Button
  variant="danger"
  loading={submitting}
  disabled={!isConfirmed}
  onclick={handleReset}
>Reset All Data</Button>
```

The `submitting` check on `disabled` is removed because `loading={submitting}` on Button already makes it inert
(Button's `inert = disabled || loading`). Only `!isConfirmed` remains as the explicit disabled gate.

- [ ] **Step 5.8: Run type check**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npx svelte-check --tsconfig ./tsconfig.json 2>&1 | grep -E "error|Error"
```

Expected: no errors.

- [ ] **Step 5.9: Run tests — expect PASS**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npx vitest run src/routes/settings/DangerZone.test.ts 2>&1 | tail -10
```

Expected: 5 passed.

- [ ] **Step 5.10: Run full suite**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npx vitest run 2>&1 | tail -5
```

Expected: 660+ passed, 0 failures.

- [ ] **Step 5.11: Commit**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npx prettier --write src/routes/settings/DangerZone.svelte src/routes/settings/DangerZone.test.ts
cd /Users/andreyyantsen/Development/uptrakit && git add frontend/src/routes/settings/DangerZone.svelte frontend/src/routes/settings/DangerZone.test.ts
git commit -m "feat(frontend): migrate DangerZone buttons to Button primitive (#3c)"
```

---

## Task 6: Full frontend gate + Playwright re-baseline

**Files:**

- Modify: Playwright snapshot baselines under `frontend/tests/e2e/` (re-baseline only)

- [ ] **Step 6.1: Run full lint + format check**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npx eslint src 2>&1 | tail -10
```

Expected: no errors.

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npx prettier --check src 2>&1 | tail -5
```

Expected: "All matched files use Prettier formatting" (or 0 files changed).

- [ ] **Step 6.2: Run svelte-check full pass**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npx svelte-check --tsconfig ./tsconfig.json 2>&1 | grep -E "error|Error"
```

Expected: no output (no errors).

- [ ] **Step 6.3: Run full vitest suite**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npx vitest run 2>&1 | tail -5
```

Expected: 660+ passed, 0 failures.

- [ ] **Step 6.4: Run frontend build to confirm no type/compile errors**

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npm run build 2>&1 | tail -10
```

Expected: build succeeds with no errors.

- [ ] **Step 6.5: Re-baseline Playwright snapshots**

If Playwright visual snapshots exist for settings:

```bash
cd /Users/andreyyantsen/Development/uptrakit/frontend && npx playwright test --update-snapshots tests/e2e/settings 2>&1 | tail -20
```

If no settings-specific e2e file exists, check:

```bash
ls /Users/andreyyantsen/Development/uptrakit/frontend/tests/e2e/
```

Re-baseline whichever file covers `/settings`. If no e2e coverage exists for settings yet, skip this step (no baselines to update).

- [ ] **Step 6.6: Commit gate confirmation**

```bash
cd /Users/andreyyantsen/Development/uptrakit && git add frontend/tests/e2e/
git commit -m "test(frontend): re-baseline Playwright snapshots for settings button migration (#3c)"
```

If no snapshot changes, skip this commit.

---

## Self-Review Checklist

**Spec coverage:**

| Spec requirement | Task |
| --- | --- |
| `+page.svelte` 5× Retry All → `variant="primary" size="sm"` | Task 1 |
| GlobalSettingsTab: GitHub Save → primary | Task 2, Step 2.4 |
| GlobalSettingsTab: NATS Save → primary | Task 2, Step 2.5 |
| GlobalSettingsTab: NATS Clear → danger (`preset-tonal-error` rule) | Task 2, Step 2.6 |
| GlobalSettingsTab: Zeroconf Save → primary | Task 2, Step 2.7 |
| GlobalSettingsTab: Network Save → primary | Task 2, Step 2.8 |
| GlobalSettingsTab: Renew Server Cert → primary | Task 2, Step 2.9 |
| GlobalSettingsTab: CA Rotate launcher → danger | Task 2, Step 2.10 |
| AuthenticationSettings: isSaving introduced | Task 3, Step 3.3 |
| AuthenticationSettings: Save → primary + loading | Task 3, Step 3.4 |
| RegistrationSettings: isSaving introduced | Task 4, Step 4.3 |
| RegistrationSettings: Save → primary + loading | Task 4, Step 4.4 |
| DangerZone: launcher → danger | Task 5, Step 5.4 |
| DangerZone: Cancel → secondary | Task 5, Steps 5.5–5.6 |
| DangerZone: Reset All Data confirm → danger + loading | Task 5, Step 5.7 |
| Non-button Skeleton classes on badge/aside — OUT OF SCOPE | Not touched |
| TabStrip — OUT OF SCOPE | Not touched |
| OIDC buttons — OUT OF SCOPE | Not touched |
| Full frontend gate | Task 6 |

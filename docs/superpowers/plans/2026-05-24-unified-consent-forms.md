# Unified Auth Consent Forms Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task.
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Unify the OAuth consent and device auth pages around a shared `ConsentPrompt` component,
switch the consent page to `PublicEntryShell`, and remove the typed redirect URI confirmation from
both the frontend and backend.

**Architecture:** A new `ConsentPrompt.svelte` component owns client identity display, trust
signals, and Approve/Deny buttons. Both pages pass page-specific content (scopes list, warnings)
as a `children` snippet. The backend typed-confirmation gate in `consent.rs` is deleted; the
`ConsentDecision` request type is simplified to match.

**Tech Stack:** Svelte 5 (runes), SvelteKit, TypeScript strict, Tailwind CSS semantic tokens,
Rust/Axum, Playwright (e2e), cargo test (integration)

---

## File Map

| File | Action |
| ---- | ------ |
| `crates/shared/web-api-types/src/oauth/requests.rs` | Remove `typed_confirmation` field from `ConsentDecision` |
| `crates/ui/web-api/src/routes/oauth/consent.rs` | Delete typed-confirmation gate; delete one test |
| `frontend/src/lib/api/oauth.ts` | Remove `typedConfirmation` param from `approveConsent` |
| `frontend/src/lib/components/ConsentPrompt.svelte` | **Create** shared component |
| `frontend/src/routes/oauth/consent/[request_id]/+page.svelte` | Full rewrite — `PublicEntryShell` + `ConsentPrompt` |
| `frontend/src/routes/device/+page.svelte` | Replace approval Callout + buttons with `ConsentPrompt` |
| `frontend/tests/e2e/oauth-consent.spec.ts` | Remove typed-confirmation tests; update selectors; add unverified test |
| `frontend/tests/e2e/public-entry.test.ts` | Update device approval assertions |
| `docs/development/ui/primitives.md` | Add `ConsentPrompt` section after `ConfirmDialog` |

---

## Task 1: Remove `typed_confirmation` from backend

**Files:**

- Modify: `crates/shared/web-api-types/src/oauth/requests.rs:135-148`
- Modify: `crates/ui/web-api/src/routes/oauth/consent.rs:273-286` (gate) and `:814-842` (test)

- [ ] **Step 1: Remove `typed_confirmation` field from `ConsentDecision`**

  Open `crates/shared/web-api-types/src/oauth/requests.rs`. Replace the struct:

  ```rust
  // BEFORE
  #[non_exhaustive]
  #[derive(Clone, Debug, Serialize, Deserialize)]
  #[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
  pub struct ConsentDecision {
      /// Hostname the user typed for unverified-client confirmation. Required
      /// when the client's `trusted_at` is `NULL`; checked outside this struct.
      pub typed_confirmation: Option<String>,
  }

  impl Validate for ConsentDecision {
      fn validate(&self) -> Result<(), ValidationError> {
          Ok(())
      }
  }
  ```

  ```rust
  // AFTER
  #[non_exhaustive]
  #[derive(Clone, Debug, Serialize, Deserialize)]
  #[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
  pub struct ConsentDecision {}

  impl Default for ConsentDecision {
      fn default() -> Self {
          Self {}
      }
  }

  impl Validate for ConsentDecision {
      fn validate(&self) -> Result<(), ValidationError> {
          Ok(())
      }
  }
  ```

- [ ] **Step 2: Delete the typed-confirmation gate in `consent.rs`**

  Open `crates/ui/web-api/src/routes/oauth/consent.rs`. Remove lines 273–286 entirely:

  ```rust
  // DELETE — entire block, including the trailing blank line:
  if client.trusted_at.is_none() {
      let expected_confirmation = loopback_or_host(&row.redirect_uri);
      let provided = body
          .typed_confirmation
          .as_deref()
          .unwrap_or("")
          .to_lowercase();
      if provided != expected_confirmation {
          return oauth_400(
              "unverified_typed_confirmation_mismatch",
              "typed confirmation does not match redirect URI hostname",
          );
      }
  }
  ```

  The next line after the block is `let consent_svc = OAuthConsentService::new(...)` — leave that
  and everything below untouched.

- [ ] **Step 3: Delete the unit test in `requests.rs` that constructs the old struct**

  In `crates/shared/web-api-types/src/oauth/requests.rs`, delete the `consent_decision_validates`
  test (currently lines 258–264) — it constructs `ConsentDecision { typed_confirmation: ... }`
  which no longer compiles:

  ```rust
  // DELETE — entire fn:
  #[test]
  fn consent_decision_validates() {
      let c = ConsentDecision {
          typed_confirmation: Some("x".into()),
      };
      assert!(c.validate().is_ok());
  }
  ```

- [ ] **Step 4: Delete the now-invalid approval test in `consent.rs`**

  In `crates/ui/web-api/src/routes/oauth/consent.rs`, delete the entire test function
  `consent_approve_unverified_wrong_typed_confirmation` (currently lines ~814–842):

  ```rust
  // DELETE — entire fn including the attribute lines and closing brace:
  #[tokio::test]
  async fn consent_approve_unverified_wrong_typed_confirmation() {
      let app = setup().await;
      let user_id = insert_test_user(&app.db).await;
      // Unverified client (trusted_at = NULL).
      let client_id = insert_oauth_client(&app.db, TEST_REDIRECT_URI, false).await;
      let request_id = insert_auth_request(&app.db, &client_id, user_id, TEST_REDIRECT_URI).await;

      let jwt_token = app
          .jwt
          .create_access_token(user_id, &[], "password", None, None)
          .expect("create_access_token");

      let body = serde_json::json!({ "typed_confirmation": "wrong-host.com" });
      let req = Request::builder()
          .method("POST")
          .uri(format!("/oauth/consent/{request_id}/approve"))
          .header("authorization", format!("Bearer {jwt_token}"))
          .header("content-type", "application/json")
          .body(Body::from(body.to_string()))
          .expect("build request");

      let resp = app.router.oneshot(req).await.expect("oneshot");

      assert_eq!(resp.status(), http::StatusCode::BAD_REQUEST);
      let body_bytes = resp.into_body().collect().await.expect("body").to_bytes();
      let resp_body: serde_json::Value = serde_json::from_slice(&body_bytes).expect("json body");
      assert_eq!(resp_body["error"], "unverified_typed_confirmation_mismatch");
  }
  ```

- [ ] **Step 5: Run backend tests to confirm clean**

  ```bash
  cargo test -p uptrakit-web-api-types -p uptrakit-web-api --all-features 2>&1 | tail -20
  ```

  Expected: all tests pass, no mention of `typed_confirmation_mismatch`.

- [ ] **Step 6: Run clippy on the changed crates**

  ```bash
  cargo clippy -p uptrakit-web-api-types -p uptrakit-web-api --all-features -- -D warnings
  ```

  Expected: no warnings.

- [ ] **Step 7: Commit**

  ```bash
  git add crates/shared/web-api-types/src/oauth/requests.rs \
          crates/ui/web-api/src/routes/oauth/consent.rs
  git commit -m "fix(oauth): remove typed redirect-uri confirmation gate

  The backend gate matched a user-typed hostname against the pre-validated
  redirect URI registered for the client. This provides no meaningful security
  (a user willing to type the hostname would approve regardless), while adding
  friction for legitimate unverified clients. A danger Callout in the UI is
  the sole remaining signal.

  Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
  ```

---

## Task 2: Create `ConsentPrompt` component

**Files:**

- Create: `frontend/src/lib/components/ConsentPrompt.svelte`

- [ ] **Step 1: Write the component**

  Create `frontend/src/lib/components/ConsentPrompt.svelte`:

  ```svelte
  <script lang="ts" module>
  	import type { Snippet } from 'svelte';

  	export type ConsentPromptTrust = 'verified' | 'unverified' | 'dcr' | 'open-metadata' | 'manual';
  </script>

  <script lang="ts">
  	import { ExternalLink } from 'lucide-svelte';
  	import { Callout } from '$lib/components/ui';
  	import Button from '$lib/components/Button.svelte';

  	let {
  		clientName,
  		clientUri,
  		trust,
  		approveDisabled = false,
  		approving,
  		onApprove,
  		onDeny,
  		children
  	}: {
  		clientName: string;
  		clientUri?: string | null;
  		trust: ConsentPromptTrust;
  		approveDisabled?: boolean;
  		approving: boolean;
  		onApprove: () => void;
  		onDeny: () => void;
  		children?: Snippet;
  	} = $props();
  </script>

  <div class="space-y-4" data-ui="consent-prompt">
  	<div>
  		<p class="text-page-title font-bold text-[var(--text-primary)]">{clientName}</p>
  		{#if clientUri}
  			<a
  				href={clientUri}
  				target="_blank"
  				rel="noopener noreferrer"
  				class="inline-flex items-center gap-1 text-sm text-[var(--text-secondary)]"
  			>
  				{clientUri}
  				<ExternalLink size={14} aria-hidden="true" />
  			</a>
  		{/if}
  	</div>

  	{#if trust === 'unverified'}
  		<Callout
  			tone="danger"
  			message="This client has not been verified. Proceed only if you trust it."
  		/>
  	{:else if trust === 'dcr'}
  		<Callout
  			tone="warning"
  			message="This client was recently registered and has not been reviewed."
  		/>
  	{/if}

  	{@render children?.()}

  	<div class="flex justify-end gap-2">
  		<Button variant="secondary" disabled={approving} onclick={onDeny}>Deny</Button>
  		<Button
  			variant="primary"
  			loading={approving}
  			disabled={approveDisabled || approving}
  			onclick={onApprove}
  		>
  			Approve
  		</Button>
  	</div>
  </div>
  ```

- [ ] **Step 2: Run type check**

  ```bash
  cd frontend && npm run check 2>&1 | tail -20
  ```

  Expected: 0 errors.

- [ ] **Step 3: Commit**

  ```bash
  git add frontend/src/lib/components/ConsentPrompt.svelte
  git commit -m "feat(ui): add ConsentPrompt shared component

  Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
  ```

---

## Task 3: Redesign OAuth consent page

**Files:**

- Modify: `frontend/src/lib/api/oauth.ts:66-74`
- Modify: `frontend/src/routes/oauth/consent/[request_id]/+page.svelte` (full rewrite)
- Modify: `frontend/tests/e2e/oauth-consent.spec.ts`

- [ ] **Step 1: Simplify `approveConsent` in `oauth.ts`**

  Open `frontend/src/lib/api/oauth.ts`. Replace the `approveConsent` function:

  ```ts
  // BEFORE
  export async function approveConsent(
  	requestId: string,
  	typedConfirmation: string | null
  ): Promise<{ redirect_to: string }> {
  	return oauthRequest(`/oauth/consent/${encodeURIComponent(requestId)}/approve`, {
  		method: 'POST',
  		body: JSON.stringify({ typed_confirmation: typedConfirmation })
  	});
  }
  ```

  ```ts
  // AFTER
  export async function approveConsent(requestId: string): Promise<{ redirect_to: string }> {
  	return oauthRequest(`/oauth/consent/${encodeURIComponent(requestId)}/approve`, {
  		method: 'POST',
  		body: JSON.stringify({})
  	});
  }
  ```

- [ ] **Step 2: Rewrite the consent page**

  Replace the entire contents of
  `frontend/src/routes/oauth/consent/[request_id]/+page.svelte`:

  ```svelte
  <script lang="ts">
  	import { onMount } from 'svelte';
  	import { page } from '$app/state';
  	import { AlertTriangle, CheckCircle } from 'lucide-svelte';
  	import { Callout } from '$lib/components/ui';
  	import PublicEntryShell from '$lib/components/ui/PublicEntryShell.svelte';
  	import ConsentPrompt from '$lib/components/ConsentPrompt.svelte';
  	import type { ConsentPromptTrust } from '$lib/components/ConsentPrompt.svelte';
  	import Link from '$lib/components/Link.svelte';
  	import { getConsentDetails, approveConsent, denyConsent, type ConsentDetails } from '$lib/api/oauth';
  	import { getUser } from '$lib/auth.svelte';

  	let details = $state<ConsentDetails | null>(null);
  	let loadError = $state<string | null>(null);
  	let submitting = $state(false);

  	const requestId = $derived(page.params.request_id ?? '');
  	const pageTitle = $derived(details ? `${details.client_name} wants access` : 'Authorize Access');

  	const LOCAL_REDIRECT_HOSTS = ['localhost', '127.0.0.1', '[::1]'];
  	const isLocalRedirect = $derived(
  		details ? LOCAL_REDIRECT_HOSTS.includes(details.redirect_uri_host) : false
  	);

  	onMount(async () => {
  		try {
  			details = await getConsentDetails(requestId);
  		} catch (e) {
  			loadError = e instanceof Error ? e.message : String(e);
  		}
  	});

  	function clientTrust(d: ConsentDetails): ConsentPromptTrust {
  		if (d.trusted_at === null) return 'unverified';
  		if (d.created_via === 'dcr') return 'dcr';
  		if (d.created_via === 'cimd_cache') return 'open-metadata';
  		return 'manual';
  	}

  	function scopeDescription(scope: string): string {
  		if (scope === 'mcp:read') return 'Read your uptrakit data (update history, host info, account profile)';
  		if (scope === 'mcp:write') return 'Trigger software updates on your behalf';
  		return scope;
  	}

  	async function handleAllow() {
  		if (!details) return;
  		submitting = true;
  		try {
  			const resp = await approveConsent(requestId);
  			window.location.href = resp.redirect_to;
  		} catch (e) {
  			submitting = false;
  			loadError = e instanceof Error ? e.message : String(e);
  		}
  	}

  	async function handleDeny() {
  		if (!details) return;
  		submitting = true;
  		try {
  			const resp = await denyConsent(requestId);
  			window.location.href = resp.redirect_to;
  		} catch (e) {
  			submitting = false;
  			loadError = e instanceof Error ? e.message : String(e);
  		}
  	}
  </script>

  <PublicEntryShell eyebrow="Authorize Access" title={pageTitle}>
  	{#if loadError}
  		<Callout tone="danger" message={loadError} />
  	{:else if details !== null}
  		<ConsentPrompt
  			clientName={details.client_name}
  			clientUri={details.client_uri}
  			trust={clientTrust(details)}
  			approveDisabled={submitting}
  			approving={submitting}
  			onApprove={handleAllow}
  			onDeny={handleDeny}
  		>
  			{#if details.metadata_change_diff}
  				<Callout
  					tone="warning"
  					message="This client's published metadata has changed since you last authorized it. Review before continuing."
  				/>
  			{/if}

  			{#if isLocalRedirect}
  				<Callout tone="warning">
  					<div class="flex items-center gap-2">
  						<AlertTriangle size={16} aria-hidden="true" />
  						<span>This client will receive credentials at a local address. Make sure it is running on this machine.</span>
  					</div>
  				</Callout>
  			{/if}

  			<ul class="space-y-1">
  				{#each details.scopes as scope (scope)}
  					<li class="flex items-start gap-2 text-sm text-[var(--text-secondary)]">
  						<CheckCircle
  							size={14}
  							class="mt-0.5 shrink-0 text-[var(--color-success)]"
  							aria-hidden="true"
  						/>
  						{scopeDescription(scope)}
  					</li>
  				{/each}
  			</ul>

  			<Callout
  				tone="info"
  				message="{details.client_name} will act using your existing permissions — it cannot do anything you cannot already do."
  			/>
  		</ConsentPrompt>
  	{:else}
  		<Callout tone="info" message="Loading…" />
  	{/if}

  	{#snippet footer()}
  		<p class="text-sm text-[var(--text-secondary)]">
  			Signed in as {getUser()?.email ?? ''}
  			· <Link href="/login?_auth_context=oauth">Switch account</Link>
  		</p>
  	{/snippet}
  </PublicEntryShell>
  ```

- [ ] **Step 3: Run type check**

  ```bash
  cd frontend && npm run check 2>&1 | tail -20
  ```

  Expected: 0 errors.

- [ ] **Step 4: Update `oauth-consent.spec.ts`**

  Replace the entire contents of `frontend/tests/e2e/oauth-consent.spec.ts`:

  ```ts
  import { expect, test } from '@playwright/test';
  import type { Page } from '@playwright/test';
  import type { ConsentDetails } from '../../src/lib/api/oauth';

  // ---------------------------------------------------------------------------
  // Session helpers
  // ---------------------------------------------------------------------------

  async function mockAuthenticatedSession(page: Page) {
  	await page.route('**/api/v1/auth/refresh', (route) =>
  		route.fulfill({
  			status: 200,
  			json: { access_token: 'test-access-token', refresh_token: 'test-refresh-token' }
  		})
  	);
  	await page.route('**/api/v1/auth/me', (route) =>
  		route.fulfill({
  			status: 200,
  			json: {
  				id: '00000000-0000-0000-0000-000000000001',
  				email: 'user@example.com',
  				first_name: 'Test',
  				last_name: 'User',
  				permissions: []
  			}
  		})
  	);
  	await page.route('**/api/v1/system/alerts', (route) => route.fulfill({ json: { alerts: [] } }));
  }

  // ---------------------------------------------------------------------------
  // Consent mock factory
  // ---------------------------------------------------------------------------

  const BASE_CONSENT: ConsentDetails = {
  	client_id: 'client-abc',
  	client_name: 'Test MCP Client',
  	client_uri: null,
  	redirect_uri: 'https://example.com/callback',
  	redirect_uri_host: 'example.com',
  	scopes: ['mcp:read'],
  	created_via: 'manual',
  	trusted_at: '2026-01-01T00:00:00Z',
  	requires_typed_confirmation: false,
  	typed_confirmation_value: '',
  	metadata_change_diff: null
  };

  async function mockConsentGet(page: Page, requestId: string, overrides: Partial<ConsentDetails>) {
  	const payload: ConsentDetails = { ...BASE_CONSENT, ...overrides };
  	await page.route(`**/oauth/consent/${requestId}`, (route) => {
  		if (route.request().method() === 'GET') {
  			route.fulfill({ status: 200, json: payload });
  		} else {
  			route.fallback();
  		}
  	});
  }

  async function mockConsentApprove(page: Page, requestId: string, redirectTo: string) {
  	await page.route(`**/oauth/consent/${requestId}/approve`, (route) =>
  		route.fulfill({ status: 200, json: { redirect_to: redirectTo } })
  	);
  }

  async function mockConsentDeny(page: Page, requestId: string, redirectTo: string) {
  	await page.route(`**/oauth/consent/${requestId}/deny`, (route) =>
  		route.fulfill({ status: 200, json: { redirect_to: redirectTo } })
  	);
  }

  // ---------------------------------------------------------------------------
  // Selectors
  // ---------------------------------------------------------------------------

  const APPROVE_BUTTON = 'button:has-text("Approve")';
  const DENY_BUTTON = 'button:has-text("Deny")';
  const CONSENT_PROMPT = '[data-ui="consent-prompt"]';
  const DANGER_CALLOUT = '[data-ui="callout"][data-tone="danger"]';
  const WARNING_CALLOUT = '[data-ui="callout"][data-tone="warning"]';

  async function navigateToConsent(page: Page, requestId: string) {
  	await page.goto(`/oauth/consent/${requestId}`);
  	await page.waitForSelector(APPROVE_BUTTON);
  }

  // ---------------------------------------------------------------------------
  // Tests
  // ---------------------------------------------------------------------------

  test.describe('oauth consent screen', () => {
  	test.beforeEach(async ({ page }) => {
  		await mockAuthenticatedSession(page);
  	});

  	test('trusted client — renders client name and Approve button', async ({ page }) => {
  		const requestId = 'req-001';
  		await mockConsentGet(page, requestId, {});

  		await navigateToConsent(page, requestId);

  		await expect(page.locator(CONSENT_PROMPT)).toContainText('Test MCP Client');
  		await expect(page.locator(APPROVE_BUTTON)).toBeEnabled();
  		await expect(page.locator(DENY_BUTTON)).toBeVisible();
  	});

  	test('unverified client — shows danger callout, Approve still enabled', async ({ page }) => {
  		const requestId = 'req-002';
  		await mockConsentGet(page, requestId, { trusted_at: null });

  		await navigateToConsent(page, requestId);

  		await expect(page.locator(DANGER_CALLOUT)).toBeVisible();
  		await expect(page.locator(DANGER_CALLOUT)).toContainText('not been verified');
  		// Approve is immediately available — no typed gate
  		await expect(page.locator(APPROVE_BUTTON)).toBeEnabled();
  	});

  	test('Approve calls approve endpoint and redirects', async ({ page }) => {
  		const requestId = 'req-003';
  		const redirectTo = 'http://localhost:5173/oauth-redirect-target';

  		await mockConsentGet(page, requestId, {});
  		await mockConsentApprove(page, requestId, redirectTo);

  		await navigateToConsent(page, requestId);

  		const approveResponse = page.waitForResponse(`**/oauth/consent/${requestId}/approve`);
  		await page.locator(APPROVE_BUTTON).click();
  		await approveResponse;

  		await page.waitForURL('**/oauth-redirect-target', { timeout: 5_000 });
  	});

  	test('Deny calls deny endpoint and redirects', async ({ page }) => {
  		const requestId = 'req-004';
  		const redirectTo = 'http://localhost:5173/oauth-denied';

  		await mockConsentGet(page, requestId, {});
  		await mockConsentDeny(page, requestId, redirectTo);

  		await navigateToConsent(page, requestId);

  		const denyResponse = page.waitForResponse(`**/oauth/consent/${requestId}/deny`);
  		await page.locator(DENY_BUTTON).click();
  		await denyResponse;

  		await page.waitForURL('**/oauth-denied', { timeout: 5_000 });
  	});

  	test('localhost redirect_uri_host shows warning callout', async ({ page }) => {
  		const requestId = 'req-005';

  		await mockConsentGet(page, requestId, {
  			redirect_uri_host: 'localhost',
  			redirect_uri: 'http://localhost:8080/callback'
  		});

  		await navigateToConsent(page, requestId);

  		await expect(page.locator(WARNING_CALLOUT)).toBeVisible();
  		await expect(page.locator(WARNING_CALLOUT)).toContainText('local');
  	});

  	test('DCR client — shows warning callout', async ({ page }) => {
  		const requestId = 'req-006';

  		await mockConsentGet(page, requestId, {
  			created_via: 'dcr',
  			trusted_at: '2026-01-01T00:00:00Z'
  		});

  		await navigateToConsent(page, requestId);

  		await expect(page.locator(WARNING_CALLOUT)).toBeVisible();
  		await expect(page.locator(WARNING_CALLOUT)).toContainText('recently registered');
  	});

  	test('scope descriptions are shown as human-readable text', async ({ page }) => {
  		const requestId = 'req-007';

  		await mockConsentGet(page, requestId, { scopes: ['mcp:read', 'mcp:write'] });

  		await navigateToConsent(page, requestId);

  		await expect(page.locator(CONSENT_PROMPT)).toContainText('Read your uptrakit data');
  		await expect(page.locator(CONSENT_PROMPT)).toContainText('Trigger software updates');
  	});
  });
  ```

- [ ] **Step 5: Run lint and type check**

  ```bash
  cd frontend && npm run check && npm run lint 2>&1 | tail -20
  ```

  Expected: 0 errors, 0 warnings.

- [ ] **Step 6: Commit**

  ```bash
  git add frontend/src/lib/api/oauth.ts \
          frontend/src/routes/oauth/consent/[request_id]/+page.svelte \
          frontend/tests/e2e/oauth-consent.spec.ts
  git commit -m "feat(ui): redesign OAuth consent page with ConsentPrompt and PublicEntryShell

  Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
  ```

---

## Task 4: Update device auth page

**Files:**

- Modify: `frontend/src/routes/device/+page.svelte`
- Modify: `frontend/tests/e2e/public-entry.test.ts`

- [ ] **Step 1: Replace approval block in device page**

  Open `frontend/src/routes/device/+page.svelte`. Add the import at the top of the `<script>` block
  (after existing imports):

  ```svelte
  import ConsentPrompt from '$lib/components/ConsentPrompt.svelte';
  ```

  Locate the `{:else if lookupPhase === 'done'}` block (currently lines ~231–265). Replace the
  entire content of that block (keep the `{:else if lookupPhase === 'done'}` line itself):

  ```svelte
  {:else if lookupPhase === 'done'}
  	{#if actionError}
  		<Callout tone="danger" title="Unable to process device request" message={actionError} />
  	{/if}
  	<!-- trust="verified": device clients are always controller-internal (uptrakit CLI).
  	     No third-party DCR path exists for device-flow clients. -->
  	<ConsentPrompt
  		clientName={lookup?.client_name ?? 'CLI'}
  		trust="verified"
  		approving={processing}
  		onApprove={onApprove}
  		onDeny={onDeny}
  	/>
  ```

  The closing `{/if}` for the outer `{:else if lookupPhase === 'done'}` stays.

- [ ] **Step 2: Run type check**

  ```bash
  cd frontend && npm run check 2>&1 | tail -20
  ```

  Expected: 0 errors.

- [ ] **Step 3: Update device tests in `public-entry.test.ts`**

  Open `frontend/tests/e2e/public-entry.test.ts`. Find the test
  `'device shows client name when lookup succeeds'` (currently checks
  `page.locator('[data-ui="callout"]').toContainText(...)`).

  Replace the assertion at the end of that test. The old assertion:

  ```ts
  await expect(page.locator('[data-ui="callout"]')).toContainText('cli-laptop-2026-05-12');
  ```

  New assertion (client name now renders in ConsentPrompt body, not a Callout):

  ```ts
  await expect(page.locator('[data-ui="consent-prompt"]')).toContainText('cli-laptop-2026-05-12');
  ```

  The tests `'device approve succeeds'` and `'device deny succeeds'` use
  `page.getByRole('button', { name: 'Approve' })` and `page.getByRole('button', { name: 'Deny' })` —
  these selectors still match the `ConsentPrompt` buttons. No change needed there.

- [ ] **Step 4: Run lint and type check**

  ```bash
  cd frontend && npm run check && npm run lint 2>&1 | tail -20
  ```

  Expected: 0 errors.

- [ ] **Step 5: Commit**

  ```bash
  git add frontend/src/routes/device/+page.svelte \
          frontend/tests/e2e/public-entry.test.ts
  git commit -m "feat(ui): replace device approval callout with ConsentPrompt

  Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
  ```

---

## Task 5: Document `ConsentPrompt` in `primitives.md`

**Files:**

- Modify: `docs/development/ui/primitives.md` — insert after the `### ConfirmDialog` section

- [ ] **Step 1: Add `ConsentPrompt` section**

  Open `docs/development/ui/primitives.md`. Locate the end of the `### ConfirmDialog` section
  (ends before `### ModalShell`). Insert the following block between them:

  ````markdown
  ---

  ### ConsentPrompt

  Shared auth-consent card used by the OAuth consent page and the device auth approval flow.
  Owns client identity display, trust signals, Approve/Deny buttons, and a `children` slot for
  page-specific content (scope list, warnings).

  ```typescript
  // frontend/src/lib/components/ConsentPrompt.svelte
  import type { Snippet } from 'svelte';

  export type ConsentPromptTrust =
    | 'verified'    // controller-internal client (device flow); no callout
    | 'unverified'  // trusted_at IS NULL; danger callout
    | 'dcr'         // dynamically registered; warning callout
    | 'open-metadata' // cimd_cache; no callout
    | 'manual';     // admin-registered; no callout

  {
    clientName: string;
    clientUri?: string | null;     // renders as external link when present
    trust: ConsentPromptTrust;
    approveDisabled?: boolean;     // disables Approve without showing loading
    approving: boolean;            // shows spinner on Approve, disables both buttons
    onApprove: () => void;
    onDeny: () => void;
    children?: Snippet;            // rendered between trust signals and action buttons
  }
  ```

  Trust signal behaviour:

  | `trust` value  | Rendered signal                                                 |
  | -------------- | --------------------------------------------------------------- |
  | `unverified`   | `<Callout tone="danger">` — "not been verified"                 |
  | `dcr`          | `<Callout tone="warning">` — "recently registered"              |
  | all others     | no callout                                                      |

  Usage (OAuth consent page):

  ```svelte
  <ConsentPrompt
    clientName={details.client_name}
    clientUri={details.client_uri}
    trust={clientTrust(details)}
    approveDisabled={submitting}
    approving={submitting}
    onApprove={handleAllow}
    onDeny={handleDeny}
  >
    <!-- page-specific content: scope list, redirect warning, metadata notice -->
  </ConsentPrompt>
  ```

  Usage (device auth page):

  ```svelte
  <ConsentPrompt
    clientName={lookup?.client_name ?? 'CLI'}
    trust="verified"
    approving={processing}
    onApprove={onApprove}
    onDeny={onDeny}
  />
  ```

  Rules:

  - Approve button: `variant="primary" loading={approving}`. Disabled when `approving` or
    `approveDisabled`.
  - Deny button: `variant="secondary"`. Disabled when `approving` (no loading spinner).
  - Button order: Deny left, Approve right (`flex justify-end gap-2`).
  - Always renders `data-ui="consent-prompt"` on the root element for test selectors.
  - Do not render `ConsentPrompt` inside a `SectionCard` — it is a standalone card-level unit.
  ````

- [ ] **Step 2: Run markdownlint**

  ```bash
  npx markdownlint --config .markdownlint.json 'docs/development/ui/primitives.md'
  ```

  Expected: no errors.

- [ ] **Step 3: Commit**

  ```bash
  git add docs/development/ui/primitives.md
  git commit -m "docs(ui): add ConsentPrompt to primitives reference

  Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
  ```

---

## Task 6: Quality gate

- [ ] **Step 1: Full frontend quality gate**

  ```bash
  cd frontend && npm run check && npm run lint && npm run format:check && npm run test && npm run build
  ```

  Expected: clean build, 0 errors, vitest passes.

- [ ] **Step 2: Full backend gate**

  ```bash
  cargo fmt --all && \
  cargo check --all-features && \
  cargo clippy --all-targets --all-features -- -D warnings && \
  cargo test --all-features 2>&1 | tail -30
  ```

  Expected: clean.

- [ ] **Step 3: Markdownlint**

  ```bash
  npx markdownlint --config .markdownlint.json '**/*.md'
  ```

  Expected: no errors.

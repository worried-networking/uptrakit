# Profile Page Redesign Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Redesign `frontend/src/routes/profile/+page.svelte` to use `TabStrip`
(Account / API Tokens), move email and password forms into modals, hide revoked
tokens with an `EmptyState` fallback, add an inline copy icon to the token
reveal, and add `lucide-svelte` as the icon library.

**Architecture:** Two-tab layout mirroring the pattern in `settings/+page.svelte`:
`activeTab` state synced to `?tab=<id>` via `goto()` with `replaceState`,
conditional `{#if}` rendering per tab. Email/password forms extracted from
inline section cards into `ModalShell` dialogs; pending-change display driven
by `user.has_pending_email_change` (refreshed via `await initialize()` on
success) rather than a local flag.

**Tech Stack:** Svelte 5, SvelteKit, `$app/state` (`page`), `$app/navigation` (`goto`), Testing Library + Vitest, `lucide-svelte`.

---

## Files

| Action | Path                                          |
| ------ | --------------------------------------------- |
| Modify | `frontend/package.json`                       |
| Modify | `frontend/src/routes/profile/+page.svelte`    |
| Modify | `frontend/src/routes/profile/profile.test.ts` |

Key reference files (read-only):

- `frontend/src/routes/settings/+page.svelte` — canonical tab + URL-sync pattern
- `frontend/src/lib/components/ui/TabStrip.svelte` — `items`, `activeId`, `ariaLabel`, `onSelect` props
- `frontend/src/lib/components/ui/EmptyState.svelte` — `title`, `description`, `actions?: Snippet` props
- `frontend/src/lib/components/ui/index.ts` — all ui exports (`TabStrip`, `EmptyState`, `StatusBadge`, etc.)
- `frontend/src/lib/components/forms/FormFieldRow.svelte` — `label`, `inputId?`, `hint?`, `error?` props
- `frontend/src/lib/components/Modal.svelte` (exported as `ModalShell`) — `title?`, `onclose`, `maxWidth?`, `footer?: Snippet`
- `frontend/src/lib/test-mocks/app-state.ts` — `page` mock (`url: new URL('http://localhost/')`)
- `frontend/src/lib/test-mocks/app-navigation.ts` — `goto` no-op mock
- `frontend/vitest.config.ts` — aliases `$app/state` and `$app/navigation` to mocks

---

## Task 1: Install lucide-svelte

**Files:**

- Modify: `frontend/package.json`

- [ ] **Step 1: Install the package**

```bash
cd frontend && npm install lucide-svelte
```

- [ ] **Step 2: Verify it appears in `dependencies`**

```bash
cd frontend && node -e "const p=require('./package.json'); if (!p.dependencies?.['lucide-svelte']) { console.error('FAIL: lucide-svelte missing from dependencies'); process.exit(1); } console.log('OK:', p.dependencies['lucide-svelte']);"
```

Expected: prints `OK: ^0.x.x` (or similar semver). Failure means it landed in `devDependencies` — reinstall with `npm install lucide-svelte` (no `--save-dev`).

- [ ] **Step 3: Commit**

```bash
git commit --only frontend/package.json frontend/package-lock.json \
  -m "feat(profile): add lucide-svelte to dependencies"
```

---

## Task 2: Tab shell — TabStrip + conditional rendering + update all affected tests

**Files:**

- Modify: `frontend/src/routes/profile/+page.svelte`
- Modify: `frontend/src/routes/profile/profile.test.ts`

This task adds the tab structure and conditionally renders account vs. tokens content.
The old inline change-email and change-password section cards temporarily remain under
the account tab — they will be replaced in Tasks 4 and 5. All existing tests that access
token-related UI must be updated to first navigate to the "API Tokens" tab.

- [ ] **Step 1: Write new tab-navigation tests (add a new `describe` block)**

Add below the existing `describe('Button Migrations', ...)` block in `profile.test.ts`:

```typescript
describe("Tab Navigation", () => {
  beforeEach(() => {
    vi.mocked(auth.getUser).mockReturnValue(user);
    vi.mocked(api.listApiTokens).mockResolvedValue({ tokens: [] });
  });

  afterEach(() => {
    vi.clearAllMocks();
  });

  it("renders Account and API Tokens tabs", async () => {
    render(ProfilePage);
    await waitFor(() =>
      expect(
        screen.getByRole("heading", { level: 1, name: "Profile" }),
      ).toBeInTheDocument(),
    );
    expect(screen.getByRole("tab", { name: "Account" })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "API Tokens" })).toBeInTheDocument();
  });

  it("Account tab is active by default", async () => {
    render(ProfilePage);
    await waitFor(() =>
      expect(screen.getByRole("tab", { name: "Account" })).toBeInTheDocument(),
    );
    expect(screen.getByRole("tab", { name: "Account" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(screen.getByRole("tab", { name: "API Tokens" })).toHaveAttribute(
      "aria-selected",
      "false",
    );
  });

  it("clicking API Tokens tab makes it active", async () => {
    render(ProfilePage);
    await waitFor(() =>
      expect(
        screen.getByRole("tab", { name: "API Tokens" }),
      ).toBeInTheDocument(),
    );
    await userEvent.click(screen.getByRole("tab", { name: "API Tokens" }));
    expect(screen.getByRole("tab", { name: "API Tokens" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(screen.getByRole("tab", { name: "Account" })).toHaveAttribute(
      "aria-selected",
      "false",
    );
  });
});
```

- [ ] **Step 2: Run new tests to confirm they fail**

```bash
cd frontend && npm test -- --reporter=verbose profile.test.ts 2>&1 | grep -E "FAIL|PASS|✓|✗|×"
```

Expected: 3 new tests fail ("Cannot read properties of undefined" or "tab not found").

- [ ] **Step 3: Add imports to `+page.svelte` script section**

Replace the existing ui import line:

```typescript
// Before:
import {
  Callout,
  DataTable,
  ModalShell,
  PageShell,
  SectionCard,
  StatusBadge,
} from "$lib/components/ui";

// After:
import {
  Callout,
  DataTable,
  EmptyState,
  ModalShell,
  PageShell,
  SectionCard,
  StatusBadge,
  TabStrip,
} from "$lib/components/ui";
import type { TabStripItem } from "$lib/components/ui";
import { page } from "$app/state";
import { goto } from "$app/navigation";
```

- [ ] **Step 4: Add tab state and URL sync to script section**

Add after the `const authMethod = $derived(getAuthMethod());` line:

```typescript
const tabItems: TabStripItem[] = [
  { id: "account", label: "Account" },
  { id: "api-tokens", label: "API Tokens" },
];

let activeTab = $state(page.url.searchParams.get("tab") ?? "account");

$effect(() => {
  const search = activeTab !== "account" ? `?tab=${activeTab}` : "";
  goto(search ? `${location.pathname}${search}` : location.pathname, {
    replaceState: true,
    keepFocus: true,
    noScroll: true,
  });
});
```

- [ ] **Step 5: Restructure the template**

Replace the entire `{#if user}...{/if}` template block with:

```svelte
{#if user}
	<PageShell title="Profile">
		<TabStrip
			items={tabItems}
			activeId={activeTab}
			ariaLabel="Profile tabs"
			onSelect={(id) => {
				activeTab = id;
			}}
		/>

		{#if activeTab === 'account'}
			<SectionCard title="Profile">
				<div data-ui="profile-details-section">
					<FormFieldRow label="First name" inputId="profile-first-name">
						<Input
							id="profile-first-name"
							type="text"
							bind:value={firstName}
							placeholder="First name"
						/>
					</FormFieldRow>
					<FormFieldRow label="Last name" inputId="profile-last-name">
						<Input
							id="profile-last-name"
							type="text"
							bind:value={lastName}
							placeholder="Last name"
						/>
					</FormFieldRow>
					<FormFieldRow label="Email" inputId="profile-email">
						<Input id="profile-email" type="email" value={user?.email ?? ''} disabled />
						{#if authMethod === 'password'}
							<Button
								variant="secondary"
								size="sm"
								onclick={() => (showChangeEmail = true)}
							>Change email</Button>
						{/if}
					</FormFieldRow>
					{#if profileError}
						<Callout tone="danger" message={profileError} />
					{/if}
					<div class="flex justify-end">
						<Button variant="primary" loading={profileSaving} onclick={handleSaveProfile}>
							Save
						</Button>
					</div>
				</div>
			</SectionCard>

			{#if authMethod === 'password'}
				<SectionCard title="Change email">
					<div data-ui="change-email-section">
						{#if emailChangeSuccess}
							<Callout tone="success">
								A confirmation link has been sent to your new address. Check your inbox and
								click the link to complete the change.
							</Callout>
						{:else if user?.has_pending_email_change}
							<Callout tone="info">
								A confirmation email has been sent. Check your inbox. If you did not request
								this change, you can cancel it.
							</Callout>
							<div class="flex justify-end">
								<Button variant="ghost" onclick={handleCancelEmailChange}>
									Cancel email change
								</Button>
							</div>
						{:else if showChangeEmail}
							<FormFieldRow label="New email address" inputId="email-new-email">
								<Input
									id="email-new-email"
									type="email"
									bind:value={newEmail}
									placeholder="new@example.com"
								/>
							</FormFieldRow>
							<FormFieldRow label="Current password" inputId="email-current-password">
								<Input
									id="email-current-password"
									type="password"
									bind:value={emailCurrentPassword}
									placeholder="Enter your password"
								/>
							</FormFieldRow>
							{#if emailError}
								<Callout tone="danger" message={emailError} />
							{/if}
							<div class="flex justify-end gap-2">
								<Button variant="ghost" onclick={() => (showChangeEmail = false)}>
									Cancel
								</Button>
								<Button
									variant="primary"
									loading={emailChanging}
									onclick={handleInitiateEmailChange}
								>
									Send confirmation email
								</Button>
							</div>
						{:else}
							<p class="text-sm text-[var(--text-secondary)]">
								Update your email address. A confirmation link will be sent to your new address.
							</p>
						{/if}
					</div>
				</SectionCard>
			{/if}

			{#if authMethod === 'password'}
				<SectionCard title="Change password">
					<div data-ui="change-password-section">
						{#if passwordChangeSuccess}
							<Callout tone="success">
								Password changed. Other sessions have been signed out.
							</Callout>
							<Button variant="secondary" onclick={() => (passwordChangeSuccess = false)}>
								Change again
							</Button>
						{:else}
							<FormFieldRow label="Current password" inputId="pw-current">
								<Input
									id="pw-current"
									type="password"
									bind:value={currentPassword}
									placeholder="Current password"
								/>
							</FormFieldRow>
							<FormFieldRow label="New password" inputId="pw-new" hint="8–128 characters.">
								<Input
									id="pw-new"
									type="password"
									bind:value={newPassword}
									placeholder="At least 8 characters"
								/>
							</FormFieldRow>
							<FormFieldRow label="Confirm new password" inputId="pw-confirm">
								<Input
									id="pw-confirm"
									type="password"
									bind:value={confirmPassword}
									placeholder="Repeat new password"
								/>
								{#if confirmPasswordError}
									<p class="text-sm text-(--color-danger)">{confirmPasswordError}</p>
								{/if}
							</FormFieldRow>
							{#if passwordError}
								<Callout tone="danger" message={passwordError} />
							{/if}
							<div class="flex justify-end">
								<Button
									variant="primary"
									loading={passwordSaving}
									onclick={handleChangePassword}
								>
									Change password
								</Button>
							</div>
						{/if}
					</div>
				</SectionCard>
			{/if}
		{/if}

		{#if activeTab === 'api-tokens'}
			<SectionCard
				title="API Tokens"
				description="API tokens allow programmatic access to Uptrakit. Treat tokens like passwords and rotate them regularly."
			>
				{#snippet actions()}
					<Button variant="primary" onclick={openCreateModal}>New Token</Button>
				{/snippet}

				<DataTable
					columns={[]}
					rows={tokens as unknown as Record<string, unknown>[]}
					{loading}
					emptyTitle="No API tokens yet."
					rowKey={(row) => (row as unknown as ApiTokenResponse).id}
				>
					{#snippet header()}
						<tr
							class="border-b border-[var(--border-subtle)] bg-[var(--bg-raised)] text-[var(--text-secondary)]"
						>
							<th
								class="table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header"
								scope="col">Name</th
							>
							<th
								class="table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header"
								scope="col">Created</th
							>
							<th
								class="table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header"
								scope="col">Status</th
							>
							<th
								class="w-24 table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header"
								scope="col"
							></th>
						</tr>
					{/snippet}
					{#snippet row(rowValue, _index)}
						{@const token = rowValue as unknown as ApiTokenResponse}
						<tr class="border-b border-[var(--border-subtle)] last:border-b-0">
							<td class="table-cell-pad text-table-body text-[var(--text-primary)]"
								>{token.name}</td
							>
							<td class="table-cell-pad text-table-body text-[var(--text-primary)]"
								>{formatDate(token.created_at)}</td
							>
							<td class="table-cell-pad text-table-body text-[var(--text-primary)]">
								{#if token.revoked_at}
									<StatusBadge tone="neutral" label="Revoked" />
								{:else}
									<StatusBadge tone="success" label="Active" />
								{/if}
							</td>
							<td class="table-cell-pad text-table-body text-[var(--text-primary)]">
								{#if !token.revoked_at}
									<Button
										variant="danger"
										size="sm"
										onclick={() => (revokeConfirm = { id: token.id, name: token.name })}
									>
										Revoke
									</Button>
								{/if}
							</td>
						</tr>
					{/snippet}
				</DataTable>
			</SectionCard>
		{/if}
	</PageShell>
{/if}
```

Keep the existing modals below unchanged (ConfirmDialog + showCreateModal ModalShell).

- [ ] **Step 5b: Verify the template compiles**

```bash
cd frontend && npm run check
```

Expected: no type errors or Svelte compile errors.
This step catches template syntax issues from the large paste before subsequent tasks build on it.

- [ ] **Step 6: Update existing tests that now require tab navigation**

In `profile.test.ts`, replace the entire `describe('Profile Route', ...)` block:

```typescript
describe("Profile Route", () => {
  beforeEach(() => {
    vi.mocked(auth.getUser).mockReturnValue(user);
    vi.mocked(api.listApiTokens).mockResolvedValue({
      tokens: [
        {
          id: "token-1",
          name: "Automation",
          created_at: "2026-03-10T12:00:00Z",
          revoked_at: null,
        },
      ],
    });
  });

  afterEach(() => {
    vi.clearAllMocks();
  });

  it("renders shared shell primitives for account and token tables", async () => {
    render(ProfilePage);

    await waitFor(() =>
      expect(
        screen.getByRole("heading", { level: 1, name: "Profile" }),
      ).toBeInTheDocument(),
    );
    expect(
      document.querySelector('[data-ui="page-shell"]'),
    ).toBeInTheDocument();
    expect(
      document.querySelector('[data-ui="section-card"]'),
    ).toBeInTheDocument();
    expect(
      document.querySelector('[data-ui="profile-details-section"]'),
    ).toBeInTheDocument();

    // Navigate to API Tokens tab to assert DataTable
    await userEvent.click(screen.getByRole("tab", { name: "API Tokens" }));
    await waitFor(() =>
      expect(screen.getByText("Automation")).toBeInTheDocument(),
    );
    expect(
      document.querySelector('[data-ui="data-table"]'),
    ).toBeInTheDocument();
  });

  it("uses shared account detail rhythm and modal footer actions", async () => {
    render(ProfilePage);

    await waitFor(() =>
      expect(
        screen.getByRole("heading", { level: 1, name: "Profile" }),
      ).toBeInTheDocument(),
    );
    expect(
      document.querySelector('[data-ui="profile-details-section"]'),
    ).toBeInTheDocument();

    await userEvent.click(screen.getByRole("tab", { name: "API Tokens" }));
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "New Token" }),
      ).toBeInTheDocument(),
    );
    await userEvent.click(screen.getByRole("button", { name: "New Token" }));
    const modalTitle = await screen.findByText("New API Token");
    const modal = modalTitle.closest('[data-ui="modal-shell"]') as HTMLElement;
    expect(modal).toBeInTheDocument();
    const footer = modal.querySelector(
      '[data-ui="profile-token-modal-footer"]',
    ) as HTMLElement;
    expect(footer).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Cancel" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Create" })).toBeInTheDocument();
  });
});
```

Then update every test in `describe('Button Migrations', ...)` to navigate to the API
Tokens tab before interacting with token UI. For each test that currently starts with
`render(ProfilePage)` followed by `await waitFor(...)` for a token button, add a tab
click between the render and the waitFor. The full updated
`describe('Button Migrations', ...)` block:

```typescript
describe("Button Migrations", () => {
  beforeEach(() => {
    vi.mocked(auth.getUser).mockReturnValue(user);
    vi.mocked(api.listApiTokens).mockResolvedValue({ tokens: [] });
  });

  afterEach(() => {
    vi.clearAllMocks();
  });

  async function goToTokensTab() {
    await waitFor(() =>
      expect(
        screen.getByRole("tab", { name: "API Tokens" }),
      ).toBeInTheDocument(),
    );
    await userEvent.click(screen.getByRole("tab", { name: "API Tokens" }));
  }

  it('New Token launcher renders variant="primary"', async () => {
    render(ProfilePage);
    await goToTokensTab();
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "New Token" }),
      ).toBeInTheDocument(),
    );
    const btn = screen.getByRole("button", { name: "New Token" });
    expect(btn).toHaveClass(
      "bg-[linear-gradient(90deg,var(--accent-deep),var(--accent))]",
    );
  });

  it('Row Revoke button renders variant="danger" size="sm"', async () => {
    const token: ApiTokenResponse = {
      id: "token-1",
      name: "CI Pipeline",
      created_at: "2026-04-19T00:00:00Z",
      revoked_at: null,
    };
    vi.mocked(api.listApiTokens).mockResolvedValue({ tokens: [token] });
    render(ProfilePage);
    await goToTokensTab();
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "Revoke" }),
      ).toBeInTheDocument(),
    );
    const btn = screen.getByRole("button", { name: "Revoke" });
    expect(btn).toHaveClass("h-[19px]");
    expect(btn).toHaveClass("bg-[var(--color-danger-bg)]");
  });

  it('New API Token modal Create state Cancel button renders variant="secondary"', async () => {
    render(ProfilePage);
    await goToTokensTab();
    const newTokenBtn = await screen.findByRole("button", {
      name: "New Token",
    });
    await userEvent.click(newTokenBtn);
    await waitFor(() =>
      expect(
        screen.getByPlaceholderText("e.g. CI Pipeline"),
      ).toBeInTheDocument(),
    );
    const cancelBtn = screen.getByRole("button", { name: "Cancel" });
    expect(cancelBtn).toHaveClass("bg-[var(--bg-raised)]");
  });

  it("New API Token modal Create state Create button already migrated (Wave 3)", async () => {
    render(ProfilePage);
    await goToTokensTab();
    const newTokenBtn = await screen.findByRole("button", {
      name: "New Token",
    });
    await userEvent.click(newTokenBtn);
    await waitFor(() =>
      expect(
        screen.getByPlaceholderText("e.g. CI Pipeline"),
      ).toBeInTheDocument(),
    );
    const createBtn = screen.getByRole("button", { name: "Create" });
    expect(createBtn).toHaveClass(
      "bg-[linear-gradient(90deg,var(--accent-deep),var(--accent))]",
    );
    expect(createBtn).toBeDisabled();
    const nameInput = screen.getByPlaceholderText("e.g. CI Pipeline");
    await userEvent.type(nameInput, "new-token");
    await waitFor(() => expect(createBtn).not.toHaveAttribute("disabled"));
    expect(createBtn).not.toHaveAttribute("aria-busy");
    expect(createBtn.textContent).toContain("Create");
  });

  it('New API Token modal Created state Copy button renders variant="secondary"', async () => {
    vi.mocked(api.createApiToken).mockResolvedValue({
      id: "token-1",
      token: "secret-token-123",
    });
    render(ProfilePage);
    await goToTokensTab();
    const newTokenBtn = await screen.findByRole("button", {
      name: "New Token",
    });
    await userEvent.click(newTokenBtn);
    await waitFor(() =>
      expect(
        screen.getByPlaceholderText("e.g. CI Pipeline"),
      ).toBeInTheDocument(),
    );
    const nameInput = screen.getByPlaceholderText("e.g. CI Pipeline");
    await userEvent.type(nameInput, "test-token");
    const createBtn = screen.getByRole("button", { name: "Create" });
    await userEvent.click(createBtn);
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Copy" })).toBeInTheDocument(),
    );
    const copyBtn = screen.getByRole("button", { name: "Copy" });
    expect(copyBtn).toHaveClass("bg-[var(--bg-raised)]");
  });

  it('New API Token modal Created state Done button renders variant="primary"', async () => {
    vi.mocked(api.createApiToken).mockResolvedValue({
      id: "token-1",
      token: "secret-token-123",
    });
    render(ProfilePage);
    await goToTokensTab();
    const newTokenBtn = await screen.findByRole("button", {
      name: "New Token",
    });
    await userEvent.click(newTokenBtn);
    await waitFor(() =>
      expect(
        screen.getByPlaceholderText("e.g. CI Pipeline"),
      ).toBeInTheDocument(),
    );
    const nameInput = screen.getByPlaceholderText("e.g. CI Pipeline");
    await userEvent.type(nameInput, "test-token");
    const createBtn = screen.getByRole("button", { name: "Create" });
    await userEvent.click(createBtn);
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Done" })).toBeInTheDocument(),
    );
    const doneBtn = screen.getByRole("button", { name: "Done" });
    expect(doneBtn).toHaveClass(
      "bg-[linear-gradient(90deg,var(--accent-deep),var(--accent))]",
    );
  });

  it("New API Token modal Copy button invokes clipboard.writeText and surfaces success toast", async () => {
    vi.mocked(api.createApiToken).mockResolvedValue({
      id: "token-1",
      token: "secret-token-123",
    });
    const writeTextMock = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", {
      value: { writeText: writeTextMock },
      writable: true,
      configurable: true,
    });
    render(ProfilePage);
    await goToTokensTab();
    const newTokenBtn = await screen.findByRole("button", {
      name: "New Token",
    });
    await userEvent.click(newTokenBtn);
    await waitFor(() =>
      expect(
        screen.getByPlaceholderText("e.g. CI Pipeline"),
      ).toBeInTheDocument(),
    );
    const nameInput = screen.getByPlaceholderText("e.g. CI Pipeline");
    await userEvent.type(nameInput, "test-token");
    const createBtn = screen.getByRole("button", { name: "Create" });
    await userEvent.click(createBtn);
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Copy" })).toBeInTheDocument(),
    );
    const copyBtn = screen.getByRole("button", { name: "Copy" });
    await userEvent.click(copyBtn);
    await waitFor(() =>
      expect(writeTextMock).toHaveBeenCalledWith("secret-token-123"),
    );
  });

  it("Out-of-scope regression: ConfirmDialog Revoke confirmation is not wrapped in Button", async () => {
    const token: ApiTokenResponse = {
      id: "token-1",
      name: "Test Token",
      created_at: "2026-04-19T00:00:00Z",
      revoked_at: null,
    };
    vi.mocked(api.listApiTokens).mockResolvedValue({ tokens: [token] });
    render(ProfilePage);
    await goToTokensTab();
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "Revoke" }),
      ).toBeInTheDocument(),
    );
    const revokeBtn = screen.getByRole("button", { name: "Revoke" });
    await userEvent.click(revokeBtn);
    await waitFor(() =>
      expect(
        screen.getByRole("heading", { name: "Revoke API Token" }),
      ).toBeInTheDocument(),
    );
    expect(
      screen.getByRole("heading", { name: "Revoke API Token" }),
    ).toBeInTheDocument();
  });
});
```

- [ ] **Step 7: Run all tests to confirm they pass**

```bash
cd frontend && npm test -- profile.test.ts 2>&1 | tail -20
```

Expected: all existing tests pass, 3 new tab-navigation tests pass.

- [ ] **Step 8: Commit**

```bash
git commit --only \
  frontend/src/routes/profile/+page.svelte \
  frontend/src/routes/profile/profile.test.ts \
  -m "feat(profile): add TabStrip with Account/API Tokens tabs and URL sync"
```

---

## Task 3: Profile card — StatusBadge + Change email button → modal trigger

**Files:**

- Modify: `frontend/src/routes/profile/+page.svelte`
- Modify: `frontend/src/routes/profile/profile.test.ts`

- [ ] **Step 1: Write failing tests — add a new `describe('Account Tab — Profile Card', ...)` block**

```typescript
describe("Account Tab — Profile Card", () => {
  afterEach(() => {
    vi.clearAllMocks();
  });

  it('shows "Change email" button for password auth', async () => {
    vi.mocked(auth.getUser).mockReturnValue(user);
    vi.mocked(auth.getAuthMethod).mockReturnValue("password");
    vi.mocked(api.listApiTokens).mockResolvedValue({ tokens: [] });
    render(ProfilePage);
    await waitFor(() =>
      expect(
        screen.getByRole("heading", { level: 1, name: "Profile" }),
      ).toBeInTheDocument(),
    );
    expect(
      screen.getByRole("button", { name: "Change email" }),
    ).toBeInTheDocument();
  });

  it('hides "Change email" button for OIDC auth', async () => {
    vi.mocked(auth.getUser).mockReturnValue(user);
    vi.mocked(auth.getAuthMethod).mockReturnValue("oidc");
    vi.mocked(api.listApiTokens).mockResolvedValue({ tokens: [] });
    render(ProfilePage);
    await waitFor(() =>
      expect(
        screen.getByRole("heading", { level: 1, name: "Profile" }),
      ).toBeInTheDocument(),
    );
    expect(
      screen.queryByRole("button", { name: "Change email" }),
    ).not.toBeInTheDocument();
  });

  it('shows "Change pending" StatusBadge when has_pending_email_change is true', async () => {
    vi.mocked(auth.getUser).mockReturnValue({
      ...user,
      has_pending_email_change: true,
    });
    vi.mocked(auth.getAuthMethod).mockReturnValue("password");
    vi.mocked(api.listApiTokens).mockResolvedValue({ tokens: [] });
    render(ProfilePage);
    await waitFor(() =>
      expect(
        screen.getByRole("heading", { level: 1, name: "Profile" }),
      ).toBeInTheDocument(),
    );
    expect(screen.getByText("Change pending")).toBeInTheDocument();
  });
});
```

- [ ] **Step 2: Run to confirm they fail**

```bash
cd frontend && npm test -- --reporter=verbose profile.test.ts 2>&1 | grep -E "FAIL|✗|×|Change email|Change pending"
```

Expected: 3 tests fail.

- [ ] **Step 3: Add `showChangeEmailModal` state + update profile card in `+page.svelte`**

In the script section, add alongside the existing `let showChangeEmail = $state(false);`
(do **not** remove it yet — the inline change-email section card from Task 2 still
references it; both variables will coexist until Task 4 deletes the inline section):

```typescript
let showChangeEmailModal = $state(false);
```

In the template, within the Profile card `FormFieldRow label="Email"`, replace:

```svelte
{#if authMethod === 'password'}
    <Button variant="secondary" size="sm" onclick={() => (showChangeEmail = true)}>Change email</Button>
{/if}
```

with:

```svelte
{#if authMethod === 'password'}
    <Button
        variant="secondary"
        size="sm"
        onclick={() => (showChangeEmailModal = true)}
    >Change email</Button>
    {#if user.has_pending_email_change}
        <StatusBadge tone="warning" label="Change pending" />
    {/if}
{/if}
```

- [ ] **Step 4: Run tests to confirm they pass**

```bash
cd frontend && npm test -- profile.test.ts 2>&1 | tail -10
```

Expected: all tests pass.

- [ ] **Step 5: Commit**

```bash
git commit --only \
  frontend/src/routes/profile/+page.svelte \
  frontend/src/routes/profile/profile.test.ts \
  -m "feat(profile): add StatusBadge for pending email + wire Change email to modal state"
```

---

## Task 4: Change Email modal (replace inline section card)

**Files:**

- Modify: `frontend/src/routes/profile/+page.svelte`
- Modify: `frontend/src/routes/profile/profile.test.ts`

- [ ] **Step 1: Write failing tests — add `describe('Account Tab — Change Email Modal', ...)`**

```typescript
describe("Account Tab — Change Email Modal", () => {
  beforeEach(() => {
    vi.mocked(auth.getUser).mockReturnValue(user);
    vi.mocked(auth.getAuthMethod).mockReturnValue("password");
    vi.mocked(api.listApiTokens).mockResolvedValue({ tokens: [] });
  });

  afterEach(() => {
    vi.clearAllMocks();
  });

  it('change email modal opens when "Change email" is clicked', async () => {
    render(ProfilePage);
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "Change email" }),
      ).toBeInTheDocument(),
    );
    await userEvent.click(screen.getByRole("button", { name: "Change email" }));
    await waitFor(() =>
      expect(
        screen.getByRole("heading", { name: "Change Email" }),
      ).toBeInTheDocument(),
    );
  });

  it("change email modal closes when Cancel is clicked", async () => {
    render(ProfilePage);
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "Change email" }),
      ).toBeInTheDocument(),
    );
    await userEvent.click(screen.getByRole("button", { name: "Change email" }));
    await waitFor(() =>
      expect(
        screen.getByRole("heading", { name: "Change Email" }),
      ).toBeInTheDocument(),
    );
    await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
    await waitFor(() =>
      expect(
        screen.queryByRole("heading", { name: "Change Email" }),
      ).not.toBeInTheDocument(),
    );
  });

  it("shows pending-change Callout when has_pending_email_change is true", async () => {
    vi.mocked(auth.getUser).mockReturnValue({
      ...user,
      has_pending_email_change: true,
    });
    render(ProfilePage);
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "Change email" }),
      ).toBeInTheDocument(),
    );
    await userEvent.click(screen.getByRole("button", { name: "Change email" }));
    await waitFor(() =>
      expect(
        screen.getByText(/A confirmation email has been sent/),
      ).toBeInTheDocument(),
    );
    expect(
      screen.getByRole("button", { name: "Cancel email change" }),
    ).toBeInTheDocument();
  });
});
```

- [ ] **Step 2: Run to confirm they fail**

```bash
cd frontend && npm test -- --reporter=verbose profile.test.ts 2>&1 | grep -E "FAIL|✗|×|Change Email|Change email modal"
```

Expected: 3 tests fail (modal not rendered yet).

- [ ] **Step 3: Update `handleInitiateEmailChange` — replace `emailChangeSuccess` with `await initialize()`**

Remove both `let emailChangeSuccess = $state(false);` and
`let showChangeEmail = $state(false);` from the script section (the inline change-email
section card that referenced `showChangeEmail` is deleted in Step 4).

Replace `handleInitiateEmailChange`:

```typescript
async function handleInitiateEmailChange() {
  if (!user) return;
  emailChanging = true;
  emailError = "";
  try {
    await initiateEmailChange(user.id, {
      new_email: newEmail,
      current_password: emailCurrentPassword,
    });
    newEmail = "";
    emailCurrentPassword = "";
  } catch (e) {
    emailError =
      e instanceof Error ? e.message : "Failed to initiate email change";
    return;
  } finally {
    emailChanging = false;
  }
  // Best-effort refresh — if initialize() fails, the modal stays open
  // and the user can close manually; don't surface a misleading error.
  await initialize().catch(() => {});
}
```

Replace `handleCancelEmailChange`:

```typescript
async function handleCancelEmailChange() {
  if (!user) return;
  try {
    await cancelEmailChange(user.id);
    showSuccess("Email change cancelled");
    await initialize();
    showChangeEmailModal = false;
  } catch (e) {
    showError(e instanceof Error ? e.message : "Failed to cancel email change");
  }
}
```

- [ ] **Step 4: Remove the old inline change-email section card from the template**

In the template, delete the entire
`{#if authMethod === 'password'}<SectionCard title="Change email">...</SectionCard>{/if}`
block that appears after the Profile card.

- [ ] **Step 5: Add the Change Email modal to the template**

Add below the `{/if}` that closes `{#if activeTab === 'account'}` block (just before `{#if activeTab === 'api-tokens'}`):

```svelte
{#if showChangeEmailModal}
	<ModalShell
		title="Change Email"
		onclose={() => {
			showChangeEmailModal = false;
		}}
		maxWidth="max-w-lg"
	>
		{#if user?.has_pending_email_change}
			<Callout
				tone="info"
				message="A confirmation email has been sent. Check your inbox. If you did not request this change, you can cancel it."
			/>
		{:else}
			<FormFieldRow label="New email address" inputId="email-new-email">
				<Input
					id="email-new-email"
					type="email"
					bind:value={newEmail}
					placeholder="new@example.com"
				/>
			</FormFieldRow>
			<FormFieldRow label="Current password" inputId="email-current-password">
				<Input
					id="email-current-password"
					type="password"
					bind:value={emailCurrentPassword}
					placeholder="Enter your password"
				/>
			</FormFieldRow>
			{#if emailError}
				<Callout tone="danger" message={emailError} />
			{/if}
		{/if}
		{#snippet footer()}
			{#if user?.has_pending_email_change}
				<Button variant="ghost" onclick={handleCancelEmailChange}>Cancel email change</Button>
				<Button variant="primary" onclick={() => (showChangeEmailModal = false)}>Close</Button>
			{:else}
				<Button variant="ghost" onclick={() => (showChangeEmailModal = false)}>Cancel</Button>
				<Button
					variant="primary"
					loading={emailChanging}
					onclick={handleInitiateEmailChange}
				>
					Send confirmation email
				</Button>
			{/if}
		{/snippet}
	</ModalShell>
{/if}
```

- [ ] **Step 6: Run tests to confirm they pass**

```bash
cd frontend && npm test -- profile.test.ts 2>&1 | tail -10
```

Expected: all tests pass.

- [ ] **Step 7: Commit**

```bash
git commit --only \
  frontend/src/routes/profile/+page.svelte \
  frontend/src/routes/profile/profile.test.ts \
  -m "feat(profile): replace inline change-email section with ModalShell"
```

---

## Task 5: Security card + Change Password modal (replace inline section card)

**Files:**

- Modify: `frontend/src/routes/profile/+page.svelte`
- Modify: `frontend/src/routes/profile/profile.test.ts`

- [ ] **Step 1: Write failing tests — add `describe('Account Tab — Security Card', ...)`**

```typescript
describe("Account Tab — Security Card", () => {
  beforeEach(() => {
    vi.mocked(api.listApiTokens).mockResolvedValue({ tokens: [] });
  });

  afterEach(() => {
    vi.clearAllMocks();
  });

  it('shows masked password row and "Change" button for password auth', async () => {
    vi.mocked(auth.getUser).mockReturnValue(user);
    vi.mocked(auth.getAuthMethod).mockReturnValue("password");
    render(ProfilePage);
    await waitFor(() =>
      expect(
        screen.getByRole("heading", { level: 1, name: "Profile" }),
      ).toBeInTheDocument(),
    );
    expect(screen.getByText("••••••••")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Change" })).toBeInTheDocument();
  });

  it("shows SSO Callout for OIDC auth", async () => {
    vi.mocked(auth.getUser).mockReturnValue(user);
    vi.mocked(auth.getAuthMethod).mockReturnValue("oidc");
    render(ProfilePage);
    await waitFor(() =>
      expect(
        screen.getByRole("heading", { level: 1, name: "Profile" }),
      ).toBeInTheDocument(),
    );
    expect(
      screen.getByText(/Your account uses single sign-on/),
    ).toBeInTheDocument();
  });

  it('change password modal opens when "Change" is clicked', async () => {
    vi.mocked(auth.getUser).mockReturnValue(user);
    vi.mocked(auth.getAuthMethod).mockReturnValue("password");
    render(ProfilePage);
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "Change" }),
      ).toBeInTheDocument(),
    );
    await userEvent.click(screen.getByRole("button", { name: "Change" }));
    await waitFor(() =>
      expect(
        screen.getByRole("heading", { name: "Change Password" }),
      ).toBeInTheDocument(),
    );
  });

  it("change password modal closes when Cancel is clicked", async () => {
    vi.mocked(auth.getUser).mockReturnValue(user);
    vi.mocked(auth.getAuthMethod).mockReturnValue("password");
    render(ProfilePage);
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "Change" }),
      ).toBeInTheDocument(),
    );
    await userEvent.click(screen.getByRole("button", { name: "Change" }));
    await waitFor(() =>
      expect(
        screen.getByRole("heading", { name: "Change Password" }),
      ).toBeInTheDocument(),
    );
    await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
    await waitFor(() =>
      expect(
        screen.queryByRole("heading", { name: "Change Password" }),
      ).not.toBeInTheDocument(),
    );
  });
});
```

- [ ] **Step 2: Run to confirm they fail**

```bash
cd frontend && npm test -- --reporter=verbose profile.test.ts 2>&1 | grep -E "FAIL|✗|×|Security|Change password"
```

Expected: 4 tests fail.

- [ ] **Step 3: Add `showChangePasswordModal` state + `closePasswordModal` helper + update `handleChangePassword`**

Remove `let passwordChangeSuccess = $state(false);` from the script section.

Add in the script section (near the other password state vars):

```typescript
let showChangePasswordModal = $state(false);

function closePasswordModal() {
  showChangePasswordModal = false;
  currentPassword = "";
  newPassword = "";
  confirmPassword = "";
  confirmPasswordError = "";
  passwordError = "";
}
```

Replace `handleChangePassword`:

```typescript
async function handleChangePassword() {
  if (!user) return;
  if (newPassword !== confirmPassword) {
    confirmPasswordError = "Passwords do not match";
    return;
  }
  confirmPasswordError = "";
  passwordError = "";
  passwordSaving = true;
  try {
    await changePassword(user.id, {
      current_password: currentPassword,
      new_password: newPassword,
    });
    closePasswordModal();
    showSuccess("Password changed. Other sessions have been signed out.");
  } catch (e) {
    passwordError =
      e instanceof Error ? e.message : "Failed to change password";
  } finally {
    passwordSaving = false;
  }
}
```

- [ ] **Step 4: Replace old change-password section card with Security card in the template**

In the account tab content, replace the `{#if authMethod === 'password'}<SectionCard title="Change password">...</SectionCard>{/if}` block with:

```svelte
<SectionCard title="Security">
    {#if authMethod === 'password'}
        <FormFieldRow label="Password">
            <span class="text-sm text-[var(--text-secondary)]">••••••••</span>
            <Button
                variant="secondary"
                size="sm"
                onclick={() => (showChangePasswordModal = true)}
            >Change</Button>
        </FormFieldRow>
    {:else}
        <Callout
            tone="info"
            message="Your account uses single sign-on. Password and email are managed by your identity provider."
        />
    {/if}
</SectionCard>
```

- [ ] **Step 5: Add the Change Password modal to the template**

Add after the Change Email modal (before `{#if activeTab === 'api-tokens'}`):

```svelte
{#if showChangePasswordModal}
	<ModalShell
		title="Change Password"
		onclose={closePasswordModal}
		maxWidth="max-w-lg"
	>
		<FormFieldRow label="Current password" inputId="pw-current">
			<Input
				id="pw-current"
				type="password"
				bind:value={currentPassword}
				placeholder="Current password"
			/>
		</FormFieldRow>
		<FormFieldRow label="New password" inputId="pw-new" hint="8–128 characters.">
			<Input
				id="pw-new"
				type="password"
				bind:value={newPassword}
				placeholder="At least 8 characters"
			/>
		</FormFieldRow>
		<FormFieldRow
			label="Confirm new password"
			inputId="pw-confirm"
			error={confirmPasswordError}
		>
			<Input
				id="pw-confirm"
				type="password"
				bind:value={confirmPassword}
				placeholder="Repeat new password"
			/>
		</FormFieldRow>
		{#if passwordError}
			<Callout tone="danger" message={passwordError} />
		{/if}
		{#snippet footer()}
			<Button variant="ghost" onclick={closePasswordModal}>Cancel</Button>
			<Button variant="primary" loading={passwordSaving} onclick={handleChangePassword}>
				Change password
			</Button>
		{/snippet}
	</ModalShell>
{/if}
```

- [ ] **Step 6: Run tests to confirm they pass**

```bash
cd frontend && npm test -- profile.test.ts 2>&1 | tail -10
```

Expected: all tests pass.

- [ ] **Step 7: Commit**

```bash
git commit --only \
  frontend/src/routes/profile/+page.svelte \
  frontend/src/routes/profile/profile.test.ts \
  -m "feat(profile): add Security card and Change Password modal, remove inline password section"
```

---

## Task 6: API Tokens tab — filter revoked tokens + EmptyState + remove Status column

**Files:**

- Modify: `frontend/src/routes/profile/+page.svelte`
- Modify: `frontend/src/routes/profile/profile.test.ts`

- [ ] **Step 1: Write failing tests — add `describe('API Tokens Tab', ...)`**

```typescript
describe("API Tokens Tab", () => {
  beforeEach(() => {
    vi.mocked(auth.getUser).mockReturnValue(user);
    vi.mocked(auth.getAuthMethod).mockReturnValue("password");
  });

  afterEach(() => {
    vi.clearAllMocks();
  });

  async function goToTokensTab() {
    await waitFor(() =>
      expect(
        screen.getByRole("tab", { name: "API Tokens" }),
      ).toBeInTheDocument(),
    );
    await userEvent.click(screen.getByRole("tab", { name: "API Tokens" }));
  }

  it("filters out revoked tokens from the DataTable", async () => {
    vi.mocked(api.listApiTokens).mockResolvedValue({
      tokens: [
        {
          id: "token-1",
          name: "Active Token",
          created_at: "2026-01-01T00:00:00Z",
          revoked_at: null,
        },
        {
          id: "token-2",
          name: "Revoked Token",
          created_at: "2026-01-02T00:00:00Z",
          revoked_at: "2026-02-01T00:00:00Z",
        },
      ],
    });
    render(ProfilePage);
    await goToTokensTab();
    await waitFor(() =>
      expect(screen.getByText("Active Token")).toBeInTheDocument(),
    );
    expect(screen.queryByText("Revoked Token")).not.toBeInTheDocument();
  });

  it("renders EmptyState when all tokens are revoked", async () => {
    vi.mocked(api.listApiTokens).mockResolvedValue({
      tokens: [
        {
          id: "token-1",
          name: "Old Token",
          created_at: "2026-01-01T00:00:00Z",
          revoked_at: "2026-02-01T00:00:00Z",
        },
      ],
    });
    render(ProfilePage);
    await goToTokensTab();
    await waitFor(() =>
      expect(screen.getByText("No API tokens")).toBeInTheDocument(),
    );
    expect(
      document.querySelector('[data-ui="data-table"]'),
    ).not.toBeInTheDocument();
  });

  it("renders EmptyState when token list is empty", async () => {
    vi.mocked(api.listApiTokens).mockResolvedValue({ tokens: [] });
    render(ProfilePage);
    await goToTokensTab();
    await waitFor(() =>
      expect(screen.getByText("No API tokens")).toBeInTheDocument(),
    );
  });
});
```

- [ ] **Step 2: Run to confirm they fail**

```bash
cd frontend && npm test -- --reporter=verbose profile.test.ts 2>&1 | grep -E "FAIL|✗|×|API Tokens Tab"
```

Expected: 3 tests fail.

- [ ] **Step 3: Add `activeTokens` derived variable to script section**

Add after the `let tokens: ApiTokenResponse[] = $state([]);` line:

```typescript
const activeTokens = $derived(tokens.filter((t) => t.revoked_at === null));
```

- [ ] **Step 4: Update API Tokens tab template — EmptyState conditional, filter, remove Status column**

Replace the entire `{#if activeTab === 'api-tokens'}` block with:

```svelte
{#if activeTab === 'api-tokens'}
	<SectionCard
		title="API Tokens"
		description="API tokens allow programmatic access to Uptrakit. Treat tokens like passwords and rotate them regularly."
	>
		{#snippet actions()}
			<Button variant="primary" onclick={openCreateModal}>New Token</Button>
		{/snippet}

		{#if activeTokens.length === 0}
			<EmptyState
				title="No API tokens"
				description="Create a token to access Uptrakit programmatically."
			>
				{#snippet actions()}
					<Button variant="ghost" onclick={openCreateModal}>New Token</Button>
				{/snippet}
			</EmptyState>
		{:else}
			<DataTable
				columns={[]}
				rows={activeTokens as unknown as Record<string, unknown>[]}
				{loading}
				rowKey={(row) => (row as unknown as ApiTokenResponse).id}
			>
				{#snippet header()}
					<tr
						class="border-b border-[var(--border-subtle)] bg-[var(--bg-raised)] text-[var(--text-secondary)]"
					>
						<th
							class="table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header"
							scope="col">Name</th
						>
						<th
							class="table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header"
							scope="col">Created</th
						>
						<th
							class="w-24 table-cell-pad text-left text-table-header font-semibold uppercase tracking-table-header"
							scope="col"
						></th>
					</tr>
				{/snippet}
				{#snippet row(rowValue, _index)}
					{@const token = rowValue as unknown as ApiTokenResponse}
					<tr class="border-b border-[var(--border-subtle)] last:border-b-0">
						<td class="table-cell-pad text-table-body text-[var(--text-primary)]"
							>{token.name}</td
						>
						<td class="table-cell-pad text-table-body text-[var(--text-primary)]"
							>{formatDate(token.created_at)}</td
						>
						<td class="table-cell-pad text-table-body text-[var(--text-primary)]">
							<Button
								variant="danger"
								size="sm"
								onclick={() => (revokeConfirm = { id: token.id, name: token.name })}
							>
								Revoke
							</Button>
						</td>
					</tr>
				{/snippet}
			</DataTable>
		{/if}
	</SectionCard>
{/if}
```

- [ ] **Step 5: Run tests to confirm they pass**

```bash
cd frontend && npm test -- profile.test.ts 2>&1 | tail -10
```

Expected: all tests pass.

- [ ] **Step 6: Commit**

```bash
git commit --only \
  frontend/src/routes/profile/+page.svelte \
  frontend/src/routes/profile/profile.test.ts \
  -m "feat(profile): filter revoked tokens, add EmptyState, remove Status column"
```

---

## Task 7: Create Token modal — inline copy icon button

**Files:**

- Modify: `frontend/src/routes/profile/+page.svelte`
- Modify: `frontend/src/routes/profile/profile.test.ts`

- [ ] **Step 1: Write failing tests — add to the existing "Button Migrations" describe or as a new describe**

Add a new `describe('Create Token Modal — Post-reveal', ...)` block:

```typescript
describe("Create Token Modal — Post-reveal", () => {
  beforeEach(() => {
    vi.mocked(auth.getUser).mockReturnValue(user);
    vi.mocked(auth.getAuthMethod).mockReturnValue("password");
    vi.mocked(api.listApiTokens).mockResolvedValue({ tokens: [] });
    vi.mocked(api.createApiToken).mockResolvedValue({
      id: "token-1",
      token: "secret-token-abc",
    });
  });

  afterEach(() => {
    vi.clearAllMocks();
  });

  async function revealToken() {
    await waitFor(() =>
      expect(
        screen.getByRole("tab", { name: "API Tokens" }),
      ).toBeInTheDocument(),
    );
    await userEvent.click(screen.getByRole("tab", { name: "API Tokens" }));
    const newTokenBtn = await screen.findByRole("button", {
      name: "New Token",
    });
    await userEvent.click(newTokenBtn);
    await waitFor(() =>
      expect(
        screen.getByPlaceholderText("e.g. CI Pipeline"),
      ).toBeInTheDocument(),
    );
    await userEvent.type(
      screen.getByPlaceholderText("e.g. CI Pipeline"),
      "test",
    );
    await userEvent.click(screen.getByRole("button", { name: "Create" }));
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Done" })).toBeInTheDocument(),
    );
  }

  it("inline copy icon button is present in post-reveal state", async () => {
    render(ProfilePage);
    await revealToken();
    expect(
      screen.getByRole("button", { name: "Copy token" }),
    ).toBeInTheDocument();
  });

  it("footer Copy button is present in post-reveal state", async () => {
    render(ProfilePage);
    await revealToken();
    expect(screen.getByRole("button", { name: "Copy" })).toBeInTheDocument();
  });
});
```

- [ ] **Step 2: Run to confirm they fail**

```bash
cd frontend && npm test -- --reporter=verbose profile.test.ts 2>&1 | grep -E "FAIL|✗|×|copy icon|Copy token"
```

Expected: 2 tests fail (no `Copy token` aria-label button exists yet).

- [ ] **Step 3: Add `Copy` import from lucide-svelte to `+page.svelte`**

Add to the imports block at the top of the script section:

```typescript
import { Copy } from "lucide-svelte";
```

- [ ] **Step 4: Add inline copy icon button to the post-reveal state in the create token modal**

In the existing `{#if showCreateModal}` block, within the `{#if createdToken}` branch, replace:

```svelte
<div class="relative">
    <pre
        class="rounded-panel bg-[var(--bg-raised)] p-3 font-mono text-sm break-all whitespace-pre-wrap">{createdToken}</pre>
</div>
```

with:

```svelte
<div class="relative">
    <pre
        class="rounded-panel bg-[var(--bg-raised)] p-3 font-mono text-sm break-all whitespace-pre-wrap">{createdToken}</pre>
    <div class="absolute top-2 right-2">
        <Button
            variant="ghost"
            size="sm"
            ariaLabel="Copy token"
            onclick={() => copyToken(createdToken!)}
        >
            <Copy size={14} />
        </Button>
    </div>
</div>
```

- [ ] **Step 5: Run tests to confirm they pass**

```bash
cd frontend && npm test -- profile.test.ts 2>&1 | tail -10
```

Expected: all tests pass.

- [ ] **Step 6: Commit**

```bash
git commit --only \
  frontend/src/routes/profile/+page.svelte \
  frontend/src/routes/profile/profile.test.ts \
  -m "feat(profile): add inline Copy icon button to token reveal modal"
```

---

## Task 8: Remove stale tests + final check

**Files:**

- Modify: `frontend/src/routes/profile/profile.test.ts`

The inline change-email section, inline change-password section, and Status column are gone. Tests referencing them must be removed.

- [ ] **Step 1: Identify stale tests to delete**

Remove the following tests (search by their `it(...)` description):

From `describe('Profile Route', ...)` — these no longer apply after the rewrite. The
describe block was already updated in Task 2; verify no lingering assertions reference
`data-ui="change-email-section"`, `data-ui="change-password-section"`, or the token
Status column.

From `describe('Button Migrations', ...)` — confirm no test still queries `data-ui="status-badge"` in a tokens context.

Run greps to find any remaining stale assertions:

```bash
grep -n 'change-email-section\|change-password-section\|data-ui="status-badge"' \
  frontend/src/routes/profile/profile.test.ts
```

```bash
grep -n 'confirmation link has been sent\|Other sessions have been signed out\|emailChangeSuccess\|passwordChangeSuccess' \
  frontend/src/routes/profile/profile.test.ts
```

Expected: no output from either command. The first catches stale `data-ui` attribute assertions
from old inline forms and the removed Status column. The second catches stale success-state text
assertions from the old inline `emailChangeSuccess` / `passwordChangeSuccess` display gates.
If any lines appear, delete them.

- [ ] **Step 2: Run the full test suite**

```bash
cd frontend && npm test -- profile.test.ts 2>&1 | tail -20
```

Expected: all tests pass, no failures.

- [ ] **Step 3: Run lint and type check**

```bash
cd frontend && npm run lint && npm run check
```

Expected: no errors.

- [ ] **Step 4: Commit**

```bash
git commit --only frontend/src/routes/profile/profile.test.ts \
  -m "test(profile): remove stale inline-form and Status column test assertions"
```

---

## Verification

After all tasks complete, run the full frontend quality gates:

```bash
cd frontend && npm run lint && npm run format:check && npm run check && npm run test && npm run build
```

Manual smoke-test checklist:

1. Navigate to `/profile` — Account tab renders by default; URL has no `?tab=` param.
2. Click "API Tokens" tab — URL becomes `?tab=api-tokens`; reload preserves tab.
3. Click "Account" tab — URL param removed.
4. (Password auth) Click "Change email" — modal opens; Cancel closes it.
5. (Password auth) With `has_pending_email_change: true` (mock or real) — "Change
   pending" badge visible; opening modal shows info Callout + "Cancel email change" button.
6. (Password auth) Click "Change" in Security card — Change Password modal opens; Cancel
   closes and clears fields.
7. Create a token — post-reveal shows warning Callout, pre block, inline copy icon button
   (`aria-label="Copy token"`), and footer "Copy" + "Done" buttons; both write to clipboard.
8. Revoke a token — it disappears from the list; if last active token, `EmptyState` with "No API tokens" renders.
9. (OIDC auth) Account tab — no "Change email" button; Security card shows SSO Callout, no "Change" button.
